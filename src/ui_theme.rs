//! Shared egui skin: existing screens inherit it without alternate gameplay UI.
use crate::settings::UiTheme;
use egui::{Color32, FontFamily, FontId, Pos2, Rect, Rounding, Stroke, TextStyle, Vec2};

pub fn apply(ctx: &egui::Context, theme: UiTheme) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("ui_theme"), theme));
    // Always start from defaults: switching back cannot retain fantasy colors or fonts.
    let mut style = egui::Style::default();
    let mut fonts = egui::FontDefinitions::default();
    let fantasy = theme == UiTheme::Fantasy;
    if fantasy {
        fonts.font_data.insert(
            "voxel_rune".into(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/voxel-rune.ttf")),
        );
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, "voxel_rune".into());
    }
    for (kind, size) in [
        (TextStyle::Body, if fantasy { 20.0 } else { 16.0 }),
        (TextStyle::Button, if fantasy { 20.0 } else { 16.0 }),
        (TextStyle::Small, if fantasy { 16.0 } else { 13.0 }),
        (TextStyle::Heading, if fantasy { 30.0 } else { 24.0 }),
        (TextStyle::Monospace, 16.0),
    ] {
        let family = if kind == TextStyle::Monospace {
            FontFamily::Monospace
        } else {
            FontFamily::Proportional
        };
        style.text_styles.insert(kind, FontId::new(size, family));
    }
    if fantasy {
        let ink = Color32::from_rgb(241, 226, 188);
        let bronze = Color32::from_rgb(164, 123, 65);
        let gold = Color32::from_rgb(236, 192, 99);
        let v = &mut style.visuals;
        v.window_rounding = Rounding::ZERO;
        v.menu_rounding = Rounding::ZERO;
        v.window_fill = Color32::from_rgb(30, 32, 31);
        v.panel_fill = Color32::from_rgb(19, 25, 29);
        v.window_stroke = Stroke::new(3.0_f32, bronze);
        v.window_highlight_topmost = false;
        v.extreme_bg_color = Color32::from_rgb(16, 19, 18);
        v.faint_bg_color = Color32::from_rgb(43, 44, 36);
        v.code_bg_color = Color32::from_rgb(18, 22, 22);
        v.hyperlink_color = gold;
        v.selection.bg_fill = Color32::from_rgb(83, 91, 51);
        v.selection.stroke = Stroke::new(2.0_f32, gold);
        v.text_cursor = Stroke::new(2.0_f32, gold);
        v.window_shadow = egui::epaint::Shadow {
            offset: Vec2::splat(5.0),
            blur: 0.0,
            spread: 0.0,
            color: Color32::from_black_alpha(150),
        };
        v.popup_shadow = v.window_shadow;
        for (widget, fill, border) in [
            (
                &mut v.widgets.noninteractive,
                Color32::from_rgb(30, 32, 31),
                bronze,
            ),
            (
                &mut v.widgets.inactive,
                Color32::from_rgb(60, 57, 44),
                bronze,
            ),
            (&mut v.widgets.hovered, Color32::from_rgb(85, 77, 48), gold),
            (&mut v.widgets.active, Color32::from_rgb(48, 61, 37), gold),
            (&mut v.widgets.open, Color32::from_rgb(54, 60, 40), gold),
        ] {
            widget.rounding = Rounding::ZERO;
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(2.0_f32, border);
            widget.fg_stroke = Stroke::new(1.0_f32, ink);
            widget.expansion = 0.0;
        }
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style.spacing.window_margin = egui::Margin::same(14.0);
        style.spacing.interact_size.y = 30.0;
        style.animation_time = 0.0;
    }
    ctx.set_fonts(fonts);
    ctx.set_style(style);
}

pub fn is_fantasy(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<UiTheme>(egui::Id::new("ui_theme"))) == Some(UiTheme::Fantasy)
}

/// Code-drawn pixel landscape, with no bitmap dependencies or animation cost.
pub fn menu_backdrop(ui: &egui::Ui) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::from_rgb(18, 26, 35));
    for i in 0..35 {
        let x = rect.left() + ((i * 137 + 31) % 997) as f32 / 997.0 * rect.width();
        let y = rect.top() + ((i * 59 + 17) % 311) as f32 / 700.0 * rect.height();
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x.floor(), y.floor()), Vec2::splat(2.0)),
            0.0,
            Color32::from_rgb(115, 131, 131),
        );
    }
    for (layer, color) in [
        Color32::from_rgb(29, 44, 48),
        Color32::from_rgb(31, 52, 47),
        Color32::from_rgb(23, 38, 34),
    ]
    .into_iter()
    .enumerate()
    {
        let step = 32.0;
        for column in 0..=(rect.width() / step) as usize {
            let rise = ((column * 7 + layer * 11) % 9) as f32 * 12.0;
            let top = rect.bottom() - 95.0 - (2 - layer) as f32 * 48.0 - rise;
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left() + column as f32 * step, top),
                    Pos2::new(rect.left() + (column + 1) as f32 * step, rect.bottom()),
                ),
                0.0,
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_switch_restores_generic_and_keeps_code_readable() {
        let ctx = egui::Context::default();
        apply(&ctx, UiTheme::Generic);
        let generic = ctx.style();
        apply(&ctx, UiTheme::Fantasy);
        assert_eq!(ctx.style().visuals.window_rounding, Rounding::ZERO);
        assert!(
            ctx.style().text_styles[&TextStyle::Body].size
                > generic.text_styles[&TextStyle::Body].size
        );
        assert_eq!(
            ctx.style().text_styles[&TextStyle::Monospace].family,
            FontFamily::Monospace
        );
        // Loading and laying out the embedded font also catches invalid font tables.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                menu_backdrop(ui);
                ui.label("Earth 12 / Mana 24 / Zażółć / 水");
            });
        });
        apply(&ctx, UiTheme::Generic);
        assert_eq!(ctx.style().visuals.window_fill, generic.visuals.window_fill);
        assert_eq!(
            ctx.style().visuals.window_rounding,
            generic.visuals.window_rounding
        );
        assert_eq!(ctx.style().text_styles, generic.text_styles);
    }
}
