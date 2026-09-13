//! Shared world directions: north is -Z, east is +X, independent of device rotation.
use egui::{Color32, Pos2};
use glam::{Mat4, Vec3};
pub const DIRECTIONS: [(&str, Vec3); 4] = [
    ("N", Vec3::NEG_Z),
    ("E", Vec3::X),
    ("S", Vec3::Z),
    ("W", Vec3::NEG_X),
];
pub fn bearing(yaw: f32) -> f32 {
    (yaw.to_degrees() + 90.).rem_euclid(360.)
}
pub fn hud(ctx: &egui::Context, yaw: f32) {
    let center = egui::pos2(ctx.screen_rect().center().x, 24.);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("heading_compass"),
    ));
    let width = (ctx.screen_rect().width() * 0.25).clamp(160., 300.);
    painter.rect_filled(
        egui::Rect::from_center_size(center, egui::vec2(width, 40.)),
        5.,
        Color32::from_black_alpha(170),
    );
    let heading = bearing(yaw);
    for (i, label) in ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]
        .iter()
        .enumerate()
    {
        let delta = (i as f32 * 45. - heading + 180.).rem_euclid(360.) - 180.;
        if delta.abs() > 80. {
            continue;
        }
        let p = center + egui::vec2(delta * width / 160., -3.);
        painter.text(
            p,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(14.),
            if i % 2 == 0 {
                Color32::WHITE
            } else {
                Color32::GRAY
            },
        );
    }
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(-4., 17.),
            center + egui::vec2(0., 10.),
            center + egui::vec2(4., 17.),
        ],
        Color32::from_rgb(240, 190, 100),
        egui::Stroke::NONE,
    ));
}
pub fn project(matrix: Mat4, point: Vec3, size: egui::Vec2) -> Option<Pos2> {
    let clip = matrix * point.extend(1.);
    if clip.w <= 0. {
        return None;
    }
    let p = clip.truncate() / clip.w;
    (p.x.abs() <= 1. && p.y.abs() <= 1. && (0. ..=1.).contains(&p.z)).then_some(egui::pos2(
        (p.x + 1.) * 0.5 * size.x,
        (1. - p.y) * 0.5 * size.y,
    ))
}
pub fn machine(ctx: &egui::Context, points: &[Option<Pos2>; 5]) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("machine_compass"),
    ));
    for (i, (label, _)) in DIRECTIONS.iter().enumerate() {
        let Some(p) = points[i + 1] else {
            continue;
        };
        let color = if i == 0 {
            Color32::from_rgb(255, 135, 100)
        } else {
            Color32::from_rgb(230, 215, 150)
        };
        if let Some(center) = points[0] {
            painter.line_segment([center, p], egui::Stroke::new(1.5_f32, color));
        }
        painter.circle_filled(p, 12., Color32::from_black_alpha(210));
        painter.text(
            p,
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(16.),
            color,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cardinal_headings_match_camera_and_map() {
        for (yaw, expected) in [
            (-90f32, 0.),
            (0., 90.),
            (90., 180.),
            (180., 270.),
            (270., 0.),
        ] {
            assert!((bearing(yaw.to_radians()) - expected).abs() < 0.001);
        }
        let mut camera = crate::camera::Camera::new(Vec3::ZERO, 1.);
        for ((_, direction), yaw) in DIRECTIONS.into_iter().zip([-90f32, 0., 90., 180.]) {
            camera.yaw = yaw.to_radians();
            assert!(camera.forward().distance(direction) < 0.001);
        }
        assert!(project(
            camera.view_proj(),
            camera.eye_position() - camera.forward(),
            egui::vec2(720., 720.)
        )
        .is_none());
    }
}
