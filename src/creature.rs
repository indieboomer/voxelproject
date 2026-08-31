use glam::Vec3;

use crate::net::PlayerId;
use crate::voxel::atlas::white_uv;
use crate::voxel::mesher::{push_cuboid_facing, MeshData, Vertex};
use crate::voxel::world::SEA_LEVEL;
use crate::voxel::World;

/// Small self-contained xorshift RNG so creature AI doesn't need an
/// external `rand` crate dependency.
pub struct SimpleRng(u64);

impl SimpleRng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreatureKind {
    Sheep,
    Chicken,
    /// Large, slow, hostile -- the only creature kind with built-in combat
    /// behavior (see `Creatures::update`'s golem aggro/attack pass). Every
    /// other kind only ever moves under Lua's `chase()`; a golem also
    /// autonomously closes on and damages the nearest player within
    /// `STONE_GOLEM_AGGRO_RADIUS`, entirely independent of any rule.
    StoneGolem,
}

impl CreatureKind {
    fn speed(self) -> f32 {
        match self {
            CreatureKind::Sheep => 1.4,
            CreatureKind::Chicken => 2.0,
            // Deliberately slower than either -- "moves slowly" is meant to
            // be an escapable, avoidable threat, not a fast ambush.
            CreatureKind::StoneGolem => 0.9,
        }
    }

    pub fn max_health(self) -> f32 {
        match self {
            CreatureKind::Sheep => 12.0,
            CreatureKind::Chicken => 6.0,
            CreatureKind::StoneGolem => 40.0,
        }
    }

    fn body_half_extent(self) -> Vec3 {
        match self {
            CreatureKind::Sheep => Vec3::new(0.35, 0.45, 0.5),
            CreatureKind::Chicken => Vec3::new(0.2, 0.22, 0.28),
            // Full size ~(1.3, 2.3, 1.3) -- taller and wider than the player
            // (0.6 x 1.8 x 0.6), reads as "large" next to everything else.
            CreatureKind::StoneGolem => Vec3::new(0.65, 1.15, 0.65),
        }
    }

    fn body_color(self) -> [f32; 3] {
        match self {
            CreatureKind::Sheep => [0.92, 0.92, 0.88],
            CreatureKind::Chicken => [0.92, 0.82, 0.25],
            CreatureKind::StoneGolem => [0.52, 0.52, 0.55],
        }
    }

    fn head_color(self) -> [f32; 3] {
        match self {
            CreatureKind::Sheep => [0.72, 0.68, 0.62],
            CreatureKind::Chicken => [0.85, 0.25, 0.2],
            CreatureKind::StoneGolem => [0.4, 0.4, 0.43],
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            CreatureKind::Sheep => 0,
            CreatureKind::Chicken => 1,
            CreatureKind::StoneGolem => 2,
        }
    }

    pub fn from_u8(v: u8) -> CreatureKind {
        match v {
            1 => CreatureKind::Chicken,
            2 => CreatureKind::StoneGolem,
            _ => CreatureKind::Sheep,
        }
    }
}

/// Appends the two boxes (body + head) representing one creature at `feet`,
/// facing `facing` radians (same convention as `Camera::forward`), to a
/// mesh being built. Shared by the live ECS-driven mesh and the
/// network-snapshot mesh so both draw identically.
fn push_creature(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    feet: Vec3,
    kind: CreatureKind,
    facing: f32,
) {
    let uv = white_uv();
    let half = kind.body_half_extent();
    // Boxes are expressed relative to `feet` (local +Z = forward) so
    // `push_cuboid_facing` can rotate them to face the creature's actual
    // heading instead of always facing world +Z.
    let body_min = Vec3::new(-half.x, 0.0, -half.z);
    let body_max = Vec3::new(half.x, half.y * 2.0, half.z);
    push_cuboid_facing(
        vertices,
        indices,
        feet,
        body_min,
        body_max,
        facing,
        kind.body_color(),
        uv,
    );

    let head_size = half * 0.55;
    let head_center_z = half.z * 0.8;
    let head_min = Vec3::new(
        -head_size.x,
        body_max.y - head_size.y * 0.5,
        head_center_z - head_size.z,
    );
    let head_max = Vec3::new(
        head_size.x,
        body_max.y + head_size.y * 1.1,
        head_center_z + head_size.z,
    );
    push_cuboid_facing(
        vertices,
        indices,
        feet,
        head_min,
        head_max,
        facing,
        kind.head_color(),
        uv,
    );
}

/// Steps `current` toward `target` (both radians) by at most `max_delta`,
/// turning whichever way is shorter. Keeps the result normalized to
/// `(-PI, PI]` so it doesn't grow unbounded over a long play session.
fn turn_toward(current: f32, target: f32, max_delta: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let diff = (target - current + PI).rem_euclid(TAU) - PI;
    let step = diff.clamp(-max_delta, max_delta);
    (current + step + PI).rem_euclid(TAU) - PI
}

struct Pos(Vec3);
struct Wander {
    target: (f32, f32),
    timer: f32,
    /// Set by a Lua module's `chase()` call and refreshed every tick it's
    /// still called; left alone it naturally clears itself once `timer`
    /// expires, so a disabled module's creatures drift back to wandering
    /// within a few seconds without any explicit "revert" bookkeeping.
    hunting: bool,
}
struct Kind(CreatureKind);
struct Health(f32);
/// Seconds until this creature may attack again -- only ever decremented/
/// read for a `StoneGolem`, but attached to every creature uniformly
/// (unused, harmless) rather than giving hecs entities two different
/// component shapes depending on kind.
struct AttackCooldown(f32);
/// Stable id exposed to Lua scripts, since a `hecs::Entity` isn't a
/// convenient thing to hand across the API boundary.
struct CreatureId(u32);
/// Current heading in radians (same convention as `Camera::forward`: 0
/// faces +X, increasing turns toward +Z). Turned toward the movement
/// direction each tick the creature is actually moving; held steady while
/// idle, so a stopped creature doesn't visibly snap back to some default.
struct Facing(f32);

const HUNT_SPEED_MULTIPLIER: f32 = 1.6;
/// How fast a creature visually turns to face its movement direction --
/// fast enough that picking a new wander target doesn't look like an
/// instant snap, but still lively. ~220 degrees/sec.
const TURN_RATE: f32 = 3.84;
/// How long a Lua-set chase target stays valid before the creature reverts
/// to normal wandering if it isn't refreshed again.
const HUNT_TARGET_TTL: f32 = 0.3;

/// How close a player has to be before a stone golem notices and starts
/// closing in, overriding whatever it was doing (wandering, or even a
/// Lua-set chase target -- see `Creatures::update`).
const STONE_GOLEM_AGGRO_RADIUS: f32 = 12.0;
/// Melee reach -- generous relative to the player/golem hitboxes so "large"
/// actually reads as dangerous up close rather than needing to overlap.
const STONE_GOLEM_ATTACK_RANGE: f32 = 2.2;
const STONE_GOLEM_ATTACK_DAMAGE: f32 = 4.0;
/// Seconds between hits once in range -- slow like its movement, so a
/// player has a real window to retreat or fight back between hits.
const STONE_GOLEM_ATTACK_COOLDOWN: f32 = 1.5;

/// Reported when `damage`/`destroy` kills a creature, so the caller can
/// fire the `on_death` event to every rule module (not just the one that
/// caused it) and so the corpse's last position is still available after
/// the entity itself has already been despawned.
pub struct DeathEvent {
    pub kind: CreatureKind,
    pub pos: Vec3,
}

pub struct Creatures {
    ecs: hecs::World,
    next_id: u32,
}

impl Creatures {
    pub fn new() -> Self {
        Self {
            ecs: hecs::World::new(),
            next_id: 1,
        }
    }

    /// Populates a brand-new world's starter creatures -- sheep and chicken
    /// only, deliberately never a golem. A hostile creature ambushing a
    /// player in their first minutes with no way to have anticipated it
    /// would undercut the "same experience in the first minutes" a fresh
    /// world is supposed to guarantee; a golem only ever appears because a
    /// rule/spell explicitly summons one (`api.spawn_creature`/
    /// `spawn_creature_near_player`), which the player asked for.
    pub fn spawn_around(&mut self, world: &World, center: Vec3, count: usize, seed: u32) {
        let mut rng = SimpleRng::new(seed as u64 ^ 0xC0FFEE);
        for i in 0..count {
            let kind = if i % 2 == 0 {
                CreatureKind::Sheep
            } else {
                CreatureKind::Chicken
            };
            if let Some((x, z)) = find_land_spot(world, &mut rng, center.x, center.z, 24.0) {
                let y = world.terrain_height(x.floor() as i32, z.floor() as i32) as f32 + 1.0;
                self.spawn_with_rng(
                    kind,
                    Vec3::new(x, y, z),
                    (seed as u64).wrapping_add(i as u64 * 7919) ^ 0xA5A5A5,
                );
            }
        }
    }

    fn spawn_with_rng(&mut self, kind: CreatureKind, pos: Vec3, rng_seed: u64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.ecs.spawn((
            Pos(pos),
            Wander {
                target: (pos.x, pos.z),
                timer: 0.0,
                hunting: false,
            },
            Kind(kind),
            Health(kind.max_health()),
            AttackCooldown(0.0),
            CreatureId(id),
            Facing(0.0),
            SimpleRng::new(rng_seed),
        ));
        id
    }

    /// Called by the Lua `spawn_creature` API function.
    pub fn spawn_one(&mut self, kind: CreatureKind, pos: Vec3, rng_seed: u64) -> u32 {
        self.spawn_with_rng(kind, pos, rng_seed)
    }

    /// Only the host runs creature AI; joined clients just render whatever
    /// positions the host's snapshot reports. `player_targets` (id +
    /// position) drives stone golem aggro -- the only creature-vs-player
    /// interaction that isn't Lua-driven; everything else about this
    /// method's per-tick wander/chase movement is unchanged for sheep and
    /// chicken. Returns `(player_id, damage)` for every golem attack that
    /// landed this tick, for the caller to apply via the same
    /// `PlayerEffect::Health` path `api.damage_player` uses.
    pub fn update(
        &mut self,
        world: &World,
        dt: f32,
        player_targets: &[(PlayerId, Vec3)],
    ) -> Vec<(PlayerId, f32)> {
        let mut attacks = Vec::new();

        for (_, (pos, wander, kind, rng, facing, cooldown)) in self.ecs.query_mut::<(
            &mut Pos,
            &mut Wander,
            &Kind,
            &mut SimpleRng,
            &mut Facing,
            &mut AttackCooldown,
        )>() {
            cooldown.0 = (cooldown.0 - dt).max(0.0);

            let aggro = if kind.0 == CreatureKind::StoneGolem {
                player_targets
                    .iter()
                    .map(|&(id, p)| (id, p, pos.0.distance(p)))
                    .filter(|&(_, _, dist)| dist <= STONE_GOLEM_AGGRO_RADIUS)
                    .min_by(|a, b| a.2.total_cmp(&b.2))
            } else {
                None
            };

            if let Some((player_id, player_pos, dist)) = aggro {
                // Beeline for the player every tick this close, overriding
                // whatever wander/Lua-chase target it had -- deliberately
                // at the golem's own (slow) speed, never
                // HUNT_SPEED_MULTIPLIER, so "moves slowly" stays true even
                // while it's actively attacking.
                let to_target = Vec3::new(player_pos.x - pos.0.x, 0.0, player_pos.z - pos.0.z);
                let horiz_dist = to_target.length();
                if horiz_dist > 0.05 {
                    let dir = to_target / horiz_dist;
                    let step = (kind.0.speed() * dt).min(horiz_dist);
                    pos.0.x += dir.x * step;
                    pos.0.z += dir.z * step;
                    facing.0 = turn_toward(facing.0, dir.z.atan2(dir.x), TURN_RATE * dt);
                }
                if dist <= STONE_GOLEM_ATTACK_RANGE && cooldown.0 <= 0.0 {
                    attacks.push((player_id, STONE_GOLEM_ATTACK_DAMAGE));
                    cooldown.0 = STONE_GOLEM_ATTACK_COOLDOWN;
                }
                // Force an immediate retarget (see the `timer <= 0.0` check
                // below) the moment aggro lapses, instead of resuming a
                // stale wander target from wherever it was before -- keeps
                // the transition back to wandering from looking like a
                // sudden beeline to some far-off point.
                wander.target = (pos.0.x, pos.0.z);
                wander.timer = 0.0;
            } else {
                wander.timer -= dt;
                if wander.timer <= 0.0 {
                    if let Some((x, z)) = find_land_spot(world, rng, pos.0.x, pos.0.z, 6.0) {
                        wander.target = (x, z);
                    }
                    wander.timer = 2.5 + rng.next_f32() * 3.5;
                    wander.hunting = false;
                }

                let to_target =
                    Vec3::new(wander.target.0 - pos.0.x, 0.0, wander.target.1 - pos.0.z);
                let dist = to_target.length();
                if dist > 0.15 {
                    let dir = to_target / dist;
                    let speed = kind.0.speed()
                        * if wander.hunting {
                            HUNT_SPEED_MULTIPLIER
                        } else {
                            1.0
                        };
                    let step = (speed * dt).min(dist);
                    pos.0.x += dir.x * step;
                    pos.0.z += dir.z * step;
                    facing.0 = turn_toward(facing.0, dir.z.atan2(dir.x), TURN_RATE * dt);
                }
            }

            let ground = world.terrain_height(pos.0.x.floor() as i32, pos.0.z.floor() as i32);
            pos.0.y = ground as f32 + 1.0;
        }

        attacks
    }

    pub fn build_mesh(&self) -> MeshData {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for (_, (pos, kind, facing)) in self.ecs.query::<(&Pos, &Kind, &Facing)>().iter() {
            push_creature(&mut vertices, &mut indices, pos.0, kind.0, facing.0);
        }
        MeshData { vertices, indices }
    }

    /// Positions + kinds + facing for broadcasting to clients over the
    /// network, so a creature turns the same way on every screen.
    pub fn snapshot(&self) -> Vec<([f32; 3], u8, f32)> {
        self.ecs
            .query::<(&Pos, &Kind, &Facing)>()
            .iter()
            .map(|(_, (pos, kind, facing))| (pos.0.to_array(), kind.0.to_u8(), facing.0))
            .collect()
    }

    /// Positions + kinds + stable ids + health, for exposing to Lua modules.
    pub fn snapshot_with_ids(&self) -> Vec<(u32, u8, [f32; 3], f32, f32)> {
        self.ecs
            .query::<(&CreatureId, &Kind, &Pos, &Health)>()
            .iter()
            .map(|(_, (id, kind, pos, health))| {
                (
                    id.0,
                    kind.0.to_u8(),
                    pos.0.to_array(),
                    health.0,
                    kind.0.max_health(),
                )
            })
            .collect()
    }

    /// Called by the Lua `chase()` API function. Returns false if no
    /// creature has this id (e.g. it despawned).
    pub fn set_chase_target(&mut self, id: u32, target: Vec3) -> bool {
        for (_, (cid, wander)) in self.ecs.query_mut::<(&CreatureId, &mut Wander)>() {
            if cid.0 == id {
                wander.target = (target.x, target.z);
                wander.timer = HUNT_TARGET_TTL;
                wander.hunting = true;
                return true;
            }
        }
        false
    }

    /// Called by the Lua `damage` API function. Returns a `DeathEvent` if
    /// this brought the creature's health to zero (and despawns it);
    /// returns `None` if the creature survived or wasn't found.
    pub fn damage(&mut self, id: u32, amount: f32) -> Option<DeathEvent> {
        let mut target = None;
        for (entity, (cid, health, kind, pos)) in self
            .ecs
            .query_mut::<(&CreatureId, &mut Health, &Kind, &Pos)>()
        {
            if cid.0 == id {
                health.0 -= amount;
                if health.0 <= 0.0 {
                    target = Some((
                        entity,
                        DeathEvent {
                            kind: kind.0,
                            pos: pos.0,
                        },
                    ));
                }
                break;
            }
        }
        if let Some((entity, event)) = target {
            let _ = self.ecs.despawn(entity);
            return Some(event);
        }
        None
    }

    /// Called by the Lua `destroy` API function: removes the creature
    /// immediately regardless of remaining health.
    pub fn destroy(&mut self, id: u32) -> Option<DeathEvent> {
        let mut target = None;
        for (entity, (cid, kind, pos)) in self.ecs.query_mut::<(&CreatureId, &Kind, &Pos)>() {
            if cid.0 == id {
                target = Some((
                    entity,
                    DeathEvent {
                        kind: kind.0,
                        pos: pos.0,
                    },
                ));
                break;
            }
        }
        if let Some((entity, event)) = target {
            let _ = self.ecs.despawn(entity);
            return Some(event);
        }
        None
    }

    /// Passive engine effect for the RedStone block (not Lua-driven): heals
    /// every creature within `radius` of `center` by `amount`, capped at
    /// each creature's max health.
    pub fn heal_near(&mut self, center: Vec3, radius: f32, amount: f32) {
        let radius_sq = radius * radius;
        for (_, (pos, kind, health)) in self.ecs.query_mut::<(&Pos, &Kind, &mut Health)>() {
            if pos.0.distance_squared(center) <= radius_sq {
                health.0 = (health.0 + amount).min(kind.0.max_health());
            }
        }
    }

    /// Whether any creature is currently in Lua-driven chase mode. Used by
    /// tests to verify the Lua API actually changes creature behavior; kept
    /// public since it's a reasonable hook for a future HUD indicator too.
    #[allow(dead_code)]
    pub fn any_hunting(&self) -> bool {
        self.ecs.query::<&Wander>().iter().any(|(_, w)| w.hunting)
    }

    /// Current heading (radians, `Camera::forward` convention) of the
    /// creature with this id, if it still exists. Exposed mainly for
    /// tests; also a reasonable hook for a future debug HUD.
    #[allow(dead_code)]
    pub fn facing_of(&self, id: u32) -> Option<f32> {
        self.ecs
            .query::<(&CreatureId, &Facing)>()
            .iter()
            .find(|(_, (cid, _))| cid.0 == id)
            .map(|(_, (_, facing))| facing.0)
    }
}

/// Builds a creature mesh straight from a network snapshot, for clients that
/// don't run creature AI locally.
pub fn mesh_for_snapshot(entries: &[([f32; 3], u8, f32)]) -> MeshData {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (pos, kind, facing) in entries {
        push_creature(
            &mut vertices,
            &mut indices,
            Vec3::from_array(*pos),
            CreatureKind::from_u8(*kind),
            *facing,
        );
    }
    MeshData { vertices, indices }
}

fn find_land_spot(
    world: &World,
    rng: &mut SimpleRng,
    cx: f32,
    cz: f32,
    radius: f32,
) -> Option<(f32, f32)> {
    for _ in 0..6 {
        let angle = rng.next_f32() * std::f32::consts::TAU;
        let dist = rng.next_f32() * radius;
        let x = cx + angle.cos() * dist;
        let z = cz + angle.sin() * dist;
        if world.terrain_height(x.floor() as i32, z.floor() as i32) > SEA_LEVEL {
            return Some((x, z));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn turn_toward_steps_by_at_most_max_delta() {
        let result = turn_toward(0.0, FRAC_PI_2, 0.1);
        assert!(
            (result - 0.1).abs() < 1e-5,
            "expected a single 0.1 rad step toward the target, got {result}"
        );
    }

    #[test]
    fn turn_toward_reaches_the_target_without_overshooting_when_close_enough() {
        let result = turn_toward(0.0, 0.05, 0.1);
        assert!(
            (result - 0.05).abs() < 1e-5,
            "a target closer than max_delta should be reached exactly, got {result}"
        );
    }

    #[test]
    fn turn_toward_takes_the_shorter_way_across_the_wrap_boundary() {
        // From just under +PI toward just under -PI: the short way is
        // forward (increasing, wrapping past PI), not backward through 0.
        let result = turn_toward(3.0, -3.0, 0.05);
        assert!(
            (result - 3.05).abs() < 1e-4,
            "expected the turn to continue increasing (wrapping), got {result}"
        );
    }

    #[test]
    fn turn_toward_normalizes_the_result_after_wrapping_past_pi() {
        let result = turn_toward(3.1, -3.1, 1.0);
        assert!(
            (-PI..=PI).contains(&result),
            "result should stay normalized to (-PI, PI], got {result}"
        );
        // Should have landed close to the target, on the correct (negative)
        // side of the wrap rather than back near +3.1.
        assert!(
            (result - (-3.0998)).abs() < 0.01,
            "expected the wrapped result near the target, got {result}"
        );
    }

    #[test]
    fn a_creature_turns_to_face_its_chase_target_over_several_ticks() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Sheep, spawn, 1);

        // Default facing is 0.0 (+X); send it due +Z instead (a 90 degree
        // turn) so any turning that happens is unambiguous.
        let target = spawn + Vec3::new(0.0, 0.0, 10.0);
        for _ in 0..40 {
            creatures.set_chase_target(id, target);
            creatures.update(&world, 1.0 / 60.0, &[]);
        }

        let facing = creatures.facing_of(id).expect("creature should still exist");
        assert!(
            (facing - FRAC_PI_2).abs() < 0.05,
            "expected the creature to have turned to face +Z (facing ~= {FRAC_PI_2}), got {facing}"
        );
    }

    #[test]
    fn a_stationary_creature_keeps_its_last_facing() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Sheep, spawn, 1);

        // Turn it to face +Z first.
        let target = spawn + Vec3::new(0.0, 0.0, 10.0);
        for _ in 0..40 {
            creatures.set_chase_target(id, target);
            creatures.update(&world, 1.0 / 60.0, &[]);
        }
        let facing_after_turn = creatures.facing_of(id).unwrap();

        // Now let the chase target lapse (stop refreshing it) right at its
        // current position -- with nothing to walk toward, facing must not
        // drift or reset.
        let pos = creatures.snapshot_with_ids()[0].2;
        creatures.set_chase_target(id, Vec3::from_array(pos));
        for _ in 0..10 {
            creatures.update(&world, 1.0 / 60.0, &[]);
        }

        let facing_after_idle = creatures.facing_of(id).unwrap();
        assert!(
            (facing_after_idle - facing_after_turn).abs() < 1e-4,
            "facing shouldn't change while not moving: {facing_after_turn} -> {facing_after_idle}"
        );
    }

    #[test]
    fn creature_kind_u8_round_trips_including_stone_golem() {
        assert_eq!(CreatureKind::from_u8(0).to_u8(), 0);
        assert_eq!(CreatureKind::from_u8(1).to_u8(), 1);
        assert_eq!(CreatureKind::from_u8(2).to_u8(), 2);
        assert_eq!(CreatureKind::StoneGolem.to_u8(), 2);
    }

    #[test]
    fn a_stone_golem_closes_in_on_a_player_within_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        // Within aggro range but well outside melee range, so this window
        // only exercises the "close the distance" part of the behavior.
        let player_pos = golem_pos + Vec3::new(6.0, 0.0, 0.0);
        let initial_dist = golem_pos.distance(player_pos);

        for _ in 0..60 {
            creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
        }

        let golem_pos_after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
        let dist_after = golem_pos_after.distance(player_pos);
        assert!(
            dist_after < initial_dist,
            "expected the golem to have moved closer to the player: {initial_dist} -> {dist_after}"
        );
    }

    #[test]
    fn a_stone_golem_ignores_a_player_outside_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = golem_pos + Vec3::new(STONE_GOLEM_AGGRO_RADIUS + 5.0, 0.0, 0.0);

        for _ in 0..120 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "a golem should never attack a player outside its aggro radius"
            );
        }
    }

    #[test]
    fn only_stone_golems_ever_attack_a_nearby_player() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Sheep, spawn, 1);
        creatures.spawn_one(CreatureKind::Chicken, spawn, 2);
        let player_id: PlayerId = 7;
        // Standing right on top of both -- if either could attack, this is
        // as favorable a setup for it as possible.
        let player_pos = spawn;

        for _ in 0..300 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "sheep and chicken must never attack a player, regardless of proximity"
            );
        }
    }

    #[test]
    fn a_stone_golem_already_in_melee_range_attacks_on_a_cooldown_not_every_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = golem_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(golem_pos.distance(player_pos) <= STONE_GOLEM_ATTACK_RANGE);

        let dt = 1.0 / 60.0;
        let duration = STONE_GOLEM_ATTACK_COOLDOWN * 3.0 + 0.5;
        let ticks = (duration / dt) as usize;
        let mut hit_times = Vec::new();
        let mut t = 0.0f32;
        for _ in 0..ticks {
            let attacks = creatures.update(&world, dt, &[(player_id, player_pos)]);
            if !attacks.is_empty() {
                assert_eq!(
                    attacks,
                    vec![(player_id, STONE_GOLEM_ATTACK_DAMAGE)],
                    "at most one hit per tick, for the right amount"
                );
                hit_times.push(t);
            }
            t += dt;
        }

        assert!(
            hit_times.len() >= 3,
            "expected several hits over {duration}s at a {STONE_GOLEM_ATTACK_COOLDOWN}s cooldown, got {}: {hit_times:?}",
            hit_times.len()
        );
        for pair in hit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= STONE_GOLEM_ATTACK_COOLDOWN - dt * 2.0,
                "hits should be spaced at least the cooldown apart, got a {gap}s gap: {hit_times:?}"
            );
        }
    }
}
