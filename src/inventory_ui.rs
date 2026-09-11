//! Local inventory selection; all assignments use the existing authoritative hotbar path.
use crate::{crafting::{Account, Element}, equipment::{Entry, Gear}, ui::UiRequests, voxel::COLLECTIBLE_BLOCKS};

#[derive(Default)]
pub struct Inventory {
    selected: Option<Entry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbered_assignment_uses_selection_and_rejects_depleted_items() {
        for (key,slot) in [(egui::Key::Num1,0),(egui::Key::Num9,8)] {
            let ctx = egui::Context::default();
            let mut account = Account::default();
            let entry = Entry::Gear(Gear::Pickaxe);
            let mut inventory = Inventory { selected: Some(entry) };
            let mut requests = UiRequests::default();
            let _ = ctx.run(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1280.0,720.0))),
                events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
                ..Default::default()
            }, |ctx|inventory.show(ctx,&account,&mut requests));
            assert_eq!(requests.select_slot,Some(slot));
            assert_eq!(requests.assign_entry,Some(Some(entry)));
            account.gear[Gear::Pickaxe as usize]=0;
            let mut requests = UiRequests {select_slot:Some(4),..Default::default()};
            let _ = ctx.run(Default::default(), |ctx|inventory.show(ctx,&account,&mut requests));
            assert_eq!(requests.assign_entry,None);
            assert_eq!(inventory.selected,None);
        }
    }
}
impl Inventory {
    pub fn show(&mut self, ctx: &egui::Context, account: &Account, requests: &mut UiRequests) {
        if self.selected.is_some_and(|e| e.count(account) == 0) { self.selected = None; }
        let size = ctx.screen_rect().size();
        // Reserve room for the larger fantasy font, window chrome and hotbar.
        let height = (size.y - 490.0).clamp(60.0, 470.0);
        egui::Window::new("Inventory")
            .anchor(egui::Align2::CENTER_TOP, [0.0, 24.0])
            .fixed_size(egui::vec2((size.x-48.0).clamp(280.0, 880.0), height+170.0))
            .collapsible(false).resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Your inventory");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close  [I / Esc]").clicked() { requests.close_inventory = true; }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    let colors = [[190,145,75],[245,120,60],[90,165,245],[105,205,100],[180,130,215]];
                    for e in Element::ALL {
                        let [r,g,b] = colors[e.index()];
                        ui.colored_label(egui::Color32::from_rgb(r,g,b), format!("{e:?}  {}",account.elements[e.index()]));
                        ui.add_space(12.0);
                    }
                });
                ui.separator();
                ui.label("Select an item or resource, then press 1–9 to assign a hotbar slot.");
                ui.columns(2, |columns| {
                    columns[0].heading("Items");
                    columns[0].small("Tools and weapons");
                    egui::ScrollArea::vertical().id_source("inventory_items").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).max_height(height).min_scrolled_height(height).show(&mut columns[0], |ui| {
                        for g in Gear::ALL { self.row(ui, Entry::Gear(g), account); }
                    });
                    let mut resources: Vec<_> = COLLECTIBLE_BLOCKS.iter().copied().map(Entry::Resource).filter(|e| e.count(account)>0).collect();
                    resources.sort_by_key(|e|e.name());
                    columns[1].heading("Resources");
                    columns[1].small(format!("{} types · unlimited storage",resources.len()));
                    egui::ScrollArea::vertical().id_source("inventory_resources").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).max_height(height).min_scrolled_height(height).show(&mut columns[1], |ui| {
                        if resources.is_empty() {ui.label("No resources yet. Gather plants by hand or use your tools to mine.");}
                        for e in resources { self.row(ui,e,account); }
                    });
                });
                ui.separator();
                ui.label(self.selected.map_or("Nothing selected".into(), |e|format!("Selected: {} — press 1–9 or click a hotbar slot",e.name())));
                if ui.button(format!("Clear slot {} (empty hand)",account.hotbar.active+1)).clicked() {
                    requests.assign_entry=Some(None);
                    self.selected=None;
                }
            });
        let keys = [egui::Key::Num1,egui::Key::Num2,egui::Key::Num3,egui::Key::Num4,egui::Key::Num5,egui::Key::Num6,egui::Key::Num7,egui::Key::Num8,egui::Key::Num9];
        let slot = ctx.input(|i|keys.iter().position(|k|i.key_pressed(*k))).or(requests.select_slot);
        if let Some(slot) = slot {
            requests.select_slot=Some(slot);
            if let Some(entry) = self.selected.filter(|e|e.count(account)>0) { requests.assign_entry=Some(Some(entry)); }
        }
    }
    fn row(&mut self, ui: &mut egui::Ui, entry: Entry, account: &Account) {
        let count = entry.count(account);
        ui.add_enabled_ui(count>0, |ui| {
            let response = ui.horizontal(|ui| {
                crate::equipment_ui::icon(ui,entry);
                ui.selectable_label(self.selected==Some(entry),format!("{}   ×{}",entry.name(),count))
            }).inner;
            if response.clicked() { self.selected=Some(entry); }
            response.on_hover_text(match entry {
                Entry::Resource(b)=>crate::resource_ui::description(b),
                Entry::Gear(Gear::Pickaxe)=>"Mine stone, ores and soil".into(),
                Entry::Gear(Gear::Axe)=>"Gather wood and plants".into(),
                _=>"Attack creatures".into(),
            });
        });
    }
}
