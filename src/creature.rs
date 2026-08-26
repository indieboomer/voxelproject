use glam::Vec3;

use crate::voxel::atlas::white_uv;
use crate::voxel::mesher::{push_cuboid, MeshData, Vertex};
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
}

impl CreatureKind {
    fn speed(self) -> f32 {
        match self {
            CreatureKind::Sheep => 1.4,
            CreatureKind::Chicken => 2.0,
        }
    }

    pub fn max_health(self) -> f32 {
        match self {
            CreatureKind::Sheep => 12.0,
            CreatureKind::Chicken => 6.0,
        }
    }

    fn body_half_extent(self) -> Vec3 {
        match self {
            CreatureKind::Sheep => Vec3::new(0.35, 0.45, 0.5),
            CreatureKind::Chicken => Vec3::new(0.2, 0.22, 0.28),
        }
    }

    fn body_color(self) -> [f32; 3] {
        match self {
            CreatureKind::Sheep => [0.92, 0.92, 0.88],
            CreatureKind::Chicken => [0.92, 0.82, 0.25],
        }
    }

    fn head_color(self) -> [f32; 3] {
        match self {
            CreatureKind::Sheep => [0.72, 0.68, 0.62],
            CreatureKind::Chicken => [0.85, 0.25, 0.2],
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            CreatureKind::Sheep => 0,
            CreatureKind::Chicken => 1,
        }
    }

    pub fn from_u8(v: u8) -> CreatureKind {
        match v {
            1 => CreatureKind::Chicken,
            _ => CreatureKind::Sheep,
        }
    }
}

/// Appends the two boxes (body + head) representing one creature at `feet`
/// to a mesh being built. Shared by the live ECS-driven mesh and the
/// network-snapshot mesh so both draw identically.
fn push_creature(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    feet: Vec3,
    kind: CreatureKind,
) {
    let uv = white_uv();
    let half = kind.body_half_extent();
    let body_min = Vec3::new(feet.x - half.x, feet.y, feet.z - half.z);
    let body_max = Vec3::new(feet.x + half.x, feet.y + half.y * 2.0, feet.z + half.z);
    push_cuboid(vertices, indices, body_min, body_max, kind.body_color(), uv);

    let head_size = half * 0.55;
    let head_center_z = feet.z + half.z * 0.8;
    let head_min = Vec3::new(
        feet.x - head_size.x,
        body_max.y - head_size.y * 0.5,
        head_center_z - head_size.z,
    );
    let head_max = Vec3::new(
        feet.x + head_size.x,
        body_max.y + head_size.y * 1.1,
        head_center_z + head_size.z,
    );
    push_cuboid(vertices, indices, head_min, head_max, kind.head_color(), uv);
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
/// Stable id exposed to Lua scripts, since a `hecs::Entity` isn't a
/// convenient thing to hand across the API boundary.
struct CreatureId(u32);

const HUNT_SPEED_MULTIPLIER: f32 = 1.6;
/// How long a Lua-set chase target stays valid before the creature reverts
/// to normal wandering if it isn't refreshed again.
const HUNT_TARGET_TTL: f32 = 0.3;

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
            CreatureId(id),
            SimpleRng::new(rng_seed),
        ));
        id
    }

    /// Called by the Lua `spawn_creature` API function.
    pub fn spawn_one(&mut self, kind: CreatureKind, pos: Vec3, rng_seed: u64) -> u32 {
        self.spawn_with_rng(kind, pos, rng_seed)
    }

    /// Only the host runs creature AI; joined clients just render whatever
    /// positions the host's snapshot reports.
    pub fn update(&mut self, world: &World, dt: f32) {
        for (_, (pos, wander, kind, rng)) in self
            .ecs
            .query_mut::<(&mut Pos, &mut Wander, &Kind, &mut SimpleRng)>()
        {
            wander.timer -= dt;
            if wander.timer <= 0.0 {
                if let Some((x, z)) = find_land_spot(world, rng, pos.0.x, pos.0.z, 6.0) {
                    wander.target = (x, z);
                }
                wander.timer = 2.5 + rng.next_f32() * 3.5;
                wander.hunting = false;
            }

            let to_target = Vec3::new(wander.target.0 - pos.0.x, 0.0, wander.target.1 - pos.0.z);
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
            }

            let ground = world.terrain_height(pos.0.x.floor() as i32, pos.0.z.floor() as i32);
            pos.0.y = ground as f32 + 1.0;
        }
    }

    pub fn build_mesh(&self) -> MeshData {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for (_, (pos, kind)) in self.ecs.query::<(&Pos, &Kind)>().iter() {
            push_creature(&mut vertices, &mut indices, pos.0, kind.0);
        }
        MeshData { vertices, indices }
    }

    /// Positions + kinds for broadcasting to clients over the network.
    pub fn snapshot(&self) -> Vec<([f32; 3], u8)> {
        self.ecs
            .query::<(&Pos, &Kind)>()
            .iter()
            .map(|(_, (pos, kind))| (pos.0.to_array(), kind.0.to_u8()))
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
}

/// Builds a creature mesh straight from a network snapshot, for clients that
/// don't run creature AI locally.
pub fn mesh_for_snapshot(entries: &[([f32; 3], u8)]) -> MeshData {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (pos, kind) in entries {
        push_creature(
            &mut vertices,
            &mut indices,
            Vec3::from_array(*pos),
            CreatureKind::from_u8(*kind),
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
