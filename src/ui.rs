use std::time::{Duration, Instant};

use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::player::Player;
use crate::scripting::ScriptHost;

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
    pub remember_index: Option<usize>,
    pub spellbook_action: Option<crate::spellbook_ui::Action>,
    pub open_spellbook: bool,
    pub close_workshop: bool,

    pub detach_rule: Option<usize>,
    pub eat_food: Option<crate::voxel::BlockType>,
    pub eat_harvest: Option<String>,
    pub camp_action: Option<crate::adventure::Action>,
    pub quest_action: Option<crate::quests::Action>,
    pub close_journal: bool,
    pub close_campfire: bool,
    pub automation: Option<crate::automation::Action>,
    pub close_inventory: bool,
    pub invite_friends: bool,
    pub crafting: Option<crate::crafting::Action>,
    pub toggle_index: Option<usize>,
    pub delete_index: Option<usize>,
    /// Set when the player clicks "Run" on an instant spell (`Module::is_instant`).
    pub run_index: Option<usize>,
    pub submit_prompt: Option<String>,
    pub cancel_generation: bool,
    pub approve_proposal: bool,
    pub reject_proposal: bool,
    pub confirm_quit: bool,
    pub cancel_quit: bool,
    /// Inventory assignment; highlighting/searching never changes the active item.
    pub assign_entry: Option<Option<crate::equipment::Entry>>,
    pub select_slot: Option<usize>,
    /// Set when the player submits a line from the chat box.
    pub send_chat: Option<String>,
}

pub struct Ui {
    pub spellbook: crate::spellbook_ui::Panel,
    pub spell_hud: Option<(String, bool)>,
    pub aimed_object: String,
    pub aimed_enchantments: Vec<String>,
    pub workshop_kind: crate::spell_workshop::Kind,
    pub compass_yaw: f32,
    pub machine_compass: [Option<egui::Pos2>; 5],
    pub journal: crate::adventure_ui::Journal,
    pub campfire: crate::campfire_ui::Panel,
    pub automation: crate::automation_ui::Panel,
    pickup_rows: Vec<(crate::voxel::BlockType, u32)>,
    pickup_started: Instant,
    #[cfg(feature = "dev-playtest")]
    pub agent_nameplate: Option<(egui::Pos2, String)>,
    #[cfg(feature = "dev-playtest")]
    pub agent_home_marker: Option<(egui::Pos2, &'static str)>,
    pub nameplates: Vec<(egui::Pos2, String)>,
    pub chat_bubbles: Vec<(egui::Pos2, String, f32)>,
    pub settings: crate::settings::SettingsPanel,
    ctx: egui::Context,
    state: State,
    renderer: Renderer,
    /// Index into `scripting.modules` of the rule whose source is currently
    /// shown in the "Rule Source" viewer window, if any. Purely local UI
    /// state -- doesn't need to round-trip through `App`.
    viewing_index: Option<usize>,
    pub map: crate::map_ui::Map,
    pub inventory_open: bool,
    pub workshop_selected: Option<usize>,
    last_hotbar: Option<(usize, Option<crate::equipment::Entry>)>,
    selected_until: Instant,
    inventory: crate::inventory_ui::Inventory,
}

impl Ui {
    pub fn show_pickup(&mut self, contents: Vec<(crate::voxel::BlockType, u32)>) {
        if contents.is_empty() {
            return;
        }
        if self.pickup_started.elapsed().as_secs_f32() >= 4.0 {
            self.pickup_rows.clear();
        }
        for (block, amount) in contents {
            if amount == 0 {
                continue;
            }
            if let Some((_, count)) = self.pickup_rows.iter_mut().find(|(b, _)| *b == block) {
                *count = count.saturating_add(amount);
            } else if self.pickup_rows.len() < 12 {
                self.pickup_rows.push((block, amount));
            }
        }
        self.pickup_started = Instant::now();
    }

    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat, window: &Window) -> Self {
        let ctx = egui::Context::default();
        let settings = crate::settings::SettingsPanel::new(&ctx);
        let state = State::new(ctx.clone(), egui::ViewportId::ROOT, window, None, None);
        let renderer = Renderer::new(device, output_format, None, 1);
        Self {
            spellbook: Default::default(),
            spell_hud: None,
            aimed_object: String::new(),
            aimed_enchantments: Vec::new(),
            workshop_kind: Default::default(),
            pickup_rows: Vec::new(),
            journal: Default::default(),
            campfire: Default::default(),
            automation: Default::default(),
            pickup_started: Instant::now(),
            nameplates: Vec::new(),
            compass_yaw: 0.,
            machine_compass: [None; 5],
            #[cfg(feature = "dev-playtest")]
            agent_nameplate: None,
            #[cfg(feature = "dev-playtest")]
            agent_home_marker: None,
            chat_bubbles: Vec::new(),
            settings,
            ctx,
            state,
            renderer,
            viewing_index: None,
            map: Default::default(),
            inventory_open: false,
            workshop_selected: None,
            last_hotbar: None,
            selected_until: Instant::now(),
            inventory: Default::default(),
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
    pub fn run(
        &mut self,
        window: &Window,
        contents: impl FnOnce(&egui::Context),
    ) -> egui::FullOutput {
        let raw_input = self.state.take_egui_input(window);
        self.ctx.run(raw_input, |ctx| {
            if self.settings.open {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if self.settings.values.appearance.ui_theme == crate::settings::UiTheme::Fantasy
                    {
                        crate::ui_theme::menu_backdrop(ui);
                    }
                });
                self.settings.draw(ctx);
            } else {
                contents(ctx);
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        window: &Window,
        console_open: bool,
        prompt_input: &mut String,
        is_host: bool,
        can_prompt: bool,
        scripting: &ScriptHost,
        recent_index: Option<usize>,
        generation_status: Option<&str>,
        toasts: &[Toast],
        fps: f32,
        quit_dialog_open: bool,
        player: &Player,
        chat_open: bool,
        chat_input: &mut String,
        chat_log: &[ChatEntry],
        crafting_ui: &mut crate::crafting_ui::CraftingUi,
        registry: &crate::crafting::Registry,
        world: &crate::voxel::World,
        creatures: &crate::creature::Creatures,
        players: &[glam::Vec3],
        lobby_code: Option<&str>,
        proposal_review: Option<(&str, &str, &str)>,
    ) -> (egui::FullOutput, UiRequests) {
        let raw_input = self.state.take_egui_input(window);
        let mut requests = UiRequests::default();
        let mut viewing_index = self.viewing_index;
        let mut open_settings = false;

        let full_output = self.ctx.run(raw_input, |ctx| {
            if let Some((requester, prompt, summary)) = proposal_review {
                egui::Window::new("Guest rule approval")
                    .collapsible(false)
                    .resizable(true)
                    .default_width(460.0)
                    .show(ctx, |ui| {
                        ui.strong(format!("Request from {requester}"));
                        ui.label(format!("Prompt: {prompt}"));
                        ui.label(format!("Validated rule: {summary}"));
                        ui.small("Validation passed. Nothing runs until approved.");
                        ui.horizontal(|ui| {
                            if ui.button("Approve").clicked() { requests.approve_proposal = true; }
                            if ui.button("Reject").clicked() { requests.reject_proposal = true; }
                        });
                    });
            }
            let age=self.pickup_started.elapsed().as_secs_f32();
            if !self.pickup_rows.is_empty() && age<4.0 {
                let enter=(age/0.2).clamp(0.0,1.0);
                let fade=((4.0-age)/0.7).clamp(0.0,1.0)*enter;
                egui::Area::new(egui::Id::new("loot_acquired"))
                    .anchor(egui::Align2::RIGHT_CENTER,egui::vec2(-24.0+(1.0-enter)*30.0,-40.0))
                    .interactable(false)
                    .show(ctx,|ui| {
                        ui.set_opacity(fade);
                        egui::Frame::none().fill(egui::Color32::from_rgba_unmultiplied(15,24,34,225))
                            .rounding(4.0).inner_margin(12.0).show(ui,|ui| {
                                ui.label(egui::RichText::new("Acquired").color(egui::Color32::from_rgb(140,225,255)).strong());
                                for &(block,amount) in &self.pickup_rows {
                                    ui.horizontal(|ui| {
                                        crate::resource_ui::icon(ui,block);
                                        ui.label(format!("+{amount} {}",block.name()));
                                    });
                                }
                            });
                    });
                ctx.request_repaint();
            }
            let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Background,egui::Id::new("player_names")));
            if !self.settings.open && !self.map.open && !self.journal.open && !console_open && !quit_dialog_open {
                crate::compass::hud(ctx,self.compass_yaw);
                if self.automation.open || self.automation.build.is_some() {crate::compass::machine(ctx,&self.machine_compass);}
            }
            #[cfg(feature = "dev-playtest")]
            if let Some((pos,label))=self.agent_home_marker {
                painter.circle_stroke(pos,7.0,egui::Stroke::new(2.0_f32,egui::Color32::LIGHT_BLUE));
                painter.text(pos-egui::vec2(0.0,10.0),egui::Align2::CENTER_BOTTOM,label,egui::FontId::proportional(14.0),egui::Color32::LIGHT_BLUE);
            }
            #[cfg(feature = "dev-playtest")]
            if let Some((pos, status)) = &self.agent_nameplate {
                let status_layout = painter.layout(status.clone(),egui::FontId::proportional(13.0),egui::Color32::from_gray(175),260.0);
                let status_pos = *pos-egui::vec2(status_layout.size().x*0.5,status_layout.size().y);
                let name_pos = *pos-egui::vec2(0.0,status_layout.size().y+3.0);
                painter.text(name_pos+egui::vec2(1.0,1.0),egui::Align2::CENTER_BOTTOM,"Agent1",egui::FontId::proportional(16.0),egui::Color32::BLACK);
                painter.text(name_pos,egui::Align2::CENTER_BOTTOM,"Agent1",egui::FontId::proportional(16.0),egui::Color32::from_rgb(255,65,65));
                painter.galley_with_override_text_color(status_pos+egui::vec2(1.0,1.0),status_layout.clone(),egui::Color32::BLACK);
                painter.galley(status_pos,status_layout,egui::Color32::from_gray(175));
            }
            for (pos,name) in &self.nameplates {
                painter.text(*pos+egui::vec2(1.0,1.0),egui::Align2::CENTER_BOTTOM,name,egui::FontId::proportional(16.0),egui::Color32::BLACK);
                painter.text(*pos,egui::Align2::CENTER_BOTTOM,name,egui::FontId::proportional(16.0),egui::Color32::WHITE);
            }
            for (anchor,text,opacity) in &self.chat_bubbles {
                let color = egui::Color32::WHITE.linear_multiply(*opacity);
                let line = crate::emoticons::layout(&painter,text,color,240.0);
                let top_left = *anchor-egui::vec2(line.galley.size().x*0.5,line.galley.size().y);
                let rect = egui::Rect::from_min_size(top_left,line.galley.size()).expand2(egui::vec2(8.0,6.0));
                painter.rect_filled(rect,6.0,egui::Color32::from_rgba_unmultiplied(20,25,32,225).linear_multiply(*opacity));
                line.paint(&painter,top_left,*opacity);
            }
            if self.settings.open {
                // No underlying controls run while the settings panel owns input.
                self.settings.draw(ctx);
                return;
            }
            if self.spellbook.open {
                requests.select_slot = crate::equipment_ui::hotbar_with_spells(
                    ctx, &player.crafting, true, true, false, &scripting.spellbook,
                );
                requests.spellbook_action=self.spellbook.draw(ctx,&scripting.spellbook,is_host,
                    registry.mana_charge(crate::crafting::INSTANT_MANA)==0);
                return;
            }
            if self.campfire.open {
                requests.camp_action = self.campfire.draw(ctx, player, world);
                requests.close_campfire = !self.campfire.open;
                return;
            }
            if self.journal.open {
                requests.camp_action=self.journal.draw(ctx,player);
                requests.quest_action=self.journal.quest_request.take();
                requests.close_journal=!self.journal.open;
                return;
            }
            self.map.home=player.crafting.adventure.home.map(crate::adventure::feet);
            if self.map.open {self.map.draw(ctx,world,player.position,self.compass_yaw);return;}
            requests.crafting = crafting_ui.draw(ctx, registry, &player.crafting, world, creatures, player.position, players);
            requests.automation = self.automation.draw(ctx,&world.automation,&player.crafting,registry);
            status_hud(ctx,player,fps);
            if !console_open && !chat_open && !quit_dialog_open && !crafting_ui.open && !self.inventory_open && !self.automation.open {
                self.journal.hud(ctx,player,self.map.waypoint);
                crate::enchantment::hud(ctx,&self.aimed_object,&self.aimed_enchantments);
            }

            let active=player.crafting.hotbar.active.min(8);
            let entry=player.crafting.hotbar.entry();
            if self.last_hotbar!=Some((active,entry)) {
                self.last_hotbar=Some((active,entry));self.selected_until=Instant::now()+Duration::from_secs(2);
            }
            if !console_open {
                requests.select_slot=crate::equipment_ui::hotbar_with_spells(ctx,&player.crafting,self.inventory_open || self.spellbook.open,self.inventory_open || self.spellbook.open || Instant::now()<self.selected_until,self.automation.tools_suspended(),&scripting.spellbook);
            }
            if !self.inventory_open && !self.spellbook.open && !console_open && !chat_open && !quit_dialog_open && !crafting_ui.open && !self.automation.tools_suspended() {
                if let Some((text,ready))=&self.spell_hud {
                    crate::equipment_ui::spell_hud(ctx,text,*ready);
                }
            }
            if self.inventory_open {
                self.inventory.feedback = crafting_ui.feedback.clone();
                self.inventory.show(ctx, &player.crafting, player.health, registry, &mut requests, &scripting.spellbook);
                if requests.crafting.is_some() {
                    if crafting_ui.pending {requests.crafting=None;}
                    else {crafting_ui.pending=true;}
                }
            }

            // Read-only viewer for one rule's generated Lua -- so you can
            // actually see what the LLM wrote (or what a hand-written
            // module contains) instead of just trusting the ON/OFF/ERR
            // status. Stays open (tracking by index) until closed or the
            // module list shrinks out from under it.
            if let Some(vi) = viewing_index {
                match scripting.modules.get(vi) {
                    Some(m) => {
                        let mut open = true;
                        egui::Window::new(format!("Spell Source: {}", m.name))
                            .resizable(true)
                            .collapsible(false)
                            .default_width(560.0)
                            .default_height(420.0)
                            .open(&mut open)
                            .show(ctx, |ui| {
                                ui.label(format!("Prompt: {}", m.prompt));
                                if let Some(meaning) = m.source.lines().find_map(|line| line.strip_prefix(crate::llm::intent::SUMMARY_PREFIX)) {
                                    ui.label(format!("Interpretation: {meaning}"));
                                }
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
                            crate::emoticons::label(ui,&t.text,t.color);
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
                                    crate::emoticons::label(ui,&entry.text,entry.color);
                                }
                            });
                        if chat_open {
                            ui.separator();
                            let response = ui.text_edit_singleline(chat_input);
                            if crate::emoticons::picker(ui,chat_input) {
                                response.request_focus();
                                if let Some(mut state) = egui::TextEdit::load_state(ctx,response.id) {
                                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                                        egui::text::CCursor::new(chat_input.chars().count()))));
                                    state.store(ctx,response.id);
                                }
                            }
                            if !response.has_focus() && !response.lost_focus() {
                                response.request_focus();
                            }
                            let submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if submitted && !chat_input.trim().is_empty() {
                                requests.send_chat = Some(chat_input.trim().to_string());
                            }
                            ui.label("Click an emoticon or type :smile: / :) — Enter to send, Esc to close");
                        } else {
                            ui.label("T to chat");
                        }
                    });
            }

            if console_open {
                crate::spell_workshop::draw(ctx, prompt_input, &mut self.workshop_kind,
                    is_host, can_prompt, scripting, recent_index,
                    generation_status, registry, &mut viewing_index, &mut self.workshop_selected, &mut requests);
            }
            if quit_dialog_open {
                egui::Window::new("Quit to Main Menu?")
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .resizable(false)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        if let Some(code) = lobby_code {
                            ui.label("Steam lobby - share this code with your friends:");
                            ui.label(code);
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Copy lobby code").clicked() { ui.output_mut(|output| output.copied_text = code.to_owned()); }
                                if ui.button("Invite Steam friends").clicked() { requests.invite_friends = true; }
                            });
                            ui.separator();
                        }
                        if ui.button("Settings").clicked() { open_settings = true; }
                        ui.separator();
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

        if open_settings {
            self.settings.open = true;
        }
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

/// Position status below the actual shortcut-panel bounds in either theme.
pub(crate) fn status_hud(ctx: &egui::Context, player: &Player, fps: f32) {
    let hints = egui::Window::new("fps")
        .title_bar(false)
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 56.0])
        .resizable(false)
        .collapsible(false)
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "{fps:.0} FPS | I: Inventory | C: Craft | M: Map | F10: Settings"
            ));
                                    ui.label("J: Field journal · Gestures: , Hello | . Dance | / Angry");
            ui.label("B: Automation · F: Configure device");
        });

    egui::Window::new("health")
        .title_bar(false)
        .anchor(
            egui::Align2::RIGHT_TOP,
            [
                -8.0,
                hints
                    .as_ref()
                    .map_or(110.0, |h| h.response.rect.bottom() + 10.0),
            ],
        )
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
            if player.satiety <= crate::hunger::HUNGRY {
                let hunger_color = if player.satiety <= crate::hunger::STARVING {
                    egui::Color32::from_rgb(220, 90, 90)
                } else {
                    egui::Color32::from_rgb(230, 190, 90)
                };
                ui.colored_label(
                    hunger_color,
                    if player.satiety <= crate::hunger::STARVING {
                        "HUNGRY - eat to stop health loss".to_string()
                    } else {
                        "HUNGRY".to_string()
                    },
                );
            }
            if player.wetness > 0.05 {
                ui.colored_label(
                    egui::Color32::from_rgb(120, 190, 230),
                    format!("Wet: {:.0}%", player.wetness * 100.0),
                );
            }
            if player.poisoned {
                ui.colored_label(egui::Color32::from_rgb(140, 210, 100), "POISONED");
            }
            // Only shown while it's actually relevant (currently
            // draining, or still catching back up) rather than
            // permanently cluttering the HUD on dry land.
            if player.oxygen < 100.0 {
                let oxygen_color = if player.oxygen <= 0.0 {
                    egui::Color32::from_rgb(220, 90, 90)
                } else {
                    egui::Color32::from_rgb(120, 190, 230)
                };
                ui.colored_label(oxygen_color, format!("Oxygen: {:.0}/100", player.oxygen));
            }
        });
}
