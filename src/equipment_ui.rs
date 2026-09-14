//! Procedural pixel silhouettes for gear; resource icons use the world atlas.
use crate::equipment::{Entry, Gear};
pub fn icon(ui: &mut egui::Ui, entry: Entry) {
    if let Entry::Resource(b) = entry {
        crate::resource_ui::icon(ui, b);
        return;
    }
    let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
    let Entry::Gear(g) = entry else {
        return;
    };
    let metal = egui::Color32::from_rgb(199, 218, 222);
    for (min, max, c) in crate::held_item::parts(Some(Entry::Gear(g))) {
        let color = egui::Color32::from_rgb((c[0]*255.) as u8,(c[1]*255.) as u8,(c[2]*255.) as u8);
        let a = r.min + egui::vec2((min.x + 0.5) * 24.0, (1.0 - max.y) * 24.0);
        let b = r.min + egui::vec2((max.x + 0.5) * 24.0, (1.0 - min.y) * 24.0);
        ui.painter()
            .rect_filled(egui::Rect::from_min_max(a, b), 0.0, color);
    }
    if matches!(g,Gear::Bow|Gear::Longbow) {
        ui.painter().line_segment(
            [
                r.left_top() + egui::vec2(7.0, 2.0),
                r.left_bottom() + egui::vec2(7.0, -2.0),
            ],
            egui::Stroke::new(1.0_f32, metal),
        );
    }
}

pub fn hotbar(
    ctx: &egui::Context,
    account: &crate::crafting::Account,
    interactive: bool,
    show_name: bool,
    disabled: bool,
) -> Option<usize> {
    let interactive = interactive && !disabled;
    let active = account.hotbar.active.min(8);
    let entry = account.hotbar.entry();
    let mut selected_slot = None;
    let slot_width = ((ctx.screen_rect().width() - 48.0) / 9.0).clamp(30.0, 62.0);
    let width = 9.0 * slot_width + 8.0 * 4.0;
    egui::Area::new(egui::Id::new("nine_slot_hotbar"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -12.0])
        .show(ctx, |ui| {
            ui.set_width(width);
            if show_name || disabled {
                let (r, _) = ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::hover());
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    if disabled { "Tools paused — exit machine mode to restore" } else { entry.map_or("Empty hand", |e| e.name()) },
                    egui::FontId::proportional(18.0),
                    egui::Color32::from_rgb(245, 232, 199),
                );
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for i in 0..9 {
                    let selected = !disabled && i == active;
                    let (r, response) = ui.allocate_exact_size(
                        egui::vec2(slot_width, 68.0),
                        if interactive {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );
                    let bronze = egui::Color32::from_rgb(130, 91, 53);
                    let gold = egui::Color32::from_rgb(255, 214, 112);
                    let ivory = egui::Color32::from_rgb(245, 232, 199);
                    ui.painter().rect(
                        r,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(24, 22, 28, 225),
                        egui::Stroke::new(
                            if selected { 2.5_f32 } else { 1.0_f32 },
                            if selected { gold } else { bronze },
                        ),
                    );
                    ui.painter().text(
                        r.left_top() + egui::vec2(5.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", i + 1),
                        egui::FontId::proportional(14.0),
                        ivory,
                    );
                    if let Some(e) = account.hotbar.slots[i] {
                        let count = e.count(account);
                        let ir = egui::Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
                        let mut icon_ui =
                            ui.child_ui(ir, egui::Layout::top_down(egui::Align::Center));
                        icon_ui.add_enabled_ui(count > 0, |ui| icon(ui, e));
                        ui.painter().text(
                            r.right_bottom() - egui::vec2(5.0, 4.0),
                            egui::Align2::RIGHT_BOTTOM,
                            format!("{count}"),
                            egui::FontId::proportional(14.0),
                            if count > 0 {
                                ivory
                            } else {
                                egui::Color32::GRAY
                            },
                        );
                        response.clone().on_hover_text(e.name());
                    }
                    if disabled {
                        ui.painter().rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(65, 65, 65, 190));
                    }
                    if interactive && response.clicked() {
                        selected_slot = Some(i);
                    }
                }
            });
        });
    selected_slot
}
