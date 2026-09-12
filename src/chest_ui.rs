//! Storage-only chest view. Transfers use the existing host-owned inventory transactions.
use crate::automation::{account_count, element_id, Action, Device, Inventory};
use crate::crafting::Account;

fn label(id: &str) -> String {
    if let Some(block) = id
        .strip_prefix("resource:")
        .and_then(crate::voxel::BlockType::from_name)
    {
        return block.name().to_owned();
    }
    let (kind, name) = id.split_once(':').unwrap_or(("", id));
    let words = name.replace('_', " ");
    let mut chars = words.chars();
    let name = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    match kind {
        "element" => format!("{name} element"),
        "creature" => format!("Bound {words}"),
        _ => name,
    }
}

pub fn draw(
    ctx: &egui::Context,
    chest: &Device,
    account: &Account,
    open: &mut bool,
    selected: &mut String,
    amount: &mut u32,
    feedback: &str,
) -> Option<Action> {
    let mut carried = Inventory::new();
    for (block, &count) in crate::voxel::COLLECTIBLE_BLOCKS
        .iter()
        .zip(&account.resources)
    {
        if count > 0 {
            carried.insert(format!("resource:{}", block.id()), count);
        }
    }
    for i in 0..5 {
        if account.elements[i] > 0 {
            carried.insert(element_id(i).into(), account.elements[i]);
        }
    }
    for id in ["item:axe", "item:pickaxe", "item:sword"] {
        let count = account_count(account, id);
        if count > 0 {
            carried.insert(id.into(), count);
        }
    }
    carried.extend(
        account
            .production_goods
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(id, n)| (id.clone(), *n)),
    );
    let mut stored = chest.items.clone();
    for (id, count) in &chest.output {
        let entry = stored.entry(id.clone()).or_default();
        *entry = entry.saturating_add(*count);
    }
    if !carried.contains_key(selected) && !stored.contains_key(selected) {
        *selected = carried
            .keys()
            .next()
            .or_else(|| stored.keys().next())
            .cloned()
            .unwrap_or_default();
    }
    let mut action = None;
    egui::Window::new("Chest")
        .open(open)
        .default_width(660.0)
        .resizable(false)
        .default_pos(ctx.screen_rect().center() - egui::vec2(340.0, 260.0))
        .show(ctx, |ui| {
            ui.label("Select an item to move between your inventory and this chest.");
            ui.add_space(8.0);
            ui.columns(2, |columns| {
                for (index, items, title, empty) in [
                    (0, &carried, "Your inventory", "Your inventory is empty."),
                    (1, &stored, "Chest contents", "This chest is empty."),
                ] {
                    let ui = &mut columns[index];
                    ui.heading(title);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_source(("chest_items", index))
                            .auto_shrink([false, false])
                            .max_height(260.0)
                            .min_scrolled_height(260.0)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                if items.is_empty() {
                                    ui.weak(empty);
                                }
                                let mut rows: Vec<_> = items
                                    .iter()
                                    .filter(|(_, n)| **n > 0)
                                    .map(|(id, n)| (label(id), id, n))
                                    .collect();
                                rows.sort_by(|a, b| a.0.cmp(&b.0));
                                for (name, id, count) in rows {
                                    if ui
                                        .selectable_label(
                                            *selected == *id,
                                            format!("{name}  ×{count}"),
                                        )
                                        .clicked()
                                    {
                                        *selected = id.clone();
                                    }
                                }
                            });
                    });
                }
            });
            ui.add_space(8.0);
            let have = carried.get(selected).copied().unwrap_or(0);
            let stock = stored.get(selected).copied().unwrap_or(0);
            ui.horizontal(|ui| {
                ui.label(if selected.is_empty() {
                    "No item selected".into()
                } else {
                    label(selected)
                });
                ui.separator();
                ui.label("Amount");
                ui.add(egui::DragValue::new(amount).clamp_range(1..=have.max(stock).max(1)));
            });
            ui.columns(2, |columns| {
                columns[0].horizontal(|ui| {
                    if ui
                        .add_enabled(
                            have >= *amount,
                            egui::Button::new(format!("Store {}", amount)),
                        )
                        .clicked()
                    {
                        action = Some(Action::Deposit {
                            cell: chest.cell,
                            item: selected.clone(),
                            amount: *amount,
                        });
                    }
                    if ui
                        .add_enabled(have > 0, egui::Button::new("Store all"))
                        .clicked()
                    {
                        action = Some(Action::Deposit {
                            cell: chest.cell,
                            item: selected.clone(),
                            amount: have,
                        });
                    }
                });
                columns[1].horizontal(|ui| {
                    if ui
                        .add_enabled(
                            stock >= *amount,
                            egui::Button::new(format!("Take {}", amount)),
                        )
                        .clicked()
                    {
                        action = Some(Action::Withdraw {
                            cell: chest.cell,
                            item: selected.clone(),
                            amount: *amount,
                        });
                    }
                    if ui
                        .add_enabled(stock > 0, egui::Button::new("Take all"))
                        .clicked()
                    {
                        action = Some(Action::Withdraw {
                            cell: chest.cell,
                            item: selected.clone(),
                            amount: stock,
                        });
                    }
                });
            });
            if !feedback.is_empty() {
                ui.add_space(6.0);
                ui.label(if feedback == "Device updated" {
                    "Transfer complete."
                } else {
                    feedback
                });
            }
        });
    action
}
