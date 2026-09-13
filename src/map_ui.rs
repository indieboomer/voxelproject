//! Cached overhead terrain with live authoritative build and player markers.
use crate::voxel::{BlockType, World};
use egui::{Color32, Pos2, Vec2};

pub struct Map {
    pub open: bool,
    pub waypoint: Option<glam::Vec3>,
    pub home: Option<glam::Vec3>,
    center: Option<Vec2>,
    span: f32,
    cached: Option<(u32, i32, i32, u32)>,
    texture: Option<egui::TextureHandle>,
    entrances: Vec<(i32, i32, i32)>,
}
impl Default for Map {
    fn default() -> Self {
        Self {
            open: false,
            waypoint: None,
            home: None,
            center: None,
            span: 512.0,
            cached: None,
            texture: None,
            entrances: Vec::new(),
        }
    }
}
impl Map {
    pub fn toggle(&mut self, pos: glam::Vec3) {
        self.open = !self.open;
        if self.open {
            self.center = Some(Vec2::new(pos.x, pos.z));
        }
    }
    pub fn draw(&mut self, ctx: &egui::Context, world: &World, player: glam::Vec3) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let side = (ctx.screen_rect().height() - 310.0)
            .min(ctx.screen_rect().width() - 60.0)
            .clamp(160.0, 620.0);
        egui::Window::new("World map [M]")
            .default_width(660.0)
            .open(&mut open)
            .resizable(false)
            .default_pos(Pos2::new((ctx.screen_rect().width() - 680.0) * 0.5, 12.0))
            .show(ctx, |ui| {
                let mut center = self.center.unwrap_or(Vec2::new(player.x, player.z));
                ui.horizontal(|ui| {
                    if ui.button("Center on me").clicked() {
                        center = Vec2::new(player.x, player.z);
                    }
                    if ui.button("Zoom in").clicked() {
                        self.span = (self.span * 0.5).max(128.0);
                    }
                    if ui.button("Zoom out").clicked() {
                        self.span = (self.span * 2.0).min(1024.0);
                    }
                    ui.label(format!("{} blocks across", self.span as u32));
                });
                ui.horizontal(|ui| {
                    if ui.button("Clear waypoint").clicked() {self.waypoint=None;}
                    if let Some(home)=self.home {
                        if ui.button("Mark recovery camp").clicked() {self.waypoint=Some(home);}
                    }
                    if ui.button("Mark nearest cave").clicked() {
                        self.waypoint=crate::underground::entrances(world,
                            (player.x as i32-192,player.z as i32-192),(player.x as i32+192,player.z as i32+192))
                            .into_iter().min_by_key(|p| ((p.0 as f32-player.x).powi(2)+(p.2 as f32-player.z).powi(2)) as u32)
                            .map(crate::adventure::feet);
                    }
                });
                ui.label(format!(
                    "Your position: X {:.0}, Y {:.0}, Z {:.0}   |   North is up",
                    player.x, player.y, player.z
                ));
                let (rect, response) = ui
                    .horizontal(|ui| {
                        ui.add_space(((ui.available_width() - side) * 0.5).max(0.0));
                        ui.allocate_exact_size(Vec2::splat(side), egui::Sense::click_and_drag())
                    })
                    .inner;
                if response.secondary_clicked() {
                    if let Some(p)=response.interact_pointer_pos() {
                        let world_pos=center+(p-rect.center())*(self.span/side);
                        self.waypoint=Some(glam::Vec3::new(world_pos.x,world.terrain_height(world_pos.x as i32,world_pos.y as i32) as f32+1.,world_pos.y));
                    }
                }
                if response.dragged() {
                    center -= ui.input(|i| i.pointer.delta()) * (self.span / side);
                }
                if response.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 0.1 {
                        self.span = (self.span * (-scroll * 0.003).exp()).clamp(128.0, 1024.0);
                    }
                }
                center = center.clamp(Vec2::splat(-998_000.0), Vec2::splat(998_000.0));
                self.center = Some(center);
                let key = (
                    world.seed,
                    center.x.floor() as i32,
                    center.y.floor() as i32,
                    self.span as u32,
                );
                if self.cached != Some(key) {
                    let mut image = egui::ColorImage::new([128, 128], Color32::BLACK);
                    for z in 0..128 {
                        for x in 0..128 {
                            let wx = (center.x - self.span * 0.5 + x as f32 * self.span / 128.0)
                                .floor() as i32;
                            let wz = (center.y - self.span * 0.5 + z as f32 * self.span / 128.0)
                                .floor() as i32;
                            let h = world.terrain_height(wx, wz);
                            let water = world.generation.shape == crate::worldgen::Shape::Mainland
                                && crate::voxel::terrain::tributary(wx, wz, world.seed)
                                    .is_some_and(|p| p.0 < p.1);
                            image[(x, z)] = if h < crate::voxel::world::SEA_LEVEL || water {
                                Color32::from_rgb(38, 104, 160)
                            } else if h <= crate::voxel::world::SEA_LEVEL + 1 {
                                Color32::from_rgb(192, 178, 116)
                            } else if world.generation.surface == crate::worldgen::Surface::Snow {
                                Color32::from_rgb(220, 232, 237)
                            } else if world.generation.surface == crate::worldgen::Surface::Sand {
                                Color32::from_rgb(198, 183, 126)
                            } else {
                                Color32::from_rgb(
                                    (60 + h).min(180) as u8,
                                    (100 + h * 2).min(200) as u8,
                                    (52 + h).min(130) as u8,
                                )
                            };
                        }
                    }
                    if let Some(texture) = &mut self.texture {
                        texture.set(image, egui::TextureOptions::NEAREST);
                    } else {
                        self.texture = Some(ctx.load_texture(
                            "world_map",
                            image,
                            egui::TextureOptions::NEAREST,
                        ));
                    }
                    let min = (
                        (center.x - self.span / 2.) as i32,
                        (center.y - self.span / 2.) as i32,
                    );
                    let max = (
                        (center.x + self.span / 2.) as i32,
                        (center.y + self.span / 2.) as i32,
                    );
                    self.entrances = crate::underground::entrances(world, min, max);
                    self.cached = Some(key);
                }
                let painter = ui.painter_at(rect);
                if let Some(texture) = &self.texture {
                    painter.image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                        Color32::WHITE,
                    );
                }
                let project = |x: f32, z: f32| {
                    rect.center() + Vec2::new(x - center.x, z - center.y) * (side / self.span)
                };
                let mut structures = std::collections::BTreeSet::new();
                for (&(x, _, z), block) in &world.edits {
                    if block.is_solid() && *block != BlockType::Air {
                        structures.insert((x, z));
                    }
                }
                for d in world.automation.devices.values() {
                    structures.insert((d.cell.0, d.cell.2));
                }
                for (x, z) in structures {
                    let p = project(x as f32 + 0.5, z as f32 + 0.5);
                    if rect.contains(p) {
                        painter.rect_filled(
                            egui::Rect::from_center_size(p, Vec2::splat(4.0)),
                            0.0,
                            Color32::YELLOW,
                        );
                    }
                }
                for &(x, _, z) in &self.entrances {
                    let p = project(x as f32, z as f32);
                    if rect.contains(p) {
                        painter.circle_filled(p, 5.0, Color32::from_rgb(25, 16, 35));
                        painter.circle_stroke(
                            p,
                            5.0,
                            egui::Stroke::new(2.0_f32, Color32::from_rgb(210, 135, 245)),
                        );
                        if response.hover_pos().is_some_and(|h| h.distance(p) < 10.0) {
                            response
                                .clone()
                                .on_hover_text(format!("Cave entrance: X {x}, Z {z}"));
                        }
                    }
                }
                let p = project(player.x, player.z);
                for (target,color,label) in [(self.home,Color32::from_rgb(240,170,80),"Camp"),(self.waypoint,Color32::from_rgb(100,220,235),"Waypoint")] {
                    if let Some(target)=target {
                        let p=project(target.x,target.z);
                        if rect.contains(p) {painter.circle_stroke(p,7.,egui::Stroke::new(2.0_f32,color));painter.text(p+egui::vec2(0.,9.),egui::Align2::CENTER_TOP,label,egui::FontId::proportional(12.),color);}
                    }
                }
                if rect.contains(p) {
                    painter.circle_filled(p, 6.0, Color32::WHITE);
                    painter.circle_filled(p, 3.0, Color32::from_rgb(230, 65, 65));
                }
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::WHITE, "You");
                    ui.colored_label(Color32::from_rgb(210, 135, 245), "Cave entrances");
                    ui.colored_label(Color32::YELLOW, "Built blocks / devices / chests");
                });
                ui.small("Drag to pan. Scroll to zoom. Right-click to mark a waypoint. M or Esc returns.");
            });
        self.open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_reveals_entrances_without_generating_chunks_or_rewards() {
        let world = World::new(42);
        let entrances = crate::underground::entrances(&world, (-256, -256), (256, 256));
        assert!(!entrances.is_empty());
        assert!(world.chunks.is_empty());
        assert!(world.cave_layouts.borrow().is_empty());
        assert!(world.automation.devices.is_empty());
        let mut map = Map::default();
        map.toggle(glam::Vec3::new(-17., 70., 35.));
        assert!(map.open);
        assert_eq!(map.center, Some(Vec2::new(-17., 35.)));
        map.toggle(glam::Vec3::ZERO);
        assert!(!map.open);
    }
}
