use std::time::{Duration, Instant};

use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::player::Player;
use crate::scripting::ScriptHost;
use crate::voxel::{BlockType, COLLECTIBLE_BLOCKS};

const TOAST_LIFETIME: Duration = Duration::from_secs(6);
/// Longer-lived than a normal toast, since it flags something the player
/// actually needs to notice and act on (e.g. "go click Enable") rather than
/// an FYI that's fine to miss.
const IMPORTANT_TOAST_LIFETIME: Duration = Duration::from_secs(14);
pub const IMPORTANT_TOAST_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);

pub struct Toast {
    pub text: String,
    pub color: egui::Color32,
    expires: Instant,
}

impl Toast {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: egui::Color32::WHITE,
            expires: Instant::now() + TOAST_LIFETIME,
        }
    }

    /// A highlighted, longer-lived toast for things that need the player's
    /// attention -- e.g. a freshly generated rule waiting to be enabled.
    pub fn important(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: IMPORTANT_TOAST_COLOR,
            expires: Instant::now() + IMPORTANT_TOAST_LIFETIME,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires
    }
}

/// Color a player-authored chat line is shown in, distinguishing "someone
/// said X" from a system notification (white/`IMPORTANT_TOAST_COLOR`) at a
/// glance in the shared scrollback.
pub const CHAT_MESSAGE_COLOR: egui::Color32 = egui::Color32::from_rgb(140, 210, 255);
/// Oldest entries are dropped past this so the log can't grow unbounded
/// over a long session.
pub const CHAT_LOG_CAPACITY: usize = 200;

/// One line in the persistent chat/notification scrollback -- unlike
/// `Toast`, these never expire on their own; `App` caps the backing `Vec`
/// at `CHAT_LOG_CAPACITY` instead.
pub struct ChatEntry {
    pub text: String,
    pub color: egui::Color32,
}

/// What the player asked the UI to do this frame; `App` acts on these after
/// the egui pass since the widget closures can't safely call back into game
/// state directly.
#[derive(Default)]
pub struct UiRequests {
    pub toggle_index: Option<usize>,
    pub delete_index: Option<usize>,
    /// Set when the player clicks "Run" on an instant spell (`Module::is_instant`).
    pub run_index: Option<usize>,
    pub submit_prompt: Option<String>,
    pub confirm_quit: bool,
    pub cancel_quit: bool,
    /// Set when the player clicks "Select" on a resource in the Resources
    /// panel -- becomes the new block placed by a right click.
    pub select_block: Option<BlockType>,
    /// Set when the player submits a line from the chat box.
    pub send_chat: Option<String>,
}

pub struct Ui {
    ctx: egui::Context,
    state: State,
    renderer: Renderer,
    /// Index into `scripting.modules` of the rule whose source is currently
    /// shown in the "Rule Source" viewer window, if any. Purely local UI
    /// state -- doesn't need to round-trip through `App`.
    viewing_index: Option<usize>,
}

impl Ui {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat, window: &Window) -> Self {
        let ctx = egui::Context::default();
        let state = State::new(ctx.clone(), egui::ViewportId::ROOT, window, None, None);
        let renderer = Renderer::new(device, output_format, None, 1);
        Self {
            ctx,
            state,
            renderer,
            viewing_index: None,
        }
    }

    /// Feed a winit event to egui. Should be called for every window event
    /// regardless of game state -- harmless when no widget has focus.
    pub fn handle_event(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.state.on_window_event(window, event);
    }

    /// Runs a one-off egui frame with caller-provided content, for UI that
    /// isn't the in-game HUD (currently just the main menu). Pass the
    /// returned `FullOutput` to `render`.
    pub fn run(&mut self, window: &Window, contents: impl FnOnce(&egui::Context)) -> egui::FullOutput {
        let raw_input = self.state.take_egui_input(window);
        self.ctx.run(raw_input, contents)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        window: &Window,
        console_open: bool,
        prompt_input: &mut String,
        is_host: bool,
        scripting: &ScriptHost,
        recent_index: Option<usize>,
        generation_status: Option<&str>,
        toasts: &[Toast],
        fps: f32,
        quit_dialog_open: bool,
        player: &Player,
        selected_block: Option<BlockType>,
        chat_open: bool,
        chat_input: &mut String,
        chat_log: &[ChatEntry],
    ) -> (egui::FullOutput, UiRequests) {
        let raw_input = self.state.take_egui_input(window);
        let mut requests = UiRequests::default();
        let mut viewing_index = self.viewing_index;

        let full_output = self.ctx.run(raw_input, |ctx| {
            egui::Window::new("fps")
                .title_bar(false)
                .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
                .resizable(false)
                .collapsible(false)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.label(format!("{fps:.0} FPS"));
                });

            egui::Window::new("health")
                .title_bar(false)
                .anchor(egui::Align2::RIGHT_TOP, [-8.0, 32.0])
                .resizable(false)
                .collapsible(false)
                .interactable(false)
                .show(ctx, |ui| {
                    let health_color = if player.health <= 25.0 {
                        egui::Color32::from_rgb(220, 90, 90)
                    } else if player.health <= 60.0 {
                        IMPORTANT_TOAST_COLOR
                    } else {
                        egui::Color32::from_rgb(100, 200, 100)
                    };
                    ui.colored_label(health_color, format!("Health: {:.0}/100", player.health));
                    if player.poisoned {
                        ui.colored_label(egui::Color32::from_rgb(140, 210, 100), "POISONED");
                    }
                });

            egui::Window::new("Rules")
                .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    if scripting.modules.is_empty() {
                        ui.label("No rules or spells loaded. Press ~ to describe one.");
                    }
                    for (i, m) in scripting.modules.iter().enumerate() {
                        let (status, status_color) = if m.is_instant {
                            ("SPELL", egui::Color32::from_rgb(180, 140, 230))
                        } else if m.error.is_some() {
                            ("ERR", egui::Color32::from_rgb(220, 90, 90))
                        } else if m.enabled {
                            ("ON", egui::Color32::from_rgb(100, 200, 100))
                        } else {
                            ("OFF", IMPORTANT_TOAST_COLOR)
                        };
                        let is_recent = Some(i) == recent_index;
                        let marker = if is_recent { "-> " } else { "" };
                        ui.horizontal(|ui| {
                            ui.colored_label(status_color, format!("{marker}{} [{status}]", m.name));
                            let version_label = match m.api_version() {
                                Some(v) => format!("api v{v}"),
                                None => "api version unknown".to_string(),
                            };
                            ui.weak(version_label);
                            let view_label = if viewing_index == Some(i) { "Hide Code" } else { "View Code" };
                            if ui.small_button(view_label).clicked() {
                                viewing_index = if viewing_index == Some(i) { None } else { Some(i) };
                            }
                            if is_host {
                                if m.is_instant {
                                    if ui.small_button("Run").clicked() {
                                        requests.run_index = Some(i);
                                    }
                                } else {
                                    let label = if m.enabled { "Disable" } else { "Enable" };
                                    if ui.small_button(label).clicked() {
                                        requests.toggle_index = Some(i);
                                    }
                                }
                                if ui.small_button("Delete").clicked() {
                                    requests.delete_index = Some(i);
                                }
                            }
                        });
                        // Spell it out for something that just came out of
                        // generation -- easy to miss otherwise, since the
                        // toast announcing it fades after a few seconds. A
                        // spell has no enabled state to flag, so it's always
                        // worth pointing at Run while still "recent".
                        if is_recent && m.error.is_none() && is_host {
                            if m.is_instant {
                                ui.colored_label(
                                    IMPORTANT_TOAST_COLOR,
                                    "New -- click Run above to cast it",
                                );
                            } else if !m.enabled {
                                ui.colored_label(
                                    IMPORTANT_TOAST_COLOR,
                                    "New -- click Enable above to activate it",
                                );
                            }
                        }
                        if let Some(err) = &m.error {
                            ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
                        }
                    }
                });

            egui::Window::new("Resources")
                .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    for &block in COLLECTIBLE_BLOCKS.iter() {
                        let count = player.resource_count(block);
                        let is_selected = selected_block == Some(block);
                        ui.horizontal(|ui| {
                            let marker = if is_selected { "-> " } else { "" };
                            let color = if is_selected {
                                egui::Color32::from_rgb(120, 200, 255)
                            } else if count > 0 {
                                egui::Color32::WHITE
                            } else {
                                egui::Color32::GRAY
                            };
                            ui.colored_label(color, format!("{marker}{} x{count}", block.name()));
                            if ui
                                .add_enabled(count > 0, egui::Button::new("Select").small())
                                .clicked()
                            {
                                requests.select_block = Some(block);
                            }
                        });
                    }
                    ui.separator();
                    ui.label("Break blocks to gather them, right-click to place the selected one.");
                });

            // Read-only viewer for one rule's generated Lua -- so you can
            // actually see what the LLM wrote (or what a hand-written
            // module contains) instead of just trusting the ON/OFF/ERR
            // status. Stays open (tracking by index) until closed or the
            // module list shrinks out from under it.
            if let Some(vi) = viewing_index {
                match scripting.modules.get(vi) {
                    Some(m) => {
                        let mut open = true;
                        egui::Window::new(format!("Rule Source: {}", m.name))
                            .resizable(true)
                            .collapsible(false)
                            .default_width(560.0)
                            .default_height(420.0)
                            .open(&mut open)
                            .show(ctx, |ui| {
                                ui.label(format!("Prompt: {}", m.prompt));
                                if let Some(err) = &m.error {
                                    ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
                                }
                                ui.separator();
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    let mut code = m.source.clone();
                                    ui.add(
                                        egui::TextEdit::multiline(&mut code)
                                            .code_editor()
                                            .desired_width(f32::INFINITY),
                                    );
                                });
                            });
                        if !open {
                            viewing_index = None;
                        }
                    }
                    None => viewing_index = None,
                }
            }

            if !toasts.is_empty() {
                egui::Window::new("notifications")
                    .title_bar(false)
                    .anchor(egui::Align2::LEFT_BOTTOM, [8.0, -8.0])
                    .resizable(false)
                    .interactable(false)
                    .show(ctx, |ui| {
                        for t in toasts {
                            ui.colored_label(t.color, &t.text);
                        }
                    });
            }

            // Persistent scrollback: every toast-worthy system message
            // (rule generated/enabled/crashed, players joining, ...) plus
            // real player chat, so nothing is lost once its toast fades.
            // Sits above the toast strip; only takes screen space once
            // there's something to show or the player is actively typing.
            if chat_open || !chat_log.is_empty() {
                egui::Window::new("Chat")
                    .anchor(egui::Align2::LEFT_BOTTOM, [8.0, -170.0])
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.set_max_width(380.0);
                        egui::ScrollArea::vertical()
                            .max_height(if chat_open { 220.0 } else { 100.0 })
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for entry in chat_log {
                                    ui.colored_label(entry.color, &entry.text);
                                }
                            });
                        if chat_open {
                            ui.separator();
                            let response = ui.text_edit_singleline(chat_input);
                            if !response.has_focus() && !response.lost_focus() {
                                response.request_focus();
                            }
                            let submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if submitted && !chat_input.trim().is_empty() {
                                requests.send_chat = Some(chat_input.trim().to_string());
                            }
                            ui.label("Enter to send, Esc to close");
                        } else {
                            ui.label("T to chat");
                        }
                    });
            }

            if console_open {
                egui::Window::new("Rule Console")
                    .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.set_min_width(420.0);
                        if is_host {
                            ui.label("Describe a rule, or an instant action, then press Enter:");
                            let response = ui.text_edit_singleline(prompt_input);
                            if !response.has_focus() && !response.lost_focus() {
                                response.request_focus();
                            }
                            let submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if submitted && !prompt_input.trim().is_empty() {
                                requests.submit_prompt = Some(prompt_input.trim().to_string());
                            }
                            if let Some(status) = generation_status {
                                ui.label(status);
                            }
                        } else {
                            ui.label("Only the host can generate rules.");
                        }
                        ui.label("Esc to close");
                    });
            }

            if quit_dialog_open {
                egui::Window::new("Quit to Main Menu?")
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.label("Quit to the main menu?");
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Quit to Menu").clicked() {
                                requests.confirm_quit = true;
                            }
                            if ui.button("Cancel").clicked() {
                                requests.cancel_quit = true;
                            }
                        });
                    });
            }
        });

        self.viewing_index = viewing_index;
        (full_output, requests)
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        window: &Window,
        full_output: egui::FullOutput,
        size: [u32; 2],
    ) {
        self.state
            .handle_platform_output(window, full_output.platform_output);
        let clipped_primitives = self
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        for (id, delta) in &full_output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: full_output.pixels_per_point,
        };
        self.renderer.update_buffers(
            device,
            queue,
            encoder,
            &clipped_primitives,
            &screen_descriptor,
        );

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            self.renderer
                .render(&mut rpass, &clipped_primitives, &screen_descriptor);
        }

        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
