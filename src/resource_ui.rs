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
    let source = match info.source {
        "crafted" => "Crafting only",
        "both" => "Natural + craftable",
        _ => "Natural",
    };
    format!(
        "{} | {source}\n{}\n{} hits to gather",
        info.category,
        info.location,
        block.hardness()
    )
}
