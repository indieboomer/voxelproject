//! Procedural pixel silhouettes for gear; resource icons use the world atlas.
use crate::equipment::{Entry, Gear};
pub fn icon_with_book(ui: &mut egui::Ui, entry: Entry, book: &crate::spellbook::Spellbook) {
    if let Entry::Spell(id) = entry {
        if let Some(spell) = book.get(id) {
            crate::spell_art::image(ui, spell.artwork(), egui::vec2(24., 24.));
            return;
        }
    }
    icon(ui, entry);
}
pub fn icon(ui: &mut egui::Ui, entry: Entry) {
    if let Entry::Resource(b) = entry {
        crate::resource_ui::icon(ui, b);
        return;
    }
    let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
    if let Entry::Spell(id) = entry {
        let c = r.center();
        let color = egui::Color32::from_rgb(185, 130, 255);
        let points = vec![
            c + egui::vec2(0., -11.),
            c + egui::vec2(9., 0.),
            c + egui::vec2(0., 11.),
            c + egui::vec2(-9., 0.),
        ];
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            egui::Color32::from_rgb(60, 30, 90),
            egui::Stroke::new(1.5_f32, color),
        ));
        ui.painter()
            .circle_filled(c, 3., egui::Color32::from_rgb(255, 220, 140));
        for i in 0..(id % 3 + 1) {
            ui.painter().line_segment(
                [
                    c + egui::vec2(-3. + i as f32 * 3., -7.),
                    c + egui::vec2(-3. + i as f32 * 3., -4.),
                ],
                egui::Stroke::new(1_f32, color),
            );
        }
        return;
    }
    let Entry::Gear(g) = entry else {
        return;
    };
    let metal = egui::Color32::from_rgb(199, 218, 222);
    for (min, max, c) in crate::held_item::parts(Some(Entry::Gear(g))) {
        let color = egui::Color32::from_rgb(
            (c[0] * 255.) as u8,
            (c[1] * 255.) as u8,
            (c[2] * 255.) as u8,
        );
        let a = r.min + egui::vec2((min.x + 0.5) * 24.0, (1.0 - max.y) * 24.0);
        let b = r.min + egui::vec2((max.x + 0.5) * 24.0, (1.0 - min.y) * 24.0);
        ui.painter()
            .rect_filled(egui::Rect::from_min_max(a, b), 0.0, color);
    }
    if matches!(g, Gear::Bow | Gear::Longbow) {
        ui.painter().line_segment(
            [
                r.left_top() + egui::vec2(7.0, 2.0),
                r.left_bottom() + egui::vec2(7.0, -2.0),
            ],
            egui::Stroke::new(1.0_f32, metal),
        );
    }
}

#[cfg(test)]
pub fn hotbar(
    ctx: &egui::Context,
    account: &crate::crafting::Account,
    interactive: bool,
    show_name: bool,
    disabled: bool,
) -> Option<usize> {
    hotbar_with_spells(
        ctx,
        account,
        interactive,
        show_name,
        disabled,
        &crate::spellbook::Spellbook::default(),
    )
}

pub fn entry_name(entry: Entry, book: &crate::spellbook::Spellbook) -> &str {
    if let Entry::Spell(id) = entry {
        book.get(id)
            .map_or("Unavailable spell", |s| s.name.as_str())
    } else {
        entry.name()
    }
}

pub fn spell_hud(ctx: &egui::Context, text: &str, ready: bool) {
    egui::Area::new(egui::Id::new("spell_cast_hud"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0., -116.])
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(egui::Color32::from_black_alpha(190))
                .inner_margin(6.)
                .show(ui, |ui| {
                    ui.colored_label(
                        if ready {
                            egui::Color32::LIGHT_GREEN
                        } else {
                            egui::Color32::from_rgb(255, 200, 125)
                        },
                        text,
                    );
                });
        });
}

pub fn hotbar_with_spells(
    ctx: &egui::Context,
    account: &crate::crafting::Account,
    interactive: bool,
    show_name: bool,
    disabled: bool,
    book: &crate::spellbook::Spellbook,
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
            if show_name || disabled || matches!(entry, Some(Entry::Spell(_))) {
                let (r, _) = ui.allocate_exact_size(egui::vec2(width, 24.0), egui::Sense::hover());
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    if disabled {
                        "Tools paused — exit machine mode to restore"
                    } else {
                        entry.map_or("Empty hand", |e| entry_name(e, book))
                    },
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
                    if let Some(e) = account.hotbar.slots[i]
                        .filter(|e| !matches!(e,Entry::Gear(g) if !g.enabled() || *g==Gear::Torch))
                    {
                        let count = e.count(account);
                        let ir = egui::Rect::from_center_size(r.center(), egui::vec2(24.0, 24.0));
                        let mut icon_ui =
                            ui.child_ui(ir, egui::Layout::top_down(egui::Align::Center));
                        icon_ui.add_enabled_ui(count > 0, |ui| icon_with_book(ui, e, book));
                        ui.painter().text(
                            r.right_bottom() - egui::vec2(5.0, 4.0),
                            egui::Align2::RIGHT_BOTTOM,
                            if matches!(e, Entry::Spell(_)) {
                                if count > 0 {
                                    "LMB".into()
                                } else {
                                    "!".into()
                                }
                            } else {
                                format!("{count}")
                            },
                            egui::FontId::proportional(14.0),
                            if count > 0 {
                                ivory
                            } else {
                                egui::Color32::GRAY
                            },
                        );
                        response.clone().on_hover_text(entry_name(e, book));
                    }
                    if disabled {
                        ui.painter().rect_filled(
                            r,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(65, 65, 65, 190),
                        );
                    }
                    if interactive && response.clicked() {
                        selected_slot = Some(i);
                    }
                }
            });
        });
    selected_slot
}
