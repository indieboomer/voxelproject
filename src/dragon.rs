//! Solitary dragon territories and the authoritative ground/flight controller.
use super::*;
use serde::{Deserialize, Serialize};

pub(super) const MIN_HOME_SPACING: f32 = 128.0;
const REGION_SIZE: f32 = 256.0;
const HOME_RADIUS: f32 = 64.0;
const FLIGHT_SPEED: f32 = 7.0;
const VERTICAL_SPEED: f32 = 5.0;
const CRUISE_HEIGHT: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Ground,
    Takeoff,
    Fly,
    Land,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dragon {
    pub(super) home: Vec3,
    phase: Phase,
    elapsed: f32,
    patrol_angle: f32,
    initialized: bool,
}

/// Separate, defaultable save data preserves compatibility with older worlds.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DragonSave {
    regions: std::collections::BTreeSet<(i32, i32)>,
    states: std::collections::BTreeMap<u32, Dragon>,
}

impl Dragon {
    pub(super) fn new(home: Vec3, seed: u64) -> Self {
        Self {
            home,
            phase: Phase::Ground,
            elapsed: 0.0,
            patrol_angle: SimpleRng::new(seed).next_f32() * std::f32::consts::TAU,
            initialized: false,
        }
    }
    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.elapsed = 0.0;
    }
}

pub(super) fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

/// One solitary resident on the highest sampled hill in most territories.
/// Search the central half so neighboring homes remain at least 128 blocks apart.
fn candidate(world: &World, region: (i32, i32)) -> Option<(CreatureKind, Vec3, u64)> {
    let seed = (world.seed as u64)
        ^ (region.0 as u64).wrapping_mul(0x9E3779B185EBCA87)
        ^ (region.1 as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
        ^ 0xD4A60A;
    let mut rng = SimpleRng::new(seed);
    let green = world.generation.abundance("dragon_green") as f32;
    let red = world.generation.abundance("dragon_red") as f32;
    if rng.next_f32() >= (0.8 * (green + red) / 200.0).min(1.0) {
        return None;
    }
    let cx = (region.0 as f32 + 0.5) * REGION_SIZE;
    let cz = (region.1 as f32 + 0.5) * REGION_SIZE;
    let mut peak = (
        world.terrain_height(cx as i32, cz as i32),
        cx as i32,
        cz as i32,
    );
    for dx in (-64..=64).step_by(8) {
        for dz in (-64..=64).step_by(8) {
            let x = cx as i32 + dx;
            let z = cz as i32 + dz;
            let h = world.terrain_height(x, z);
            if h > peak.0 {
                peak = (h, x, z);
            }
        }
    }
    let coarse = peak;
    for x in coarse.1 - 7..=coarse.1 + 7 {
        for z in coarse.2 - 7..=coarse.2 + 7 {
            if (x - cx as i32).abs() > 64 || (z - cz as i32).abs() > 64 {
                continue;
            }
            let h = world.terrain_height(x, z);
            if h > peak.0 {
                peak = (h, x, z);
            }
        }
    }
    let (ground, x, z) = peak;
    if ground <= SEA_LEVEL {
        return None;
    }
    let kind = if rng.next_f32() * (green + red) < green {
        CreatureKind::DragonGreen
    } else {
        CreatureKind::DragonRed
    };
    Some((
        kind,
        Vec3::new(x as f32, ground as f32 + 1.0, z as f32),
        seed,
    ))
}

impl Creatures {
    /// Called only by the host as players explore. No dragon belongs to the
    /// ordinary starter scatter. Marked regions stay consumed even after death.
    pub fn discover_dragons(&mut self, world: &World, players: &[(PlayerId, Vec3)]) {
        if !self.has_population_room() || self.ecs.query::<&Dragon>().iter().count() >= 8 {
            return;
        }
        for &(_, player) in players {
            if !player.is_finite() || player.abs().max_element() > 1_000_000.0 {
                continue;
            }
            let region = (
                (player.x / REGION_SIZE).floor() as i32,
                (player.z / REGION_SIZE).floor() as i32,
            );
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let cell = (region.0 + dx, region.1 + dz);
                    if self.dragon_regions.contains(&cell) {
                        continue;
                    }
                    let Some((kind, mut home, seed)) = *self
                        .dragon_candidates
                        .entry(cell)
                        .or_insert_with(|| candidate(world, cell))
                    else {
                        self.dragon_regions.insert(cell);
                        continue;
                    };
                    if horizontal_distance(player, home) > 160.0 {
                        continue;
                    }
                    // Avoid popping a predator into melee range of any guest.
                    if players
                        .iter()
                        .any(|&(_, p)| horizontal_distance(p, home) < 80.0)
                    {
                        continue;
                    }
                    if self
                        .ecs
                        .query::<&Dragon>()
                        .iter()
                        .any(|(_, d)| horizontal_distance(d.home, home) < MIN_HOME_SPACING)
                    {
                        self.dragon_regions.insert(cell);
                        continue;
                    }
                    home.y = surface(world, home, 6.0);
                    if !self.has_population_room()
                        || self.ecs.query::<&Dragon>().iter().count() >= 8
                    {
                        return;
                    }
                    self.spawn_one(kind, home, seed);
                    self.dragon_regions.insert(cell);
                }
            }
        }
    }

    pub fn save_dragons(&self) -> DragonSave {
        DragonSave {
            regions: self.dragon_regions.clone(),
            states: self
                .ecs
                .query::<(&CreatureId, &Dragon)>()
                .iter()
                .map(|(_, (id, d))| (id.0, d.clone()))
                .collect(),
        }
    }

    pub fn restore_dragons(&mut self, save: DragonSave) {
        self.dragon_regions = save.regions;
        for (_, (id, dragon)) in self.ecs.query_mut::<(&CreatureId, &mut Dragon)>() {
            if let Some(saved) = save.states.get(&id.0) {
                if saved.home.is_finite()
                    && saved.home.abs().max_element() <= 1_000_000.0
                    && saved.elapsed.is_finite()
                    && saved.elapsed >= 0.0
                    && saved.patrol_angle.is_finite()
                {
                    *dragon = saved.clone();
                }
            }
        }
    }
}

/// Highest terrain/solid surface beneath the body footprint. Loaded columns
/// include player edits and trees; distant columns use procedural terrain.
fn surface(world: &World, pos: Vec3, radius: f32) -> f32 {
    let mut top = 0.0f32;
    for dx in [-radius, 0.0, radius] {
        for dz in [-radius, 0.0, radius] {
            let x = (pos.x + dx).floor() as i32;
            let z = (pos.z + dz).floor() as i32;
            let cell = crate::voxel::chunk::world_to_chunk(x, z);
            let h = if world.chunks.contains_key(&cell) {
                (0..crate::voxel::chunk::CHUNK_Y)
                    .rev()
                    .find(|&y| world.is_solid(x, y, z))
                    .unwrap_or(0)
            } else {
                world.terrain_height(x, z)
            };
            top = top.max((h.max(SEA_LEVEL) + 1) as f32);
        }
    }
    top
}

fn approach(value: f32, goal: f32, step: f32) -> f32 {
    value + (goal - value).clamp(-step, step)
}

/// Oriented body box at the rendered scale. Wings/tail tips are excluded so
/// an empty gap under a wing is not treated as the dragon's body.
pub(super) fn body_hit(origin: Vec3, direction: Vec3, facing: f32, reach: f32) -> Option<f32> {
    let (s, c) = facing.sin_cos();
    let local = |v: Vec3| Vec3::new(v.x * s - v.z * c, v.y, v.x * c + v.z * s);
    let origin = local(origin);
    let direction = local(direction);
    let min = Vec3::new(-4.0, 0.0, -8.0);
    let max = Vec3::new(4.0, 12.5, 10.0);
    let mut near = 0.0f32;
    let mut far = reach;
    for axis in 0..3 {
        if direction[axis].abs() < 0.00001 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
        } else {
            let a = (min[axis] - origin[axis]) / direction[axis];
            let b = (max[axis] - origin[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
    }
    Some(near)
}

fn within_home(home: Vec3, goal: Vec3) -> Vec3 {
    let delta = Vec3::new(goal.x - home.x, 0.0, goal.z - home.z);
    let delta = delta.clamp_length_max(HOME_RADIUS);
    Vec3::new(home.x + delta.x, goal.y, home.z + delta.z)
}

pub(super) struct Step {
    pub attack: Option<BehaviorTarget>,
    pub walked: f32,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    world: &World,
    dt: f32,
    dragon: &mut Dragon,
    pos: &mut Pos,
    facing: &mut Facing,
    anim: &mut AnimState,
    cooldown: &mut AttackCooldown,
    attack_anim: &mut AttackAnimTimer,
    wander: &mut Wander,
    kind: CreatureKind,
    target: Option<(BehaviorTarget, Vec3)>,
    may_attack: bool,
) -> Step {
    let dt = dt.clamp(0.0, 0.1);
    let old = pos.0;
    let floor = surface(world, pos.0, 6.0);
    if !dragon.initialized {
        dragon.initialized = true;
        if pos.0.y > floor + 2.0 {
            dragon.enter(Phase::Fly);
        } else {
            pos.0.y = floor;
        }
    }
    dragon.elapsed += dt;
    // A target cannot lure a dragon into another dragon's territory.
    let target = target.filter(|(_, p)| horizontal_distance(dragon.home, *p) <= HOME_RADIUS);
    wander.timer -= dt;
    if wander.timer <= 0.0 {
        wander.hunting = false;
    }
    let scripted = wander
        .hunting
        .then(|| Vec3::new(wander.target.0, pos.0.y, wander.target.1));
    let patrol =
        dragon.home + Vec3::new(dragon.patrol_angle.cos(), 0.0, dragon.patrol_angle.sin()) * 28.0;
    let goal = within_home(
        dragon.home,
        if dragon.phase == Phase::Land {
            dragon.home
        } else {
            target
                .map(|(_, p)| p)
                .or(scripted)
                .unwrap_or(if dragon.phase == Phase::Ground {
                    dragon.home
                } else {
                    patrol
                })
        },
    );
    let distance = horizontal_distance(pos.0, goal);
    let stopping_distance = if target.is_some() && dragon.phase != Phase::Land {
        10.0
    } else {
        1.0
    };
    if distance < 2.0 && target.is_none() && scripted.is_none() {
        dragon.patrol_angle += 1.7;
    }

    match dragon.phase {
        Phase::Ground
            if dragon.elapsed >= if target.is_some() { 10.0 } else { 24.0 }
                || (target.is_some() && distance > 30.0 && dragon.elapsed > 2.0) =>
        {
            dragon.enter(Phase::Takeoff)
        }
        Phase::Takeoff if pos.0.y >= floor + CRUISE_HEIGHT - 0.5 => dragon.enter(Phase::Fly),
        Phase::Fly if dragon.elapsed >= 12.0 => dragon.enter(Phase::Land),
        Phase::Land if pos.0.y <= floor + 0.15 => {
            pos.0.y = floor;
            dragon.enter(Phase::Ground);
        }
        _ => (),
    }

    let flying = dragon.phase != Phase::Ground;
    let speed = if flying { FLIGHT_SPEED } else { kind.speed() };
    let delta = Vec3::new(goal.x - pos.0.x, 0.0, goal.z - pos.0.z);
    let mut next = pos.0;
    if distance > stopping_distance && dragon.phase != Phase::Takeoff {
        let dir = delta / distance;
        next += dir * (speed * dt).min(distance - stopping_distance);
        facing.0 = turn_toward(facing.0, dir.z.atan2(dir.x), 1.2 * dt);
    } else if target.is_some() && distance > 0.01 {
        facing.0 = turn_toward(facing.0, delta.z.atan2(delta.x), 1.2 * dt);
    }
    next = within_home(dragon.home, next);
    let next_floor = surface(world, next, 6.0);
    let water = world.terrain_height(next.x.floor() as i32, next.z.floor() as i32) <= SEA_LEVEL;
    if !flying && (water || next_floor > pos.0.y + 1.5 || next_floor < pos.0.y - 2.0) {
        // Take off to cross a cliff or obstruction instead of walking through it.
        dragon.enter(Phase::Takeoff);
        next = pos.0;
    } else if flying && next_floor + 3.0 > pos.0.y && dragon.phase != Phase::Land {
        // Climb before advancing into a hillside or a loaded player-built wall.
        next.x = pos.0.x;
        next.z = pos.0.z;
    }
    let desired_height = match dragon.phase {
        Phase::Land if water => next_floor + 6.0,
        Phase::Ground | Phase::Land => surface(world, next, 6.0),
        Phase::Takeoff => floor + CRUISE_HEIGHT,
        Phase::Fly => (next_floor + if target.is_some() { 6.0 } else { CRUISE_HEIGHT })
            .max(target.map_or(0.0, |(_, p)| p.y + 6.0)),
    };
    // During landing, hold position above a raised obstruction until clear.
    if dragon.phase == Phase::Land && next_floor > pos.0.y {
        next.x = pos.0.x;
        next.z = pos.0.z;
    }
    next.y = approach(pos.0.y, desired_height, VERTICAL_SPEED * dt);
    // Surface changes (e.g. a newly loaded chunk or player edit) must not bury the body.
    next.y = next.y.max(surface(world, next, 6.0));
    pos.0 = next;

    let airborne = dragon.phase != Phase::Ground;
    let mut result = Step {
        attack: None,
        walked: if airborne {
            0.0
        } else {
            horizontal_distance(old, next)
        },
    };
    if let Some((id, target_pos)) = target {
        let range = if dragon.phase == Phase::Fly {
            22.0
        } else {
            kind.attack_range()
        };
        let attack_phase = matches!(dragon.phase, Phase::Ground | Phase::Fly);
        let mouth = pos.0 + Vec3::Y * 3.0;
        let to_target = target_pos + Vec3::Y - mouth;
        let facing_target = distance < 0.1
            || Vec3::new(facing.0.cos(), 0.0, facing.0.sin()).dot(delta.normalize_or_zero()) > 0.7;
        if may_attack
            && attack_phase
            && facing_target
            && cooldown.0 <= 0.0
            && pos.0.distance(target_pos) <= range
            && crate::raycast::raycast(
                world,
                mouth,
                to_target.normalize_or_zero(),
                to_target.length(),
            )
            .is_none()
        {
            result.attack = Some(id);
            cooldown.0 = kind.attack_cooldown();
            attack_anim.0 = 0.8;
        }
    }
    let clip = if attack_anim.0 > 0.0 {
        if airborne {
            AnimClip::AttackFly
        } else {
            AnimClip::AttackWalk
        }
    } else if airborne {
        AnimClip::Fly
    } else if horizontal_distance(old, next) > 0.001 {
        AnimClip::Walk
    } else {
        AnimClip::Idle
    };
    if anim.clip != clip {
        anim.clip = clip;
        anim.time = 0.0;
    } else {
        anim.time += dt;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_world() -> World {
        let mut world = World::new(71);
        world.generation.shape = crate::worldgen::Shape::Flat;
        world.generation.relief = 0;
        world.generation.trees = 0;
        world
    }
    fn spawn(world: &World) -> Vec3 {
        Vec3::new(0.0, surface(world, Vec3::ZERO, 6.0), 0.0)
    }

    #[test]
    fn dragons_walk_take_off_attack_in_flight_and_land_without_teleporting() {
        let world = flat_world();
        for kind in [CreatureKind::DragonGreen, CreatureKind::DragonRed] {
            let mut creatures = Creatures::new();
            let home = spawn(&world);
            let id = creatures.spawn_one(kind, home, 3);
            let target = home + Vec3::X * 22.0;
            let mut clips = std::collections::HashSet::new();
            let mut ground_hits = 0;
            let mut air_hits = 0;
            let mut previous = home;
            let mut took_off = false;
            let mut landed = false;
            for _ in 0..1800 {
                let hits = creatures.update(&world, 1.0 / 30.0, &[(5, target)]);
                let pos = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
                assert!(
                    (pos.y - previous.y).abs() <= VERTICAL_SPEED / 30.0 + 0.001,
                    "vertical jump: {previous:?} -> {pos:?}, clip {:?}",
                    creatures.anim_clip_of(id)
                );
                assert!(pos.y >= home.y);
                let clip = creatures.anim_clip_of(id).unwrap();
                clips.insert(clip.to_u8());
                if !hits.is_empty() {
                    if clip == AnimClip::AttackFly {
                        air_hits += 1;
                    }
                    if clip == AnimClip::AttackWalk {
                        ground_hits += 1;
                    }
                }
                if pos.y > home.y + 10.0 {
                    took_off = true;
                }
                if took_off && pos.y <= home.y + 0.01 {
                    landed = true;
                }
                previous = pos;
            }
            assert!(clips.contains(&AnimClip::Walk.to_u8()));
            assert!(clips.contains(&AnimClip::Fly.to_u8()));
            assert!(
                ground_hits > 0 && air_hits > 0,
                "ground={ground_hits}, air={air_hits}"
            );
            assert!(landed, "dragon must return to ground combat after flying");
        }
    }

    #[test]
    fn rare_homes_are_deterministic_and_separated_across_both_colors() {
        let world = flat_world();
        let mut homes = Vec::new();
        for x in -10..10 {
            for z in -10..10 {
                let a = candidate(&world, (x, z));
                assert_eq!(a, candidate(&world, (x, z)));
                if let Some((kind, pos, _)) = a {
                    homes.push((kind, pos));
                }
            }
        }
        assert!(
            (280..360).contains(&homes.len()),
            "expected rare occupied regions: {}",
            homes.len()
        );
        assert!(homes.iter().any(|(k, _)| *k == CreatureKind::DragonGreen));
        assert!(homes.iter().any(|(k, _)| *k == CreatureKind::DragonRed));
        for (i, (_, a)) in homes.iter().enumerate() {
            for (_, b) in &homes[i + 1..] {
                assert!(horizontal_distance(*a, *b) >= MIN_HOME_SPACING);
            }
        }
    }

    #[test]
    fn homes_choose_high_ground_and_respect_color_exclusions() {
        let mut world = World::new(71);
        world.generation.creatures.insert("dragon_red".into(), 0);
        world
            .generation
            .creatures
            .insert("dragon_green".into(), 200);
        let mut count = 0;
        for rx in -2..=2 {
            for rz in -2..=2 {
                let Some((kind, home, _)) = candidate(&world, (rx, rz)) else {
                    continue;
                };
                assert_eq!(kind, CreatureKind::DragonGreen);
                let cx = rx * 256 + 128;
                let cz = rz * 256 + 128;
                for dx in (-64..=64).step_by(8) {
                    for dz in (-64..=64).step_by(8) {
                        assert!(home.y >= world.terrain_height(cx + dx, cz + dz) as f32 + 1.0);
                    }
                }
                count += 1;
            }
        }
        assert!(count > 0);
        world.generation.creatures.insert("dragon_green".into(), 0);
        assert!(candidate(&world, (0, 0)).is_none());
    }

    #[test]
    fn discovery_is_solitary_and_dead_dragons_do_not_respawn_after_reload() {
        let world = flat_world();
        let (_, home, _) = (-10..10)
            .flat_map(|x| (-10..10).map(move |z| (x, z)))
            .find_map(|cell| candidate(&world, cell))
            .unwrap();
        let players = [(1, home + Vec3::X * 120.0), (2, home + Vec3::X * 130.0)];
        let mut creatures = Creatures::new();
        for _ in 0..10 {
            creatures.discover_dragons(&world, &players);
        }
        let residents = creatures.snapshot_with_ids();
        assert!(!residents.is_empty());
        for (i, a) in residents.iter().enumerate() {
            for b in &residents[i + 1..] {
                assert!(
                    horizontal_distance(Vec3::from_array(a.2), Vec3::from_array(b.2))
                        >= MIN_HOME_SPACING
                );
            }
            creatures.destroy(a.0);
        }
        let save: DragonSave =
            serde_json::from_str(&serde_json::to_string(&creatures.save_dragons()).unwrap())
                .unwrap();
        let mut restored = Creatures::new();
        restored.restore_dragons(save);
        restored.discover_dragons(&world, &players);
        assert!(restored.snapshot_with_ids().is_empty());
    }

    #[test]
    fn dragon_leashes_and_script_spawn_validation_prevent_packs() {
        let world = flat_world();
        let mut creatures = Creatures::new();
        let home = spawn(&world);
        let first = creatures.spawn_one(CreatureKind::DragonGreen, home, 1);
        let mut draft = CreatureDraft::new(&creatures);
        assert!(draft
            .spawn(CreatureKind::DragonRed, home + Vec3::X * 10.0, 2)
            .is_none());
        let second = draft
            .spawn(
                CreatureKind::DragonRed,
                home + Vec3::X * MIN_HOME_SPACING,
                2,
            )
            .unwrap();
        assert!(draft
            .spawn(
                CreatureKind::DragonGreen,
                home + Vec3::X * (MIN_HOME_SPACING + 10.0),
                3
            )
            .is_none());
        draft.commit(&mut creatures);
        for _ in 0..1200 {
            creatures.set_chase_target(first, home + Vec3::X * 160.0);
            creatures.set_chase_target(second, home + Vec3::X * 160.0);
            creatures.update(&world, 1.0 / 30.0, &[(5, home + Vec3::X * 160.0)]);
            let positions = creatures.snapshot_with_ids();
            let a = Vec3::from_array(positions[0].2);
            let b = Vec3::from_array(positions[1].2);
            assert!(horizontal_distance(a, b) >= MIN_HOME_SPACING - 2.0 * HOME_RADIUS - 0.01);
        }
    }

    #[test]
    fn flight_state_survives_reload_and_mesh_matches_the_client() {
        let world = flat_world();
        let mut creatures = Creatures::new();
        let home = spawn(&world);
        creatures.spawn_one(CreatureKind::DragonRed, home + Vec3::Y * 20.0, 1);
        creatures.update(&world, 0.1, &[]);
        assert_eq!(creatures.snapshot()[0].3, AnimClip::Fly.to_u8());
        assert!(creatures.take_audio_events().steps.is_empty());
        let states: DragonSave =
            serde_json::from_str(&serde_json::to_string(&creatures.save_dragons()).unwrap())
                .unwrap();
        let mut loaded = Creatures::new();
        loaded.restore_saved(&creatures.snapshot_with_ids(), 99);
        loaded.restore_dragons(states);
        creatures.update(&world, 0.1, &[]);
        loaded.update(&world, 0.1, &[]);
        assert_eq!(creatures.snapshot_with_ids(), loaded.snapshot_with_ids());
        let models = Models::load();
        let host = loaded.build_mesh(&models);
        let client = mesh_for_snapshot(&loaded.snapshot(), &models);
        assert_eq!(host.indices, client.indices);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&host.vertices),
            bytemuck::cast_slice::<_, u8>(&client.vertices)
        );
    }

    #[test]
    fn dragon_ground_attacks_respect_walls_and_suppression() {
        let mut world = flat_world();
        let home = spawn(&world);
        let target = home + Vec3::X * 12.0;
        let mut creatures = Creatures::new();
        let id = creatures.spawn_one(CreatureKind::DragonGreen, home, 1);
        creatures
            .attack_policies
            .insert((1, 1), vec![AttackPolicy::SuppressCreature(id)]);
        assert!(creatures.update(&world, 0.01, &[(5, target)]).is_empty());
        creatures.attack_policies.clear();
        world.ensure_chunk_loaded(0, 0);
        for y in home.y as i32..home.y as i32 + 8 {
            world.set_block(8, y, 0, crate::voxel::block::BlockType::Stone);
        }
        assert!(creatures.update(&world, 0.01, &[(5, target)]).is_empty());
    }

    #[test]
    fn large_dragon_body_is_hittable_from_the_side_but_not_through_walls() {
        let mut world = flat_world();
        let home = spawn(&world);
        let mut creatures = Creatures::new();
        let id = creatures.spawn_one(CreatureKind::DragonGreen, home, 1);
        let eye = home + Vec3::new(0.0, 1.62, 6.0);
        assert_eq!(
            creatures.weapon_target(&world, eye, -Vec3::Z, 3.2),
            Some(id)
        );
        assert_eq!(
            creatures.weapon_target(&world, eye + Vec3::Z * 4.0, -Vec3::Z, 3.2),
            None
        );
        world.ensure_chunk_loaded(0, 0);
        world.set_block(
            0,
            eye.y.floor() as i32,
            5,
            crate::voxel::block::BlockType::Stone,
        );
        assert_eq!(creatures.weapon_target(&world, eye, -Vec3::Z, 3.2), None);
    }

    #[test]
    fn flying_dragon_climbs_before_crossing_a_wall_and_attacks_need_line_of_sight() {
        let mut world = flat_world();
        let home = spawn(&world);
        world.ensure_chunk_loaded(0, 0);
        for y in home.y as i32..home.y as i32 + 16 {
            for z in 0..12 {
                world.set_block(8, y, z, crate::voxel::block::BlockType::Stone);
            }
        }
        let mut creatures = Creatures::new();
        let id = creatures.spawn_one(CreatureKind::DragonGreen, home + Vec3::Y * 6.0, 1);
        let close_target = home + Vec3::X * 12.0;
        assert!(creatures
            .update(&world, 0.1, &[(5, close_target)])
            .is_empty());
        let far_target = home + Vec3::X * 28.0;
        let mut crossed = false;
        for _ in 0..300 {
            creatures.set_chase_target(id, far_target);
            creatures.update(&world, 1.0 / 30.0, &[]);
            let pos = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
            if pos.x + 6.0 >= 8.0 && pos.x - 6.0 < 9.0 {
                assert!(pos.y >= home.y + 16.0, "body entered wall at {pos:?}");
                crossed = true;
            }
        }
        assert!(
            crossed,
            "dragon should climb over an obstruction, not remain stuck in front of it"
        );
    }
}
