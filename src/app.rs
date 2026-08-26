use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

use crate::camera::Camera;
use crate::creature::{mesh_for_snapshot, Creatures};
use crate::daynight::{sky_lighting, DAY_LENGTH_SECS};
use crate::input::Input;
use crate::llm::{LlmClient, PendingGeneration};
use crate::net::{
    self, decode, encode, LaunchConfig, Packet, PlayerId, ReliableChannel, ReliableMsg,
    UnreliableMsg, CONNECTION_TIMEOUT, HOST_PLAYER_ID, SNAPSHOT_INTERVAL,
};
use crate::player::Player;
use crate::raycast::raycast;
use crate::remote_player::{self, RemotePlayer};
use crate::save::{load_world, save_world};
use crate::scripting::{Module, ScriptHost, TICK_INTERVAL as LUA_TICK_INTERVAL};
use crate::ui::{Toast, Ui};
use crate::voxel::atlas::ATLAS_BYTES;
use crate::voxel::chunk::world_to_chunk;
use crate::voxel::mesher::{build_chunk_mesh, MeshData, Vertex};
use crate::voxel::{BlockType, World, CHUNK_X, CHUNK_Z};
use crate::weather::{Weather, WeatherState};

const REDSTONE_HEAL_RADIUS: f32 = 4.0;
const REDSTONE_HEAL_AMOUNT: f32 = 0.5;

const SHADOW_MAP_SIZE: u32 = 2048;
/// Half-width of the sun's orthographic frustum, in blocks. Big enough to
/// cover the visible area around the player without wasting shadow-map
/// resolution on terrain far outside the render distance.
const SHADOW_ORTHO_HALF_SIZE: f32 = 48.0;
/// How far back along the sun direction the "virtual light" sits.
const SHADOW_LIGHT_DISTANCE: f32 = 80.0;

const RENDER_RADIUS: i32 = 5;
const UNLOAD_RADIUS: i32 = RENDER_RADIUS + 2;
const MESH_BUDGET_PER_FRAME: usize = 8;
const REACH: f32 = 6.0;
const CREATURE_COUNT: usize = 9;

struct HostNet {
    socket: UdpSocket,
    clients: HashMap<SocketAddr, PlayerId>,
    remote_players: HashMap<PlayerId, RemotePlayer>,
    next_player_id: PlayerId,
    reliable: ReliableChannel,
    broadcast_timer: f32,
    port: u16,
}

struct ClientNet {
    socket: UdpSocket,
    server_addr: SocketAddr,
    player_id: PlayerId,
    reliable: ReliableChannel,
    remote_players: HashMap<PlayerId, RemotePlayer>,
    creature_snapshot: Vec<([f32; 3], u8)>,
    send_timer: f32,
    last_server_packet: Instant,
    lost_connection_logged: bool,
}

enum NetRole {
    Host(HostNet),
    Joined(ClientNet),
}

/// State machine for one in-flight LLM rule generation: request, and (on a
/// validation failure) exactly one automatic retry with the error fed back
/// to the model, matching the runtime rule pipeline's "validate" step.
enum GenerationState {
    Idle,
    Waiting {
        user_request: String,
        pending: PendingGeneration,
        is_retry: bool,
    },
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    fog_color: [f32; 4],
    sun_dir: [f32; 4],
    light_params: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LightUniform {
    view_proj: [[f32; 4]; 4],
}

/// Places a "virtual light" back along the sun direction from the player
/// and looks at them with an orthographic frustum -- simple directional
/// shadows anchored to wherever the player currently is, rather than
/// covering the whole (effectively infinite) world.
fn light_view_proj(sun_dir: Vec3, player_pos: Vec3) -> glam::Mat4 {
    let light_pos = player_pos + sun_dir * SHADOW_LIGHT_DISTANCE;
    let view = glam::Mat4::look_at_rh(light_pos, player_pos, Vec3::Y);
    let h = SHADOW_ORTHO_HALF_SIZE;
    let proj = glam::Mat4::orthographic_rh(-h, h, -h, h, 1.0, SHADOW_LIGHT_DISTANCE * 2.5);
    proj * view
}

struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

fn upload_mesh(device: &wgpu::Device, mesh: &MeshData) -> Option<GpuMesh> {
    if mesh.indices.is_empty() {
        return None;
    }
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh vbuf"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh ibuf"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Some(GpuMesh {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
    })
}

pub struct App {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,

    render_pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,

    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,

    shadow_pipeline: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    shadow_light_buffer: wgpu::Buffer,
    shadow_light_bind_group: wgpu::BindGroup,
    shadow_sample_bind_group: wgpu::BindGroup,

    camera: Camera,
    player: Player,
    world: World,
    input: Input,
    creatures: Creatures,
    time_of_day: f32,
    weather: WeatherState,
    net: NetRole,
    local_player_id: PlayerId,
    scripting: ScriptHost,
    lua_tick_timer: f32,
    llm: LlmClient,
    generation: GenerationState,
    next_rule_id: u32,
    last_generated_index: Option<usize>,

    ui: Ui,
    console_open: bool,
    prompt_input: String,
    toasts: Vec<Toast>,
    pending_egui_output: Option<egui::FullOutput>,

    chunk_meshes: HashMap<(i32, i32), GpuMesh>,
    entity_mesh: Option<GpuMesh>,
    selected_block: BlockType,
    cursor_grabbed: bool,

    last_frame: Instant,
    title_timer: f32,
    frame_count: u32,
    last_fps: f32,
}

impl App {
    pub async fn new(window: Arc<Window>, launch: LaunchConfig) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("failed to create surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("failed to find a suitable GPU adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .expect("failed to create device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_view = create_depth_view(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera buffer"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let (texture_bgl, texture_bind_group) = create_atlas_bind_group(&device, &queue);
        let shadow = create_shadow_resources(&device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[&camera_bgl, &texture_bgl, &shadow.sample_bgl],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("main pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let (
            world,
            spawn_pos,
            yaw,
            pitch,
            time_of_day,
            net,
            local_player_id,
            spawn_creatures,
            scripting,
        ) = match launch.connect {
            None => {
                let (world, spawn_pos, yaw, pitch, time_of_day, scripting) = match load_world() {
                    Some(loaded) => (
                        loaded.world,
                        loaded.player_pos,
                        loaded.yaw,
                        loaded.pitch,
                        loaded.time_of_day,
                        ScriptHost::load_from_save(&loaded.modules),
                    ),
                    None => {
                        let seed = (Instant::now().elapsed().as_nanos() as u32) ^ 0x9E3779B9;
                        let world = World::new(seed);
                        let h = world.terrain_height(0, 0) as f32 + 2.0;
                        (
                            world,
                            Vec3::new(0.5, h, 0.5),
                            -90f32.to_radians(),
                            0.0,
                            0.28,
                            ScriptHost::scan_dir("modules"),
                        )
                    }
                };
                let socket = net::bind_nonblocking(&format!("0.0.0.0:{}", launch.port))
                    .unwrap_or_else(|e| {
                        eprintln!(
                            "Failed to bind UDP port {}: {e}. Pick a different --port.",
                            launch.port
                        );
                        std::process::exit(1);
                    });
                log::info!("Hosting on port {}", launch.port);
                let host_net = HostNet {
                    socket,
                    clients: HashMap::new(),
                    remote_players: HashMap::new(),
                    next_player_id: 1,
                    reliable: ReliableChannel::new(),
                    broadcast_timer: 0.0,
                    port: launch.port,
                };
                (
                    world,
                    spawn_pos,
                    yaw,
                    pitch,
                    time_of_day,
                    NetRole::Host(host_net),
                    HOST_PLAYER_ID,
                    true,
                    scripting,
                )
            }
            Some(server_addr) => {
                let (socket, player_id, world, spawn_pos, time_of_day, reliable) =
                    join_handshake(server_addr);
                let client_net = ClientNet {
                    socket,
                    server_addr,
                    player_id,
                    reliable,
                    remote_players: HashMap::new(),
                    creature_snapshot: Vec::new(),
                    send_timer: 0.0,
                    last_server_packet: Instant::now(),
                    lost_connection_logged: false,
                };
                (
                    world,
                    spawn_pos,
                    -90f32.to_radians(),
                    0.0,
                    time_of_day,
                    NetRole::Joined(client_net),
                    player_id,
                    false,
                    ScriptHost::new(),
                )
            }
        };

        let mut camera = Camera::new(spawn_pos, config.width as f32 / config.height as f32);
        camera.yaw = yaw;
        camera.pitch = pitch;
        let player = Player::new(spawn_pos);

        let mut creatures = Creatures::new();
        if spawn_creatures {
            creatures.spawn_around(&world, spawn_pos, CREATURE_COUNT, world.seed);
        }
        let weather = WeatherState::new(world.seed);

        let llm = LlmClient::new(launch.llm_url.clone());
        let ui = Ui::new(&device, config.format, &window);

        let mut app = Self {
            window,
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            depth_view,
            camera_buffer,
            camera_bind_group,
            texture_bind_group,
            shadow_pipeline: shadow.pipeline,
            shadow_view: shadow.view,
            shadow_light_buffer: shadow.light_buffer,
            shadow_light_bind_group: shadow.light_bind_group,
            shadow_sample_bind_group: shadow.sample_bind_group,
            camera,
            player,
            world,
            input: Input::new(),
            creatures,
            time_of_day,
            weather,
            net,
            local_player_id,
            scripting,
            lua_tick_timer: 0.0,
            llm,
            generation: GenerationState::Idle,
            next_rule_id: 0,
            last_generated_index: None,
            ui,
            console_open: false,
            prompt_input: String::new(),
            toasts: Vec::new(),
            pending_egui_output: None,
            chunk_meshes: HashMap::new(),
            entity_mesh: None,
            selected_block: BlockType::Grass,
            cursor_grabbed: false,
            last_frame: Instant::now(),
            title_timer: 0.0,
            frame_count: 0,
            last_fps: 0.0,
        };

        app.grab_cursor(true);
        app.update_chunks();
        app
    }

    fn grab_cursor(&mut self, grab: bool) {
        self.cursor_grabbed = grab;
        if grab {
            if self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok()
            {
                self.window.set_cursor_visible(false);
            }
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.window.set_cursor_visible(true);
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
        self.camera.aspect = new_size.width as f32 / new_size.height as f32;
    }

    pub fn window_event(&mut self, event: &WindowEvent) {
        // Always let egui see the event -- harmless when nothing is
        // focused, and it needs everything (including mouse position) to
        // hit-test the rules panel's buttons.
        self.ui.handle_event(&self.window, event);

        match event {
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let ElementState::Pressed = key_event.state else {
                    if let PhysicalKey::Code(code) = key_event.physical_key {
                        self.input.key_event(code, key_event.state);
                    }
                    return;
                };
                let PhysicalKey::Code(code) = key_event.physical_key else {
                    return;
                };
                if code == KeyCode::KeyT && !self.console_open {
                    self.open_console();
                    return;
                }
                if code == KeyCode::Escape && self.console_open {
                    self.close_console();
                    return;
                }
                if !self.console_open {
                    self.input.key_event(code, key_event.state);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.console_open {
                    self.input.mouse_button_event(*button, *state);
                    if *state == ElementState::Pressed
                        && *button == MouseButton::Left
                        && !self.cursor_grabbed
                    {
                        self.grab_cursor(true);
                    }
                }
            }
            _ => {}
        }
    }

    fn open_console(&mut self) {
        self.console_open = true;
        self.input.release_all();
        self.grab_cursor(false);
    }

    fn close_console(&mut self) {
        self.console_open = false;
        self.grab_cursor(true);
    }

    pub fn device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.cursor_grabbed {
                self.input.mouse_delta.0 += delta.0 as f32;
                self.input.mouse_delta.1 += delta.1 as f32;
            }
        }
    }

    pub fn update(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;

        if self.input.toggle_cursor {
            self.grab_cursor(!self.cursor_grabbed);
        }

        const SENSITIVITY: f32 = 0.0022;
        self.camera.yaw += self.input.mouse_delta.0 * SENSITIVITY;
        self.camera.pitch -= self.input.mouse_delta.1 * SENSITIVITY;
        self.camera.pitch = self
            .camera
            .pitch
            .clamp(-89f32.to_radians(), 89f32.to_radians());

        let forward = self.camera.forward();
        let right = self.camera.right();
        self.player
            .update(&self.world, &self.input, forward, right, dt);
        self.camera.position = self.player.position;

        if let Some(i) = self.input.hotbar_select {
            if let Some(b) = BlockType::from_hotbar_index(i) {
                self.selected_block = b;
            }
        }

        if !self.console_open && (self.input.left_clicked || self.input.right_clicked) {
            let origin = self.camera.eye_position();
            let dir = self.camera.forward();
            if let Some(hit) = raycast(&self.world, origin, dir, REACH) {
                if self.input.left_clicked {
                    let broken = self
                        .world
                        .get_block(hit.target.0, hit.target.1, hit.target.2);
                    if broken == BlockType::Crystal {
                        self.player.carrying_crystal = true;
                    }
                    self.apply_block_edit(hit.target.0, hit.target.1, hit.target.2, BlockType::Air);
                } else if !self.would_hit_player(hit.place) {
                    self.apply_block_edit(
                        hit.place.0,
                        hit.place.1,
                        hit.place.2,
                        self.selected_block,
                    );
                }
            }
        }

        if self.input.save_requested {
            if matches!(self.net, NetRole::Host(_)) {
                save_world(
                    &self.world,
                    &self.player,
                    &self.camera,
                    self.time_of_day,
                    self.scripting.save_entries(),
                );
            } else {
                log::warn!("Only the host can save the world.");
            }
        }

        self.input.end_frame();
        self.update_chunks();
        self.poll_network(dt);
        self.poll_generation();

        if matches!(self.net, NetRole::Host(_)) {
            self.time_of_day = (self.time_of_day + dt / DAY_LENGTH_SECS).rem_euclid(1.0);
            self.weather.update(dt);
            self.creatures.update(&self.world, dt);

            self.lua_tick_timer += dt;
            if self.lua_tick_timer >= LUA_TICK_INTERVAL {
                self.lua_tick_timer = 0.0;
                let players = self.host_player_positions();
                let block_edits = self.scripting.run_tick(
                    &self.world,
                    &mut self.creatures,
                    &players,
                    self.time_of_day,
                    &mut self.weather,
                );
                for (x, y, z, block) in block_edits {
                    self.apply_block_edit(x, y, z, block);
                }
                for &(x, y, z) in self.world.redstone_positions.iter() {
                    let center = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    self.creatures
                        .heal_near(center, REDSTONE_HEAL_RADIUS, REDSTONE_HEAL_AMOUNT);
                }
            }
        }

        let mut mesh = match &self.net {
            NetRole::Host(_) => self.creatures.build_mesh(),
            NetRole::Joined(client) => mesh_for_snapshot(&client.creature_snapshot),
        };
        let remote_players = match &self.net {
            NetRole::Host(host) => &host.remote_players,
            NetRole::Joined(client) => &client.remote_players,
        };
        mesh.extend(remote_player::build_mesh(
            remote_players,
            self.local_player_id,
        ));
        self.entity_mesh = upload_mesh(&self.device, &mesh);

        self.toasts.retain(|t| !t.is_expired());
        let is_host = matches!(self.net, NetRole::Host(_));
        let generation_status = match &self.generation {
            GenerationState::Waiting { is_retry: true, .. } => {
                Some("Retrying with error feedback...")
            }
            GenerationState::Waiting {
                is_retry: false, ..
            } => Some("Generating..."),
            GenerationState::Idle => None,
        };
        let (full_output, requests) = self.ui.draw(
            &self.window,
            self.console_open,
            &mut self.prompt_input,
            is_host,
            &self.scripting,
            self.last_generated_index,
            generation_status,
            &self.toasts,
            self.last_fps,
        );
        self.pending_egui_output = Some(full_output);

        if let Some(idx) = requests.toggle_index {
            if let Some((name, enabled)) = self.scripting.toggle_at(idx) {
                self.notify_all(format!(
                    "Rule '{name}' {}",
                    if enabled { "activated" } else { "deactivated" }
                ));
            }
        }
        if let Some(idx) = requests.delete_index {
            if let Some(name) = self.scripting.remove(idx) {
                if self.last_generated_index == Some(idx) {
                    self.last_generated_index = None;
                }
                self.notify_all(format!("Rule '{name}' deleted"));
            }
        }
        if let Some(prompt) = requests.submit_prompt {
            if matches!(self.generation, GenerationState::Idle) {
                self.prompt_input.clear();
                self.start_generation(prompt);
            }
        }

        self.title_timer += dt;
        self.frame_count += 1;
        if self.title_timer >= 0.5 {
            let fps = self.frame_count as f32 / self.title_timer;
            self.last_fps = fps;
            let hour = (self.time_of_day * 24.0) as u32;
            let minute = ((self.time_of_day * 24.0 - hour as f32) * 60.0) as u32;
            let role_info = match &self.net {
                NetRole::Host(host) => {
                    format!("Hosting :{} ({} joined)", host.port, host.clients.len())
                }
                NetRole::Joined(client) => format!(
                    "Connected to {} as P{}",
                    client.server_addr, client.player_id
                ),
            };
            self.window.set_title(&format!(
                "Voxel Project | {:02}:{:02} | {} | Block: {} | FPS: {:.0} | T rules/generate, F5 save, Esc mouse",
                hour,
                minute,
                role_info,
                self.selected_block.name(),
                fps
            ));
            self.title_timer = 0.0;
            self.frame_count = 0;
        }
    }

    fn notify_all(&mut self, text: String) {
        self.toasts.push(Toast::new(text.clone()));
        if let NetRole::Host(host) = &mut self.net {
            let addrs: Vec<SocketAddr> = host.clients.keys().copied().collect();
            for addr in addrs {
                host.reliable
                    .send(&host.socket, addr, ReliableMsg::Notify(text.clone()));
            }
        }
    }

    /// All known player positions, host included. Used both for the Lua
    /// World API and (indirectly) mirrors what the network snapshot sends,
    /// so `api.players()` matches what other players actually see.
    fn host_player_positions(&self) -> Vec<(PlayerId, Vec3, bool)> {
        let NetRole::Host(host) = &self.net else {
            return Vec::new();
        };
        let mut players = vec![(
            HOST_PLAYER_ID,
            self.player.position,
            self.player.carrying_crystal,
        )];
        for (&id, rp) in host.remote_players.iter() {
            players.push((id, rp.pos, rp.carrying_crystal));
        }
        players
    }

    /// Runtime rule pipeline step 1-2: the host submits a prompt (typed into
    /// the in-game console), and it goes to the LLM along with the World
    /// API doc and an example module.
    fn start_generation(&mut self, user_request: String) {
        log::info!("Generating a rule from: {user_request}");
        self.notify_all(format!("Host is generating a rule: \"{user_request}\""));
        let pending = self.llm.generate(&user_request);
        self.generation = GenerationState::Waiting {
            user_request,
            pending,
            is_retry: false,
        };
    }

    /// Runtime rule pipeline steps 3-4: receives the generated Lua source
    /// and validates it (syntax + `on_tick` contract, via `Module::load`).
    /// On failure, gives the model one chance to fix it before giving up --
    /// still step 4, just with the error fed back instead of a silent drop.
    fn poll_generation(&mut self) {
        let GenerationState::Waiting { pending, .. } = &self.generation else {
            return;
        };
        let Some(result) = pending.poll() else {
            return;
        };
        let GenerationState::Waiting {
            user_request,
            is_retry,
            ..
        } = std::mem::replace(&mut self.generation, GenerationState::Idle)
        else {
            unreachable!()
        };

        let code = match result {
            Ok(code) => code,
            Err(e) => {
                log::error!("Rule generation failed: {e}");
                return;
            }
        };

        let name = format!("rule_{}", self.next_rule_id);
        match Module::load(name.clone(), user_request.clone(), code.clone()) {
            Ok(module) => {
                self.next_rule_id += 1;
                let idx = self.scripting.add_generated(module);
                self.last_generated_index = Some(idx);
                log::info!(
                    "Generated rule module '{name}' from prompt (open the Rules panel to activate)"
                );
                self.notify_all(format!("New rule generated: '{name}' (not yet active)"));
            }
            Err(err) if !is_retry => {
                log::warn!("Generated rule failed validation, asking the model to fix it: {err}");
                let pending = self.llm.retry(&user_request, &code, &err);
                self.generation = GenerationState::Waiting {
                    user_request,
                    pending,
                    is_retry: true,
                };
            }
            Err(err) => {
                log::error!("Generated rule failed validation twice, giving up: {err}");
                self.toasts
                    .push(Toast::new(format!("Rule generation failed: {err}")));
            }
        }
    }

    fn apply_block_edit(&mut self, x: i32, y: i32, z: i32, block: BlockType) {
        self.world.set_block(x, y, z, block);
        match &mut self.net {
            NetRole::Host(host) => {
                let addrs: Vec<SocketAddr> = host.clients.keys().copied().collect();
                for addr in addrs {
                    host.reliable.send(
                        &host.socket,
                        addr,
                        ReliableMsg::BlockEdit { x, y, z, block },
                    );
                }
            }
            NetRole::Joined(client) => {
                client.reliable.send(
                    &client.socket,
                    client.server_addr,
                    ReliableMsg::BlockEdit { x, y, z, block },
                );
            }
        }
    }

    fn poll_network(&mut self, dt: f32) {
        match &mut self.net {
            NetRole::Host(_) => self.poll_host_network(dt),
            NetRole::Joined(_) => self.poll_client_network(dt),
        }
    }

    fn poll_host_network(&mut self, dt: f32) {
        let mut buf = [0u8; 8192];
        loop {
            let (n, from) = {
                let NetRole::Host(host) = &self.net else {
                    unreachable!()
                };
                match host.socket.recv_from(&mut buf) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            };
            let Some(packet) = decode(&buf[..n]) else {
                continue;
            };
            self.handle_host_packet(packet, from);
        }

        let NetRole::Host(host) = &mut self.net else {
            unreachable!()
        };
        host.reliable.resend_due(&host.socket);

        // Drop clients we haven't heard from in a while.
        let stale: Vec<PlayerId> = host
            .remote_players
            .iter()
            .filter(|(_, rp)| rp.last_seen.elapsed() > CONNECTION_TIMEOUT)
            .map(|(&id, _)| id)
            .collect();
        for id in stale {
            host.remote_players.remove(&id);
            host.clients.retain(|_, pid| *pid != id);
            log::info!("Player {id} timed out.");
        }

        host.broadcast_timer += dt;
        if host.broadcast_timer >= SNAPSHOT_INTERVAL {
            host.broadcast_timer = 0.0;
            let mut players: Vec<(PlayerId, [f32; 3], f32, bool)> = vec![(
                HOST_PLAYER_ID,
                self.player.position.to_array(),
                self.camera.yaw,
                self.player.carrying_crystal,
            )];
            for (&id, rp) in host.remote_players.iter() {
                players.push((id, rp.pos.to_array(), rp.yaw, rp.carrying_crystal));
            }
            let snapshot = UnreliableMsg::Snapshot {
                time_of_day: self.time_of_day,
                weather: self.weather.current.to_u8(),
                players,
                creatures: self.creatures.snapshot(),
            };
            let bytes = encode(&Packet::Unreliable(snapshot));
            for addr in host.clients.keys() {
                let _ = host.socket.send_to(&bytes, *addr);
            }
        }
    }

    fn handle_host_packet(&mut self, packet: Packet, from: SocketAddr) {
        let NetRole::Host(host) = &mut self.net else {
            unreachable!()
        };
        match packet {
            Packet::Reliable { id, msg } => {
                ReliableChannel::ack_reply(&host.socket, from, id);
                if !host.reliable.mark_seen(from, id) {
                    return;
                }
                match msg {
                    ReliableMsg::Hello => {
                        if host.clients.contains_key(&from) {
                            return;
                        }
                        let player_id = host.next_player_id;
                        host.next_player_id += 1;
                        let spawn =
                            self.player.position + Vec3::new((player_id as f32) * 2.0, 0.0, 2.0);
                        host.clients.insert(from, player_id);
                        host.remote_players.insert(
                            player_id,
                            RemotePlayer {
                                pos: spawn,
                                yaw: 0.0,
                                carrying_crystal: false,
                                last_seen: Instant::now(),
                            },
                        );
                        host.reliable.send(
                            &host.socket,
                            from,
                            ReliableMsg::Welcome {
                                player_id,
                                seed: self.world.seed,
                                time_of_day: self.time_of_day,
                                spawn: spawn.to_array(),
                                edits: self.world.edits.iter().map(|(k, v)| (*k, *v)).collect(),
                            },
                        );
                        log::info!("Player {player_id} joined from {from}");
                        self.notify_all(format!("Player {player_id} joined"));
                    }
                    ReliableMsg::BlockEdit { x, y, z, block } => {
                        let addrs: Vec<SocketAddr> = host.clients.keys().copied().collect();
                        self.world.set_block(x, y, z, block);
                        let NetRole::Host(host) = &mut self.net else {
                            unreachable!()
                        };
                        for addr in addrs {
                            if addr == from {
                                continue;
                            }
                            host.reliable.send(
                                &host.socket,
                                addr,
                                ReliableMsg::BlockEdit { x, y, z, block },
                            );
                        }
                    }
                    _ => {}
                }
            }
            Packet::Ack { id } => host.reliable.ack(id),
            Packet::Unreliable(UnreliableMsg::PlayerState {
                pos,
                yaw,
                carrying_crystal,
            }) => {
                if let Some(&player_id) = host.clients.get(&from) {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.pos = Vec3::from_array(pos);
                        rp.yaw = yaw;
                        rp.carrying_crystal = carrying_crystal;
                        rp.last_seen = Instant::now();
                    }
                }
            }
            Packet::Unreliable(_) => {}
        }
    }

    fn poll_client_network(&mut self, dt: f32) {
        let mut buf = [0u8; 8192];
        loop {
            let (n, from) = {
                let NetRole::Joined(client) = &self.net else {
                    unreachable!()
                };
                match client.socket.recv_from(&mut buf) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            };
            let NetRole::Joined(client) = &self.net else {
                unreachable!()
            };
            if from != client.server_addr {
                continue;
            }
            let Some(packet) = decode(&buf[..n]) else {
                continue;
            };
            self.handle_client_packet(packet);
        }

        let NetRole::Joined(client) = &mut self.net else {
            unreachable!()
        };
        client.reliable.resend_due(&client.socket);
        if !client.lost_connection_logged
            && client.last_server_packet.elapsed() > CONNECTION_TIMEOUT
        {
            log::warn!("Lost connection to host.");
            client.lost_connection_logged = true;
        }

        client.send_timer += dt;
        if client.send_timer >= SNAPSHOT_INTERVAL {
            client.send_timer = 0.0;
            let msg = UnreliableMsg::PlayerState {
                pos: self.player.position.to_array(),
                yaw: self.camera.yaw,
                carrying_crystal: self.player.carrying_crystal,
            };
            let bytes = encode(&Packet::Unreliable(msg));
            let _ = client.socket.send_to(&bytes, client.server_addr);
        }
    }

    fn handle_client_packet(&mut self, packet: Packet) {
        let NetRole::Joined(client) = &mut self.net else {
            unreachable!()
        };
        client.last_server_packet = Instant::now();
        client.lost_connection_logged = false;
        match packet {
            Packet::Reliable { id, msg } => {
                ReliableChannel::ack_reply(&client.socket, client.server_addr, id);
                if !client.reliable.mark_seen(client.server_addr, id) {
                    return;
                }
                match msg {
                    ReliableMsg::BlockEdit { x, y, z, block } => {
                        self.world.set_block(x, y, z, block);
                    }
                    ReliableMsg::PlayerLeft { player_id } => {
                        let NetRole::Joined(client) = &mut self.net else {
                            unreachable!()
                        };
                        client.remote_players.remove(&player_id);
                    }
                    ReliableMsg::Notify(text) => {
                        self.toasts.push(Toast::new(text));
                    }
                    _ => {}
                }
            }
            Packet::Ack { id } => client.reliable.ack(id),
            Packet::Unreliable(UnreliableMsg::Snapshot {
                time_of_day,
                weather,
                players,
                creatures,
            }) => {
                self.time_of_day = time_of_day;
                self.weather.current = Weather::from_u8(weather);
                let NetRole::Joined(client) = &mut self.net else {
                    unreachable!()
                };
                client.creature_snapshot = creatures;
                let now = Instant::now();
                for (id, pos, yaw, carrying_crystal) in players {
                    client
                        .remote_players
                        .entry(id)
                        .and_modify(|rp| {
                            rp.pos = Vec3::from_array(pos);
                            rp.yaw = yaw;
                            rp.carrying_crystal = carrying_crystal;
                            rp.last_seen = now;
                        })
                        .or_insert(RemotePlayer {
                            pos: Vec3::from_array(pos),
                            yaw,
                            carrying_crystal,
                            last_seen: now,
                        });
                }
            }
            Packet::Unreliable(_) => {}
        }
    }

    fn would_hit_player(&self, block: (i32, i32, i32)) -> bool {
        let p = self.player.position;
        let bx = block.0 as f32;
        let by = block.1 as f32;
        let bz = block.2 as f32;
        let overlaps_x = p.x + 0.3 > bx && p.x - 0.3 < bx + 1.0;
        let overlaps_z = p.z + 0.3 > bz && p.z - 0.3 < bz + 1.0;
        let overlaps_y = p.y + 1.8 > by && p.y < by + 1.0;
        overlaps_x && overlaps_y && overlaps_z
    }

    fn update_chunks(&mut self) {
        let pcx = (self.player.position.x.floor() as i32).div_euclid(CHUNK_X);
        let pcz = (self.player.position.z.floor() as i32).div_euclid(CHUNK_Z);

        for cx in (pcx - RENDER_RADIUS)..=(pcx + RENDER_RADIUS) {
            for cz in (pcz - RENDER_RADIUS)..=(pcz + RENDER_RADIUS) {
                self.world.ensure_chunk_loaded(cx, cz);
            }
        }

        let mut rebuilt = 0usize;
        let dirty_keys: Vec<(i32, i32)> = self
            .world
            .chunks
            .iter()
            .filter(|(_, c)| c.dirty)
            .map(|(k, _)| *k)
            .take(MESH_BUDGET_PER_FRAME)
            .collect();

        for key in dirty_keys {
            let mesh_data = {
                let chunk = self.world.chunks.get(&key).unwrap();
                build_chunk_mesh(&self.world, chunk)
            };
            match upload_mesh(&self.device, &mesh_data) {
                Some(gpu_mesh) => {
                    self.chunk_meshes.insert(key, gpu_mesh);
                }
                None => {
                    self.chunk_meshes.remove(&key);
                }
            }
            if let Some(chunk) = self.world.chunks.get_mut(&key) {
                chunk.dirty = false;
            }
            rebuilt += 1;
        }
        let _ = rebuilt;

        let unload: Vec<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .filter(|(cx, cz)| (cx - pcx).abs() > UNLOAD_RADIUS || (cz - pcz).abs() > UNLOAD_RADIUS)
            .copied()
            .collect();
        for key in unload {
            self.world.unload_chunk(key.0, key.1);
            self.chunk_meshes.remove(&key);
        }
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let lighting = sky_lighting(self.time_of_day);
        let sky = lighting.sky_color;
        let light_view_proj = light_view_proj(lighting.sun_dir, self.player.position);
        let uniform = CameraUniform {
            view_proj: self.camera.view_proj().to_cols_array_2d(),
            light_view_proj: light_view_proj.to_cols_array_2d(),
            camera_pos: [
                self.camera.eye_position().x,
                self.camera.eye_position().y,
                self.camera.eye_position().z,
                1.0,
            ],
            fog_color: [sky[0], sky[1], sky[2], 1.0],
            sun_dir: [
                lighting.sun_dir.x,
                lighting.sun_dir.y,
                lighting.sun_dir.z,
                0.0,
            ],
            light_params: [lighting.ambient, lighting.sun_intensity, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        self.queue.write_buffer(
            &self.shadow_light_buffer,
            0,
            bytemuck::bytes_of(&LightUniform { view_proj: light_view_proj.to_cols_array_2d() }),
        );

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

        // Skip the shadow pass entirely when the sun is below the horizon
        // -- the main shader already zeroes sun_intensity then, so shadows
        // wouldn't be visible anyway.
        if lighting.sun_intensity > 0.0 {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            shadow_pass.set_pipeline(&self.shadow_pipeline);
            shadow_pass.set_bind_group(0, &self.shadow_light_bind_group, &[]);
            for mesh in self.chunk_meshes.values() {
                shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            if let Some(mesh) = &self.entity_mesh {
                shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: sky[0] as f64,
                            g: sky[1] as f64,
                            b: sky[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            rpass.set_pipeline(&self.render_pipeline);
            rpass.set_bind_group(0, &self.camera_bind_group, &[]);
            rpass.set_bind_group(1, &self.texture_bind_group, &[]);
            rpass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
            for mesh in self.chunk_meshes.values() {
                rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            if let Some(mesh) = &self.entity_mesh {
                rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }

        if let Some(full_output) = self.pending_egui_output.take() {
            self.ui.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &view,
                &self.window,
                full_output,
                [self.config.width, self.config.height],
            );
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }

    pub fn save(&self) {
        if matches!(self.net, NetRole::Host(_)) {
            save_world(
                &self.world,
                &self.player,
                &self.camera,
                self.time_of_day,
                self.scripting.save_entries(),
            );
        }
    }
}

/// Blocks (with a resend-until-acked handshake) until the host answers with
/// a `Welcome`, or gives up and exits the process after a timeout.
fn join_handshake(
    server_addr: SocketAddr,
) -> (UdpSocket, PlayerId, World, Vec3, f32, ReliableChannel) {
    let socket = net::bind_nonblocking("0.0.0.0:0").unwrap_or_else(|e| {
        eprintln!("Failed to open a local UDP socket: {e}");
        std::process::exit(1);
    });

    let mut reliable = ReliableChannel::new();
    let hello_id = reliable.send(&socket, server_addr, ReliableMsg::Hello);
    log::info!("Connecting to {server_addr}...");

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut buf = [0u8; 8192];
    loop {
        if Instant::now() > deadline {
            eprintln!(
                "Failed to connect to {server_addr}: no response from host. \
                 Is it running, and is the address/port correct?"
            );
            std::process::exit(1);
        }
        reliable.resend_due(&socket);
        match socket.recv_from(&mut buf) {
            Ok((n, from)) if from == server_addr => {
                if let Some(Packet::Reliable {
                    id,
                    msg:
                        ReliableMsg::Welcome {
                            player_id,
                            seed,
                            time_of_day,
                            spawn,
                            edits,
                        },
                }) = decode(&buf[..n])
                {
                    ReliableChannel::ack_reply(&socket, server_addr, id);
                    reliable.forget(hello_id);
                    let mut world = World::new(seed);
                    for (pos, block) in edits {
                        world.edits.insert(pos, block);
                    }
                    world.rebuild_redstone_positions();
                    log::info!("Connected to {server_addr} as player {player_id}");
                    return (
                        socket,
                        player_id,
                        world,
                        Vec3::from_array(spawn),
                        time_of_day,
                        reliable,
                    );
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                eprintln!("Network error while connecting: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// Decodes the embedded texture atlas (see `voxel::atlas`) and uploads it,
/// returning the bind group layout (needed once, for the pipeline) and the
/// bind group itself (bound every frame). Nearest filtering keeps the
/// pixel-art look sharp instead of blurring it like a photo texture would.
fn create_atlas_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let image = image::load_from_memory(ATLAS_BYTES)
        .expect("embedded atlas.png should decode")
        .to_rgba8();
    let (width, height) = image.dimensions();
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("atlas sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("atlas bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atlas bind group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    (layout, bind_group)
}

struct ShadowResources {
    pipeline: wgpu::RenderPipeline,
    view: wgpu::TextureView,
    light_buffer: wgpu::Buffer,
    light_bind_group: wgpu::BindGroup,
    /// Bind group layout for sampling the shadow map from the main
    /// pipeline (its group 2) -- exposed so `App::new` can include it when
    /// building that pipeline's layout.
    sample_bgl: wgpu::BindGroupLayout,
    sample_bind_group: wgpu::BindGroup,
}

/// Sets up everything needed for a simple single-cascade directional
/// shadow map: a depth texture rendered from the sun's point of view each
/// frame, and the resources the main pass needs to sample it back.
fn create_shadow_resources(device: &wgpu::Device) -> ShadowResources {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow map"),
        size: wgpu::Extent3d { width: SHADOW_MAP_SIZE, height: SHADOW_MAP_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shadow light buffer"),
        size: std::mem::size_of::<LightUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow light bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow light bind group"),
        layout: &light_bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: light_buffer.as_entire_binding() }],
    });

    let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
    });
    let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow pipeline layout"),
        bind_group_layouts: &[&light_bgl],
        push_constant_ranges: &[],
    });
    // Only position is read; reusing the main Vertex buffer's stride means
    // no separate shadow-only mesh data is needed.
    let position_only_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 }],
    };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow pipeline"),
        layout: Some(&shadow_pipeline_layout),
        vertex: wgpu::VertexState { module: &shadow_shader, entry_point: "vs_main", buffers: &[position_only_layout] },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Cw,
            // No culling: the mesher only emits *exposed* faces (no
            // interior/underside geometry), so unlike a closed mesh there's
            // often no "back face" at a similar depth to fall back on --
            // culling either side can leave gaps with nothing to write
            // shadow depth. Acne is handled by the shader-side bias instead.
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let sample_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow sample bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    });
    let sample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow sample bind group"),
        layout: &sample_bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
        ],
    });

    ShadowResources { pipeline, view, light_buffer, light_bind_group, sample_bgl, sample_bind_group }
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth texture"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[allow(dead_code)]
fn chunk_of(pos: Vec3) -> (i32, i32) {
    world_to_chunk(pos.x.floor() as i32, pos.z.floor() as i32)
}
