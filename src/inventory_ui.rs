//! Local inventory selection; all assignments use the existing authoritative hotbar path.
use crate::{
    crafting::{Account, Element},
    equipment::{Entry, Gear},
    ui::UiRequests,
    voxel::COLLECTIBLE_BLOCKS,
};

#[derive(Default)]
pub struct Inventory {
    selected: Option<Entry>,
    pub feedback: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spells_assign_with_number_keys_only_when_available() {
        let mut book = crate::spellbook::Spellbook::default();
        let module = crate::scripting::Module::load(
            "Heal".into(),
            "Heal target".into(),
            include_str!("../modules/target_heal.lua").into(),
        )
        .unwrap();
        let id = book.remember(&module, "Host").unwrap();
        let registry = crate::crafting::Registry::load().unwrap();
        for available in [true, false] {
            let ctx = egui::Context::default();
            let mut account = Account::default();
            if available {
                book.sync_hotbar(&mut account);
            }
            let mut inventory = Inventory {
                selected: Some(Entry::Spell(id)),
                ..Default::default()
            };
            let mut requests = UiRequests::default();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 720.),
                    )),
                    events: vec![egui::Event::Key {
                        key: egui::Key::Num9,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ctx| inventory.show(ctx, &account, 100., &registry, &mut requests, &book),
            );
            assert_eq!(
                requests.assign_entry,
                available.then_some(Some(Entry::Spell(id)))
            );
        }
    }
    #[test]
    fn numbered_assignment_uses_selection_and_rejects_depleted_items() {
        for (key, slot) in [(egui::Key::Num1, 0), (egui::Key::Num9, 8)] {
            let ctx = egui::Context::default();
            let mut account = Account::default();
            let entry = Entry::Gear(Gear::Pickaxe);
            let mut inventory = Inventory {
                selected: Some(entry),
                ..Default::default()
            };
            let registry = crate::crafting::Registry::load().unwrap();
            let mut requests = UiRequests::default();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 720.0),
                    )),
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ctx| {
                    inventory.show(
                        ctx,
                        &account,
                        100.,
                        &registry,
                        &mut requests,
                        &crate::spellbook::Spellbook::default(),
                    )
                },
            );
            assert_eq!(requests.select_slot, Some(slot));
            assert_eq!(requests.assign_entry, Some(Some(entry)));
            account.gear[Gear::Pickaxe as usize] = 0;
            let mut requests = UiRequests {
                select_slot: Some(4),
                ..Default::default()
            };
            let _ = ctx.run(Default::default(), |ctx| {
                inventory.show(
                    ctx,
                    &account,
                    100.,
                    &registry,
                    &mut requests,
                    &crate::spellbook::Spellbook::default(),
                )
            });
            assert_eq!(requests.assign_entry, None);
            assert_eq!(inventory.selected, Some(entry));
        }
    }
}
impl Inventory {
    #[cfg(test)]
    pub(crate) fn preview_selection(&mut self, entry: Entry) {
        self.selected = Some(entry);
    }
    fn actions(
        &self,
        ui: &mut egui::Ui,
        account: &Account,
        health: f32,
        registry: &crate::crafting::Registry,
        requests: &mut UiRequests,
    ) {
        use crate::crafting::{Action, ObjectKind};
        match self.selected {
            Some(Entry::Spell(_)) => {
                ui.label("Select its hotbar slot, then left-click to cast at your aim.");
                if ui.button("Manage spells in Spellbook [K]").clicked() {
                    requests.open_spellbook = true;
                }
            }
            Some(Entry::Gear(g)) => {
                for salvage in [false, true] {
                    let (_, _, mana) = crate::crafting::gear_formula(g, salvage).unwrap();
                    let mana = registry.mana_charge(mana);
                    let available = if salvage {
                        Entry::Gear(g).count(account) > 0
                    } else {
                        g.known(account)
                            && g.ingredients(false)
                                .iter()
                                .all(|(b, n)| Entry::Resource(*b).count(account) >= *n)
                    };
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                available && account.mana >= mana,
                                egui::Button::new(if salvage {
                                    "Decompose 1 item"
                                } else {
                                    "Create 1 item"
                                }),
                            )
                            .clicked()
                        {
                            requests.crafting = Some(if salvage {
                                Action::SalvageGear(g)
                            } else {
                                Action::CraftGear(g)
                            });
                        }
                        ui.label(format!(
                            "{} {}; {mana} mana",
                            if salvage { "Returns" } else { "Uses" },
                            g.recipe_text(salvage)
                        ));
                    });
                }
            }
            Some(Entry::Resource(block)) => {
                if let Some(healing) = crate::food::healing(block) {
                    ui.horizontal_wrapped(|ui| {
                        let ready = health.is_finite()
                            && health > 0.
                            && health < crate::player::MAX_HEALTH
                            && Entry::Resource(block).count(account) > 0;
                        if ui
                            .add_enabled(
                                ready,
                                egui::Button::new(format!("Eat 1 (+{healing:.0} health)")),
                            )
                            .clicked()
                        {
                            requests.eat_food = Some(block);
                        }
                        if health >= crate::player::MAX_HEALTH {
                            ui.label("Health full - food is kept.");
                        }
                        if block == crate::voxel::BlockType::Meat {
                            ui.small("Campfire cooking improves this to +25 health.");
                        }
                    });
                }
                let comp = registry.composition(ObjectKind::Resource, block.id());
                ui.label(format!(
                    "Returns: {}",
                    Element::ALL
                        .iter()
                        .filter(|e| comp[e.index()] > 0)
                        .map(|e| format!("{} {e:?}", comp[e.index()]))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                let mana = registry.mana_charge(1);
                if ui
                    .add_enabled(
                        account.mana >= mana && comp != [0; 5],
                        egui::Button::new(format!("Decompose 1 resource ({mana} mana)")),
                    )
                    .clicked()
                {
                    requests.crafting = Some(Action::Extract { block, amount: 1 });
                }
            }
            None => {
                ui.small("Select equipment to create or decompose it; select food to eat or resources to extract elements.");
            }
        }
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        account: &Account,
        health: f32,
        registry: &crate::crafting::Registry,
        requests: &mut UiRequests,
        book: &crate::spellbook::Spellbook,
    ) {
        if self
            .selected
            .is_some_and(|e| matches!(e, Entry::Resource(_)) && e.count(account) == 0)
        {
            self.selected = None;
        }
        let size = ctx.screen_rect().size();
        // Reserve room for the larger fantasy font, window chrome and hotbar.
        let height = (size.y - 610.0).clamp(60.0, 400.0);
        egui::Window::new("Inventory")
            .anchor(egui::Align2::CENTER_TOP, [0.0, 24.0])
            .fixed_size(egui::vec2((size.x-48.0).clamp(280.0, 1100.0), height+340.0))
            .collapsible(false).resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Your inventory");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close  [I / Esc]").clicked() { requests.close_inventory = true; }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("Health: {health:.0} / {:.0}",crate::player::MAX_HEALTH));
                    ui.strong(format!("Mana: {}",account.mana)).on_hover_text("+1 every 5 seconds, up to 100. Instant: 5. New rule: 20. Convert elements to mana in Crafting [C].");
                    let colors = [[190,145,75],[245,120,60],[90,165,245],[105,205,100],[180,130,215]];
                    for e in Element::ALL {
                        let [r,g,b] = colors[e.index()];
                        ui.colored_label(egui::Color32::from_rgb(r,g,b), format!("{e:?}  {}",account.elements[e.index()]));
                        ui.add_space(12.0);
                    }
                });
                ui.separator();
                ui.label("Select an item, resource or spell, then press 1–9 to assign a hotbar slot.");
                ui.horizontal(|ui| {
                    ui.label(if crate::torch::equipped(account) {"Left hand: burning torch"}else{"Left hand: empty"});
                    if ui.add_enabled(account.gear[Gear::Torch as usize]>0,egui::Button::new(if account.torch_equipped {"Put torch away"}else{"Equip torch"})).clicked(){requests.crafting=Some(crate::crafting::Action::EquipTorch(!account.torch_equipped));}
                });
                ui.columns(3, |columns| {
                    columns[0].heading("Items");
                    columns[0].small("Tools and weapons");
                    egui::ScrollArea::vertical().id_source("inventory_items").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).max_height(height).min_scrolled_height(height).show(&mut columns[0], |ui| {
                        for g in Gear::available() { self.row(ui, Entry::Gear(g), account); }
                    });
                    let mut resources: Vec<_> = COLLECTIBLE_BLOCKS.iter().copied().map(Entry::Resource).filter(|e| e.count(account)>0).collect();
                    resources.sort_by_key(|e|e.name());
                    columns[1].heading("Resources");
                    columns[1].small(format!("{} types · unlimited storage",resources.len()));
                    egui::ScrollArea::vertical().id_source("inventory_resources").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).max_height(height).min_scrolled_height(height).show(&mut columns[1], |ui| {
                        if resources.is_empty() {ui.label("No resources yet. Gather plants by hand or use your tools to mine.");}
                        for e in resources { self.row(ui,e,account); }
                    });
                    columns[2].heading("Spells");
                    columns[2].small("Remembered spells");
                    egui::ScrollArea::vertical().id_source("inventory_spells").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).max_height(height).min_scrolled_height(height).show(&mut columns[2], |ui| {
                        if book.spells.is_empty() {ui.label("Remember an instant spell in Rules, or ask the host to share one from the Spellbook.");}
                        for spell in &book.spells {
                            let entry=Entry::Spell(spell.id);
                            ui.horizontal(|ui| {
                                crate::equipment_ui::icon_with_book(ui,entry,book);
                                let label=if spell.ready() {spell.name.clone()} else {format!("{} (review)",spell.name)};
                                if ui.selectable_label(self.selected==Some(entry),label)
                                    .on_hover_text(format!("{}\n{} mana · {:.1}s cooldown · {}\n{}",spell.description,registry.mana_charge(spell.mana_cost),spell.cooldown_seconds,spell.target.label(),if entry.count(account)>0 {"Press 1–9 to assign"}else{"Open Spellbook to review"})).clicked() {self.selected=Some(entry);}
                            });
                        }
                    });
                });
                ui.separator();
                ui.label(self.selected.map_or("Nothing selected".into(), |e|if e==Entry::Gear(Gear::Torch) {"Selected: Torch — use the left-hand Equip torch button above".into()}else{format!("Selected: {} — press 1–9 or click a hotbar slot",crate::equipment_ui::entry_name(e,book))}));
                self.actions(ui,account,health,registry,requests);
                if !self.feedback.is_empty() {ui.label(&self.feedback);}
                if ui.button(format!("Clear slot {} (empty hand)",account.hotbar.active+1)).clicked() {
                    requests.assign_entry=Some(None);
                    self.selected=None;
                }
            });
        let keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let slot = ctx
            .input(|i| keys.iter().position(|k| i.key_pressed(*k)))
            .or(requests.select_slot);
        if let Some(slot) = slot {
            requests.select_slot = Some(slot);
            if let Some(entry) = self
                .selected
                .filter(|e| e.count(account) > 0 && *e != Entry::Gear(Gear::Torch))
            {
                requests.assign_entry = Some(Some(entry));
            }
        }
    }
    fn row(&mut self, ui: &mut egui::Ui, entry: Entry, account: &Account) {
        let count = entry.count(account);
        ui.add_enabled_ui(count > 0 || matches!(entry, Entry::Gear(_)), |ui| {
            let response = ui
                .horizontal(|ui| {
                    crate::equipment_ui::icon(ui, entry);
                    ui.selectable_label(
                        self.selected == Some(entry),
                        format!("{}   ×{}", entry.name(), count),
                    )
                })
                .inner;
            if response.clicked() {
                self.selected = Some(entry);
            }
            response.on_hover_text(match entry {
                Entry::Resource(b) => crate::resource_ui::description(b),
                Entry::Gear(g) => g.description().into(),
                Entry::Spell(_) => "Select a slot, then left-click to cast".into(),
            });
        });
    }
}
