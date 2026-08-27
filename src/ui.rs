use std::time::{Duration, Instant};

use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::scripting::ScriptHost;

const TOAST_LIFETIME: Duration = Duration::from_secs(6);
/// Longer-lived than a normal toast, since it flags something the player
/// actually needs to notice and act on (e.g. "go click Enable") rather than
/// an FYI that's fine to miss.
const IMPORTANT_TOAST_LIFETIME: Duration = Duration::from_secs(14);
const IMPORTANT_TOAST_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 60);

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

/// What the player asked the UI to do this frame; `App` acts on these after
/// the egui pass since the widget closures can't safely call back into game
/// state directly.
#[derive(Default)]
pub struct UiRequests {
    pub toggle_index: Option<usize>,
    pub delete_index: Option<usize>,
    pub submit_prompt: Option<String>,
    pub confirm_quit: bool,
    pub cancel_quit: bool,
}

pub struct Ui {
    ctx: egui::Context,
    state: State,
    renderer: Renderer,
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
    ) -> (egui::FullOutput, UiRequests) {
        let raw_input = self.state.take_egui_input(window);
        let mut requests = UiRequests::default();

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

            egui::Window::new("Rules")
                .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    if scripting.modules.is_empty() {
                        ui.label("No rules loaded. Press ~ to describe one.");
                    }
                    for (i, m) in scripting.modules.iter().enumerate() {
                        let (status, status_color) = if m.error.is_some() {
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
                            if is_host {
                                let label = if m.enabled { "Disable" } else { "Enable" };
                                if ui.small_button(label).clicked() {
                                    requests.toggle_index = Some(i);
                                }
                                if ui.small_button("Delete").clicked() {
                                    requests.delete_index = Some(i);
                                }
                            }
                        });
                        // Spell it out for a rule that just came out of
                        // generation and is still sitting disabled -- easy
                        // to miss otherwise, since the toast announcing it
                        // fades after a few seconds.
                        if is_recent && !m.enabled && m.error.is_none() && is_host {
                            ui.colored_label(
                                IMPORTANT_TOAST_COLOR,
                                "New -- click Enable above to activate it",
                            );
                        }
                        if let Some(err) = &m.error {
                            ui.colored_label(egui::Color32::from_rgb(220, 90, 90), err);
                        }
                    }
                });

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

            if console_open {
                egui::Window::new("Rule Console")
                    .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.set_min_width(420.0);
                        if is_host {
                            ui.label("Describe a rule for the world, then press Enter:");
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
