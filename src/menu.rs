use crate::transport::JoinTarget;
use std::sync::Arc;

use winit::event::WindowEvent;
use winit::window::Window;

use crate::net::{LaunchConfig, MAX_NICKNAME_LEN};
use crate::save;
use crate::ui::Ui;

/// What the player picked from the main menu, for `main.rs` to act on --
/// either constructing the real game (`App::new`) or exiting. Every path
/// that actually launches a game carries the nickname entered on the
/// nickname screen.
pub enum MenuAction {
    NewWorld { nickname: String },
    LoadWorld { nickname: String },
    Join { addr: JoinTarget, nickname: String },
    Quit,
}

/// Which top-level action the nickname screen is gathering a name for,
/// so submitting it can build the right `MenuAction`. `Copy` so the screen
/// UI can pull an owned value out of `Screen::Nickname` up front and stop
/// borrowing `screen`, instead of holding that borrow across widget calls
/// that also need to reassign `screen` (e.g. the Back button).
#[derive(Clone, Copy)]
enum PendingAction {
    NewWorld,
    LoadWorld,
    Join(JoinTarget),
}

/// Which screen of the menu is showing. Both `Join` and `Nickname` are
/// reached from `Main` and return to it via "Back" -- there's no deeper
/// nesting than that.
enum Screen {
    Main,
    /// Entering a host address, on the way to `Nickname(Join(addr))`.
    Join,
    Nickname(PendingAction),
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
    screen: Screen,
    join_input: String,
    nickname_input: String,
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
            screen: Screen::Main,
            join_input: String::new(),
            nickname_input: String::new(),
            error: None,
            port: launch.port,
            llm_url: launch.llm_url,
        }
    }

    pub fn window_event(&mut self, event: &WindowEvent) {
        if let WindowEvent::KeyboardInput { event: key, .. } = event {
            if key.state == winit::event::ElementState::Pressed && !key.repeat {
                use winit::keyboard::{PhysicalKey, KeyCode};
                if key.physical_key == PhysicalKey::Code(KeyCode::F10) {
                    self.ui.settings.open = !self.ui.settings.open;
                } else if key.physical_key == PhysicalKey::Code(KeyCode::Escape) && self.ui.settings.open {
                    self.ui.settings.open = false;
                }
            }
        }
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
        self.screen = Screen::Join;
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

        #[cfg(feature = "steam")]
        {
            crate::steam_transport::poll_runtime();
            if let Some(id) = crate::steam_transport::take_invite() {
                self.join_input = format!("steam:{id}");
                self.screen = Screen::Nickname(PendingAction::Join(JoinTarget::SteamLobby(id)));
                self.error = None;
            }
        }
        let mut action = None;
        let mut error = self.error.clone();
        let mut screen = std::mem::replace(&mut self.screen, Screen::Main);
        let mut join_input = std::mem::take(&mut self.join_input);
        let mut nickname_input = std::mem::take(&mut self.nickname_input);
        let has_save = self.has_save;
        let fantasy = self.ui.settings.values.appearance.ui_theme == crate::settings::UiTheme::Fantasy;
        let button_size = if fantasy { [320.0, 44.0] } else { [260.0, 40.0] };
        let mut open_settings = false;
        let steam = self.ui.settings.values.multiplayer.mode == crate::settings::MultiplayerMode::Steam;

        let full_output = self.ui.run(&self.window, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if fantasy { crate::ui_theme::menu_backdrop(ui); }
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("Voxel Project");
                    ui.add_space(30.0);

                    if let Some(err) = &error {
                        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
                        ui.add_space(10.0);
                    }

                    match &screen {
                        Screen::Main => {
                            if ui
                                .add_sized(button_size, egui::Button::new("New World"))
                                .clicked()
                            {
                                screen = Screen::Nickname(PendingAction::NewWorld);
                                error = None;
                            }
                            ui.add_space(8.0);
                            ui.add_enabled_ui(has_save, |ui| {
                                if ui
                                    .add_sized(button_size, egui::Button::new("Load World"))
                                    .clicked()
                                {
                                    screen = Screen::Nickname(PendingAction::LoadWorld);
                                    error = None;
                                }
                            });
                            ui.add_space(8.0);
                            if ui
                                .add_sized(
                                    button_size,
                                    egui::Button::new(if steam { "Join Steam Lobby" } else { "Join Multiplayer Game" }),
                                )
                                .clicked()
                            {
                                screen = Screen::Join;
                                error = None;
                            }
                            ui.add_space(8.0);
                            if ui.add_sized(button_size, egui::Button::new("Settings")).clicked() {
                                open_settings = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add_sized(button_size, egui::Button::new("Quit"))
                                .clicked()
                            {
                                action = Some(MenuAction::Quit);
                            }
                        }
                        Screen::Join => {
                            ui.label(if steam { "Steam lobby code (steam:ID):" } else { "Host address (ip:port):" });
                            let resp = ui.text_edit_singleline(&mut join_input);
                            if !resp.has_focus() && !resp.lost_focus() {
                                resp.request_focus();
                            }
                            let submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.add_space(10.0);
                            if ui
                                .add_sized(button_size, egui::Button::new("Continue"))
                                .clicked()
                                || submitted
                            {
                                let target = if steam && !join_input.trim().starts_with("steam:") {
                                    format!("steam:{}", join_input.trim())
                                } else { join_input.trim().to_string() };
                                match target.parse::<JoinTarget>() {
                                    Ok(addr) => {
                                        screen = Screen::Nickname(PendingAction::Join(addr));
                                        error = None;
                                    }
                                    Err(_) => {
                                        error = Some(
                                            "Invalid destination: use ip:port or steam:lobby_id"
                                                .to_string(),
                                        )
                                    }
                                }
                            }
                            ui.add_space(8.0);
                            if ui
                                .add_sized(button_size, egui::Button::new("Back"))
                                .clicked()
                            {
                                screen = Screen::Main;
                                error = None;
                            }
                        }
                        Screen::Nickname(pending) => {
                            // Owned copy, detached from `screen`'s borrow,
                            // so the Back button below can freely reassign
                            // `screen` without fighting this match.
                            let pending = *pending;
                            let prompt = match pending {
                                PendingAction::NewWorld => {
                                    "Starting a new world -- enter a nickname:".to_string()
                                }
                                PendingAction::LoadWorld => {
                                    "Loading your world -- enter a nickname:".to_string()
                                }
                                PendingAction::Join(addr) => {
                                    format!("Joining {addr} -- enter a nickname:")
                                }
                            };
                            ui.label(prompt);
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut nickname_input)
                                    .char_limit(MAX_NICKNAME_LEN),
                            );
                            if !resp.has_focus() && !resp.lost_focus() {
                                resp.request_focus();
                            }
                            let submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.add_space(10.0);
                            if ui
                                .add_sized(button_size, egui::Button::new("Continue"))
                                .clicked()
                                || submitted
                            {
                                let nickname = nickname_input.trim().to_string();
                                if nickname.is_empty() {
                                    error = Some("Enter a nickname".to_string());
                                } else {
                                    action = Some(match pending {
                                        PendingAction::NewWorld => {
                                            MenuAction::NewWorld { nickname }
                                        }
                                        PendingAction::LoadWorld => {
                                            MenuAction::LoadWorld { nickname }
                                        }
                                        PendingAction::Join(addr) => {
                                            MenuAction::Join { addr, nickname }
                                        }
                                    });
                                }
                            }
                            ui.add_space(8.0);
                            if ui
                                .add_sized(button_size, egui::Button::new("Back"))
                                .clicked()
                            {
                                screen = Screen::Main;
                                error = None;
                            }
                        }
                    }
                });
            });
        });

        if open_settings { self.ui.settings.open = true; }
        self.error = error;
        self.screen = screen;
        self.join_input = join_input;
        self.nickname_input = nickname_input;

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
