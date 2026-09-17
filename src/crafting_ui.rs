use crate::crafting::{
    normalize, totals, Account, Action, Element, Formula, ObjectKind, Registry, Slot,
};
use crate::creature::Creatures;
use crate::voxel::{BlockType, World, COLLECTIBLE_BLOCKS};
use glam::Vec3;
use std::time::{Duration, Instant};

pub struct CraftingUi {
    pub open: bool,
    pub slots: Formula,
    pub feedback: String,
    pub pending: bool,
    last_submission: Option<Instant>,
    conversion_element: Element,
    conversion_amount: i64,
    extraction_block: BlockType,
    extraction_amount: u32,
    recipe_search: String,
    tab: u8,
    gear_search: String,
    gear_path: Option<usize>,
    craftable_only: bool,
    selected_gear: crate::equipment::Gear,
}
impl Default for CraftingUi {
    fn default() -> Self {
        Self {
            open: false,
            slots: [None; 5],
            feedback: String::new(),
            pending: false,
            last_submission: None,
            conversion_element: Element::Earth,
            conversion_amount: 1,
            extraction_block: BlockType::Stone,
            extraction_amount: 1,
            recipe_search: String::new(),
            tab: 0,
            gear_search: String::new(),
            gear_path: None,
            craftable_only: false,
            selected_gear: crate::equipment::Gear::Axe,
        }
    }
}
fn selector(ui: &mut egui::Ui, id: impl std::hash::Hash, element: &mut Element) {
    egui::ComboBox::from_id_source(id)
        .selected_text(format!("{element:?}"))
        .show_ui(ui, |ui| {
            for e in Element::ALL {
                ui.selectable_value(element, e, format!("{e:?}"));
            }
        });
}
impl CraftingUi {
    pub fn show_book(&mut self, kind: u8) {
        if kind >= 4 {
            return;
        }
        self.tab = 0;
        self.gear_path = Some(kind as usize + 1);
        self.gear_search.clear();
        self.craftable_only = false;
        self.selected_gear = crate::equipment::Gear::ALL[4 + kind as usize * 4];
        self.feedback = format!(
            "{}: four recipes learned. This book remains for other travelers.",
            crate::gear_catalog::BOOK_NAMES[kind as usize]
        );
    }
    #[allow(clippy::too_many_arguments)]
    fn equipment_panel(
        &mut self,
        ui: &mut egui::Ui,
        registry: &Registry,
        account: &Account,
        world: &World,
        creatures: &Creatures,
        pos: Vec3,
        players: &[Vec3],
    ) -> Option<Action> {
        use crate::equipment::{Entry, Gear};
        let mut request = None;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.gear_search)
                    .hint_text("Search equipment or effect")
                    .desired_width(260.),
            );
            ui.checkbox(&mut self.craftable_only, "Can craft now");
        });
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.gear_path, None, "All");
            for (i, path) in crate::gear_catalog::PATHS.iter().enumerate() {
                ui.selectable_value(&mut self.gear_path, Some(i), *path);
            }
        });
        ui.small(format!("{} / 4 recipe books discovered. Find floating books on dry land; look at one and press F.",account.adventure.recipe_books.count_ones()));
        let query = self.gear_search.trim().to_lowercase();
        let rows: Vec<_> = Gear::available()
            .filter(|g| self.gear_path.is_none_or(|p| p == g.path()))
            .filter(|g| {
                format!("{} {}", g.name(), g.description())
                    .to_lowercase()
                    .contains(&query)
            })
            .filter(|g| {
                !self.craftable_only
                    || registry
                        .preview(
                            account,
                            &Action::CraftGear(*g),
                            world,
                            creatures,
                            pos,
                            players,
                        )
                        .is_ok()
            })
            .collect();
        if !rows.contains(&self.selected_gear) {
            if let Some(g) = rows.first() {
                self.selected_gear = *g;
            }
        }
        let selected = self.selected_gear;
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical()
                .id_source("equipment_catalog")
                .max_height(340.)
                .show(&mut columns[0], |ui| {
                    if rows.is_empty() {
                        ui.label("No matching equipment. Change the search or filters.");
                    }
                    for g in &rows {
                        ui.horizontal(|ui| {
                            crate::equipment_ui::icon(ui, Entry::Gear(*g));
                            ui.selectable_value(
                                &mut self.selected_gear,
                                *g,
                                format!(
                                    "{}{}  ({})",
                                    if g.known(account) { "" } else { "? " },
                                    g.name(),
                                    Entry::Gear(*g).count(account)
                                ),
                            );
                        });
                    }
                });
            let ui = &mut columns[1];
            if !rows.is_empty() {
                ui.heading(selected.name());
                ui.label(selected.description());
                ui.label(format!("Owned: {}", Entry::Gear(selected).count(account)));
                if let Some(book) = selected.book() {
                    ui.small(format!(
                        "Recipe: {}",
                        crate::gear_catalog::BOOK_NAMES[book as usize]
                    ));
                }
                ui.separator();
                for (block, n) in selected.ingredients(false) {
                    let have = Entry::Resource(block).count(account);
                    ui.colored_label(
                        if have >= n {
                            egui::Color32::LIGHT_GREEN
                        } else {
                            egui::Color32::LIGHT_RED
                        },
                        format!("{}: {have} / {n}", block.name()),
                    );
                }
                let mana =
                    registry.mana_charge(crate::crafting::gear_formula(selected, false).unwrap().2);
                ui.label(format!("Mana: {} / {mana}", account.mana));
                let action = Action::CraftGear(selected);
                let status = registry.preview(account, &action, world, creatures, pos, players);
                if ui
                    .add_enabled(
                        self.ready() && status.is_ok(),
                        egui::Button::new(format!("Craft {}", selected.name())),
                    )
                    .clicked()
                {
                    request = Some(action);
                }
                if let Err(error) = status {
                    ui.label(error);
                }
                ui.small("Assign crafted equipment to your hotbar in I. Effects apply while held.");
            }
        });
        ui.separator();
        ui.collapsing("Discovered recipe library",|ui| {
            for kind in 0..4 {
                let known=account.adventure.recipe_books & (1<<kind)!=0;
                if ui.add_enabled(known,egui::Button::new(crate::gear_catalog::BOOK_NAMES[kind])).clicked(){self.show_book(kind as u8);}
            }
            ui.small("Starter equipment is always available. Books teach four specialist recipes each; none are consumed when read.");
        });
        request
    }
    fn ready(&self) -> bool {
        !self.pending
            && self
                .last_submission
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        registry: &Registry,
        account: &Account,
        world: &World,
        creatures: &Creatures,
        pos: Vec3,
        players: &[Vec3],
    ) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut request = None;
        egui::Window::new("Elemental Crafting (C / Esc to close)")
            // Keep an independently sized window for each style when switching at runtime.
            .id(egui::Id::new((
                "crafting_window",
                crate::ui_theme::is_fantasy(ctx),
            )))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(true)
            .default_width(if crate::ui_theme::is_fantasy(ctx) {
                950.0
            } else {
                720.0
            })
            .default_height(620.0)
            .max_width((ctx.screen_rect().width()-48.0).max(260.0))
            .max_height((ctx.screen_rect().height() - 70.0).max(180.0))
            .vscroll(true)
            .show(ctx, |ui| {
                ui.heading(format!("Mana: {}", account.mana));
                if registry.mana_free {ui.label("Mana-free actions enabled by host (testing)");}
                ui.horizontal_wrapped(|ui| {
                    let colors = [
                        [190, 145, 75],
                        [245, 120, 60],
                        [90, 165, 245],
                        [105, 205, 100],
                        [180, 130, 215],
                    ];
                    for e in Element::ALL {
                        let [r, g, b] = colors[e.index()];
                        ui.colored_label(
                            egui::Color32::from_rgb(r, g, b),
                            format!("{e:?}: {}", account.elements[e.index()]),
                        );
                    }
                });
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab,0,"Equipment & recipe books");
                    ui.selectable_value(&mut self.tab,1,"Elemental formulas & extraction");
                });
                ui.separator();
                if self.tab==0 {
                    request=self.equipment_panel(ui,registry,account,world,creatures,pos,players);
                    if self.pending {ui.label("Waiting for host...");}
                    ui.label(&self.feedback);
                    return;
                }
                ui.label("Ordered formula — gaps are ignored. Repeated elements are allowed.");
                let bound=Action::BindSheep;
                let available=registry.preview(account,&bound,world,creatures,pos,players);
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Bound sheep figurines: {}. Uses the sheep formula (2 Life) and its mana cost.",account.production_goods.get("creature:sheep").copied().unwrap_or(0)));
                    let button=ui.add_enabled(self.ready() && request.is_none() && available.is_ok(),egui::Button::new("Craft bound sheep figurine"));
                    if button.clicked() {request=Some(bound);}
                    if let Err(e)=available {button.on_hover_text(e);}
                });
                ui.small("Deposit the figurine into a chest [F]; release it there to bring the sheep to life.");
                ui.add_enabled_ui(self.ready(), |ui| {
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, slot) in self.slots.iter_mut().enumerate() {
                                ui.push_id(i, |ui| {
                                    ui.group(|ui| {
                                        ui.set_min_width(105.0);
                                        ui.vertical(|ui| {
                                            ui.label(format!("Slot {}", i + 1));
                                            if let Some(s) = slot {
                                                selector(ui, "element", &mut s.element);
                                                ui.add(
                                                    egui::DragValue::new(&mut s.amount)
                                                        .clamp_range(1..=u32::MAX as i64)
                                                        .prefix("x "),
                                                );
                                                if ui.button("Remove").clicked() {
                                                    *slot = None;
                                                }
                                            } else {
                                                if ui.button("Add element").clicked() {
                                                    *slot = Some(Slot {
                                                        element: Element::Earth,
                                                        amount: 1,
                                                    });
                                                }
                                                ui.label("Empty");
                                            }
                                        });
                                    });
                                });
                            }
                        });
                    });
                    if ui.button("Clear all").clicked() {
                        self.slots = [None; 5];
                    }
                    ui.label(format!("Mana cost: {}", registry.mana_cost(&self.slots)));
                    if let Ok(slots) = normalize(&self.slots) {
                        ui.label(format!(
                            "Formula: {}",
                            slots
                                .iter()
                                .map(|s| format!("{:?} x{}", s.element, s.amount))
                                .collect::<Vec<_>>()
                                .join(" → ")
                        ));
                        if let Ok(cost) = totals(&slots) {
                            ui.horizontal_wrapped(|ui| {
                                for e in Element::ALL {
                                    let color = if account.elements[e.index()] >= cost[e.index()] {
                                        egui::Color32::LIGHT_GREEN
                                    } else {
                                        egui::Color32::LIGHT_RED
                                    };
                                    ui.colored_label(
                                        color,
                                        format!(
                                            "{e:?}: {}/{}",
                                            account.elements[e.index()],
                                            cost[e.index()]
                                        ),
                                    );
                                }
                            });
                        }
                    }
                    match registry.matched(&self.slots) {
                        Ok(r) => {
                            ui.label(format!(
                                "Result: {} x{} ({:?})",
                                r.output.id, r.output.quantity, r.output.kind
                            ));
                        }
                        Err(e) => {
                            ui.label(e);
                        }
                    }
                    let action = Action::Craft(self.slots);
                    let status = registry.preview(account, &action, world, creatures, pos, players);
                    if let Err(e) = &status {
                        ui.colored_label(egui::Color32::LIGHT_RED, e);
                    }
                    if ui
                        .add_enabled(status.is_ok(), egui::Button::new("Craft"))
                        .clicked()
                    {
                        request = Some(action);
                    }
                    ui.separator();
                    ui.label(format!(
                        "Convert elements to mana (1 element = {} mana)",
                        registry.conversion_rate
                    ));
                    ui.horizontal(|ui| {
                        selector(ui, "conversion", &mut self.conversion_element);
                        ui.add(
                            egui::DragValue::new(&mut self.conversion_amount)
                                .clamp_range(1..=u32::MAX as i64),
                        );
                        ui.label(format!(
                            "→ {} mana",
                            self.conversion_amount as u64 * registry.conversion_rate as u64
                        ));
                    });
                    let action = Action::Convert {
                        element: self.conversion_element,
                        amount: self.conversion_amount,
                    };
                    let status = registry.preview(account, &action, world, creatures, pos, players);
                    if let Err(e) = &status {
                        ui.label(e);
                    }
                    if ui
                        .add_enabled(
                            status.is_ok() && request.is_none(),
                            egui::Button::new("Confirm mana conversion"),
                        )
                        .clicked()
                    {
                        request = Some(action);
                    }
                    ui.separator();
                    ui.label(
                        "Extract elements: consumes gathered resources. Mine blocks to restock.",
                    );
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_source("extraction")
                            .selected_text(self.extraction_block.name())
                            .show_ui(ui, |ui| {
                                for (i, b) in COLLECTIBLE_BLOCKS.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.extraction_block,
                                        *b,
                                        format!("{} ({})", b.name(), account.resources[i]),
                                    );
                                }
                            });
                        ui.add(
                            egui::DragValue::new(&mut self.extraction_amount)
                                .clamp_range(1..=u32::MAX),
                        );
                    });
                    let comp =
                        registry.composition(ObjectKind::Resource, self.extraction_block.id());
                    ui.label(format!(
                        "Receive: {}",
                        Element::ALL
                            .iter()
                            .map(|e| format!(
                                "{e:?} {}",
                                comp[e.index()] as u64 * self.extraction_amount as u64
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    ui.label(format!("Decomposition cost: {} mana",registry.mana_charge(self.extraction_amount)));
                    let action = Action::Extract {
                        block: self.extraction_block,
                        amount: self.extraction_amount,
                    };
                    let status = registry.preview(account, &action, world, creatures, pos, players);
                    if let Err(e) = &status {
                        ui.label(e);
                    }
                    if ui
                        .add_enabled(
                            status.is_ok() && request.is_none(),
                            egui::Button::new("Confirm extraction"),
                        )
                        .clicked()
                    {
                        request = Some(action);
                    }
                    ui.collapsing("Formula book", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.recipe_search)
                                .hint_text("Search formula or resource"),
                        );
                        let query = self.recipe_search.trim().to_lowercase();
                        egui::ScrollArea::vertical()
                            .id_source("formula_book")
                            .max_height(250.0)
                            .show(ui, |ui| {
                                for r in &registry.recipes {
                                    let block = BlockType::from_name(&r.output.id);
                                    let name = block.map_or(r.output.id.as_str(), |b| b.name());
                                    if !format!("{} {}", name, r.id).to_lowercase().contains(&query)
                                    {
                                        continue;
                                    }
                                    ui.horizontal(|ui| {
                                        if let Some(block) = block {
                                            crate::resource_ui::icon(ui, block);
                                        }
                                        let cost = registry.mana_charge(registry.mana_costs[r.inputs.len() - 1]);
                                        let label = format!(
                                            "{} x{}: {} ({} mana)",
                                            name, r.output.quantity,
                                            r.inputs
                                                .iter()
                                                .map(|s| format!("{:?} x{}", s.element, s.amount))
                                                .collect::<Vec<_>>()
                                                .join(" + "),
                                            cost
                                        );
                                        let response = ui.button(label);
                                        if response.clicked() {
                                            self.slots = [None; 5];
                                            for (i, s) in r.inputs.iter().enumerate() {
                                                self.slots[i] = Some(*s);
                                            }
                                        }
                                        if let Some(block) = block {
                                            response.on_hover_text(
                                                crate::resource_ui::description(block),
                                            );
                                        }
                                    });
                                }
                            });
                    });
                });
                if self.pending {
                    ui.label("Waiting for host…");
                }
                ui.label(&self.feedback);
            });
        if request.is_some() {
            self.pending = true;
            self.last_submission = Some(Instant::now());
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_host_replies_do_not_turn_a_double_click_into_two_purchases() {
        let mut ui = CraftingUi::default();
        assert!(ui.ready());
        ui.last_submission = Some(Instant::now());
        // Even if a local host answers in the same frame, retain the double-click guard.
        ui.pending = false;
        assert!(!ui.ready());
        ui.last_submission = Some(Instant::now() - Duration::from_secs(1));
        assert!(ui.ready());
        ui.pending = true;
        assert!(!ui.ready());
    }
    #[test]
    fn crafting_screen_renders_empty_and_known_formulas_without_mutating_balances() {
        let ctx = egui::Context::default();
        let registry = Registry::parse(include_str!("../data/crafting.json")).unwrap();
        let account = Account {
            elements: [10; 5],
            mana: 20,
            ..Account::default()
        };
        let world = World::new(1);
        let creatures = Creatures::new();
        let mut ui = CraftingUi {
            open: true,
            ..CraftingUi::default()
        };
        for known in [false, true] {
            ui.tab = 1;
            if known {
                ui.slots[0] = Some(Slot {
                    element: Element::Earth,
                    amount: 1,
                });
            }
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                assert!(ui
                    .draw(
                        ctx,
                        &registry,
                        &account,
                        &world,
                        &creatures,
                        Vec3::ZERO,
                        &[]
                    )
                    .is_none());
            });
            assert!(!output.shapes.is_empty());
            assert!(!ui.pending);
        }
        assert_eq!(account.elements, [10; 5]);
        assert_eq!(account.mana, 20);
    }
}
