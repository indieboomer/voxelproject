//! Shared resource atlas icons, using the same pixels as placed world blocks.
use crate::voxel::{atlas, BlockType};

pub fn icon(ui: &mut egui::Ui, block: BlockType) {
    let key = egui::Id::new("resource_atlas_texture");
    let existing = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(key));
    let texture = existing.unwrap_or_else(|| {
        let image = image::load_from_memory(atlas::ATLAS_BYTES)
            .expect("embedded atlas")
            .to_rgba8();
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let texture =
            ui.ctx()
                .load_texture("resource_atlas", pixels, egui::TextureOptions::NEAREST);
        ui.ctx()
            .data_mut(|data| data.insert_temp(key, texture.clone()));
        texture
    });
    let [u0, v0, u1, v1] = atlas::uv_rect(atlas::tile_for(block, 2));
    ui.add(
        egui::Image::new((texture.id(), egui::vec2(24.0, 24.0))).uv(egui::Rect::from_min_max(
            egui::pos2(u0, v0),
            egui::pos2(u1, v1),
        )),
    );
}

pub fn description(block: BlockType) -> String {
    let info = crate::voxel::resource_catalog::info(block);
    let creature_loot=(0..=12).any(|kind|crate::loot::rewards(crate::creature::CreatureKind::from_u8(kind)).iter().any(|(b,_)|*b==block));
    let source = match (info.source,creature_loot) {
        ("loot",_) => "Creature loot",
        ("cooked",_) => "Campfire cooking",
        ("crafted",true) => "Crafting + creature loot",
        (_,true) => "Natural/craftable + creature loot",
        ("crafted",false) => "Crafting only",
        ("both",false) => "Natural + craftable",
        _ => "Natural",
    };
    let mut description=format!(
        "{} | {source}\n{}\n{} hits to gather",
        info.category,
        if creature_loot {"Also recovered from defeated creatures"} else {info.location},
        block.hardness()
    );
    if let Some(healing)=crate::food::healing(block) {description.push_str(&format!("\nEat in inventory [I]: restores {healing:.0} health."));}
    if crate::food::inventory_only(block) {description=format!("Food | {source}\n{}\nEat in inventory [I]: restores {:.0} health.",info.location,crate::food::healing(block).unwrap());}
    description
}
