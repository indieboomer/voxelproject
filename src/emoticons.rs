//! Original vector chat icons, drawn in egui without external assets or fonts.
use egui::{Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emote {
    Smile,
    Laugh,
    Wink,
    Sad,
    Angry,
    Surprised,
    Heart,
    ThumbsUp,
}
pub const ALL: &[Emote] = &[
    Emote::Smile,
    Emote::Laugh,
    Emote::Wink,
    Emote::Sad,
    Emote::Angry,
    Emote::Surprised,
    Emote::Heart,
    Emote::ThumbsUp,
];

impl Emote {
    pub fn code(self) -> &'static str {
        match self {
            Self::Smile => ":smile:",
            Self::Laugh => ":laugh:",
            Self::Wink => ":wink:",
            Self::Sad => ":sad:",
            Self::Angry => ":angry:",
            Self::Surprised => ":surprised:",
            Self::Heart => ":heart:",
            Self::ThumbsUp => ":thumbsup:",
        }
    }
}

#[derive(Debug, PartialEq)]
enum Part<'a> {
    Text(&'a str),
    Icon(Emote),
}

fn parse(text: &str) -> Vec<Part<'_>> {
    let aliases = [
        (":-)", Emote::Smile),
        (":)", Emote::Smile),
        (":D", Emote::Laugh),
        (";-)", Emote::Wink),
        (";)", Emote::Wink),
        (":-(", Emote::Sad),
        (":(", Emote::Sad),
        (">:(", Emote::Angry),
        (":O", Emote::Surprised),
        ("<3", Emote::Heart),
    ];
    let mut parts = Vec::new();
    let (mut start, mut at) = (0, 0);
    while at < text.len() {
        let rest = &text[at..];
        let found = ALL
            .iter()
            .find(|e| rest.starts_with(e.code()))
            .map(|e| (*e, e.code().len()))
            .or_else(|| {
                aliases
                    .iter()
                    .find(|(code, _)| {
                        rest.starts_with(code)
                            && (at == 0
                                || text[..at]
                                    .chars()
                                    .next_back()
                                    .is_some_and(char::is_whitespace))
                            && rest[code.len()..].chars().next().is_none_or(|c| {
                                c.is_whitespace() || matches!(c, '!' | '?' | ',' | '.')
                            })
                    })
                    .map(|(code, e)| (*e, code.len()))
            });
        if let Some((emote, len)) = found {
            if at > start {
                parts.push(Part::Text(&text[start..at]));
            }
            parts.push(Part::Icon(emote));
            at += len;
            start = at;
        } else {
            at += rest.chars().next().unwrap().len_utf8();
        }
    }
    if start < text.len() {
        parts.push(Part::Text(&text[start..]));
    }
    parts
}

pub fn draw(p: &Painter, rect: Rect, icon: Emote, alpha: f32) {
    let point = |x: f32, y: f32| rect.min + Vec2::new(x * rect.width(), y * rect.height());
    let color = |r, g, b| Color32::from_rgb(r, g, b).linear_multiply(alpha);
    let ink = color(65, 39, 32);
    let stroke = Stroke::new((rect.width() * 0.065).max(1.0), ink);
    let line = |points: &[(f32, f32)]| {
        p.add(Shape::line(
            points.iter().map(|&(x, y)| point(x, y)).collect(),
            stroke,
        ));
    };
    if icon == Emote::Heart {
        let red = color(239, 70, 103);
        p.circle_filled(point(0.33, 0.35), rect.width() * 0.23, red);
        p.circle_filled(point(0.67, 0.35), rect.width() * 0.23, red);
        p.add(Shape::convex_polygon(
            vec![point(0.1, 0.38), point(0.9, 0.38), point(0.5, 0.91)],
            red,
            Stroke::NONE,
        ));
        p.circle_filled(
            point(0.26, 0.27),
            rect.width() * 0.065,
            color(255, 171, 183),
        );
        return;
    }
    if icon == Emote::ThumbsUp {
        let gold = color(255, 194, 68);
        p.rect_filled(
            Rect::from_min_max(point(0.31, 0.4), point(0.87, 0.88)),
            rect.width() * 0.09,
            gold,
        );
        p.rect_filled(
            Rect::from_min_max(point(0.36, 0.1), point(0.56, 0.63)),
            rect.width() * 0.08,
            gold,
        );
        p.rect_filled(
            Rect::from_min_max(point(0.07, 0.47), point(0.28, 0.9)),
            rect.width() * 0.04,
            color(76, 154, 232),
        );
        line(&[(0.64, 0.56), (0.85, 0.56)]);
        line(&[(0.64, 0.71), (0.85, 0.71)]);
        return;
    }
    let face = if icon == Emote::Angry {
        color(245, 119, 69)
    } else {
        color(255, 205, 74)
    };
    p.circle_filled(rect.center(), rect.width() * 0.46, ink);
    p.circle_filled(rect.center(), rect.width() * 0.415, face);
    if icon == Emote::Wink {
        line(&[(0.23, 0.39), (0.39, 0.39)]);
    } else {
        p.circle_filled(point(0.32, 0.39), rect.width() * 0.055, ink);
    }
    p.circle_filled(point(0.68, 0.39), rect.width() * 0.055, ink);
    match icon {
        Emote::Smile | Emote::Wink => line(&[
            (0.27, 0.59),
            (0.36, 0.68),
            (0.5, 0.72),
            (0.64, 0.68),
            (0.73, 0.59),
        ]),
        Emote::Laugh => {
            p.circle_filled(point(0.5, 0.64), rect.width() * 0.19, ink);
            p.rect_filled(
                Rect::from_min_max(point(0.36, 0.47), point(0.64, 0.55)),
                1.0,
                color(255, 249, 227),
            );
            p.circle_filled(point(0.5, 0.75), rect.width() * 0.085, color(246, 108, 105));
        }
        Emote::Sad | Emote::Angry => {
            line(&[
                (0.29, 0.75),
                (0.38, 0.65),
                (0.5, 0.62),
                (0.62, 0.65),
                (0.71, 0.75),
            ]);
            if icon == Emote::Angry {
                line(&[(0.21, 0.25), (0.42, 0.32)]);
                line(&[(0.58, 0.32), (0.79, 0.25)]);
            }
        }
        Emote::Surprised => {
            p.circle_filled(point(0.5, 0.67), rect.width() * 0.13, ink);
        }
        _ => (),
    }
}

pub struct Line {
    pub galley: std::sync::Arc<egui::Galley>,
    icons: Vec<(usize, Emote)>,
}

pub fn layout(p: &Painter, text: &str, color: Color32, width: f32) -> Line {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let mut icons = Vec::new();
    for part in parse(text) {
        match part {
            Part::Text(text) => job.append(
                text,
                0.0,
                egui::TextFormat::simple(FontId::proportional(16.0), color),
            ),
            Part::Icon(icon) => {
                icons.push((job.sections.len(), icon));
                // An invisible fixed-width glyph reserves inline space, including wrapping.
                job.append(
                    "M",
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::monospace(20.0),
                        color: Color32::TRANSPARENT,
                        extra_letter_spacing: 9.0,
                        line_height: Some(24.0),
                        ..Default::default()
                    },
                );
            }
        }
    }
    Line {
        galley: p.layout_job(job),
        icons,
    }
}

impl Line {
    pub fn paint(&self, p: &Painter, origin: Pos2, alpha: f32) {
        p.galley(origin, self.galley.clone(), Color32::WHITE);
        for row in &self.galley.rows {
            for glyph in &row.glyphs {
                if let Some((_, icon)) = self
                    .icons
                    .iter()
                    .find(|(section, _)| *section == glyph.section_index as usize)
                {
                    let center = origin + glyph.logical_rect().center().to_vec2();
                    draw(
                        p,
                        Rect::from_center_size(center, Vec2::splat(20.0)),
                        *icon,
                        alpha,
                    );
                }
            }
        }
    }
}

pub fn label(ui: &mut egui::Ui, text: &str, color: Color32) {
    let line = layout(ui.painter(), text, color, ui.available_width());
    let (rect, response) = ui.allocate_exact_size(line.galley.size(), egui::Sense::hover());
    line.paint(ui.painter(), rect.min, 1.0);
    response.on_hover_text(text);
}

pub fn picker(ui: &mut egui::Ui, draft: &mut String) -> bool {
    let mut inserted = false;
    ui.horizontal(|ui| {
        for &icon in ALL {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
            }
            draw(ui.painter(), rect.shrink(3.0), icon, 1.0);
            if response.clicked() {
                let separator = if draft.is_empty() || draft.ends_with(char::is_whitespace) {
                    ""
                } else {
                    " "
                };
                let addition = format!("{separator}{} ", icon.code());
                if draft.chars().count() + addition.chars().count() <= crate::net::MAX_CHAT_LEN {
                    draft.push_str(&addition);
                    inserted = true;
                }
            }
            response.on_hover_text(icon.code());
        }
    });
    inserted
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_codes_aliases_and_unicode_without_rewriting_other_text() {
        assert_eq!(
            parse("Hi :smile: <3"),
            vec![
                Part::Text("Hi "),
                Part::Icon(Emote::Smile),
                Part::Text(" "),
                Part::Icon(Emote::Heart)
            ]
        );
        assert_eq!(
            parse("żółw:heart:猫"),
            vec![
                Part::Text("żółw"),
                Part::Icon(Emote::Heart),
                Part::Text("猫")
            ]
        );
        for text in ["https://example.com/:)", "x<3", ":unknown:", ":smi"] {
            assert_eq!(parse(text), vec![Part::Text(text)]);
        }
        for &icon in ALL {
            assert_eq!(parse(icon.code()), vec![Part::Icon(icon)]);
        }
    }
    #[test]
    fn wrapped_inline_icons_keep_their_own_layout_space() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            let painter = ctx.debug_painter();
            let line = layout(
                &painter,
                ":smile: :heart: :wink: :thumbsup:",
                Color32::WHITE,
                55.0,
            );
            assert!(line.galley.rows.len() > 1);
            assert_eq!(line.icons.len(), 4);
            assert_eq!(
                line.galley
                    .rows
                    .iter()
                    .flat_map(|r| &r.glyphs)
                    .filter(|g| line
                        .icons
                        .iter()
                        .any(|(s, _)| *s == g.section_index as usize))
                    .count(),
                4
            );
            line.paint(&painter, Pos2::ZERO, 0.5);
        });
    }
}
