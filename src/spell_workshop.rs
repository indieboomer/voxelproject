//! Creation and management share one workshop; gameplay actions remain host requests.
use crate::{scripting::ScriptHost, ui::UiRequests};
use egui::{Color32, RichText};

const GOLD: Color32 = Color32::from_rgb(236, 192, 99);

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
    enchant: &mut bool,
    target: &str,
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
                    heading(ui, "2. TARGET (OPTIONAL)", "target_help",
                        "The target is selected when you open the workshop. Close it, aim at an object, and reopen to choose another target. Instant spells can use a target without becoming enchantments.");
                    frame().show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new("Target locked").color(GOLD));
                        ui.label(if target.is_empty() { "No target selected" } else { target });
                        ui.horizontal_wrapped(|ui| {
                            ui.add_enabled(is_host && (!target.is_empty() || *enchant) && status.is_none(),
                                egui::Checkbox::new(enchant, "Enchant this object"));
                            help(ui, "enchantment_help", "An enchantment binds a permanent spell to this object. Review the code, then choose Enchant + enable. Losing the object stops future effects; earlier world changes remain. Packing a device ends its enchantments.");
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(status.is_none(), egui::Button::new("Clear").min_size(egui::vec2(80.0, 38.0))).clicked() {
                            prompt.clear();
                            *enchant = false;
                        }
                        let ready = can_prompt && status.is_none() && !prompt.trim().is_empty()
                            && (!*enchant || (is_host && !target.is_empty()));
                        let generate = ui.add_enabled(ready,
                            egui::Button::new(RichText::new("Generate Spell").strong())
                                .fill(Color32::from_rgb(27, 86, 44)).min_size(egui::vec2(150.0, 38.0))).clicked();
                        if ready && (generate || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))) {
                            requests.submit_prompt = Some(prompt.trim().to_owned());
                        }
                        help(ui, "generate_help", &format!("Creation costs {} mana on success. Instant spells cost {} mana per successful cast. Failed generation and casts are free. Generated spells need review before activation. Guest spells are sent to the host for approval.", registry.mana_charge(crate::crafting::RULE_MANA), registry.mana_charge(crate::crafting::INSTANT_MANA)));
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
                            ui.label("Write a prompt, choose an optional target, and generate your spell.");
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
                                        let (kind, color) = if module.attachment.is_some() || module.attachment_candidate.is_some() {
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
                                        if module.is_instant {
                                            if ui.button("Run").clicked() { requests.run_index = Some(i); }
                                            if ui.button("Remember").clicked() { requests.remember_index = Some(i); }
                                        } else {
                                            let label = if module.enabled { "ON · Disable" } else { "OFF · Enable" };
                                            if ui.add_enabled(module.attachment_candidate.is_none(), egui::Button::new(label)).clicked() { requests.toggle_index = Some(i); }
                                            if module.attachment.is_none() {
                                                if ui.add_enabled(module.attachment_candidate.is_some() || !target.is_empty(), egui::Button::new("Enchant + enable")).clicked() { requests.attach_rule = Some(i); }
                                            } else if ui.button("Remove enchantment").clicked() { requests.detach_rule = Some(i); }
                                        }
                                    } else { ui.small(if module.enabled { "ON" } else { "OFF" }); }
                                    if ui.small_button(if *viewing == Some(i) { "Hide Code" } else { "View Code" }).clicked() {
                                        *viewing = if *viewing == Some(i) { None } else { Some(i) };
                                    }
                                    if is_host && ui.small_button(RichText::new("Delete").color(Color32::LIGHT_RED)).clicked() { requests.delete_index = Some(i); }
                                });
                                if let Some(binding) = &module.attachment {
                                    ui.small(format!("Enchanting {}", binding.target.label()));
                                    if let Some(lost) = &binding.lost { ui.colored_label(Color32::LIGHT_RED, lost); }
                                } else if let Some(target) = &module.attachment_candidate {
                                    ui.small(format!("Review enchantment target: {}", target.label()));
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
