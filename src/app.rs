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
use crate::llm::{classify_prompt, derive_rule_name, LlmClient, PendingGeneration, PromptKind};
use crate::net::{
    self, decode, encode, LaunchConfig, NotifyKind, Packet, PlayerId, ReliableChannel,
    ReliableMsg, SnapshotPlayer, UnreliableMsg, CONNECTION_TIMEOUT, HOST_PLAYER_ID,
    MAX_NICKNAME_LEN, SNAPSHOT_INTERVAL,
};
use crate::player::{Player, MAX_ATTRIBUTE_MULTIPLIER, MAX_HEALTH, MIN_ATTRIBUTE_MULTIPLIER};
use crate::raycast::raycast;
use crate::remote_player::{self, RemotePlayer};
use crate::save::{load_world, save_world};
use crate::scripting::{
    BlockBreakEvent, Module, PlayerEffect, PlayerSnapshot, ScriptHost, TickOutcome,
    TICK_INTERVAL as LUA_TICK_INTERVAL,
};
use crate::ui::{
    ChatEntry, Toast, Ui, CHAT_LOG_CAPACITY, CHAT_MESSAGE_COLOR, IMPORTANT_TOAST_COLOR,
};
use crate::voxel::atlas::ATLAS_BYTES;
use crate::voxel::chunk::world_to_chunk;
use crate::voxel::mesher::{build_chunk_mesh, MeshData, Vertex};
use crate::voxel::{BlockType, World, CHUNK_X, CHUNK_Z};
use crate::weather::{Weather, WeatherState};
use crate::world_api_validate;

const REDSTONE_HEAL_RADIUS: f32 = 4.0;
const REDSTONE_HEAL_AMOUNT: f32 = 0.5;
/// How often a poisoned player loses health, and by how much -- see
/// world_api/schema.yaml's `poison_tick_secs`/`api.set_poisoned`. Pure
/// engine state, not driven by Lua at all; a module only ever flips
/// `poisoned` on or off via `api.set_poisoned`.
const POISON_TICK_INTERVAL: f32 = 10.0;
const POISON_DAMAGE_PER_TICK: f32 = 1.0;
/// Horizontal speed above which a remote player (no real sprint flag over
/// the network) is approximated as "running" for `api.players()`. Above the
/// 4.5 walk speed but comfortably below the 7.5 sprint speed in `player.rs`.
const REMOTE_SPRINT_THRESHOLD: f32 = 6.0;

fn horizontal_speed(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
}

/// A seed for a brand-new world, different across (almost) every launch --
/// terrain, creature placement, and weather all derive from `World::seed`
/// (see `App::new`'s `world.seed`-seeded `Creatures::spawn_around`/
/// `WeatherState::new`/`build_rain_particles`), so this one value is what
/// actually varies "New World" from run to run.
///
/// Previously this was `Instant::now().elapsed().as_nanos()` -- but calling
/// `.elapsed()` immediately on an `Instant` just created measures the tiny,
/// near-constant time the two calls take back-to-back, not wall-clock time,
/// so real launches produced almost the same seed every time (hence every
/// world "looking the same"). Wall-clock time since the Unix epoch actually
/// varies between launches; XORed with the process id so two launches
/// starting in the same nanosecond-mod-2^32 window (unlikely, but the low
/// 32 bits of a nanosecond counter wrap every ~4.3s) still diverge.
///
/// Nothing about spawn position, starter modules, starting time-of-day, or
/// creature count depends on the seed (see the call site) -- only terrain/
/// creature-scatter/weather *appearance* varies, so the first-minutes
/// experience stays the same rule set and pacing every time, just on
/// differently laid-out terrain.
fn random_world_seed() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0);
    nanos ^ std::process::id() ^ 0x9E3779B9
}

/// The user-facing word for a `PromptKind`, used in generation status
/// messages/logs so a spell request doesn't get called a "rule" while it's
/// still in flight (its actual kind, `Module::is_instant`, isn't known
/// until the model's output is loaded).
fn generation_noun(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::Rule => "rule",
        PromptKind::Instant => "spell",
    }
}

fn is_in_water(world: &World, pos: Vec3) -> bool {
    world.get_block(pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32)
        == BlockType::Water
}

/// Mirrors `Player::update`'s own `below` check, since remote players don't
/// run local physics on the host.
fn approximate_on_ground(world: &World, pos: Vec3) -> bool {
    world.is_solid(
        pos.x.floor() as i32,
        (pos.y - 0.05).floor() as i32,
        pos.z.floor() as i32,
    )
}

/// Trims, caps the length, and falls back to a generic name for an empty
/// or all-whitespace nickname. Applied to our own local nickname at launch
/// and, since it arrives over the network, defensively again wherever the
/// host stores one it received from a client.
fn sanitize_nickname(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "Player".to_string()
    } else {
        trimmed.chars().take(MAX_NICKNAME_LEN).collect()
    }
}

const SHADOW_MAP_SIZE: u32 = 2048;
/// Half-width of the sun's orthographic frustum, in blocks. Big enough to
/// cover the visible area around the player without wasting shadow-map
/// resolution on terrain far outside the render distance.
const SHADOW_ORTHO_HALF_SIZE: f32 = 48.0;
/// How far back along the sun direction the "virtual light" sits.
const SHADOW_LIGHT_DISTANCE: f32 = 80.0;

/// Number of falling streaks drawn around the camera while it's raining.
const RAIN_PARTICLE_COUNT: usize = 1400;
/// Half-width of the square column of streaks centered on the camera --
/// kept fairly tight so the streaks read as dense rain up close rather than
/// a handful of sparse lines scattered across the whole view.
const RAIN_RADIUS: f32 = 16.0;
/// Vertical extent of the streak volume -- a streak that falls past the
/// bottom wraps back to the top, so this is also the wrap period.
const RAIN_HEIGHT: f32 = 20.0;
const RAIN_FALL_SPEED: f32 = 20.0;
const RAIN_STREAK_LENGTH: f32 = 0.9;
const RAIN_ALPHA: f32 = 0.55;

fn next_rand(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 24) as f32
}

/// Fixed per-particle (x, z, phase) offsets, generated once and reused every
/// frame -- only the fall phase advances (via `App::water_time`), so the
/// particle set doesn't need regenerating or storing per-particle state.
fn build_rain_particles(seed: u32) -> Vec<(f32, f32, f32)> {
    let mut state = (seed as u64) | 1;
    (0..RAIN_PARTICLE_COUNT)
        .map(|_| {
            let ox = (next_rand(&mut state) * 2.0 - 1.0) * RAIN_RADIUS;
            let oz = (next_rand(&mut state) * 2.0 - 1.0) * RAIN_RADIUS;
            let phase = next_rand(&mut state) * RAIN_HEIGHT;
            (ox, oz, phase)
        })
        .collect()
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RainVertex {
    position: [f32; 3],
    alpha: f32,
}

impl RainVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem::size_of;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<RainVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

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
    creature_snapshot: Vec<([f32; 3], u8, f32)>,
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
        kind: PromptKind,
        pending: PendingGeneration,
        is_retry: bool,
    },
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    /// Inverse of `view_proj`, used only by sky.wgsl to unproject a screen
    /// pixel back into a world-space view ray (the sky has no real geometry
    /// to derive one from otherwise). Computed CPU-side once per frame --
    /// cheap, and far simpler than reconstructing it in the shader.
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    fog_color: [f32; 4],
    zenith_color: [f32; 4],
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

    /// Draws the gradient sky/sun/moon/stars background -- see sky.wgsl.
    /// Issued first in the main pass, no depth test, so every other draw
    /// call simply paints over it.
    sky_pipeline: wgpu::RenderPipeline,

    rain_pipeline: wgpu::RenderPipeline,
    rain_vertex_buffer: wgpu::Buffer,
    /// Fixed (x, z, phase) offsets for each rain streak, relative to the
    /// camera -- see `build_rain_particles`.
    rain_particles: Vec<(f32, f32, f32)>,

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
    /// Block breaks (local or network-relayed from joined clients) collected
    /// since the last Lua tick, fed into `on_block_break` and cleared after.
    pending_block_breaks: Vec<BlockBreakEvent>,
    net: NetRole,
    local_player_id: PlayerId,
    /// Shown instead of "P{id}" in chat and join notifications. Set once
    /// at launch from the menu's nickname prompt (or `--nickname`/default
    /// for a direct `--connect` launch); never changes mid-session.
    local_nickname: String,
    scripting: ScriptHost,
    lua_tick_timer: f32,
    /// Host-only, like `lua_tick_timer` -- see `POISON_TICK_INTERVAL`.
    poison_tick_timer: f32,
    llm: LlmClient,
    generation: GenerationState,
    next_rule_id: u32,
    last_generated_index: Option<usize>,

    ui: Ui,
    console_open: bool,
    prompt_input: String,
    toasts: Vec<Toast>,
    /// Persistent chat/notification scrollback -- every toast (rule
    /// generated/crashed/etc.) plus real player chat, capped at
    /// `CHAT_LOG_CAPACITY`. See `log_message`.
    chat_log: Vec<ChatEntry>,
    chat_open: bool,
    chat_input: String,
    pending_egui_output: Option<egui::FullOutput>,
    quit_dialog_open: bool,
    /// Set once the player confirms the quit dialog; `main.rs` checks this
    /// after every `update()` and swaps the game out for a fresh main menu
    /// on the same window.
    return_to_menu: bool,

    chunk_meshes: HashMap<(i32, i32), GpuMesh>,
    entity_mesh: Option<GpuMesh>,
    /// The block a right click places, chosen from gathered resources via
    /// the hotbar keys or the Resources panel. `None` until the player has
    /// picked something (or gathered anything) to place.
    selected_block: Option<BlockType>,
    /// Position of the block the player is currently chipping away at with
    /// left-click, and hits landed on it so far -- reset whenever a click
    /// targets a different position. A block breaks once hits reach its
    /// `BlockType::hardness()`; hardness-0 blocks (bedrock) never do.
    mining_target: Option<(i32, i32, i32)>,
    mining_hits: u32,
    cursor_grabbed: bool,

    last_frame: Instant,
    title_timer: f32,
    frame_count: u32,
    last_fps: f32,
    /// Free-running clock, independent of `time_of_day`, purely to drive the
    /// water surface animation in the shader.
    water_time: f32,
}

impl App {
    pub async fn new(window: Arc<Window>, launch: LaunchConfig) -> Result<Self, String> {
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

        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[&camera_bgl],
            push_constant_ranges: &[],
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky pipeline"),
            layout: Some(&sky_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &sky_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
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
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            // Depth-compatible with the main pass's attachment (required to
            // share it) but always-pass/no-write: this draw runs first and
            // fills every pixel unconditionally, and every subsequent
            // depth-tested draw call naturally paints over it wherever real
            // geometry exists.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let rain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rain shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("rain.wgsl").into()),
        });
        let rain_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rain pipeline layout"),
            bind_group_layouts: &[&camera_bgl],
            push_constant_ranges: &[],
        });
        let rain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rain pipeline"),
            layout: Some(&rain_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rain_shader,
                entry_point: "vs_main",
                buffers: &[RainVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &rain_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Read-only: streaks should be hidden behind terrain, but
                // shouldn't occlude each other or write depth themselves.
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let rain_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rain vertex buffer"),
            size: (RAIN_PARTICLE_COUNT * 2 * std::mem::size_of::<RainVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
                let loaded = if launch.fresh { None } else { load_world() };
                let (world, spawn_pos, yaw, pitch, time_of_day, scripting) = match loaded {
                    Some(loaded) => (
                        loaded.world,
                        loaded.player_pos,
                        loaded.yaw,
                        loaded.pitch,
                        loaded.time_of_day,
                        ScriptHost::load_from_save(&loaded.modules),
                    ),
                    None => {
                        let seed = random_world_seed();
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
                    .map_err(|e| {
                        format!(
                            "Failed to bind UDP port {}: {e}. Pick a different port.",
                            launch.port
                        )
                    })?;
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
                    join_handshake(server_addr, &launch.nickname)?;
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
        let rain_particles = build_rain_particles(world.seed);

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
            sky_pipeline,
            rain_pipeline,
            rain_vertex_buffer,
            rain_particles,
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
            pending_block_breaks: Vec::new(),
            net,
            local_player_id,
            local_nickname: sanitize_nickname(&launch.nickname),
            scripting,
            lua_tick_timer: 0.0,
            poison_tick_timer: 0.0,
            llm,
            generation: GenerationState::Idle,
            next_rule_id: 0,
            last_generated_index: None,
            ui,
            console_open: false,
            prompt_input: String::new(),
            toasts: Vec::new(),
            chat_log: Vec::new(),
            chat_open: false,
            chat_input: String::new(),
            pending_egui_output: None,
            quit_dialog_open: false,
            return_to_menu: false,
            chunk_meshes: HashMap::new(),
            entity_mesh: None,
            selected_block: None,
            mining_target: None,
            mining_hits: 0,
            cursor_grabbed: false,
            last_frame: Instant::now(),
            title_timer: 0.0,
            frame_count: 0,
            last_fps: 0.0,
            water_time: 0.0,
        };

        app.grab_cursor(true);
        app.update_chunks();
        Ok(app)
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
                if code == KeyCode::Backquote && !self.quit_dialog_open && !self.chat_open {
                    self.toggle_console();
                    return;
                }
                if code == KeyCode::KeyT
                    && !self.console_open
                    && !self.quit_dialog_open
                    && !self.chat_open
                {
                    self.open_chat();
                    return;
                }
                if code == KeyCode::Escape {
                    if self.console_open {
                        self.close_console();
                    } else if self.chat_open {
                        self.close_chat();
                    } else if self.quit_dialog_open {
                        self.cancel_quit_dialog();
                    } else {
                        self.open_quit_dialog();
                    }
                    return;
                }
                if !self.console_open && !self.quit_dialog_open && !self.chat_open {
                    self.input.key_event(code, key_event.state);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.console_open && !self.quit_dialog_open && !self.chat_open {
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

    fn toggle_console(&mut self) {
        if self.console_open {
            self.close_console();
        } else {
            self.open_console();
        }
    }

    fn open_quit_dialog(&mut self) {
        self.quit_dialog_open = true;
        self.input.release_all();
        self.grab_cursor(false);
    }

    fn cancel_quit_dialog(&mut self) {
        self.quit_dialog_open = false;
        self.grab_cursor(true);
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

    fn open_chat(&mut self) {
        self.chat_open = true;
        self.input.release_all();
        self.grab_cursor(false);
    }

    fn close_chat(&mut self) {
        self.chat_open = false;
        self.grab_cursor(true);
    }

    /// Appends one line to the persistent chat/notification scrollback,
    /// dropping the oldest entry once `CHAT_LOG_CAPACITY` is exceeded.
    fn log_message(&mut self, text: String, color: egui::Color32) {
        if self.chat_log.len() >= CHAT_LOG_CAPACITY {
            self.chat_log.remove(0);
        }
        self.chat_log.push(ChatEntry { text, color });
    }

    /// Sends a chat message from the local player. The host logs and
    /// broadcasts it directly; a joined client hands it to the host, which
    /// attributes it to the sender and relays it back to everyone
    /// (including the sender) as a `Notify` -- so there's no separate local
    /// echo path to keep in sync with the relayed one.
    fn send_chat(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if matches!(self.net, NetRole::Host(_)) {
            self.broadcast_chat(format!("{}: {text}", self.local_nickname));
        } else if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(
                &client.socket,
                client.server_addr,
                ReliableMsg::ChatMessage(text),
            );
        }
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
        // Wrap well before f32 precision would start eating into a sine's
        // period -- the animation is periodic anyway so this is seamless.
        self.water_time = (self.water_time + dt) % 10_000.0;

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
                self.selected_block = Some(b);
            }
        }

        if !self.console_open
            && !self.chat_open
            && (self.input.left_clicked || self.input.right_clicked)
        {
            let origin = self.camera.eye_position();
            let dir = self.camera.forward();
            if let Some(hit) = raycast(&self.world, origin, dir, REACH) {
                if self.input.left_clicked {
                    let target_block = self
                        .world
                        .get_block(hit.target.0, hit.target.1, hit.target.2);
                    if target_block.is_unbreakable() {
                        self.toasts.push(Toast::new(format!(
                            "{} is unbreakable",
                            target_block.name()
                        )));
                    } else {
                        if self.mining_target != Some(hit.target) {
                            self.mining_target = Some(hit.target);
                            self.mining_hits = 0;
                        }
                        self.mining_hits += 1;

                        if self.mining_hits >= target_block.hardness() {
                            self.mining_target = None;
                            self.mining_hits = 0;
                            let broken = target_block;
                            if broken == BlockType::Crystal {
                                self.player.carrying_crystal = true;
                            }
                            self.player.add_resource(broken);
                            // Only the host records this for the Lua tick --
                            // a joined client's own break is picked up on
                            // the host via the network-relayed
                            // `ReliableMsg::BlockEdit` path instead, so
                            // recording it here too would double-fire.
                            if broken != BlockType::Air && matches!(self.net, NetRole::Host(_)) {
                                self.pending_block_breaks.push(BlockBreakEvent {
                                    x: hit.target.0,
                                    y: hit.target.1,
                                    z: hit.target.2,
                                    block: broken,
                                    player_id: self.local_player_id,
                                });
                            }
                            self.apply_block_edit(
                                hit.target.0,
                                hit.target.1,
                                hit.target.2,
                                BlockType::Air,
                            );
                            for pos in self.world.flood_from(hit.target) {
                                self.apply_block_edit(pos.0, pos.1, pos.2, BlockType::Water);
                            }
                        }
                    }
                } else if !self.would_hit_player(hit.place) {
                    match self.selected_block {
                        Some(block) if self.player.take_resource(block) => {
                            self.apply_block_edit(hit.place.0, hit.place.1, hit.place.2, block);
                        }
                        Some(block) => {
                            self.toasts.push(Toast::new(format!("Out of {}", block.name())));
                        }
                        None => {
                            self.toasts.push(Toast::new(
                                "No block selected -- gather resources first",
                            ));
                        }
                    }
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
            let player_targets: Vec<(PlayerId, Vec3)> = self
                .host_player_positions()
                .iter()
                .map(|p| (p.id, p.pos))
                .collect();
            let golem_attacks = self.creatures.update(&self.world, dt, &player_targets);
            for (player_id, damage) in golem_attacks {
                self.apply_player_effect(PlayerEffect::Health {
                    player_id,
                    delta: -damage,
                });
            }

            self.lua_tick_timer += dt;
            if self.lua_tick_timer >= LUA_TICK_INTERVAL {
                self.lua_tick_timer = 0.0;
                let players = self.host_player_positions();
                let outcome = self.scripting.run_tick(
                    &self.world,
                    &mut self.creatures,
                    &players,
                    &mut self.time_of_day,
                    &mut self.weather,
                    &self.pending_block_breaks,
                    self.player.resources_snapshot(),
                );
                self.pending_block_breaks.clear();
                self.apply_tick_outcome(outcome);
                for &(x, y, z) in self.world.redstone_positions.iter() {
                    let center = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    self.creatures
                        .heal_near(center, REDSTONE_HEAL_RADIUS, REDSTONE_HEAL_AMOUNT);
                }
            }

            self.poison_tick_timer += dt;
            if self.poison_tick_timer >= POISON_TICK_INTERVAL {
                self.poison_tick_timer = 0.0;
                self.apply_poison_ticks();
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
        let generation_status: Option<String> = match &self.generation {
            GenerationState::Waiting {
                kind, is_retry: true, ..
            } => Some(format!(
                "Retrying the {} with error feedback...",
                generation_noun(*kind)
            )),
            GenerationState::Waiting {
                kind,
                is_retry: false,
                ..
            } => Some(format!("Generating a {}...", generation_noun(*kind))),
            GenerationState::Idle => None,
        };
        let (full_output, requests) = self.ui.draw(
            &self.window,
            self.console_open,
            &mut self.prompt_input,
            is_host,
            &self.scripting,
            self.last_generated_index,
            generation_status.as_deref(),
            &self.toasts,
            self.last_fps,
            self.quit_dialog_open,
            &self.player,
            self.selected_block,
            self.chat_open,
            &mut self.chat_input,
            &self.chat_log,
        );
        self.pending_egui_output = Some(full_output);

        if let Some(block) = requests.select_block {
            self.selected_block = Some(block);
        }

        if let Some(text) = requests.send_chat {
            self.chat_input.clear();
            self.send_chat(text);
        }

        if requests.confirm_quit {
            self.return_to_menu = true;
        }
        if requests.cancel_quit {
            self.cancel_quit_dialog();
        }

        if let Some(idx) = requests.toggle_index {
            if let Some((name, enabled)) = self.scripting.toggle_at(idx) {
                self.notify_all(format!(
                    "Rule '{name}' {}",
                    if enabled { "activated" } else { "deactivated" }
                ));
            }
        }
        if let Some(idx) = requests.run_index {
            self.run_instant(idx);
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
            let block_info = match self.selected_block {
                Some(b) => b.name(),
                None => "none",
            };
            let mining_info = match self.mining_target {
                Some(pos) => {
                    let target = self.world.get_block(pos.0, pos.1, pos.2);
                    format!(" | Mining {}: {}/{}", target.name(), self.mining_hits, target.hardness())
                }
                None => String::new(),
            };
            self.window.set_title(&format!(
                "Voxel Project | {:02}:{:02} | {} | Block: {} | FPS: {:.0}{} | ~ rules/generate, T chat, F5 save, F11 fullscreen, Esc quit",
                hour,
                minute,
                role_info,
                block_info,
                fps,
                mining_info
            ));
            self.title_timer = 0.0;
            self.frame_count = 0;
        }
    }

    /// Sends a raw `Notify` to every connected client; no-op if not
    /// hosting. Callers add their own local toast/log entry with whatever
    /// styling fits the message.
    fn broadcast_notify(&mut self, kind: NotifyKind, text: &str) {
        if let NetRole::Host(host) = &mut self.net {
            let addrs: Vec<SocketAddr> = host.clients.keys().copied().collect();
            for addr in addrs {
                host.reliable.send(
                    &host.socket,
                    addr,
                    ReliableMsg::Notify {
                        kind,
                        text: text.to_string(),
                    },
                );
            }
        }
    }

    fn notify_all(&mut self, text: String) {
        self.toasts.push(Toast::new(text.clone()));
        self.log_message(text.clone(), egui::Color32::WHITE);
        self.broadcast_notify(NotifyKind::Info, &text);
    }

    /// Like `notify_all`, but for things the player actually needs to
    /// notice and act on (e.g. "a new rule is waiting to be enabled"), not
    /// just an FYI that's fine to scroll past.
    fn notify_all_important(&mut self, text: String) {
        self.toasts.push(Toast::important(text.clone()));
        self.log_message(text.clone(), IMPORTANT_TOAST_COLOR);
        self.broadcast_notify(NotifyKind::Important, &text);
    }

    /// Local-only version of `notify_all_important` -- toast + chat log,
    /// no network broadcast. For host-side status only the host can act
    /// on, e.g. rule generation failing (only the host generates rules).
    fn notify_important(&mut self, text: String) {
        self.toasts.push(Toast::important(text.clone()));
        self.log_message(text, IMPORTANT_TOAST_COLOR);
    }

    /// Broadcasts one already-formatted chat line (e.g. "P2: hello") to the
    /// chat log/toasts and every connected client. Used for both the
    /// host's own chat messages and ones relayed from a joined client.
    fn broadcast_chat(&mut self, formatted: String) {
        self.toasts.push(Toast::new(formatted.clone()));
        self.log_message(formatted.clone(), CHAT_MESSAGE_COLOR);
        self.broadcast_notify(NotifyKind::Chat, &formatted);
    }

    /// All known player state, host included. Used for the Lua World API.
    /// Remote players' `velocity`/`on_ground`/`sprinting` are approximated
    /// on the host from position deltas and shared-world queries, since the
    /// network protocol only carries a client's position/yaw per tick.
    fn host_player_positions(&self) -> Vec<PlayerSnapshot> {
        let NetRole::Host(host) = &self.net else {
            return Vec::new();
        };
        let mut players = vec![PlayerSnapshot {
            id: HOST_PLAYER_ID,
            pos: self.player.position,
            carrying_crystal: self.player.carrying_crystal,
            velocity: self.player.velocity,
            on_ground: self.player.on_ground,
            sprinting: self.player.sprinting,
            in_water: is_in_water(&self.world, self.player.position),
            health: self.player.health,
            poisoned: self.player.poisoned,
            speed_multiplier: self.player.speed_multiplier,
            jump_multiplier: self.player.jump_multiplier,
        }];
        for (&id, rp) in host.remote_players.iter() {
            players.push(PlayerSnapshot {
                id,
                pos: rp.pos,
                carrying_crystal: rp.carrying_crystal,
                velocity: rp.velocity,
                on_ground: approximate_on_ground(&self.world, rp.pos),
                sprinting: horizontal_speed(rp.velocity) > REMOTE_SPRINT_THRESHOLD,
                in_water: is_in_water(&self.world, rp.pos),
                health: rp.health,
                poisoned: rp.poisoned,
                speed_multiplier: rp.speed_multiplier,
                jump_multiplier: rp.jump_multiplier,
            });
        }
        players
    }

    /// Runtime rule pipeline step 1-2: the host submits a prompt (typed into
    /// the in-game console), and it goes to the LLM along with the World
    /// API doc and an example module.
    fn start_generation(&mut self, user_request: String) {
        // A cheap deterministic heuristic (see llm::classify_prompt), not a
        // hard requirement -- it just tells the model which contract to
        // write; `poll_generation` accepts whatever it actually produces
        // after at most one corrective retry.
        let kind = classify_prompt(&user_request);
        let noun = generation_noun(kind);
        log::info!("Generating a {noun} from: {user_request}");
        self.notify_all(format!("Host is generating a {noun}: \"{user_request}\""));
        let pending = self.llm.generate(&user_request, kind);
        self.generation = GenerationState::Waiting {
            user_request,
            kind,
            pending,
            is_retry: false,
        };
    }

    /// Picks a short, meaningful name for a newly generated rule (e.g.
    /// "chickens_flee" instead of "rule_3") by pulling keywords out of the
    /// user's own prompt -- see `llm::derive_rule_name`. Falls back to the
    /// old numbered scheme if the prompt didn't yield anything usable, and
    /// disambiguates against currently loaded rules either way so two
    /// similar prompts don't collide.
    fn make_rule_name(&self, prompt: &str) -> String {
        let base =
            derive_rule_name(prompt).unwrap_or_else(|| format!("rule_{}", self.next_rule_id));
        if !self.scripting.modules.iter().any(|m| m.name == base) {
            return base;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}_{n}");
            if !self.scripting.modules.iter().any(|m| m.name == candidate) {
                return candidate;
            }
            n += 1;
        }
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
            kind,
            is_retry,
            ..
        } = std::mem::replace(&mut self.generation, GenerationState::Idle)
        else {
            unreachable!()
        };
        let noun = generation_noun(kind);

        let code = match result {
            Ok(code) => code,
            Err(e) => {
                log::error!("{noun} generation failed: {e}");
                self.notify_important(format!("{noun} generation failed: {e}"));
                return;
            }
        };

        let name = self.make_rule_name(&user_request);
        // Pre-flight lint (see world_api_validate) before ever handing the
        // source to a real Lua VM -- catches hallucinated/misspelled World
        // API calls and stale block-kind literals with a specific,
        // actionable message, rather than letting them surface as an
        // opaque Module::load or first-tick runtime error.
        let validation_issues = world_api_validate::validate_source(&code);
        let load_result = if validation_issues.is_empty() {
            let tagged_code = world_api_validate::tag_with_api_version(&code);
            Module::load(name.clone(), user_request.clone(), tagged_code)
        } else {
            let joined = validation_issues
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            Err(joined)
        };
        match load_result {
            Ok(module) => {
                // classify_prompt is a hint, not a hard requirement -- if
                // the model wrote the other contract, give it exactly one
                // chance to redo it (reusing the same retry pipeline a
                // validation failure uses), but if it *still* doesn't match
                // afterward, accept the module as whatever kind it actually
                // turned out to be rather than failing outright.
                let kind_matches = module.is_instant == matches!(kind, PromptKind::Instant);
                if !kind_matches && !is_retry {
                    log::warn!(
                        "Generated module's contract didn't match the expected kind, asking \
                         the model to redo it"
                    );
                    let hint = match kind {
                        PromptKind::Instant => {
                            "This should have been an instant spell -- define \
                             `function on_cast(api, event)` that does its whole effect once, \
                             not `on_tick`."
                        }
                        PromptKind::Rule => {
                            "This should have been a continuous rule -- define \
                             `function on_tick(api)`, not `on_cast`."
                        }
                    };
                    let pending = self.llm.retry(&user_request, kind, &code, hint);
                    self.generation = GenerationState::Waiting {
                        user_request,
                        kind,
                        pending,
                        is_retry: true,
                    };
                    return;
                }

                let is_instant = module.is_instant;
                self.next_rule_id += 1;
                let idx = self.scripting.add_generated(module);
                self.last_generated_index = Some(idx);
                let (label, action) = if is_instant {
                    ("spell", "click Run to cast it")
                } else {
                    ("rule", "click Enable to activate it")
                };
                log::info!("Generated {label} module '{name}' from prompt (open the Rules panel and {action})");
                self.notify_all_important(format!(
                    "New {label} generated: '{name}' -- open the Rules panel and {action}"
                ));
            }
            Err(err) if !is_retry => {
                log::warn!("Generated {noun} failed validation, asking the model to fix it: {err}");
                let pending = self.llm.retry(&user_request, kind, &code, &err);
                self.generation = GenerationState::Waiting {
                    user_request,
                    kind,
                    pending,
                    is_retry: true,
                };
            }
            Err(err) => {
                log::error!("Generated {noun} failed validation twice, giving up: {err}");
                self.notify_important(format!("{noun} generation failed: {err}"));
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

    /// Applies everything one `ScriptHost::run_tick`/`run_cast` call
    /// produced -- shared between the regular tick loop and `run_instant`
    /// (a spell's Run click) so both go through identical block-edit
    /// replication, crash/broadcast notification, and item-grant handling.
    fn apply_tick_outcome(&mut self, outcome: TickOutcome) {
        for (x, y, z, block) in outcome.block_edits {
            self.apply_block_edit(x, y, z, block);
        }
        // A rule/spell can pass load-time validation and still hit a
        // runtime error the first time it actually executes (e.g. calling
        // an undefined helper) -- surface that loudly rather than letting
        // it go silently dark.
        for message in outcome.crashes {
            self.notify_all_important(message);
        }
        // api.broadcast: relay through the same host-to-all path every
        // other rule-triggered notification already uses.
        for message in outcome.broadcasts {
            self.notify_all(message);
        }
        for effect in outcome.player_effects {
            self.apply_player_effect(effect);
        }
    }

    /// Applies one `PlayerEffect` to its target's authoritative state --
    /// directly on the host's own `Player` for `HOST_PLAYER_ID`, or on that
    /// connection's `RemotePlayer` record for anyone else (silently
    /// dropped if `player_id` isn't connected, matching the World API
    /// action that produced it already having checked this in Lua and
    /// returned `false`). Health/poisoned/speed/jump reach the *remote*
    /// player's own client purely by riding the next `Snapshot` broadcast
    /// (see world_api/schema.yaml's `replication.player_attributes`) --
    /// only item grants need an explicit targeted message, since inventory
    /// doesn't ride the snapshot.
    fn apply_player_effect(&mut self, effect: PlayerEffect) {
        match effect {
            PlayerEffect::GiveItem { player_id, block, amount } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.add_resources(block, amount);
                    return;
                }
                let NetRole::Host(host) = &mut self.net else {
                    return;
                };
                let Some(addr) = host
                    .clients
                    .iter()
                    .find(|(_, &id)| id == player_id)
                    .map(|(&addr, _)| addr)
                else {
                    return;
                };
                host.reliable
                    .send(&host.socket, addr, ReliableMsg::GrantItem { block, amount });
            }
            PlayerEffect::TakeItem { player_id, block, amount } => {
                // Only ever queued for HOST_PLAYER_ID -- see take_item's
                // doc in world_api/schema.yaml. The Lua-visible success
                // bool already reflected a host_resources snapshot check
                // at call time, and nothing can have changed
                // self.player.resources between then and now within the
                // same tick, so this always succeeds when it gets here.
                if player_id == HOST_PLAYER_ID {
                    self.player.take_resources(block, amount);
                }
            }
            PlayerEffect::Health { player_id, delta } => {
                if player_id == HOST_PLAYER_ID {
                    if delta >= 0.0 {
                        self.player.heal(delta);
                    } else {
                        self.player.damage(-delta);
                    }
                } else if let NetRole::Host(host) = &mut self.net {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.health = (rp.health + delta).clamp(0.0, MAX_HEALTH);
                    }
                }
            }
            PlayerEffect::Poisoned { player_id, poisoned } => {
                let changed = if player_id == HOST_PLAYER_ID {
                    let was = self.player.poisoned;
                    self.player.poisoned = poisoned;
                    was != poisoned
                } else if let NetRole::Host(host) = &mut self.net {
                    match host.remote_players.get_mut(&player_id) {
                        Some(rp) => {
                            let was = rp.poisoned;
                            rp.poisoned = poisoned;
                            was != poisoned
                        }
                        None => false,
                    }
                } else {
                    false
                };
                // Edge-triggered, not every call -- a rule re-asserting
                // "still poisoned" every tick shouldn't spam a toast.
                if changed {
                    let verb = if poisoned { "poisoned" } else { "no longer poisoned" };
                    self.notify_all(format!("P{player_id} is {verb}"));
                }
            }
            PlayerEffect::SpeedMultiplier { player_id, multiplier } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.set_speed_multiplier(multiplier);
                } else if let NetRole::Host(host) = &mut self.net {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.speed_multiplier =
                            multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
                    }
                }
            }
            PlayerEffect::JumpMultiplier { player_id, multiplier } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.set_jump_multiplier(multiplier);
                } else if let NetRole::Host(host) = &mut self.net {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.jump_multiplier =
                            multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
                    }
                }
            }
        }
    }

    /// Damages every currently-poisoned player by `POISON_DAMAGE_PER_TICK`
    /// -- called every `POISON_TICK_INTERVAL` seconds, host-only. Pure
    /// engine state, not a `PlayerEffect`/World API action: no Lua callback
    /// fires for this, matching world_api/schema.yaml's `poison_tick_secs`
    /// note that it's "only observable by polling ... .health from on_tick".
    fn apply_poison_ticks(&mut self) {
        if self.player.poisoned {
            self.player.damage(POISON_DAMAGE_PER_TICK);
        }
        if let NetRole::Host(host) = &mut self.net {
            for rp in host.remote_players.values_mut() {
                if rp.poisoned {
                    rp.health = (rp.health - POISON_DAMAGE_PER_TICK).max(0.0);
                }
            }
        }
    }

    /// Runs one instant spell exactly once -- the Rules panel Run button's
    /// entry point. Host-only, like generating/activating a rule; a no-op
    /// otherwise (mirrors the same restriction `is_host` already gates in
    /// the UI, checked again here since `requests.run_index` is plain user
    /// input the UI layer can't fully trust on its own).
    fn run_instant(&mut self, index: usize) {
        if !matches!(self.net, NetRole::Host(_)) {
            return;
        }
        let Some(module) = self.scripting.modules.get(index) else {
            return;
        };
        if !module.is_instant {
            return;
        }
        let name = module.name.clone();
        let players = self.host_player_positions();
        let outcome = self.scripting.run_cast(
            index,
            &self.world,
            &mut self.creatures,
            &players,
            &mut self.time_of_day,
            &mut self.weather,
            HOST_PLAYER_ID,
            self.player.resources_snapshot(),
        );
        let had_crash = !outcome.crashes.is_empty();
        self.apply_tick_outcome(outcome);
        if !had_crash {
            self.notify_all(format!("Host cast '{name}'"));
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
            let mut players: Vec<SnapshotPlayer> = vec![SnapshotPlayer {
                id: HOST_PLAYER_ID,
                pos: self.player.position.to_array(),
                yaw: self.camera.yaw,
                carrying_crystal: self.player.carrying_crystal,
                health: self.player.health,
                poisoned: self.player.poisoned,
                speed_multiplier: self.player.speed_multiplier,
                jump_multiplier: self.player.jump_multiplier,
            }];
            for (&id, rp) in host.remote_players.iter() {
                players.push(SnapshotPlayer {
                    id,
                    pos: rp.pos.to_array(),
                    yaw: rp.yaw,
                    carrying_crystal: rp.carrying_crystal,
                    health: rp.health,
                    poisoned: rp.poisoned,
                    speed_multiplier: rp.speed_multiplier,
                    jump_multiplier: rp.jump_multiplier,
                });
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
                    ReliableMsg::Hello { nickname } => {
                        if host.clients.contains_key(&from) {
                            return;
                        }
                        let nickname = sanitize_nickname(&nickname);
                        let player_id = host.next_player_id;
                        host.next_player_id += 1;
                        // Terrain-height-snapped, not just offset from the
                        // host's own Y -- otherwise a new player spawns
                        // floating or buried whenever the ground isn't flat
                        // between the two spawn columns.
                        let spawn_x = self.player.position.x + (player_id as f32) * 2.0;
                        let spawn_z = self.player.position.z + 2.0;
                        let spawn_y = self
                            .world
                            .terrain_height(spawn_x.floor() as i32, spawn_z.floor() as i32)
                            as f32
                            + 2.0;
                        let spawn = Vec3::new(spawn_x, spawn_y, spawn_z);
                        host.clients.insert(from, player_id);
                        host.remote_players.insert(
                            player_id,
                            RemotePlayer::new(spawn, 0.0, false, nickname.clone()),
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
                        log::info!("Player {player_id} ('{nickname}') joined from {from}");
                        self.notify_all(format!("{nickname} joined"));
                    }
                    ReliableMsg::BlockEdit { x, y, z, block } => {
                        let addrs: Vec<SocketAddr> = host.clients.keys().copied().collect();
                        let breaker_id = host.clients.get(&from).copied();
                        let old_block = self.world.get_block(x, y, z);
                        self.world.set_block(x, y, z, block);
                        if block == BlockType::Air && old_block != BlockType::Air {
                            if let Some(player_id) = breaker_id {
                                self.pending_block_breaks.push(BlockBreakEvent {
                                    x,
                                    y,
                                    z,
                                    block: old_block,
                                    player_id,
                                });
                            }
                        }
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
                    ReliableMsg::ChatMessage(text) => {
                        let sender = host
                            .clients
                            .get(&from)
                            .and_then(|player_id| host.remote_players.get(player_id))
                            .map(|rp| rp.nickname.clone());
                        if let Some(nickname) = sender {
                            self.broadcast_chat(format!("{nickname}: {text}"));
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
                        let new_pos = Vec3::from_array(pos);
                        let dt = rp.last_seen.elapsed().as_secs_f32().max(0.001);
                        rp.velocity = (new_pos - rp.pos) / dt;
                        rp.pos = new_pos;
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
                    ReliableMsg::Notify { kind, text } => {
                        let color = match kind {
                            NotifyKind::Info => egui::Color32::WHITE,
                            NotifyKind::Important => IMPORTANT_TOAST_COLOR,
                            NotifyKind::Chat => CHAT_MESSAGE_COLOR,
                        };
                        self.toasts.push(if matches!(kind, NotifyKind::Important) {
                            Toast::important(text.clone())
                        } else {
                            Toast::new(text.clone())
                        });
                        self.log_message(text, color);
                    }
                    ReliableMsg::GrantItem { block, amount } => {
                        self.player.add_resources(block, amount);
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
                let local_player_id = self.local_player_id;
                let now = Instant::now();
                for sp in players {
                    // The host is authoritative for health/poison/movement
                    // attributes -- for *my own* entry, apply them straight
                    // to the local Player that actually simulates my
                    // physics/HUD, not to a RemotePlayer record of myself.
                    if sp.id == local_player_id {
                        self.player.health = sp.health;
                        self.player.poisoned = sp.poisoned;
                        self.player.speed_multiplier = sp.speed_multiplier;
                        self.player.jump_multiplier = sp.jump_multiplier;
                        continue;
                    }
                    let NetRole::Joined(client) = &mut self.net else {
                        unreachable!()
                    };
                    client
                        .remote_players
                        .entry(sp.id)
                        .and_modify(|rp| {
                            rp.pos = Vec3::from_array(sp.pos);
                            rp.yaw = sp.yaw;
                            rp.carrying_crystal = sp.carrying_crystal;
                            rp.health = sp.health;
                            rp.poisoned = sp.poisoned;
                            rp.speed_multiplier = sp.speed_multiplier;
                            rp.jump_multiplier = sp.jump_multiplier;
                            rp.last_seen = now;
                        })
                        .or_insert_with(|| {
                            // Unused on the client -- only the host formats
                            // chat/join text, so it never asks a client for
                            // another player's nickname.
                            let mut rp = RemotePlayer::new(
                                Vec3::from_array(sp.pos),
                                sp.yaw,
                                sp.carrying_crystal,
                                String::new(),
                            );
                            rp.health = sp.health;
                            rp.poisoned = sp.poisoned;
                            rp.speed_multiplier = sp.speed_multiplier;
                            rp.jump_multiplier = sp.jump_multiplier;
                            rp
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

    /// Rebuilds the rain streak vertex buffer for this frame. Each streak is
    /// a short vertical line whose y wraps within `RAIN_HEIGHT`, driven by
    /// the already free-running `water_time` clock -- so it never needs its
    /// own per-particle state, just a fixed (x, z, phase) offset from the
    /// camera picked once at load in `build_rain_particles`.
    fn write_rain_vertices(&self, cam_pos: Vec3) {
        let half_h = RAIN_HEIGHT * 0.5;
        let mut verts: Vec<RainVertex> = Vec::with_capacity(self.rain_particles.len() * 2);
        for &(ox, oz, phase) in &self.rain_particles {
            let y = (phase - self.water_time * RAIN_FALL_SPEED).rem_euclid(RAIN_HEIGHT) - half_h;
            // Fade out near the top/bottom of the volume so a streak
            // doesn't visibly pop in/out of existence as it wraps.
            let alpha = RAIN_ALPHA * (half_h - y.abs()).clamp(0.0, 1.0);
            let x = cam_pos.x + ox;
            let z = cam_pos.z + oz;
            verts.push(RainVertex {
                position: [x, cam_pos.y + y + RAIN_STREAK_LENGTH * 0.5, z],
                alpha,
            });
            verts.push(RainVertex {
                position: [x, cam_pos.y + y - RAIN_STREAK_LENGTH * 0.5, z],
                alpha,
            });
        }
        self.queue
            .write_buffer(&self.rain_vertex_buffer, 0, bytemuck::cast_slice(&verts));
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let lighting = sky_lighting(self.time_of_day);
        let sky = lighting.sky_color;
        let zenith = lighting.zenith_color;
        let cam_pos = self.camera.eye_position();
        let light_view_proj = light_view_proj(lighting.sun_dir, self.player.position);
        let view_proj = self.camera.view_proj();
        let uniform = CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            light_view_proj: light_view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            fog_color: [sky[0], sky[1], sky[2], 1.0],
            zenith_color: [zenith[0], zenith[1], zenith[2], 1.0],
            sun_dir: [
                lighting.sun_dir.x,
                lighting.sun_dir.y,
                lighting.sun_dir.z,
                0.0,
            ],
            // z = free-running clock for the water wave animation.
            light_params: [lighting.ambient, lighting.sun_intensity, self.water_time, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        self.queue.write_buffer(
            &self.shadow_light_buffer,
            0,
            bytemuck::bytes_of(&LightUniform { view_proj: light_view_proj.to_cols_array_2d() }),
        );

        let raining = self.weather.current == Weather::Rain;
        let rain_vertex_count = if raining {
            self.write_rain_vertices(cam_pos);
            (self.rain_particles.len() * 2) as u32
        } else {
            0
        };

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

            // Sky background first -- a single fullscreen triangle with no
            // depth test, so every draw after this one simply paints over
            // it wherever real geometry exists. See sky.wgsl.
            rpass.set_pipeline(&self.sky_pipeline);
            rpass.set_bind_group(0, &self.camera_bind_group, &[]);
            rpass.draw(0..3, 0..1);

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
            if rain_vertex_count > 0 {
                rpass.set_pipeline(&self.rain_pipeline);
                rpass.set_bind_group(0, &self.camera_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.rain_vertex_buffer.slice(..));
                rpass.draw(0..rain_vertex_count, 0..1);
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

    /// Whether the player confirmed the Esc quit dialog -- `main.rs` checks
    /// this after every frame and, if set, drops this game and returns to
    /// the main menu on the same window.
    pub fn wants_return_to_menu(&self) -> bool {
        self.return_to_menu
    }
}

/// Blocks (with a resend-until-acked handshake) until the host answers with
/// a `Welcome`, or gives up with an error after a timeout. Never exits the
/// process itself -- the caller decides how to handle a failed join (the
/// main menu shows it and lets the user retry; a CLI `--connect` launch
/// prints it and exits).
fn join_handshake(
    server_addr: SocketAddr,
    nickname: &str,
) -> Result<(UdpSocket, PlayerId, World, Vec3, f32, ReliableChannel), String> {
    let socket = net::bind_nonblocking("0.0.0.0:0")
        .map_err(|e| format!("Failed to open a local UDP socket: {e}"))?;

    let mut reliable = ReliableChannel::new();
    let hello_id = reliable.send(
        &socket,
        server_addr,
        ReliableMsg::Hello {
            nickname: nickname.to_string(),
        },
    );
    log::info!("Connecting to {server_addr}...");

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut buf = [0u8; 8192];
    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "Failed to connect to {server_addr}: no response from host. \
                 Is it running, and is the address/port correct?"
            ));
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
                    return Ok((
                        socket,
                        player_id,
                        world,
                        Vec3::from_array(spawn),
                        time_of_day,
                        reliable,
                    ));
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                return Err(format!("Network error while connecting: {e}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_nickname_trims_surrounding_whitespace() {
        assert_eq!(sanitize_nickname("  Steve  "), "Steve");
    }

    #[test]
    fn sanitize_nickname_falls_back_to_a_default_when_empty_or_blank() {
        assert_eq!(sanitize_nickname(""), "Player");
        assert_eq!(sanitize_nickname("   "), "Player");
    }

    #[test]
    fn sanitize_nickname_caps_overly_long_input() {
        let long_name = "a".repeat(50);
        let result = sanitize_nickname(&long_name);
        assert_eq!(result.chars().count(), MAX_NICKNAME_LEN);
        assert_eq!(result, "a".repeat(MAX_NICKNAME_LEN));
    }

    #[test]
    fn sanitize_nickname_passes_through_a_normal_name_unchanged() {
        assert_eq!(sanitize_nickname("Alex"), "Alex");
    }

    /// Regression test for the bug this replaced: seeding from
    /// `Instant::now().elapsed()` measured almost nothing (the tiny gap
    /// between two back-to-back calls) instead of real wall-clock time, so
    /// consecutive "New World" launches got the same seed -- and since
    /// terrain/creatures/weather all derive from it, every world looked the
    /// same. A handful of calls spread over a few milliseconds should not
    /// all collide.
    #[test]
    fn random_world_seed_varies_across_calls() {
        let mut seeds = Vec::new();
        for _ in 0..20 {
            seeds.push(random_world_seed());
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        let first = seeds[0];
        assert!(
            seeds.iter().any(|&s| s != first),
            "expected at least one different seed across 20 calls, got {seeds:?}"
        );
    }
}
