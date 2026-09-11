//! Water-constrained swimming, safe surface hops, and bounded pool discovery.
use super::*;
use crate::voxel::{
    chunk::{world_to_chunk, CHUNK_Y},
    BlockType,
};
use serde::{Deserialize, Serialize};

const HALF_WIDTH: f32 = 0.7;
const HALF_HEIGHT: f32 = 0.35;
const JUMP_DURATION: f32 = 0.9;
const JUMP_HEIGHT: f32 = 1.35;

#[derive(Clone, Serialize, Deserialize)]
struct Jump {
    anchor: Vec3,
    elapsed: f32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Fish {
    goal: Vec3,
    turn_timer: f32,
    jump_timer: f32,
    jump: Option<Jump>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FishSave {
    regions: std::collections::BTreeSet<(i32, i32)>,
    states: std::collections::BTreeMap<u32, Fish>,
}

impl Fish {
    pub(super) fn new(pos: Vec3, rng: &mut SimpleRng) -> Self {
        Self {
            goal: pos,
            turn_timer: 0.0,
            jump_timer: 20.0 + rng.next_f32() * 35.0,
            jump: None,
        }
    }
}

fn body_loaded(world: &World, pos: Vec3) -> bool {
    [-HALF_WIDTH, HALF_WIDTH].iter().all(|dx| {
        [-HALF_WIDTH, HALF_WIDTH].iter().all(|dz| {
            world.chunks.contains_key(&world_to_chunk(
                (pos.x + dx).floor() as i32,
                (pos.z + dz).floor() as i32,
            ))
        })
    })
}

fn body_matches(world: &World, pos: Vec3, allowed: impl Fn(BlockType) -> bool) -> bool {
    if !pos.is_finite() || pos.abs().max_element() > 1_000_000.0 {
        return false;
    }
    for x in (pos.x - HALF_WIDTH).floor() as i32..=(pos.x + HALF_WIDTH).floor() as i32 {
        for y in (pos.y - HALF_HEIGHT).floor() as i32..=(pos.y + HALF_HEIGHT).floor() as i32 {
            for z in (pos.z - HALF_WIDTH).floor() as i32..=(pos.z + HALF_WIDTH).floor() as i32 {
                if !allowed(world.get_block(x, y, z)) {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(min: i32, max: i32) -> World {
        let mut world = World::new(71);
        world.generation.shape = crate::worldgen::Shape::Flat;
        world.generation.relief = 0;
        world.generation.trees = 0;
        for x in min..=max {
            for z in min..=max {
                let (cx, cz) = world_to_chunk(x, z);
                world.ensure_chunk_loaded(cx, cz);
                for y in 30..=32 {
                    world.set_block(x, y, z, BlockType::Water);
                }
            }
        }
        world
    }

    #[test]
    fn fish_spawn_requires_a_full_seven_by_seven_water_area() {
        let pos = Vec3::new(0.5, 31.5, 0.5);
        let mut world = pool(-3, 3);
        assert!(spawn_clear(&world, pos));
        world.set_block(3, 31, 3, BlockType::Stone);
        assert!(
            !spawn_clear(&world, pos),
            "even a dry corner invalidates the footprint"
        );
        assert!(!spawn_clear(&pool(-2, 3), pos));
        assert!(!spawn_clear(&world, Vec3::new(0.5, 35.0, 0.5)));
        assert!(!spawn_clear(&world, Vec3::NAN));
    }

    #[test]
    fn swimming_and_scripted_chases_cannot_cross_banks_or_an_underwater_wall() {
        let mut world = pool(-5, 5);
        for z in -5..=5 {
            for y in 30..=32 {
                world.set_block(1, y, z, BlockType::Stone);
            }
        }
        let mut creatures = Creatures::new();
        let start = Vec3::new(-1.5, 31.5, 0.5);
        let id = creatures.spawn_one(CreatureKind::Fish, start, 7);
        let mut min = start;
        let mut max = start;
        for _ in 0..1200 {
            creatures.set_chase_target(id, Vec3::new(4.0, 31.5, 0.5));
            assert!(creatures
                .update(&world, 1.0 / 30.0, &[(5, start)])
                .is_empty());
            let pos = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
            assert!(submerged(&world, pos));
            assert!(pos.x + HALF_WIDTH < 1.0);
            min = min.min(pos);
            max = max.max(pos);
        }
        assert!(
            (max - min).length() > 1.0,
            "fish should explore the pool, not freeze"
        );
        assert!(creatures.take_audio_events().steps.is_empty());
    }

    #[test]
    fn fish_jump_above_the_surface_and_return_to_the_same_water_column() {
        let world = pool(-4, 4);
        let mut creatures = Creatures::new();
        let start = Vec3::new(0.5, 31.5, 0.5);
        creatures.spawn_one(CreatureKind::Fish, start, 7);
        for (_, fish) in creatures.ecs.query_mut::<&mut Fish>() {
            fish.jump_timer = 0.0;
        }
        let mut saw_air = false;
        let mut returned = false;
        for _ in 0..120 {
            creatures.update(&world, 1.0 / 30.0, &[]);
            let pos = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
            assert!((pos.x - start.x).abs() < 0.001 && (pos.z - start.z).abs() < 0.001);
            if pos.y - HALF_HEIGHT > 33.0 {
                saw_air = true;
            }
            if saw_air && submerged(&world, pos) {
                returned = true;
                break;
            }
        }
        assert!(saw_air && returned);
    }

    #[test]
    fn fish_do_not_jump_through_a_low_ceiling() {
        let mut world = pool(-4, 4);
        for x in -4..=4 {
            for z in -4..=4 {
                world.set_block(x, 33, z, BlockType::Stone);
            }
        }
        let mut creatures = Creatures::new();
        creatures.spawn_one(CreatureKind::Fish, Vec3::new(0.5, 32.5, 0.5), 1);
        for (_, fish) in creatures.ecs.query_mut::<&mut Fish>() {
            fish.jump_timer = 0.0;
        }
        for _ in 0..100 {
            creatures.update(&world, 0.05, &[]);
            assert!(submerged(
                &world,
                Vec3::from_array(creatures.snapshot_with_ids()[0].2)
            ));
        }
    }

    #[test]
    fn fish_freeze_in_unloaded_chunks_but_disappear_if_the_pool_is_drained() {
        let mut world = pool(-4, 4);
        let mut creatures = Creatures::new();
        creatures.spawn_one(CreatureKind::Fish, Vec3::new(0.5, 31.5, 0.5), 1);
        let saved = creatures.snapshot_with_ids();
        world.unload_chunk(0, 0);
        creatures.update(&world, 0.1, &[]);
        assert_eq!(saved, creatures.snapshot_with_ids());
        world.ensure_chunk_loaded(0, 0);
        world.set_block(0, 31, 0, BlockType::Air);
        creatures.update(&world, 0.1, &[]);
        assert!(creatures.snapshot_with_ids().is_empty());
    }

    #[test]
    fn fish_spawn_validation_applies_to_rule_drafts() {
        let world = pool(-3, 3);
        let creatures = Creatures::new();
        let mut draft = CreatureDraft::new(&creatures);
        assert!(draft
            .spawn_in_world(&world, CreatureKind::Fish, Vec3::new(0.5, 31.5, 0.5), 1)
            .is_some());
        assert!(draft
            .spawn_in_world(&world, CreatureKind::Fish, Vec3::new(3.5, 31.5, 3.5), 1)
            .is_none());
        assert!(draft
            .spawn_in_world(&world, CreatureKind::Fish, Vec3::new(20.5, 31.5, 0.5), 1)
            .is_none());
    }

    #[test]
    fn natural_fish_spawns_use_large_pools_and_are_not_duplicated_after_reload() {
        let world = pool(-20, 20);
        let mut creatures = Creatures::new();
        for _ in 0..20 {
            creatures.discover_fish(&world, &[(5, Vec3::new(0.0, 33.0, 0.0))], 1.0);
        }
        let saved = creatures.snapshot_with_ids();
        assert!(!saved.is_empty());
        assert!(saved
            .iter()
            .all(|(_, kind, pos, _, _)| *kind == CreatureKind::Fish.to_u8()
                && spawn_clear(&world, Vec3::from_array(*pos))));
        let fish_save: FishSave =
            serde_json::from_str(&serde_json::to_string(&creatures.save_fish()).unwrap()).unwrap();
        let mut loaded = Creatures::new();
        loaded.restore_saved(&saved, 99);
        loaded.restore_fish(fish_save);
        for _ in 0..20 {
            loaded.discover_fish(&world, &[(5, Vec3::new(0.0, 33.0, 0.0))], 1.0);
        }
        assert_eq!(loaded.snapshot_with_ids().len(), saved.len());
    }

    #[test]
    fn fish_reload_mid_jump_and_render_identically_on_the_client() {
        let world = pool(-4, 4);
        let mut creatures = Creatures::new();
        let anchor = Vec3::new(0.5, 32.6, 0.5);
        creatures.spawn_one(CreatureKind::Fish, anchor, 1);
        for (_, fish) in creatures.ecs.query_mut::<&mut Fish>() {
            fish.jump_timer = 0.0;
        }
        for _ in 0..5 {
            creatures.update(&world, 0.05, &[]);
        }
        assert!(!submerged(
            &world,
            Vec3::from_array(creatures.snapshot_with_ids()[0].2)
        ));
        let mut loaded = Creatures::new();
        loaded.restore_saved(&creatures.snapshot_with_ids(), 99);
        let saved = serde_json::to_string(&creatures.save_fish()).unwrap();
        loaded.restore_fish(serde_json::from_str(&saved).unwrap());
        for _ in 0..10 {
            creatures.update(&world, 0.05, &[]);
            loaded.update(&world, 0.05, &[]);
            assert_eq!(creatures.snapshot_with_ids(), loaded.snapshot_with_ids());
        }
        let models = Models::load();
        let host = loaded.build_mesh(&models);
        let client = mesh_for_snapshot(&loaded.snapshot(), &models);
        assert_eq!(host.indices, client.indices);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&host.vertices),
            bytemuck::cast_slice::<_, u8>(&client.vertices)
        );
    }
}

fn submerged(world: &World, pos: Vec3) -> bool {
    body_matches(world, pos, |block| block == BlockType::Water)
}

/// Three cells beyond the center in every horizontal direction, with no
/// dry corners or thin cross-shaped puddles. This also checks the model's body.
pub(super) fn spawn_clear(world: &World, pos: Vec3) -> bool {
    if !submerged(world, pos) {
        return false;
    }
    let (x, y, z) = (
        pos.x.floor() as i32,
        pos.y.floor() as i32,
        pos.z.floor() as i32,
    );
    (-3..=3).all(|dx| (-3..=3).all(|dz| world.get_block(x + dx, y, z + dz) == BlockType::Water))
}

fn water_path(world: &World, from: Vec3, to: Vec3) -> bool {
    let distance = from.distance(to);
    if !distance.is_finite() || distance > 16.0 {
        return false;
    }
    let steps = (distance / 0.15).ceil().max(1.0) as usize;
    (0..=steps).all(|i| submerged(world, from.lerp(to, i as f32 / steps as f32)))
}

fn surface_anchor(world: &World, pos: Vec3) -> Option<Vec3> {
    let x = pos.x.floor() as i32;
    let z = pos.z.floor() as i32;
    let mut y = pos.y.floor() as i32;
    while y < CHUNK_Y && world.get_block(x, y, z) == BlockType::Water {
        y += 1;
    }
    if world.get_block(x, y, z) != BlockType::Air {
        return None;
    }
    let anchor = Vec3::new(pos.x, y as f32 - 0.4, pos.z);
    if !water_path(world, pos, anchor) {
        return None;
    }
    // Check the entire up-and-down corridor before committing to a jump.
    (0..=12)
        .all(|i| {
            body_matches(
                world,
                anchor + Vec3::Y * (JUMP_HEIGHT * i as f32 / 12.0),
                |block| matches!(block, BlockType::Water | BlockType::Air),
            )
        })
        .then_some(anchor)
}

impl Creatures {
    /// Search only loaded water, including player-created pools and saved edits.
    pub fn fish_spawn_near(world: &World, center: Vec3, radius: f32, seed: u64) -> Option<Vec3> {
        if !center.is_finite() || !radius.is_finite() || center.abs().max_element() > 1_000_000.0 {
            return None;
        }
        let mut rng = SimpleRng::new(seed);
        for _ in 0..24 {
            let angle = rng.next_f32() * std::f32::consts::TAU;
            let distance = rng.next_f32().sqrt() * radius.clamp(0.0, 128.0);
            let x = (center.x + angle.cos() * distance).floor() as i32;
            let z = (center.z + angle.sin() * distance).floor() as i32;
            for y in (1..CHUNK_Y - 1).rev() {
                if world.get_block(x, y, z) != BlockType::Water {
                    continue;
                }
                let pos = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                if spawn_clear(world, pos) {
                    return Some(pos);
                }
            }
        }
        None
    }

    /// At most eight loaded regions scanned per second, one fish per occupied
    /// region. Failed sites can be retried when players build or enlarge pools.
    pub fn discover_fish(&mut self, world: &World, players: &[(PlayerId, Vec3)], dt: f32) {
        self.fish_scan_timer -= dt;
        if self.fish_scan_timer > 0.0 {
            return;
        }
        self.fish_scan_timer = 1.0;
        let mut regions: Vec<_> = world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| {
                !self.fish_regions.contains(&(x, z))
                    && players.iter().any(|&(_, p)| {
                        let center = Vec3::new(x as f32 * 16.0 + 8.0, p.y, z as f32 * 16.0 + 8.0);
                        p.is_finite() && p.distance_squared(center) < 64.0 * 64.0
                    })
            })
            .collect();
        regions.sort_unstable();
        if regions.is_empty() {
            return;
        }
        for offset in 0..regions.len().min(8) {
            let cell = regions[(self.fish_scan_cursor + offset) % regions.len()];
            let seed = world.seed as u64
                ^ (cell.0 as u64).wrapping_mul(0x9E3779B185EBCA87)
                ^ (cell.1 as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
                ^ 0xF15B1234;
            let mut rng = SimpleRng::new(seed);
            if rng.next_f32() > 0.6 {
                continue;
            }
            let center = Vec3::new(cell.0 as f32 * 16.0 + 8.0, 0.0, cell.1 as f32 * 16.0 + 8.0);
            let Some(pos) = Self::fish_spawn_near(world, center, 6.0, seed) else {
                continue;
            };
            if self
                .ecs
                .query::<(&Kind, &Pos)>()
                .iter()
                .any(|(_, (kind, p))| kind.0 == CreatureKind::Fish && p.0.distance(pos) < 6.0)
            {
                continue;
            }
            self.spawn_one(CreatureKind::Fish, pos, seed);
            self.fish_regions.insert(cell);
        }
        self.fish_scan_cursor = (self.fish_scan_cursor + 8) % regions.len();
    }

    pub fn save_fish(&self) -> FishSave {
        FishSave {
            regions: self.fish_regions.clone(),
            states: self
                .ecs
                .query::<(&CreatureId, &Fish)>()
                .iter()
                .map(|(_, (id, fish))| (id.0, fish.clone()))
                .collect(),
        }
    }

    pub fn restore_fish(&mut self, save: FishSave) {
        self.fish_regions = save.regions;
        for (_, (id, fish)) in self.ecs.query_mut::<(&CreatureId, &mut Fish)>() {
            if let Some(saved) = save.states.get(&id.0) {
                if saved.goal.is_finite()
                    && saved.turn_timer.is_finite()
                    && saved.jump_timer.is_finite()
                    && saved.jump.as_ref().map_or(true, |j| {
                        j.anchor.is_finite()
                            && j.elapsed.is_finite()
                            && (0.0..=JUMP_DURATION).contains(&j.elapsed)
                    })
                {
                    *fish = saved.clone();
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    world: &World,
    dt: f32,
    fish: &mut Fish,
    pos: &mut Pos,
    facing: &mut Facing,
    anim: &mut AnimState,
    wander: &mut Wander,
    rng: &mut SimpleRng,
    scripted_target: Option<Vec3>,
) -> bool {
    let dt = dt.clamp(0.0, 0.1);
    // Unloaded chunks are reported as air; freezing avoids mistaking them for drained pools.
    if !body_loaded(world, pos.0) {
        return true;
    }
    anim.clip = AnimClip::Idle; // The authored idle clip animates the fins and tail.
    anim.time += dt;
    if let Some(jump) = &mut fish.jump {
        if !submerged(world, jump.anchor) {
            return false;
        }
        jump.elapsed = (jump.elapsed + dt).min(JUMP_DURATION);
        let t = jump.elapsed / JUMP_DURATION;
        let next = jump.anchor + Vec3::Y * (4.0 * JUMP_HEIGHT * t * (1.0 - t));
        if !body_matches(world, next, |b| {
            matches!(b, BlockType::Water | BlockType::Air)
        }) {
            pos.0 = jump.anchor;
            fish.jump = None;
        } else {
            pos.0 = next;
            if t >= 1.0 {
                fish.jump = None;
                fish.turn_timer = 0.0;
            }
        }
        return true;
    }
    if !submerged(world, pos.0) {
        return false;
    }
    fish.turn_timer -= dt;
    fish.jump_timer -= dt;
    wander.timer -= dt;
    if wander.timer <= 0.0 {
        wander.hunting = false;
    }
    let target = scripted_target.or_else(|| {
        wander
            .hunting
            .then(|| Vec3::new(wander.target.0, pos.0.y, wander.target.1))
    });
    if let Some(target) = target.filter(|p| water_path(world, pos.0, *p)) {
        fish.goal = target;
    } else if fish.turn_timer <= 0.0 || pos.0.distance(fish.goal) < 0.1 {
        fish.goal = pos.0;
        for _ in 0..12 {
            let angle = rng.next_f32() * std::f32::consts::TAU;
            let distance = 0.6 + rng.next_f32() * 3.0;
            let candidate = pos.0
                + Vec3::new(
                    angle.cos() * distance,
                    (rng.next_f32() - 0.5) * 2.0,
                    angle.sin() * distance,
                );
            if water_path(world, pos.0, candidate) {
                fish.goal = candidate;
                break;
            }
        }
        fish.turn_timer = 1.5 + rng.next_f32() * 3.0;
    }
    if fish.jump_timer <= 0.0 && target.is_none() {
        if let Some(anchor) = surface_anchor(world, pos.0) {
            fish.goal = anchor;
            if pos.0.distance(anchor) < 0.06 {
                pos.0 = anchor;
                fish.jump = Some(Jump {
                    anchor,
                    elapsed: 0.0,
                });
                fish.jump_timer = 20.0 + rng.next_f32() * 35.0;
                return true;
            }
        } else {
            fish.jump_timer = 20.0 + rng.next_f32() * 35.0;
        }
    }
    let delta = fish.goal - pos.0;
    let distance = delta.length();
    if distance > 0.001 {
        let next = pos.0 + delta / distance * (CreatureKind::Fish.speed() * dt).min(distance);
        if water_path(world, pos.0, next) {
            pos.0 = next;
            if delta.x.abs() + delta.z.abs() > 0.001 {
                facing.0 = turn_toward(facing.0, delta.z.atan2(delta.x), 2.5 * dt);
            }
        } else {
            fish.turn_timer = 0.0;
        }
    }
    true
}
