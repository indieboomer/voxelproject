use std::net::SocketAddr;
use std::sync::Arc;

use winit::event::WindowEvent;
use winit::window::Window;

use crate::net::LaunchConfig;
use crate::save;
use crate::ui::Ui;

/// What the player picked from the main menu, for `main.rs` to act on --
/// either constructing the real game (`App::new`) or exiting.
pub enum MenuAction {
    NewWorld,
    LoadWorld,
    Join(SocketAddr),
    Quit,
}

/// The main menu shown before the game itself starts. Owns just enough of
/// its own (much smaller) wgpu setup to draw a simple egui screen into the
/// same window `App` will later take over -- no voxel pipeline, no shadow
/// map, nothing gameplay-related.
pub struct MenuApp {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    ui: Ui,
    has_save: bool,
    show_join: bool,
    join_input: String,
    error: Option<String>,

    /// Carried through to whatever `LaunchConfig` a menu selection builds,
    /// so `--port`/`--llm-url` CLI overrides still apply even though the
    /// menu is what's driving the actual launch decision.
    pub port: u16,
    pub llm_url: String,
}

impl MenuApp {
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
                    label: Some("menu device"),
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

        let ui = Ui::new(&device, config.format, &window);

        Self {
            window,
            surface,
            device,
            queue,
            config,
            ui,
            has_save: save::save_exists(),
            show_join: false,
            join_input: String::new(),
            error: None,
            port: launch.port,
            llm_url: launch.llm_url,
        }
    }

    pub fn window_event(&mut self, event: &WindowEvent) {
        self.ui.handle_event(&self.window, event);
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Shows the failure from a join attempt made after the menu handed off
    /// a `Join` action, and drops the player back on the address entry
    /// screen (rather than the top-level menu) so they can just fix the
    /// address and retry.
    pub fn report_join_error(&mut self, message: String) {
        self.error = Some(message);
        self.show_join = true;
    }

    pub fn render(&mut self) -> Option<MenuAction> {
        let output = match self.surface.get_current_texture() {
            Ok(o) => o,
            Err(_) => return None,
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("menu encoder"),
            });
        {
            let _clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("menu clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.07,
                            b: 0.11,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }

        let mut action = None;
        let mut error = self.error.clone();
        let mut show_join = self.show_join;
        let mut join_input = std::mem::take(&mut self.join_input);
        let has_save = self.has_save;

        let full_output = self.ui.run(&self.window, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("Voxel Project");
                    ui.add_space(30.0);

                    if let Some(err) = &error {
                        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
                        ui.add_space(10.0);
                    }

                    if show_join {
                        ui.label("Host address (ip:port):");
                        let resp = ui.text_edit_singleline(&mut join_input);
                        if !resp.has_focus() && !resp.lost_focus() {
                            resp.request_focus();
                        }
                        let submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(10.0);
                        if ui
                            .add_sized([220.0, 36.0], egui::Button::new("Connect"))
                            .clicked()
                            || submitted
                        {
                            match join_input.trim().parse::<SocketAddr>() {
                                Ok(addr) => action = Some(MenuAction::Join(addr)),
                                Err(_) => {
                                    error = Some(
                                        "Invalid address -- expected ip:port, e.g. 192.168.1.5:7878"
                                            .to_string(),
                                    )
                                }
                            }
                        }
                        ui.add_space(8.0);
                        if ui
                            .add_sized([220.0, 36.0], egui::Button::new("Back"))
                            .clicked()
                        {
                            show_join = false;
                            error = None;
                        }
                    } else {
                        if ui
                            .add_sized([220.0, 36.0], egui::Button::new("New World"))
                            .clicked()
                        {
                            action = Some(MenuAction::NewWorld);
                        }
                        ui.add_space(8.0);
                        ui.add_enabled_ui(has_save, |ui| {
                            if ui
                                .add_sized([220.0, 36.0], egui::Button::new("Load World"))
                                .clicked()
                            {
                                action = Some(MenuAction::LoadWorld);
                            }
                        });
                        ui.add_space(8.0);
                        if ui
                            .add_sized(
                                [220.0, 36.0],
                                egui::Button::new("Join Multiplayer Game"),
                            )
                            .clicked()
                        {
                            show_join = true;
                            error = None;
                        }
                        ui.add_space(8.0);
                        if ui
                            .add_sized([220.0, 36.0], egui::Button::new("Quit"))
                            .clicked()
                        {
                            action = Some(MenuAction::Quit);
                        }
                    }
                });
            });
        });

        self.error = error;
        self.show_join = show_join;
        self.join_input = join_input;

        self.ui.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            &self.window,
            full_output,
            [self.config.width, self.config.height],
        );

        self.queue.submit(Some(encoder.finish()));
        output.present();
        action
    }
}
