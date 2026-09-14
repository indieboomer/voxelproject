//! Cosmetic moisture: bounded exposure sampling, no simulation/network changes.
use crate::{
    voxel::{mesher::Vertex, BlockType, World},
    weather::Weather,
};
use glam::Vec3;
use std::collections::HashMap;

pub fn absorption(block: BlockType) -> f32 {
    use BlockType::*;
    match block {
        Soil | Mud | Sand | Clay | Peat | Cloth | PlantFiber | WoodPulp => 0.95,
        OakWood | SpruceWood | BirchWood | CherryWood | Planks | Bricks | Mortar => 0.8,
        Iron | Copper | Tin | Silver | Gold | Steel | Bronze | Mithril | MoonSilver | Glass
        | EnchantedGlass | Obsidian | Diamond | Emerald | Ruby | Sapphire => 0.08,
        Marble | Quartz | Amethyst | Resin | Amber => 0.25,
        _ => 0.55,
    }
}
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Key {
    Player(u32),
    Npc((i32, i32, i32)),
    Guide((i32, i32, i32)),
    Creature(u64),
}
#[derive(Default)]
struct Moisture {
    level: f32,
    exposure: f32,
    sample_in: f32,
    seen: u64,
}
impl Moisture {
    fn update(&mut self, world: &World, pos: Vec3, weather: Weather, dt: f32) {
        self.sample_in -= dt;
        if self.sample_in <= 0. {
            let mut roofs = crate::shelter::Roofs::default();
            self.exposure = [
                Vec3::ZERO,
                Vec3::X * 0.3,
                -Vec3::X * 0.3,
                Vec3::Z * 0.3,
                -Vec3::Z * 0.3,
            ]
            .iter()
            .filter(|offset| !roofs.covered(world, pos + **offset))
            .count() as f32
                / 5.;
            self.sample_in = 0.2;
        }
        let soaking = weather.has_rain_particles() && self.exposure > 0.;
        let rate = if soaking {
            self.exposure / if weather == Weather::Storm { 8. } else { 12. }
        } else {
            -1. / 45.
        };
        self.level = (self.level + dt * rate).clamp(0., 1.);
    }
}
#[derive(Default)]
pub struct Actors {
    states: HashMap<Key, Moisture>,
    frame: u64,
    next: u64,
    creatures: Vec<(u8, Vec3, u64)>,
}
impl Actors {
    pub fn begin(&mut self) {
        self.frame += 1;
        self.states
            .retain(|_, s| self.frame.saturating_sub(s.seen) < 600);
    }
    pub fn sample(
        &mut self,
        key: Key,
        world: &World,
        head: Vec3,
        weather: Weather,
        dt: f32,
    ) -> f32 {
        let s = self.states.entry(key).or_default();
        s.seen = self.frame;
        s.update(world, head, weather, dt);
        s.level
    }
    /// Creature presentation snapshots have no IDs. Match nearby same-kind
    /// observations in spatial buckets; consumes each previous track only once.
    pub fn creatures(
        &mut self,
        entries: &[([f32; 3], u8, f32, u8, f32)],
        world: &World,
        weather: Weather,
        dt: f32,
    ) -> Vec<f32> {
        let mut buckets: HashMap<(u8, i32, i32, i32), Vec<(Vec3, u64)>> = HashMap::new();
        for (kind, p, id) in self.creatures.drain(..) {
            let c = (p / 4.).floor().as_ivec3();
            buckets
                .entry((kind, c.x, c.y, c.z))
                .or_default()
                .push((p, id));
        }
        let mut levels = Vec::with_capacity(entries.len());
        for &(p, kind, ..) in entries {
            let pos = Vec3::from_array(p);
            let c = (pos / 4.).floor().as_ivec3();
            let mut best = None;
            let mut distance = 16.;
            for z in -1..=1 {
                for y in -1..=1 {
                    for x in -1..=1 {
                        let key = (kind, c.x + x, c.y + y, c.z + z);
                        if let Some(items) = buckets.get(&key) {
                            for (i, (p, _)) in items.iter().enumerate() {
                                let d = p.distance_squared(pos);
                                if d < distance {
                                    distance = d;
                                    best = Some((key, i));
                                }
                            }
                        }
                    }
                }
            }
            let id = if let Some((key, i)) = best {
                buckets.get_mut(&key).unwrap().swap_remove(i).1
            } else {
                self.next += 1;
                self.next
            };
            self.creatures.push((kind, pos, id));
            levels.push(self.sample(Key::Creature(id), world, pos + Vec3::Y * 0.8, weather, dt));
        }
        levels
    }
}
/// Materials keep their absorption value; only moisture is changed. Emissive
/// flame cards and particles must never acquire a reflective water coat.
pub fn apply(vertices: &mut [Vertex], level: f32) {
    for v in vertices {
        if v.emission <= 0.01 && v.tex_layer >= 0. {
            v.wet[1] = level;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    #[test]
    #[ignore = "manual cosmetic moisture CPU benchmark"]
    fn profile_wet_actors() {
        let mut world = World::new(1);
        for x in -1..=2 {
            for z in -1..=2 {
                world
                    .chunks
                    .insert((x, z), crate::voxel::chunk::Chunk::new(x, z));
            }
        }
        let entries: Vec<_> = (0..128)
            .map(|i| {
                (
                    [((i % 16) * 2) as f32 + 0.5, 1., ((i / 16) * 2) as f32 + 0.5],
                    0,
                    0.,
                    0,
                    0.,
                )
            })
            .collect();
        let mut actors = Actors::default();
        let mut times = Vec::new();
        for frame in 0..220 {
            let start = std::time::Instant::now();
            actors.begin();
            std::hint::black_box(actors.creatures(&entries, &world, Weather::Rain, 1. / 60.));
            if frame >= 20 {
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        times.sort_by(f64::total_cmp);
        println!(
            "128 wet actors CPU median {:.3} ms p95 {:.3} ms",
            times[100], times[190]
        );
    }
    #[test]
    fn neighboring_creatures_keep_independent_moisture_when_snapshot_order_changes() {
        let mut world = World::new(1);
        world
            .chunks
            .insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        world.set_block(5, 3, 4, BlockType::Stone);
        let outside = ([4.5, 1., 4.5], 0, 0., 0, 0.);
        let inside = ([5.5, 1., 4.5], 0, 0., 0, 0.);
        let mut actors = Actors::default();
        for _ in 0..50 {
            actors.begin();
            actors.creatures(&[outside, inside], &world, Weather::Rain, 0.1);
        }
        actors.begin();
        let levels = actors.creatures(&[inside, outside], &world, Weather::Rain, 0.1);
        assert_eq!(levels[0], 0.);
        assert!(levels[1] > 0.4);
    }
    #[test]
    fn moisture_does_not_modify_flames_or_geometry() {
        let mut vertices = [Vertex::zeroed(); 3];
        vertices[0].emission = 1.;
        vertices[1].tex_layer = -1.;
        vertices[2].wet = [0.8, 0.];
        apply(&mut vertices, 1.);
        assert_eq!(vertices[0].wet, [0., 0.]);
        assert_eq!(vertices[1].wet, [0., 0.]);
        assert_eq!(vertices[2].wet, [0.8, 1.]);
    }
    #[test]
    fn moisture_accumulates_outdoors_persists_under_roof_and_drains() {
        let mut w = World::new(1);
        w.chunks
            .insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        let p = Vec3::new(4.5, 3., 4.5);
        let mut s = Moisture::default();
        for _ in 0..120 {
            s.update(&w, p, Weather::Rain, 0.1);
        }
        assert!(s.level > 0.99);
        w.set_block(4, 4, 4, BlockType::Stone);
        for _ in 0..10 {
            s.update(&w, p, Weather::Storm, 0.1);
        }
        assert!(s.level > 0.95 && s.level < 1.);
        for _ in 0..460 {
            s.update(&w, p, Weather::Rain, 0.1);
        }
        assert_eq!(s.level, 0.);
        w.set_block(4, 4, 4, BlockType::Air);
        for _ in 0..80 {
            s.update(&w, p, Weather::Storm, 0.1);
        }
        assert!(s.level > 0.97);
    }
    #[test]
    fn porous_materials_darken_more_than_polished_materials() {
        assert!(absorption(BlockType::Soil) > absorption(BlockType::Stone));
        assert!(absorption(BlockType::OakWood) > absorption(BlockType::Iron));
    }
}
