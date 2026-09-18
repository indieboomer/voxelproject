//! Focused campfire interaction panel. Adventure contracts remain in the Journal.
use crate::{adventure, adventure::Cell, player::Player};
use egui::{Color32, RichText};

#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub camp: Option<Cell>,
    pub feedback: String,
    pub cooking_seconds: f32,
    cooking_started: Option<std::time::Instant>,
    pending_cook: Option<adventure::Action>,
    fuel_level: Option<f32>,
    fuel_camp: Option<Cell>,
    fuel_updated_at: Option<std::time::Instant>,
    fuel_selection: Option<crate::voxel::BlockType>,
    cook_selection: Option<String>,
    ingredients: Vec<String>,
}

impl Panel {
    pub fn draw(&mut self, ctx: &egui::Context, player: &Player, world: &crate::voxel::World) -> Option<adventure::Action> {
        if !self.open { return None; }
        let Some(camp) = self.camp else { return None; };
        if self.cooking_started.is_some() {
            // Keep the progress timer advancing even when no mouse/keyboard event arrives.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let finished = if let Some(started) = self.cooking_started {
            self.cooking_seconds = started.elapsed().as_secs_f32().min(10.0);
            self.cooking_seconds >= 10.0
        } else { false };
        let mut open = true;
        let mut close_requested = false;
        let mut action = None;
        if finished {
            self.cooking_started = None;
            self.cooking_seconds = 0.0;
            action = self.pending_cook.take().map(|queued| queued.with_revision(player.crafting.revision));
            self.ingredients.clear();
            if action.is_some() {
                self.feedback = "Cooking complete. Adding the dish to your inventory…".into();
            } else {
                self.feedback = "Cooking finished without an ingredient to process.".into();
            }
        }
        egui::Window::new("Campfire")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2((ctx.screen_rect().width() - 24.0).min(1180.0), (ctx.screen_rect().height() - 8.0).min(1400.0)))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(Color32::from_rgb(235, 226, 207));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("CAMPFIRE").size(27.0).strong().color(Color32::from_rgb(247, 194, 83)));
                    ui.label("Fire, food and rest.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").clicked() { close_requested = true; }
                    });
                });
                ui.separator();
                let keeper = adventure::has_keeper(world, camp);
                if self.fuel_camp != Some(camp) {
                    self.fuel_camp = Some(camp);
                    self.fuel_level = None;
                    self.fuel_updated_at = Some(std::time::Instant::now());
                }
                let fuel_level = self.fuel_level.get_or_insert(if keeper { 1.0 } else { 0.65 });
                if keeper { *fuel_level = 1.0; }
                if !keeper {
                    let now = std::time::Instant::now();
                    let elapsed = self.fuel_updated_at.replace(now).map_or(0.0, |t| t.elapsed().as_secs_f32());
                    // A normal fire consumes a full bar over three minutes. The campkeeper fire is infinite.
                    *fuel_level = (*fuel_level - elapsed / 180.0).max(0.0);
                } else {
                    self.fuel_updated_at = Some(std::time::Instant::now());
                }
                ui.columns(2, |columns| {
                    columns[0].vertical_centered(|ui| {
                        ui.add_space(18.0);
                        ui.label(RichText::new("🔥").size(96.0));
                        ui.heading(RichText::new("Burning").color(Color32::from_rgb(247, 194, 83)));
                        ui.label("Warmth, food and better tomorrows.");
                        ui.add_space(18.0);
                        ui.label("Fuel");
                        ui.add(egui::ProgressBar::new(*fuel_level).text(if keeper { "100% · ∞ burn time".to_string() } else { format!("{:.0}% · burn time", *fuel_level * 100.0) }));
                        ui.add_space(14.0);
                        ui.label("Cooking");
                        ui.add(egui::ProgressBar::new(self.cooking_seconds.clamp(0.0, 10.0) / 10.0).text(if self.cooking_seconds > 0.0 { format!("Cooking… ({:.0}s)", (10.0 - self.cooking_seconds).ceil()) } else { "Ready".to_string() }));
                        ui.add_space(18.0);
                        if ui.button("Rest and set recovery camp").clicked() { action = Some(adventure::Action::Rest { camp }); }
                        if !self.feedback.is_empty() { ui.colored_label(Color32::LIGHT_YELLOW, &self.feedback); }
                    });
                    egui::ScrollArea::vertical().max_height(ctx.screen_rect().height() - 130.0).show(&mut columns[1], |ui| {
                        panel(ui, "Add Fuel", "Choose wood, coal, or gathered foliage from your inventory.", |ui| {
                            let fuel_items: Vec<_> = crate::voxel::COLLECTIBLE_BLOCKS.iter().copied().filter(|&block| {
                                let name = block.name().to_lowercase();
                                adventure::count(&player.crafting, block) > 0 && (crate::resource_defs::definition(block).fuel_value > 0 || name.contains("leaves") || name.contains("flower") || name.contains("herb") || name.contains("plant"))
                            }).collect();
                            let selected = self.fuel_selection.or_else(|| fuel_items.first().copied());
                            egui::ComboBox::from_id_source("campfire_fuel").selected_text(selected.map_or("No fuel available".into(), |b| format!("{}  ×{}", b.name(), adventure::count(&player.crafting, b)))).show_ui(ui, |ui| {
                                for block in &fuel_items { ui.selectable_value(&mut self.fuel_selection, Some(*block), format!("{}  ×{}", block.name(), adventure::count(&player.crafting, *block))); }
                            });
                            if ui.add_enabled(!keeper && selected.is_some(), egui::Button::new("Add")).clicked() { let block = selected.unwrap(); let value = crate::resource_defs::definition(block).fuel_value.max(1) as f32; *fuel_level = (*fuel_level + value * 0.05).min(1.0); self.fuel_updated_at = Some(std::time::Instant::now()); self.fuel_selection = None; self.feedback = format!("Added {} to the fire.", block.name()); }
                        });
                        panel(ui, "Light / Extinguish", "Extinguishing preserves the remaining fuel.", |ui| {
                            if keeper { ui.add_enabled(false, egui::Button::new("Campkeeper fire · ∞ fuel")); }
                            else if ui.button("Put Out Fire").clicked() { self.feedback = "The remaining fuel is preserved.".into(); }
                        });
                        panel(ui, "Cooking", "Choose ingredients from your inventory. Cooking takes about 10 seconds.", |ui| {
                            let account = &player.crafting;
                            let mut cookable: Vec<(String, String, u32)> = crate::voxel::COLLECTIBLE_BLOCKS.iter().copied().filter(|&block| adventure::count(account, block) > 0 && crate::resource_defs::definition(block).cookable).map(|block| (format!("block:{block:?}"), block.name().to_string(), adventure::count(account, block))).collect();
                            for raw in ["harvest:egg", "harvest:milk", "harvest:honey"] { if let Some(&count) = account.production_goods.get(raw) { if count > 0 { cookable.push((raw.into(), crate::food::harvest_name(raw), count)); } } }
                            let selected = self.cook_selection.clone().or_else(|| cookable.first().map(|item| item.0.clone()));
                            let selected_display = selected.as_ref().and_then(|key| cookable.iter().find(|item| &item.0 == key)).map_or("No cookable ingredients".into(), |item| format!("{}  ×{}", item.1, item.2));
                            egui::ComboBox::from_id_source("campfire_cooking").selected_text(selected_display).show_ui(ui, |ui| {
                                for (key, label, count) in &cookable { ui.selectable_value(&mut self.cook_selection, Some(key.clone()), format!("{}  ×{}", label, count)); }
                            });
                            ui.horizontal(|ui| {
                                let selected_already_added = selected.as_ref().map_or(0usize, |key| self.ingredients.iter().filter(|ingredient| *ingredient == key).count());
                                let selected_owned = selected.as_ref().and_then(|key| cookable.iter().find(|item| &item.0 == key)).map_or(0, |item| item.2);
                                if ui.add_enabled(selected.is_some() && self.ingredients.len() < 4 && selected_already_added < selected_owned as usize, egui::Button::new("Add")).clicked() { self.ingredients.push(selected.clone().unwrap()); }
                                if ui.add_enabled(!self.ingredients.is_empty(), egui::Button::new("Clear")).clicked() { self.ingredients.clear(); }
                            });
                            ui.label(format!("Ingredients: {} / 4", self.ingredients.len()));
                            for ingredient in &self.ingredients { ui.small(cookable.iter().find(|item| &item.0 == ingredient).map_or(ingredient.as_str(), |item| item.1.as_str())); }
                            let selected_count = selected.as_ref().and_then(|key| cookable.iter().find(|item| &item.0 == key).map(|item| item.2)).unwrap_or(0);
                            ui.label(format!("Selected ingredient  ×{selected_count}"));
                            let single_action = selected.as_ref().and_then(|key| match key.as_str() {
                                "block:Meat" => Some(adventure::Action::Cook { camp, amount: 1, revision: account.revision }),
                                "block:Pumpkin" => Some(adventure::Action::CookHarvest { camp, item: "pumpkin".into(), amount: 1, revision: account.revision }),
                                "block:BrownMushroom" => Some(adventure::Action::CookHarvest { camp, item: "brown_mushroom".into(), amount: 1, revision: account.revision }),
                                "block:Glowcap" => Some(adventure::Action::CookHarvest { camp, item: "glowcap".into(), amount: 1, revision: account.revision }),
                                raw if raw.starts_with("harvest:") => Some(adventure::Action::CookHarvest { camp, item: raw.to_string(), amount: 1, revision: account.revision }),
                                _ => None,
                            });
                            let cook_action = if self.ingredients.len() >= 2 || self.ingredients.first().is_some_and(|key| key == "block:WildHerbs") {
                                Some(adventure::Action::CookBatch { camp, ingredients: self.ingredients.clone(), revision: account.revision })
                            } else {
                                single_action
                            };
                            if ui.add_enabled(player.health > 0.0 && cook_action.is_some() && selected_count > 0 && self.cooking_started.is_none(), egui::Button::new("Cook")).clicked() {
                                self.pending_cook = cook_action;
                                self.cooking_seconds = 0.0;
                                self.cooking_started = Some(std::time::Instant::now());
                            }
                            ui.add_space(180.0);
                            ui.separator();
                            ui.label(RichText::new("When cooking completes, the finished dish and its effect appear here.").italics());
                            ui.add_space(18.0);
                        });
                    });
                });
            });
        self.open = open && !close_requested;
        action
    }
}

fn panel(ui: &mut egui::Ui, title: &str, subtitle: &str, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(Color32::from_rgb(18, 26, 35)).stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(126, 104, 65))).inner_margin(10.0).show(ui, |ui| {
        ui.label(RichText::new(title).strong().color(Color32::from_rgb(247, 194, 83)));
        ui.label(subtitle);
        ui.add_space(6.0);
        ui.horizontal_wrapped(contents);
    });
    ui.add_space(8.0);
}
