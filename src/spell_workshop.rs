//! Creation and management share one workshop; gameplay actions remain host requests.
use crate::{scripting::ScriptHost, ui::UiRequests};
use egui::{Color32, RichText};

const GOLD: Color32 = Color32::from_rgb(236, 192, 99);

/// Capture the definition when Comment is clicked, so list changes cannot retarget a draft.
pub struct SpellComment {
    pub name: String,
    pub prompt: String,
    pub source: String,
    pub saved_spells: Vec<(u64, u32)>,
    pub text: String,
    focus: bool,
}

impl SpellComment {
    fn new(module: &crate::scripting::Module, scripting: &ScriptHost) -> Self {
        Self {
            name: module.name.clone(), prompt: module.prompt.clone(), source: module.source.clone(),
            saved_spells: scripting.spellbook.spells.iter().filter(|s| s.source == module.source)
                .map(|s| (s.id, s.revision)).collect(),
            text: String::new(), focus: true,
        }
    }

    pub fn chat_line(&self, code_id: &str) -> String {
        // Leave room for the spell identity inside the existing 512-character chat limit.
        let name: String = self.name.chars().filter(|c| !c.is_control()).take(64).collect();
        let text: String = self.text.trim().chars().take(360)
            .map(|c| if c.is_control() { ' ' } else { c }).collect();
        format!("Spell comment [code {code_id}] \"{name}\": {text}")
    }
}

pub fn draw_comment(ctx: &egui::Context, draft: &mut Option<SpellComment>, requests: &mut UiRequests) {
    let Some(comment) = draft.as_mut() else { return; };
    let mut open = true;
    let mut submit = false;
    let mut cancel = false;
    egui::Window::new("Comment on spell").id(egui::Id::new("spell_comment"))
        .open(&mut open).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width((ctx.screen_rect().width() - 48.0).clamp(240.0, 520.0))
        .collapsible(false).resizable(true).show(ctx, |ui| {
            ui.heading(&comment.name);
            ui.label("What did you expect, and what actually happened?");
            let response = ui.add(egui::TextEdit::multiline(&mut comment.text).char_limit(360)
                .desired_rows(5).desired_width(f32::INFINITY)
                .hint_text("Expected... Observed..."));
            if comment.focus { response.request_focus(); comment.focus = false; }
            ui.small(format!("{} / 360 characters", comment.text.chars().count()));
            ui.small("Posts to session chat with this spell attached. F5 saves the host's chat log.");
            ui.horizontal(|ui| {
                let ready = !comment.text.trim().is_empty();
                submit = ui.add_enabled(ready, egui::Button::new("Post comment")).clicked()
                    || (ready && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)));
                cancel = ui.button("Cancel").clicked();
            });
        });
    if submit { requests.submit_spell_comment = draft.take(); }
    else if !open || cancel { *draft = None; }
}

#[cfg(test)]
mod comment_tests {
    use super::*;

    #[test]
    fn comment_keeps_original_spell_after_list_changes_and_fits_chat() {
        let mut scripting = ScriptHost::new();
        let source = "function on_cast(api, event) end";
        let module = crate::scripting::Module::load("First spell".into(), "Original prompt".into(), source.into()).unwrap();
        let id = scripting.spellbook.remember(&module, "Tester").unwrap();
        let mut draft = SpellComment::new(&module, &scripting);
        scripting.add_generated(module).unwrap();
        scripting.remove(0);
        scripting.add_generated(crate::scripting::Module::load("Replacement".into(), "Different prompt".into(), source.into()).unwrap()).unwrap();
        assert_eq!(draft.name, "First spell");
        assert_eq!(draft.prompt, "Original prompt");
        assert_eq!(draft.source, source);
        assert_eq!(draft.saved_spells[0].0, id);
        draft.name = "猫".repeat(100);
        draft.text = "ż".repeat(360);
        let line = draft.chat_line("0123456789abcdef");
        assert!(line.chars().count() <= crate::net::MAX_CHAT_LEN);
        assert_eq!(crate::net::chat_text(&line), Some(line.clone()));
        assert!(line.ends_with(&draft.text));
    }

    #[test]
    fn comment_editor_rejects_blank_and_posts_once() {
        let ctx = egui::Context::default();
        let scripting = ScriptHost::new();
        let module = crate::scripting::Module::load("Heal".into(), "Heal sheep".into(), "function on_cast(api, event) end".into()).unwrap();
        let mut draft = Some(SpellComment::new(&module, &scripting));
        let mut requests = UiRequests::default();
        for text in ["   ", "Expected healing; health stayed unchanged."] {
            draft.as_mut().unwrap().text = text.into();
            let modifiers = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280., 720.))),
                modifiers,
                events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers }],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| draw_comment(ctx, &mut draft, &mut requests));
            if text.trim().is_empty() { assert!(requests.submit_spell_comment.is_none()); assert!(draft.is_some()); }
        }
        let posted = requests.submit_spell_comment.take().unwrap();
        assert_eq!(posted.name, "Heal");
        assert_eq!(posted.text.trim(), "Expected healing; health stayed unchanged.");
        assert!(draft.is_none());
        let _ = ctx.run(Default::default(), |ctx| draw_comment(ctx, &mut draft, &mut requests));
        assert!(requests.submit_spell_comment.is_none());
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind { #[default] Auto, Instant, Permanent, Enchantment }
impl Kind {
    pub fn prepare(self, prompt: &str) -> (crate::llm::PromptKind, String) {
        use crate::llm::PromptKind;
        match self {
            Self::Auto => (crate::llm::classify_prompt(prompt), prompt.into()),
            Self::Instant => (PromptKind::Instant, prompt.into()),
            Self::Permanent => (PromptKind::Rule, prompt.into()),
            Self::Enchantment => (PromptKind::Rule, format!("{prompt}\n[BOUND_OBJECT_RULE] Reusable enchantment template. No object is selected yet. In on_tick use api.get_rule_target(); return if nil. The target is chosen on each hotbar cast. Never hard-code a concrete object ID or coordinates. Use target.kind (creature, block, device); creature identity is target.id. Guard unsupported kinds. Include -- spell_target: creature, block, device, or any to declare supported targets. The engine stops an instance when its object disappears or is replaced.")),
        }
    }
}

fn frame() -> egui::Frame {
    egui::Frame::none()
        .fill(Color32::from_rgb(18, 24, 33))
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(126, 104, 65)))
        .inner_margin(10.0)
        .rounding(3.0)
}

fn help(ui: &mut egui::Ui, id: &str, text: &str) {
    let response = ui.small_button(RichText::new("?").color(GOLD));
    let popup = ui.make_persistent_id(id);
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup));
    }
    egui::popup::popup_below_widget(ui, popup, &response, |ui| {
        ui.set_max_width(290.0);
        ui.label(text);
    });
}

fn heading(ui: &mut egui::Ui, title: &str, id: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(GOLD));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            help(ui, id, hint);
        });
    });
    ui.add_space(6.0);
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    prompt: &mut String,
    spell_kind: &mut Kind,
    is_host: bool,
    can_prompt: bool,
    scripting: &ScriptHost,
    recent: Option<usize>,
    status: Option<&str>,
    registry: &crate::crafting::Registry,
    viewing: &mut Option<usize>,
    selected: &mut Option<usize>,
    requests: &mut UiRequests,
) {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 48.0).clamp(280.0, 1240.0);
    // The workshop is intentionally compact: the spell list and creation
    // controls remain scrollable, while the window leaves the world visible.
    // Keep the whole window at roughly two thirds of the previous height.
    let height = ((screen.height() - 160.0) * (2.0 / 3.0)).max(220.0);
    let mut open = true;
    egui::Window::new("Spell Workshop")
        .id(egui::Id::new("spell_workshop"))
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(width, height + 70.0))
        .max_height(height + 70.0)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            // Keep the workshop dense enough to review spells alongside the editor,
            // even when the fantasy theme uses larger fonts elsewhere.
            for (style, size) in [(egui::TextStyle::Body, 16.0), (egui::TextStyle::Button, 16.0), (egui::TextStyle::Small, 13.0)] {
                if let Some(font) = ui.style_mut().text_styles.get_mut(&style) { font.size = size; }
            }
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
            ui.visuals_mut().override_text_color = Some(Color32::from_rgb(233, 226, 206));
            ui.horizontal(|ui| {
                ui.label("Describe a spell and let magic shape the world.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").clicked() {
                        requests.close_workshop = true;
                    }
                    if ui.button("Open Spellbook [K]").clicked() {
                        requests.open_spellbook = true;
                    }
                });
            });
            ui.separator();
            // Both columns scroll independently, including at small window sizes.
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical().id_source("workshop_creation").max_height(height)
                    .auto_shrink([false, false]).show(&mut columns[0], |ui| {
                    heading(ui, "1. WRITE YOUR PROMPT", "prompt_help",
                        "Describe what should happen, when, and to whom. An instant spell runs once; a permanent spell keeps responding to world events. Enter adds a new line; Ctrl+Enter generates.");
                    ui.add_enabled_ui(can_prompt && status.is_none(), |ui| {
                        ui.add_sized([ui.available_width(), (height * 0.20).max(70.0)],
                            egui::TextEdit::multiline(prompt).hint_text("Make bluebells grow around this tree when it rains...").desired_width(f32::INFINITY));
                    });
                    ui.add_space(8.0);
                    heading(ui, "2. SPELL TYPE", "target_help",
                        "Enchantments are reusable spells. Remember one, assign it to the hotbar, then aim and left-click to attach an instance. No target is needed here.");
                    frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add_enabled_ui(status.is_none(), |ui| { ui.horizontal_wrapped(|ui| {
                            for (kind, label) in [(Kind::Auto, "Auto"), (Kind::Instant, "Instant"), (Kind::Permanent, "Permanent"), (Kind::Enchantment, "Enchantment")] {
                                ui.selectable_value(spell_kind, kind, label);
                            }
                        }); });
                        if *spell_kind == Kind::Enchantment {
                            ui.small("Remember, assign to hotbar, then aim and left-click. Target chosen when casting.");
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(status.is_none(), egui::Button::new("Clear").min_size(egui::vec2(80.0, 38.0))).clicked() {
                            prompt.clear();
                            *spell_kind = Kind::Auto;
                        }
                        let ready = can_prompt && status.is_none() && !prompt.trim().is_empty();
                        let generate = ui.add_enabled(ready,
                            egui::Button::new(RichText::new("Generate Spell").strong())
                                .fill(Color32::from_rgb(27, 86, 44)).min_size(egui::vec2(150.0, 38.0))).clicked();
                        if ready && (generate || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))) {
                            requests.submit_prompt = Some(prompt.trim().to_owned());
                        }
                        help(ui, "generate_help", &format!("Permanent rules cost {} mana to create. Enchantment templates are free to generate and remember; each successful attachment costs {} mana. Instant spells cost {} mana per successful cast. Failed generation and casts are free. Guest spells are sent to the host for approval.", registry.mana_charge(crate::crafting::RULE_MANA), registry.mana_charge(crate::crafting::RULE_MANA), registry.mana_charge(crate::crafting::INSTANT_MANA)));
                    });
                    ui.add_space(10.0);
                    heading(ui, "3. STATUS", "status_help", "Generation and validation updates appear here. Review the resulting spell in the list before running or enabling it. Errors are shown on the affected spell.");
                    frame().show(ui, |ui| {
                        ui.set_min_size(egui::vec2(ui.available_width(), 72.0));
                        if let Some(status) = status {
                            ui.horizontal(|ui| { ui.spinner(); ui.strong("Creating your spell..."); });
                            ui.label(status);
                            ui.add_space(8.0);
                            if ui.button("Cancel generation").clicked() { requests.cancel_generation = true; }
                        } else if !can_prompt {
                            ui.label("Guest prompting is disabled by the host.");
                        } else if let Some(module) = recent.and_then(|i| scripting.modules.get(i)) {
                            ui.colored_label(GOLD, format!("Latest spell: {}", module.name));
                            if let Some(error) = &module.error { ui.colored_label(Color32::LIGHT_RED, error); }
                            else { ui.label("Ready to review in your created spells."); }
                        } else {
                            ui.strong("Ready to create");
                            ui.label("Write a prompt, choose a spell type, and generate your spell.");
                        }
                    });
                });
                egui::ScrollArea::vertical().id_source("workshop_spells").max_height(height)
                    .auto_shrink([false, false]).show(&mut columns[1], |ui| {
                    heading(ui, &format!("YOUR CREATED SPELLS ({})", scripting.modules.len()), "spells_help",
                        "Newest spells appear first. Instant spells run once and can be remembered in the Spellbook. Permanent spells stay active until disabled. Enchantments bind their effects to an object.");
                    if scripting.modules.is_empty() {
                        frame().show(ui, |ui| { ui.label("Your first spell starts with an idea."); });
                    }
                    for (i, module) in scripting.modules.iter().enumerate().rev() {
                        ui.push_id(i, |ui| {
                            let mut card = frame();
                            if recent == Some(i) || *selected == Some(i) { card.stroke = egui::Stroke::new(2.0_f32, GOLD); }
                            let card_response = card.show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    crate::spell_art::image(ui, module.artwork, egui::Vec2::splat(56.0));
                                    ui.vertical(|ui| {
                                        if ui.selectable_label(*selected == Some(i), RichText::new(&module.name).color(GOLD).strong()).clicked() {
                                            *selected = Some(i);
                                        }
                                        let description = module.source.lines().find_map(|line| line.strip_prefix(crate::llm::intent::SUMMARY_PREFIX))
                                            .unwrap_or(&module.prompt);
                                        ui.add(egui::Label::new(RichText::new(description).small()).wrap(true));
                                        let (kind, color) = if module.attachment.is_some() || crate::spellbook::is_enchantment_source(&module.source) {
                                            ("Enchantment", Color32::LIGHT_BLUE)
                                        } else if module.is_instant { ("Instant", Color32::from_rgb(206, 163, 240)) }
                                        else { ("Permanent", Color32::LIGHT_GREEN) };
                                        ui.colored_label(color, kind);
                                    });
                                });
                                ui.horizontal_wrapped(|ui| {
                                    if module.is_instant {
                                        ui.small(format!("Mana cost: {}", registry.mana_charge(crate::crafting::INSTANT_MANA)));
                                    }
                                    if is_host {
                                        if crate::spellbook::is_enchantment_source(&module.source) && module.attachment.is_none() {
                                            ui.small(format!("Targets: {}", crate::spellbook::target_requirement(&module.source).label()));
                                            ui.small(format!("Mana per attachment: {}", registry.mana_charge(crate::crafting::RULE_MANA)));
                                            if ui.button("Remember").clicked() { requests.remember_index = Some(i); }
                                        } else if module.is_instant {
                                            if ui.button("Run").clicked() { requests.run_index = Some(i); }
                                            if ui.button("Remember").clicked() { requests.remember_index = Some(i); }
                                        } else {
                                            let label = if module.enabled { "ON · Disable" } else { "OFF · Enable" };
                                            if ui.button(label).clicked() { requests.toggle_index = Some(i); }
                                            if module.attachment.is_some() && ui.button("Remove enchantment").clicked() { requests.detach_rule = Some(i); }
                                        }
                                    } else { ui.small(if module.enabled { "ON" } else { "OFF" }); }
                                    if ui.small_button(if *viewing == Some(i) { "Hide Code" } else { "View Code" }).clicked() {
                                        *viewing = if *viewing == Some(i) { None } else { Some(i) };
                                    }
                                    if is_host && ui.small_button(RichText::new("Delete").color(Color32::LIGHT_RED)).clicked() { requests.delete_index = Some(i); }
                                    if ui.small_button("Comment").clicked() {
                                        requests.edit_spell_comment = Some(SpellComment::new(module, scripting));
                                    }
                                });
                                if let Some(binding) = &module.attachment {
                                    ui.small(format!("Enchanting {}", binding.target.label()));
                                    if let Some(lost) = &binding.lost { ui.colored_label(Color32::LIGHT_RED, lost); }
                                }
                                if let Some(error) = &module.error { ui.colored_label(Color32::LIGHT_RED, error); }
                            }).response;
                            if card_response.clicked() {
                                *selected = Some(i);
                            }
                            ui.add_space(6.0);
                        });
                    }
                });
            });
        });
    requests.close_workshop |= !open;
}
