//! Versioned, deterministic illustrations. Only bounded data reaches the renderer.
//! Recipes are the lossless image source stored with spells/modules, not file paths.
use crate::settings::SpellArtwork;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{mpsc, Mutex, OnceLock};

macro_rules! vocabulary {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename=$text)] $variant),+ }
        impl $name { fn values() -> Vec<&'static str> { vec![$($text),+] } }
    }
}
vocabulary!(Subject { Rune=>"rune", Sheep=>"sheep", Wolf=>"wolf", Creature=>"creature", Dragon=>"dragon", Person=>"person", Sword=>"sword", Shield=>"shield", Crystal=>"crystal", Tree=>"tree", Block=>"block", Heart=>"heart", Hand=>"hand", Flame=>"flame", Cloud=>"cloud", Skull=>"skull" });
vocabulary!(Effect { Aura=>"aura", Heal=>"heal", Fire=>"fire", Frost=>"frost", Lightning=>"lightning", Growth=>"growth", Summon=>"summon", Protect=>"protect", Hunt=>"hunt", Transform=>"transform" });
vocabulary!(Accent { Stars=>"stars", Moon=>"moon", Sun=>"sun", Rain=>"rain", Crystal=>"crystal", Heart=>"heart", Chain=>"chain", None=>"none" });
vocabulary!(Palette { Arcane=>"arcane", Ember=>"ember", Verdant=>"verdant", Frost=>"frost", Gold=>"gold", Blood=>"blood" });

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Recipe {
    pub version: u8,
    pub subject: Subject,
    pub effect: Effect,
    pub accent: Accent,
    pub palette: Palette,
}
impl Default for Recipe {
    fn default() -> Self {
        Self {
            version: 1,
            subject: Subject::Rune,
            effect: Effect::Aura,
            accent: Accent::Stars,
            palette: Palette::Arcane,
        }
    }
}
/// Cosmetic output must not invalidate an otherwise usable generated rule.
pub fn optional_recipe<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Recipe>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<Recipe>(value)
        .ok()
        .filter(|r| r.supported()))
}
impl Recipe {
    pub fn schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,
            "required":["version","subject","effect","accent","palette"],"properties":{
            "version":{"type":"integer","enum":[1]},
            "subject":{"type":"string","enum":Subject::values()},
            "effect":{"type":"string","enum":Effect::values()},
            "accent":{"type":"string","enum":Accent::values()},
            "palette":{"type":"string","enum":Palette::values()}}})
    }
    pub fn supported(self) -> bool {
        self.version == 1
    }
    pub fn color(self) -> [u8; 3] {
        match self.palette {
            Palette::Arcane => [177, 133, 244],
            Palette::Ember => [255, 153, 69],
            Palette::Verdant => [108, 221, 160],
            Palette::Frost => [108, 205, 248],
            Palette::Gold => [243, 209, 125],
            Palette::Blood => [243, 109, 128],
        }
    }
    pub fn tint(self) -> egui::Color32 {
        let [r, g, b] = self.color();
        egui::Color32::from_rgb(r, g, b)
    }
    pub fn from_source(source: &str, description: &str) -> Self {
        if let Some(recipe) = source
            .lines()
            .find_map(|s| s.strip_prefix("-- Intent plan: "))
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|p| p.get("artwork").cloned())
            .and_then(|v| serde_json::from_value::<Self>(v).ok())
            .filter(|r| r.supported())
        {
            return recipe;
        }
        Self::fallback(description)
    }
    /// Legacy/manual modules have no AI recipe. This affects presentation only.
    pub fn fallback(description: &str) -> Self {
        let text = description.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
        let subject = if has(&["sheep"]) {
            Subject::Sheep
        } else if has(&["wolf", "wolves"]) {
            Subject::Wolf
        } else if has(&["dragon"]) {
            Subject::Dragon
        } else if has(&["sword"]) {
            Subject::Sword
        } else if has(&["crystal"]) {
            Subject::Crystal
        } else if has(&["tree", "forest"]) {
            Subject::Tree
        } else if has(&["creature", "monster", "goblin"]) {
            Subject::Creature
        } else if has(&["heal", "health"]) {
            Subject::Heart
        } else if has(&["stone", "block", "soil", "mud"]) {
            Subject::Block
        } else if has(&["rain", "weather", "storm"]) {
            Subject::Cloud
        } else if has(&["player", "person"]) {
            Subject::Person
        } else {
            Subject::Rune
        };
        let (effect, palette) = if has(&["heal", "health"]) {
            (Effect::Heal, Palette::Verdant)
        } else if has(&["fire", "burn", "flame"]) {
            (Effect::Fire, Palette::Ember)
        } else if has(&["ice", "frost", "freeze"]) {
            (Effect::Frost, Palette::Frost)
        } else if has(&["hunt", "attack", "damage", "kill"]) {
            (Effect::Hunt, Palette::Blood)
        } else if has(&["protect", "shield", "safe"]) {
            (Effect::Protect, Palette::Gold)
        } else if has(&["lightning", "storm"]) {
            (Effect::Lightning, Palette::Frost)
        } else if has(&["spawn", "summon"]) {
            (Effect::Summon, Palette::Arcane)
        } else if has(&["grow", "tree"]) {
            (Effect::Growth, Palette::Verdant)
        } else {
            (Effect::Transform, Palette::Arcane)
        };
        let accent = if has(&["night", "moon"]) {
            Accent::Moon
        } else if has(&["rain"]) {
            Accent::Rain
        } else if has(&["attach", "bound"]) {
            Accent::Chain
        } else if has(&["crystal"]) {
            Accent::Crystal
        } else {
            Accent::Stars
        };
        Self {
            version: 1,
            subject,
            effect,
            accent,
            palette,
        }
    }
}

const SIZE: usize = 128;
const CACHE_LIMIT: usize = 128;
type Color = [u8; 4];
struct Canvas {
    pixels: Vec<u8>,
}
impl Canvas {
    fn paint(&mut self, x: usize, y: usize, c: Color) {
        let i = (y * SIZE + x) * 4;
        let a = c[3] as u32;
        for k in 0..3 {
            self.pixels[i + k] =
                ((c[k] as u32 * a + self.pixels[i + k] as u32 * (255 - a)) / 255) as u8;
        }
        self.pixels[i + 3] = 255;
    }
    fn shape(&mut self, bounds: [f32; 4], c: Color, inside: impl Fn(f32, f32) -> bool) {
        for y in (bounds[1].floor().max(0.) as usize)..(bounds[3].ceil().min(128.) as usize) {
            for x in (bounds[0].floor().max(0.) as usize)..(bounds[2].ceil().min(128.) as usize) {
                let mut hits = 0;
                for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    if inside(x as f32 + dx, y as f32 + dy) {
                        hits += 1;
                    }
                }
                if hits > 0 {
                    let mut color = c;
                    color[3] = (c[3] as u32 * hits / 4) as u8;
                    self.paint(x, y, color);
                }
            }
        }
    }
    fn ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32, c: Color) {
        self.shape([x - rx, y - ry, x + rx, y + ry], c, |px, py| {
            ((px - x) / rx).powi(2) + ((py - y) / ry).powi(2) <= 1.
        });
    }
    fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, c: Color) {
        let r = width / 2.;
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len = dx * dx + dy * dy;
        self.shape(
            [
                a.0.min(b.0) - r,
                a.1.min(b.1) - r,
                a.0.max(b.0) + r,
                a.1.max(b.1) + r,
            ],
            c,
            |x, y| {
                let t = if len > 0. {
                    ((x - a.0) * dx + (y - a.1) * dy) / len
                } else {
                    0.
                }
                .clamp(0., 1.);
                (x - a.0 - t * dx).powi(2) + (y - a.1 - t * dy).powi(2) <= r * r
            },
        );
    }
    fn polygon(&mut self, points: &[(f32, f32)], c: Color) {
        let mut bounds = [128., 128., 0f32, 0f32];
        for &(x, y) in points {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        }
        self.shape(bounds, c, |x, y| {
            let mut hit = false;
            let mut j = points.len() - 1;
            for i in 0..points.len() {
                let a = points[i];
                let b = points[j];
                if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
                    hit = !hit;
                }
                j = i;
            }
            hit
        });
    }
    fn ring(&mut self, x: f32, y: f32, rx: f32, ry: f32, c: Color) {
        for i in 0..64 {
            let a = i as f32 * std::f32::consts::TAU / 64.;
            let b = (i + 1) as f32 * std::f32::consts::TAU / 64.;
            self.line(
                (x + rx * a.cos(), y + ry * a.sin()),
                (x + rx * b.cos(), y + ry * b.sin()),
                1.,
                c,
            );
        }
    }
    fn star(&mut self, x: f32, y: f32, r: f32, c: Color) {
        self.polygon(
            &[
                (x, y - r),
                (x + 1., y - 1.),
                (x + r, y),
                (x + 1., y + 1.),
                (x, y + r),
                (x - 1., y + 1.),
                (x - r, y),
                (x - 1., y - 1.),
            ],
            c,
        );
    }
}

/// Renderer v1 remains stable: saved recipes recreate the same image bytes offline.
pub fn render(recipe: Recipe) -> Vec<u8> {
    let recipe = if recipe.supported() {
        recipe
    } else {
        Recipe::default()
    };
    let [r, g, b] = recipe.color();
    let light = [r, g, b, 255];
    let ivory = [250, 235, 199, 255];
    let dark = [24, 28, 45, 255];
    let mut c = Canvas {
        pixels: vec![0; SIZE * SIZE * 4],
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = ((x as f32 - 64.).powi(2) + (y as f32 - 57.).powi(2)).sqrt() / 95.;
            let glow = (1. - distance).max(0.);
            let i = (y * SIZE + x) * 4;
            for (k, tint) in [r, g, b].iter().enumerate() {
                c.pixels[i + k] = (8. + *tint as f32 * (0.06 + glow * 0.22)) as u8;
            }
            c.pixels[i + 3] = 255;
        }
    }
    // Engraved orbital geometry and a luminous pedestal keep the collection coherent.
    c.ring(64., 60., 44., 44., [r, g, b, 70]);
    c.ring(64., 60., 40., 40., [r, g, b, 35]);
    c.ellipse(64., 106., 42., 9., [r, g, b, 25]);
    c.ring(64., 104., 32., 5., [r, g, b, 160]);
    for (x, y, s) in [
        (17., 33., 2.),
        (105., 48., 3.),
        (28., 85., 2.),
        (88., 16., 2.),
        (111., 91., 2.),
        (48., 23., 1.),
    ] {
        c.star(x, y, s, [r, g, b, 180]);
    }
    match recipe.effect {
        Effect::Fire => {
            for (x, y) in [(29., 87.), (93., 88.), (43., 94.)] {
                c.polygon(
                    &[
                        (x - 9., y),
                        (x - 4., y - 17.),
                        (x + 1., y - 33.),
                        (x + 5., y - 15.),
                        (x + 10., y),
                        (x, y + 8.),
                    ],
                    [255, 128, 42, 190],
                );
            }
        }
        Effect::Lightning => {
            for x in [28., 92.] {
                c.polygon(
                    &[
                        (x + 8., 24.),
                        (x - 7., 56.),
                        (x + 1., 56.),
                        (x - 5., 82.),
                        (x + 15., 46.),
                        (x + 5., 46.),
                    ],
                    [173, 232, 255, 235],
                );
            }
        }
        Effect::Frost => {
            for (x, y) in [(28., 45.), (97., 80.)] {
                for i in 0..3 {
                    let a = i as f32 * std::f32::consts::PI / 3.;
                    c.line(
                        (x - 10. * a.cos(), y - 10. * a.sin()),
                        (x + 10. * a.cos(), y + 10. * a.sin()),
                        2.,
                        [162, 230, 255, 220],
                    );
                }
            }
        }
        Effect::Protect => {
            c.polygon(
                &[
                    (64., 22.),
                    (100., 36.),
                    (94., 80.),
                    (64., 107.),
                    (34., 80.),
                    (28., 36.),
                ],
                [r, g, b, 45],
            );
        }
        Effect::Hunt => {
            for x in [29., 40., 51.] {
                c.line((x + 22., 28.), (x, 98.), 2., [255, 93, 107, 130]);
            }
        }
        Effect::Growth => {
            for x in [29., 97.] {
                c.line((x, 97.), (x, 53.), 2., light);
                for y in [63., 78., 90.] {
                    c.ellipse(x - 5., y, 6., 3., [92, 210, 139, 200]);
                    c.ellipse(x + 5., y - 6., 6., 3., [92, 210, 139, 200]);
                }
            }
        }
        Effect::Summon | Effect::Transform => {
            c.ring(64., 64., 49., 24., [r, g, b, 100]);
        }
        _ => {}
    }
    match recipe.subject {
        Subject::Sheep => {
            for (x, y, rx, ry) in [
                (48., 65., 15., 15.),
                (62., 59., 17., 18.),
                (78., 64., 14., 15.),
                (62., 72., 23., 13.),
            ] {
                c.ellipse(x, y, rx, ry, ivory);
            }
            c.line((49., 77.), (47., 92.), 6., dark);
            c.line((74., 77.), (76., 92.), 6., dark);
            c.ellipse(82., 65., 10., 13., dark);
            c.ellipse(91., 56., 7., 3., ivory);
            c.ellipse(85., 62., 2., 2., light);
        }
        Subject::Wolf | Subject::Creature | Subject::Dragon => {
            c.polygon(
                &[
                    (42., 42.),
                    (48., 23.),
                    (62., 42.),
                    (81., 26.),
                    (87., 58.),
                    (79., 81.),
                    (64., 92.),
                    (46., 77.),
                ],
                ivory,
            );
            c.polygon(&[(45., 60.), (61., 65.), (57., 70.)], dark);
            c.polygon(&[(68., 65.), (83., 59.), (74., 70.)], dark);
            c.polygon(&[(59., 78.), (72., 78.), (65., 85.)], dark);
            if recipe.subject == Subject::Dragon {
                c.polygon(&[(42., 50.), (19., 38.), (26., 77.), (41., 67.)], light);
                c.polygon(&[(88., 50.), (112., 38.), (102., 77.), (88., 67.)], light);
            }
        }
        Subject::Crystal => {
            c.polygon(
                &[
                    (64., 28.),
                    (87., 49.),
                    (80., 86.),
                    (64., 99.),
                    (44., 80.),
                    (42., 48.),
                ],
                light,
            );
            c.polygon(&[(64., 28.), (64., 99.), (44., 80.), (42., 48.)], ivory);
            c.line((64., 30.), (77., 52.), 2., [255, 255, 255, 230]);
        }
        Subject::Sword => {
            c.polygon(
                &[(69., 23.), (76., 37.), (66., 76.), (57., 74.), (62., 35.)],
                ivory,
            );
            c.line((46., 73.), (78., 81.), 5., light);
            c.line((61., 79.), (57., 98.), 7., [142, 93, 63, 255]);
            c.ellipse(56., 99., 5., 4., light);
        }
        Subject::Shield => {
            c.polygon(
                &[
                    (64., 30.),
                    (87., 40.),
                    (83., 76.),
                    (64., 94.),
                    (44., 77.),
                    (40., 40.),
                ],
                ivory,
            );
            c.polygon(
                &[
                    (64., 37.),
                    (79., 45.),
                    (76., 73.),
                    (64., 85.),
                    (51., 73.),
                    (48., 45.),
                ],
                light,
            );
            c.star(64., 59., 14., dark);
        }
        Subject::Heart => {
            c.ellipse(52., 54., 15., 15., ivory);
            c.ellipse(76., 54., 15., 15., ivory);
            c.polygon(&[(37., 56.), (91., 56.), (64., 91.)], ivory);
        }
        Subject::Tree => {
            c.line((64., 92.), (64., 49.), 9., [178, 134, 87, 255]);
            for (x, y, r) in [
                (47., 56., 15.),
                (78., 54., 16.),
                (63., 40., 20.),
                (62., 60., 18.),
            ] {
                c.ellipse(x, y, r, r, light);
            }
            c.line((64., 66.), (49., 54.), 3., ivory);
        }
        Subject::Block => {
            c.polygon(
                &[
                    (64., 34.),
                    (89., 48.),
                    (89., 79.),
                    (64., 94.),
                    (38., 79.),
                    (38., 48.),
                ],
                light,
            );
            c.polygon(&[(64., 34.), (89., 48.), (64., 63.), (38., 48.)], ivory);
            c.polygon(
                &[(38., 48.), (64., 63.), (64., 94.), (38., 79.)],
                [r / 2, g / 2, b / 2, 255],
            );
        }
        Subject::Cloud => {
            for (x, y, rx, ry) in [
                (44., 62., 14., 14.),
                (60., 52., 20., 20.),
                (79., 63., 18., 15.),
                (62., 71., 26., 9.),
            ] {
                c.ellipse(x, y, rx, ry, ivory);
            }
            for x in [46., 62., 78.] {
                c.line((x, 85.), (x - 4., 94.), 2., light);
            }
        }
        Subject::Flame => {
            c.polygon(
                &[
                    (64., 24.),
                    (69., 52.),
                    (82., 41.),
                    (91., 72.),
                    (82., 89.),
                    (63., 97.),
                    (43., 86.),
                    (37., 66.),
                    (51., 44.),
                    (50., 66.),
                ],
                light,
            );
            c.polygon(&[(64., 55.), (75., 80.), (64., 93.), (53., 81.)], ivory);
        }
        Subject::Skull => {
            c.ellipse(64., 56., 25., 26., ivory);
            c.polygon(&[(48., 66.), (80., 66.), (77., 87.), (51., 87.)], ivory);
            for x in [53., 75.] {
                c.ellipse(x, 60., 6., 7., dark);
            }
            for x in [57., 64., 71.] {
                c.line((x, 79.), (x, 88.), 2., dark);
            }
        }
        Subject::Hand => {
            c.ellipse(65., 70., 17., 21., ivory);
            for (a, b) in [
                ((51., 66.), (48., 44.)),
                ((59., 60.), (59., 35.)),
                ((68., 59.), (70., 33.)),
                ((77., 64.), (83., 42.)),
                ((52., 75.), (38., 62.)),
            ] {
                c.line(a, b, 7., ivory);
            }
            c.polygon(&[(55., 82.), (77., 82.), (76., 98.), (56., 98.)], light);
            c.star(65., 69., 9., light);
        }
        Subject::Person => {
            c.ellipse(64., 43., 10., 12., ivory);
            c.polygon(&[(54., 57.), (74., 57.), (84., 89.), (44., 89.)], ivory);
            c.line((54., 60.), (38., 74.), 6., light);
            c.line((74., 60.), (90., 49.), 6., light);
            c.star(93., 42., 7., ivory);
        }
        Subject::Rune => {
            c.polygon(&[(64., 28.), (88., 61.), (64., 94.), (40., 61.)], ivory);
            c.polygon(&[(64., 39.), (78., 61.), (64., 82.), (50., 61.)], dark);
            c.star(64., 61., 12., light);
        }
    }
    if recipe.effect == Effect::Heal {
        for (x, y) in [(28., 70.), (96., 44.), (85., 91.)] {
            c.line((x - 5., y), (x + 5., y), 3., [145, 255, 189, 255]);
            c.line((x, y - 5.), (x, y + 5.), 3., [145, 255, 189, 255]);
        }
    }
    match recipe.accent {
        Accent::Moon => {
            c.ellipse(99., 24., 9., 9., ivory);
            c.ellipse(103., 21., 8., 8., [22, 27, 45, 255]);
        }
        Accent::Sun => {
            c.ellipse(99., 24., 6., 6., ivory);
            c.star(99., 24., 12., ivory);
        }
        Accent::Rain => {
            for x in [88., 97., 106.] {
                c.line((x, 17.), (x - 4., 28.), 2., ivory);
            }
        }
        Accent::Crystal => {
            c.polygon(&[(99., 13.), (107., 23.), (99., 35.), (91., 23.)], ivory);
        }
        Accent::Heart => {
            c.ellipse(95., 20., 5., 5., ivory);
            c.ellipse(103., 20., 5., 5., ivory);
            c.polygon(&[(90., 21.), (108., 21.), (99., 32.)], ivory);
        }
        Accent::Chain => {
            c.ring(96., 20., 6., 4., ivory);
            c.ring(102., 27., 6., 4., ivory);
        }
        Accent::Stars => {
            c.star(99., 22., 8., ivory);
            c.star(89., 31., 3., light);
        }
        Accent::None => {}
    }
    c.pixels
}

/// Local presentation only: the canonical saved recipe is unchanged by this pass.
pub fn pixelize(pixels: &[u8]) -> Vec<u8> {
    assert_eq!(pixels.len(), SIZE * SIZE * 4);
    let mut result = vec![0; pixels.len()];
    for by in (0..SIZE).step_by(4) {
        for bx in (0..SIZE).step_by(4) {
            let mut sum = [0u32; 3];
            for y in by..by + 4 {
                for x in bx..bx + 4 {
                    for k in 0..3 {
                        sum[k] += pixels[(y * SIZE + x) * 4 + k] as u32;
                    }
                }
            }
            let color = sum.map(|v| (((v / 16 + 8) / 17) * 17).min(255) as u8);
            for y in by..by + 4 {
                for x in bx..bx + 4 {
                    let i = (y * SIZE + x) * 4;
                    result[i..i + 3].copy_from_slice(&color);
                    result[i + 3] = 255;
                }
            }
        }
    }
    result
}
pub fn apply_style(ctx: &egui::Context, style: SpellArtwork) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("spell_art_style"), style));
}
type ImageKey = (Recipe, SpellArtwork);
struct Worker {
    send: mpsc::Sender<ImageKey>,
    receive: mpsc::Receiver<(ImageKey, Vec<u8>)>,
    pending: HashSet<ImageKey>,
    ready: HashMap<ImageKey, Vec<u8>>,
    order: VecDeque<ImageKey>,
}
fn worker() -> &'static Mutex<Worker> {
    static WORKER: OnceLock<Mutex<Worker>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (send, jobs) = mpsc::channel::<ImageKey>();
        let (done, receive) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(key) = jobs.recv() {
                let pixels = render(key.0);
                let pixels = if key.1 == SpellArtwork::Pixelized {
                    pixelize(&pixels)
                } else {
                    pixels
                };
                if done.send((key, pixels)).is_err() {
                    break;
                }
            }
        });
        Mutex::new(Worker {
            send,
            receive,
            pending: HashSet::new(),
            ready: HashMap::new(),
            order: VecDeque::new(),
        })
    })
}
#[derive(Default)]
struct Textures {
    entries: VecDeque<(ImageKey, egui::TextureHandle)>,
}
pub fn image(ui: &mut egui::Ui, recipe: Recipe, size: egui::Vec2) -> egui::Response {
    let style = ui
        .ctx()
        .data(|d| d.get_temp::<SpellArtwork>(egui::Id::new("spell_art_style")))
        .unwrap_or_default();
    let image_key = (recipe, style);
    let key = egui::Id::new("spell_art_textures_v1");
    let cache = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<std::sync::Arc<Mutex<Textures>>>(key)
            .clone()
    });
    let mut cache = cache.lock().unwrap();
    let mut texture = cache
        .entries
        .iter()
        .find(|(r, _)| *r == image_key)
        .map(|(_, t)| t.clone());
    if texture.is_none() {
        let mut worker = worker().lock().unwrap();
        while let Ok((r, pixels)) = worker.receive.try_recv() {
            worker.pending.remove(&r);
            if worker.ready.len() >= CACHE_LIMIT {
                if let Some(old) = worker.order.pop_front() {
                    worker.ready.remove(&old);
                }
            }
            worker.order.push_back(r);
            worker.ready.insert(r, pixels);
        }
        if let Some(pixels) = worker.ready.get(&image_key) {
            let t = ui.ctx().load_texture(
                "spell artwork",
                egui::ColorImage::from_rgba_unmultiplied([SIZE, SIZE], pixels),
                if style == SpellArtwork::Pixelized {
                    egui::TextureOptions::NEAREST
                } else {
                    egui::TextureOptions::LINEAR
                },
            );
            if cache.entries.len() >= CACHE_LIMIT {
                cache.entries.pop_front();
            }
            cache.entries.push_back((image_key, t.clone()));
            texture = Some(t);
        } else if worker.pending.len() < CACHE_LIMIT && worker.pending.insert(image_key) {
            let _ = worker.send.send(image_key);
        }
    }
    if let Some(texture) = texture {
        ui.add(egui::Image::new((texture.id(), size)))
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4., egui::Color32::from_rgb(23, 25, 40));
        ui.painter().circle_stroke(
            rect.center(),
            size.min_elem() * 0.25,
            egui::Stroke::new(1_f32, recipe.tint()),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pixel_pass_is_deterministic_quantized_and_preserves_canonical_art() {
        let recipe = Recipe::fallback("sheep at night");
        let original = render(recipe);
        let before = original.clone();
        let pixelized = pixelize(&original);
        assert_ne!(pixelized, original);
        assert_eq!(pixelized, pixelize(&original));
        assert_eq!(original, before);
        for by in (0..128).step_by(4) {
            for bx in (0..128).step_by(4) {
                let index = (by * 128 + bx) * 4;
                let color = &pixelized[index..index + 4];
                assert!(color[..3].iter().all(|c| c % 17 == 0));
                assert_eq!(color[3], 255);
                for y in by..by + 4 {
                    for x in bx..bx + 4 {
                        assert_eq!(&pixelized[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4], color);
                    }
                }
            }
        }
    }
    #[test]
    fn recipes_round_trip_as_lossless_images_and_reject_unbounded_data() {
        let a = Recipe::fallback("Heal sheep at night");
        let bytes = render(a);
        let restored: Recipe = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert_eq!(bytes, render(restored));
        assert_eq!(bytes.len(), 128 * 128 * 4);
        assert_ne!(bytes, render(Recipe::fallback("Burn a sword in rain")));
        assert!(serde_json::from_str::<Recipe>(r#"{"version":1,"subject":"file:///secret","effect":"heal","accent":"moon","palette":"verdant"}"#).is_err());
    }
    #[test]
    fn source_metadata_wins_and_old_modules_have_stable_fallbacks() {
        let recipe = Recipe {
            subject: Subject::Dragon,
            ..Default::default()
        };
        let source = format!(
            "-- Intent plan: {}\nfunction on_cast(api,e) end",
            serde_json::json!({"artwork":recipe})
        );
        assert_eq!(Recipe::from_source(&source, "heal"), recipe);
        assert_eq!(
            Recipe::from_source("-- Intent plan: broken", "heal"),
            Recipe::fallback("heal")
        );
    }
    #[test]
    #[ignore = "writes target/spell-art-sheet.png and measures CPU rendering"]
    fn preview_and_measure_artwork() {
        let subjects = [
            Subject::Rune,
            Subject::Sheep,
            Subject::Wolf,
            Subject::Dragon,
            Subject::Sword,
            Subject::Shield,
            Subject::Crystal,
            Subject::Tree,
            Subject::Block,
            Subject::Heart,
            Subject::Cloud,
            Subject::Skull,
        ];
        let palettes = [
            Palette::Arcane,
            Palette::Verdant,
            Palette::Blood,
            Palette::Ember,
            Palette::Gold,
            Palette::Frost,
        ];
        let effects = [
            Effect::Aura,
            Effect::Heal,
            Effect::Hunt,
            Effect::Fire,
            Effect::Protect,
            Effect::Frost,
        ];
        let mut sheet = image::RgbaImage::new(6 * 128, 2 * 128);
        let start = std::time::Instant::now();
        for (i, subject) in subjects.into_iter().enumerate() {
            let pixels = render(Recipe {
                subject,
                palette: palettes[i % 6],
                effect: effects[i % 6],
                ..Default::default()
            });
            let image = image::RgbaImage::from_raw(128, 128, pixels).unwrap();
            image::imageops::replace(
                &mut sheet,
                &image,
                (i % 6 * 128) as i64,
                (i / 6 * 128) as i64,
            );
        }
        println!("12 illustrations: {:?} total CPU time", start.elapsed());
        std::fs::create_dir_all("target").unwrap();
        sheet.save("target/spell-art-sheet.png").unwrap();
    }
}
