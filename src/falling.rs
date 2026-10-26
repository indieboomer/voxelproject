//! Falling blocks (docs/FALLING_BLOCKS.md): any group of blocks that loses
//! its connection to the ground falls straight down, keeping its exact shape,
//! until it lands on support. Trees, rock outcrops and building pieces all
//! behave the same way; nothing tips over or rotates.
//!
//! Every source cell has exactly one owner at a time: a world block, then a
//! falling `Body` (the world cell is cleared by an ordinary edit), then a
//! world block again, an exact whole number of cells lower (ordinary edits).
//! Plants it lands on are crushed into an item drop with exact counts; a body
//! with nothing below it ever becomes one drop. The host owns all of it;
//! clients draw bodies from spawn messages and snapshot poses.
//!
//! The previous physics-based tree felling (toppling rigid bodies) is kept
//! on the `falling-trees-physics` branch.

use glam::{IVec3, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::voxel::{BlockType, World};

type Pos = (i32, i32, i32);

/// A support search that visits more cells than this assumes the group is
/// part of the ground: removing a block from a hillside never drops the hill.
pub const MAX_COMPONENT: usize = 4096;
/// Simultaneous falling bodies; further groups wait with their cells in place.
pub const MAX_BODIES: usize = 8;
const GRAVITY: f32 = 20.0;
const MAX_SPEED: f32 = 30.0;

const DIRS: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

fn at(p: Pos, d: IVec3) -> Pos {
    (p.0 + d.x, p.1 + d.y, p.2 + d.z)
}

/// Blocks that hold their group in place and never move: the world floor,
/// unbreakable blocks, devices/campfires (moved only by their own tools),
/// and anything at the edge of loaded terrain.
fn anchor(world: &World, p: Pos, block: BlockType) -> bool {
    p.1 <= 0
        || block.is_unbreakable()
        || matches!(block, BlockType::AutomationDevice | BlockType::Campfire)
        || world.automation.device_at(p).is_some()
        || !world.chunks.contains_key(&crate::voxel::chunk::world_to_chunk(p.0, p.2))
}

/// Structure blocks: anything solid. Plants and water are not structure.
fn structural(block: BlockType) -> bool {
    block.is_solid()
}

/// The solid group containing `start`, and whether it is anchored (touches
/// the world floor, an anchor block or unloaded terrain, or is too large to
/// be anything but ground). `None` when `start` is not structure.
fn group(world: &World, start: Pos) -> Option<(Vec<(Pos, BlockType)>, bool)> {
    if !structural(world.get_block(start.0, start.1, start.2)) {
        return None;
    }
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut cells = Vec::new();
    while let Some(p) = queue.pop_front() {
        let block = world.get_block(p.0, p.1, p.2);
        if anchor(world, p, block) {
            return Some((cells, true));
        }
        cells.push((p, block));
        if cells.len() > MAX_COMPONENT {
            return Some((cells, true));
        }
        for d in DIRS {
            let n = at(p, d);
            if !seen.contains(&n) && structural(world.get_block(n.0, n.1, n.2)) {
                seen.insert(n);
                queue.push_back(n);
            }
        }
    }
    Some((cells, false))
}

/// Unsupported groups next to `around` (a cell whose block was just removed),
/// each with the plants standing on it (they ride along).
pub fn unsupported_groups(world: &World, around: Pos) -> Vec<Vec<(Pos, BlockType)>> {
    let mut done: HashSet<Pos> = HashSet::new();
    let mut result = Vec::new();
    for d in DIRS {
        let seed = at(around, d);
        if done.contains(&seed) {
            continue;
        }
        let Some((cells, anchored)) = group(world, seed) else {
            continue;
        };
        done.extend(cells.iter().map(|(p, _)| *p));
        if anchored {
            continue;
        }
        let own: HashSet<Pos> = cells.iter().map(|(p, _)| *p).collect();
        let mut riders: Vec<(Pos, BlockType)> = own
            .iter()
            .map(|p| (p.0, p.1 + 1, p.2))
            .filter(|p| !own.contains(p))
            .filter_map(|p| {
                let b = world.get_block(p.0, p.1, p.2);
                (b != BlockType::Air && !structural(b) && b != BlockType::Water).then_some((p, b))
            })
            .collect();
        riders.sort_by_key(|(p, _)| *p);
        let mut cells = cells;
        cells.extend(riders);
        result.push(cells);
    }
    result
}

/// A falling group. Cell offsets are relative to `origin`, whose center
/// `pos` tracks; `fallen` is how far it has dropped so far, in cells.
#[derive(Clone, Debug)]
pub struct Body {
    pub id: u32,
    pub origin: [i32; 3],
    pub cells: Vec<([i32; 3], BlockType)>,
    pub pos: Vec3,
    pub rot: Quat,
    pub fallen: f32,
    speed: f32,
}

/// What clients need to draw a body: sent once, poses follow in snapshots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyInfo {
    pub id: u32,
    pub origin: [i32; 3],
    pub cells: Vec<([i8; 3], BlockType)>,
}

impl Body {
    fn new(id: u32, cells: &[(Pos, BlockType)]) -> Self {
        let origin = cells[0].0;
        Self {
            id,
            origin: [origin.0, origin.1, origin.2],
            cells: cells.iter().map(|(p, b)| ([p.0 - origin.0, p.1 - origin.1, p.2 - origin.2], *b)).collect(),
            pos: Vec3::new(origin.0 as f32 + 0.5, origin.1 as f32 + 0.5, origin.2 as f32 + 0.5),
            rot: Quat::IDENTITY,
            fallen: 0.0,
            speed: 0.0,
        }
    }

    pub fn info(&self) -> BodyInfo {
        BodyInfo {
            id: self.id,
            origin: self.origin,
            cells: self.cells.iter().map(|(o, b)| (o.map(|c| c.clamp(-127, 127) as i8), *b)).collect(),
        }
    }

    /// World cells when lowered by `drop` whole cells.
    fn cells_at(&self, drop: i32) -> impl Iterator<Item = (Pos, BlockType)> + '_ {
        let o = self.origin;
        self.cells.iter().map(move |(c, b)| ((o[0] + c[0], o[1] + c[1] - drop, o[2] + c[2]), *b))
    }

    fn center(&self) -> Vec3 {
        let n = self.cells.len().max(1) as f32;
        self.cells.iter().map(|(c, _)| Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32)).sum::<Vec3>() / n
            + Vec3::new(self.origin[0] as f32 + 0.5, self.origin[1] as f32 + 0.5 - self.fallen, self.origin[2] as f32 + 0.5)
    }

    /// Item counts this body is worth: one existing yield per source cell.
    pub fn contents(&self) -> Vec<(BlockType, u32)> {
        tally(self.cells.iter().map(|(_, b)| *b))
    }
}

fn tally(blocks: impl Iterator<Item = BlockType>) -> Vec<(BlockType, u32)> {
    let mut counts: Vec<(BlockType, u32)> = Vec::new();
    for b in blocks {
        let item = b.wood_resource();
        match counts.iter_mut().find(|(k, _)| *k == item) {
            Some((_, n)) => *n += 1,
            None => counts.push((item, 1)),
        }
    }
    counts
}

/// Why a body cannot drop to a given level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Blocked {
    /// Solid ground (or a body that already landed this frame): land here.
    Ground,
    /// A player is in the way: wait above them until they move.
    Player,
    /// The level is below the world: nothing will ever hold it.
    Void,
}

fn blocked(world: &World, body: &Body, drop: i32, players: &[Vec3], landed: &HashSet<Pos>) -> Option<Blocked> {
    let mut player = false;
    for (p, _) in body.cells_at(drop) {
        if p.1 < 0 {
            return Some(Blocked::Void);
        }
        let b = world.get_block(p.0, p.1, p.2);
        if structural(b) || landed.contains(&p) || world.automation.device_at(p).is_some() {
            return Some(Blocked::Ground);
        }
        player |= players.iter().any(|f| {
            let (lo, hi) = (*f - Vec3::new(0.3, 0.0, 0.3), *f + Vec3::new(0.3, 1.8, 0.3));
            lo.x < (p.0 + 1) as f32 && hi.x > p.0 as f32 && lo.y < (p.1 + 1) as f32 && hi.y > p.1 as f32
                && lo.z < (p.2 + 1) as f32 && hi.z > p.2 as f32
        });
    }
    player.then_some(Blocked::Player)
}

/// How a body ended: placed whole cells lower (plants it landed on are
/// crushed into items), or converted entirely to one drop when nothing below
/// can ever hold it. Every source cell is accounted for exactly once.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Placed { cells: Vec<(Pos, BlockType)>, crushed: Vec<(BlockType, u32)>, at: Vec3 },
    Dropped(Vec3, Vec<(BlockType, u32)>),
}

/// All falling bodies and pending support checks (host-side state).
pub struct Falling {
    pub bodies: Vec<Body>,
    queued: VecDeque<Vec<(Pos, BlockType)>>,
    checks: Vec<Pos>,
    next_id: u32,
}

impl Default for Falling {
    fn default() -> Self {
        Self { bodies: Vec::new(), queued: VecDeque::new(), checks: Vec::new(), next_id: 1 }
    }
}

impl Falling {
    /// Record that a structure block at `pos` was removed.
    pub fn note_removed(&mut self, pos: Pos) {
        self.checks.push(pos);
    }

    /// Support checks for recorded removals: groups to detach now (the caller
    /// clears their cells with edits, then calls `spawn`).
    pub fn take_detached(&mut self, world: &World) -> Vec<Vec<(Pos, BlockType)>> {
        let mut out: Vec<Vec<(Pos, BlockType)>> = Vec::new();
        for pos in std::mem::take(&mut self.checks) {
            for cells in unsupported_groups(world, pos) {
                if out.iter().any(|c| c.iter().any(|x| cells.contains(x))) {
                    continue; // already claimed this frame
                }
                out.push(cells);
            }
        }
        let pending: Vec<_> = self.queued.drain(..).chain(out).collect();
        let mut ready = Vec::new();
        for cells in pending {
            if self.bodies.len() + ready.len() < MAX_BODIES {
                ready.push(cells);
            } else {
                self.queued.push_back(cells);
            }
        }
        // A queued group may have changed meanwhile: keep cells still intact.
        ready
            .into_iter()
            .filter_map(|cells| {
                let cells: Vec<_> = cells.into_iter().filter(|(p, b)| world.get_block(p.0, p.1, p.2) == *b).collect();
                (!cells.is_empty()).then_some(cells)
            })
            .collect()
    }

    pub fn spawn(&mut self, cells: &[(Pos, BlockType)]) -> &Body {
        let id = self.next_id;
        self.next_id += 1;
        self.bodies.push(Body::new(id, cells));
        self.bodies.last().unwrap()
    }

    /// Drops every body under gravity, straight down. A body lands as soon
    /// as the next whole cell below any part of it is ground; it waits above
    /// a player in the way. Landed bodies are removed and returned.
    pub fn step(&mut self, world: &World, dt: f32, players: &[Vec3]) -> Vec<(u32, Outcome)> {
        let dt = dt.min(1.0 / 20.0);
        let mut landed: HashSet<Pos> = HashSet::new();
        let mut finished = Vec::new();
        let mut index = 0;
        while index < self.bodies.len() {
            let body = &mut self.bodies[index];
            body.speed = (body.speed + GRAVITY * dt).min(MAX_SPEED);
            let target = body.fallen + body.speed * dt;
            // Advance cell by cell so nothing tunnels through a thin floor.
            let mut stop = None;
            let mut level = body.fallen.floor() as i32;
            while (level as f32) < target {
                match blocked(world, body, level + 1, players, &landed) {
                    None => level += 1,
                    Some(why) => {
                        stop = Some((level, why));
                        break;
                    }
                }
            }
            match stop {
                Some((level, Blocked::Player)) => {
                    body.fallen = level as f32;
                    body.speed = 0.0;
                }
                Some((level, why)) => {
                    let body = self.bodies.remove(index);
                    let outcome = Self::land(world, &body, level, why == Blocked::Void);
                    if let Outcome::Placed { cells, .. } = &outcome {
                        landed.extend(cells.iter().map(|(p, _)| *p));
                    }
                    finished.push((body.id, outcome));
                    continue;
                }
                None => body.fallen = target,
            }
            body.pos = Vec3::new(body.origin[0] as f32 + 0.5, body.origin[1] as f32 + 0.5 - body.fallen, body.origin[2] as f32 + 0.5);
            index += 1;
        }
        finished
    }

    /// Lands every body now at the lowest level it can reach (before saving).
    pub fn resolve_all(&mut self, world: &World, players: &[Vec3]) -> Vec<(u32, Outcome)> {
        let mut landed: HashSet<Pos> = HashSet::new();
        let mut finished = Vec::new();
        for body in std::mem::take(&mut self.bodies) {
            let mut level = body.fallen.floor() as i32;
            let void = loop {
                match blocked(world, &body, level + 1, players, &landed) {
                    None => level += 1,
                    Some(Blocked::Void) => break true,
                    Some(_) => break false,
                }
            };
            let outcome = Self::land(world, &body, level, void);
            if let Outcome::Placed { cells, .. } = &outcome {
                landed.extend(cells.iter().map(|(p, _)| *p));
            }
            finished.push((body.id, outcome));
        }
        finished
    }

    fn land(world: &World, body: &Body, level: i32, void: bool) -> Outcome {
        let mut lowered = body.clone();
        lowered.fallen = level as f32;
        if void {
            return Outcome::Dropped(lowered.center(), body.contents());
        }
        // Plants and decorations in the landing cells are crushed into items;
        // water is displaced. Settled branch blocks become their wood.
        let cells: Vec<(Pos, BlockType)> = body.cells_at(level).map(|(p, b)| (p, b.wood_resource())).collect();
        let crushed = tally(cells.iter().map(|(p, _)| world.get_block(p.0, p.1, p.2)).filter(|b| {
            *b != BlockType::Air && *b != BlockType::Water && crate::voxel::COLLECTIBLE_BLOCKS.contains(b)
        }));
        Outcome::Placed { cells, crushed, at: lowered.center() }
    }
}

/// The body's look in local space (relative to its origin cell's center),
/// rebuilt in a scratch world as the blocks it will land as.
pub fn body_mesh(info: &BodyInfo, shaping: bool) -> crate::voxel::mesher::MeshData {
    use crate::voxel::chunk::{world_to_chunk, Chunk, CHUNK_Y};
    let mut scratch = World::new(0);
    scratch.terrain_shaping = shaping;
    let low = info.cells.iter().fold(IVec3::splat(i32::MAX), |m, (o, _)| m.min(IVec3::new(o[0] as i32, o[1] as i32, o[2] as i32)));
    let shift = IVec3::new(2 - low.x, (CHUNK_Y / 2).max(2 - low.y), 2 - low.z);
    for (o, _) in &info.cells {
        let p = shift + IVec3::new(o[0] as i32, o[1] as i32, o[2] as i32);
        let key = world_to_chunk(p.x, p.z);
        scratch.chunks.entry(key).or_insert_with(|| Chunk::new(key.0, key.1));
    }
    for (o, block) in &info.cells {
        let p = shift + IVec3::new(o[0] as i32, o[1] as i32, o[2] as i32);
        if (0..CHUNK_Y).contains(&p.y) {
            // Edits, like the landed blocks: no living-tree look, no sway.
            scratch.set_block(p.x, p.y, p.z, block.wood_resource());
        }
    }
    let mut mesh = crate::voxel::mesher::MeshData { vertices: Vec::new(), indices: Vec::new() };
    let mut keys: Vec<_> = scratch.chunks.keys().copied().collect();
    keys.sort();
    for key in keys {
        mesh.extend(crate::voxel::mesher::build_chunk_mesh(&scratch, &scratch.chunks[&key]));
    }
    let center = shift.as_vec3() + Vec3::splat(0.5);
    for v in &mut mesh.vertices {
        v.position = (Vec3::from(v.position) - center).to_array();
        v.wind = 0.0;
    }
    mesh
}

/// `local` placed at a body pose.
pub fn posed(local: &crate::voxel::mesher::MeshData, pos: Vec3, rot: Quat) -> crate::voxel::mesher::MeshData {
    let vertices = local
        .vertices
        .iter()
        .map(|v| {
            let mut v = *v;
            v.position = (pos + rot * Vec3::from(v.position)).to_array();
            v.normal = (rot * Vec3::from(v.normal)).to_array();
            v
        })
        .collect();
    crate::voxel::mesher::MeshData { vertices, indices: local.indices.clone() }
}

/// Client-side view of host bodies: spawn info plus the latest pose.
#[derive(Default)]
pub struct Mirror {
    pub bodies: HashMap<u32, (BodyInfo, Vec3, Quat)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::chunk::Chunk;

    /// Chunks around the origin with a stone floor (y 0..=2, top surface 3)
    /// plus generated `cells` written as chunk data.
    fn world(cells: &[(Pos, BlockType)]) -> World {
        let mut world = World::new(4);
        for cx in -1..=1 {
            for cz in -1..=1 {
                let mut chunk = Chunk::new(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        for y in 0..3 {
                            chunk.set_local(x, y, z, BlockType::Stone);
                        }
                    }
                }
                world.chunks.insert((cx, cz), chunk);
            }
        }
        for &((x, y, z), b) in cells {
            let key = crate::voxel::chunk::world_to_chunk(x, z);
            world.chunks.get_mut(&key).unwrap().set_local(x.rem_euclid(16), y, z.rem_euclid(16), b);
        }
        world
    }

    fn run(falling: &mut Falling, w: &World, players: &[Vec3]) -> Vec<(u32, Outcome)> {
        let mut out = Vec::new();
        for _ in 0..600 {
            out.extend(falling.step(w, 1.0 / 60.0, players));
            if falling.bodies.is_empty() {
                break;
            }
        }
        out
    }

    /// Removes `removed` and detaches what that leaves unsupported.
    fn detach(w: &mut World, removed: Pos) -> Falling {
        w.set_block(removed.0, removed.1, removed.2, BlockType::Air);
        let mut falling = Falling::default();
        falling.note_removed(removed);
        for cells in falling.take_detached(w) {
            for &((x, y, z), _) in &cells {
                w.set_block(x, y, z, BlockType::Air);
            }
            falling.spawn(&cells);
        }
        falling
    }

    /// A tree standing on the floor: trunk y 3..=7 at (5, 5), leaves above.
    fn tree() -> Vec<(Pos, BlockType)> {
        let mut cells: Vec<_> = (3..=7).map(|y| ((5, y, 5), BlockType::OakWood)).collect();
        cells.push(((6, 6, 5), BlockType::OakBranch));
        for dx in -1..=1 {
            for dz in -1..=1 {
                cells.push(((5 + dx, 8, 5 + dz), BlockType::OakLeaves));
            }
        }
        cells
    }

    #[test]
    fn a_cut_tree_drops_straight_down_onto_its_stump_upright() {
        let mut w = world(&tree());
        let mut falling = detach(&mut w, (5, 4, 5)); // cut above the bottom log
        assert_eq!(falling.bodies.len(), 1);
        let count = falling.bodies[0].cells.len();
        let Outcome::Placed { cells, crushed, .. } = run(&mut falling, &w, &[]).pop().unwrap().1 else {
            panic!("expected placement")
        };
        assert!(crushed.is_empty());
        assert_eq!(cells.len(), count);
        // Same shape, one cell lower, nothing rotated: the trunk now stands
        // on the stump and the limb became an ordinary log.
        assert!(cells.contains(&((5, 4, 5), BlockType::OakWood)));
        assert!(cells.contains(&((5, 6, 5), BlockType::OakWood)));
        assert!(cells.contains(&((6, 5, 5), BlockType::OakWood)));
        assert!(cells.contains(&((5, 7, 5), BlockType::OakLeaves)));
        assert!(cells.iter().all(|(p, _)| w.get_block(p.0, p.1, p.2) == BlockType::Air));
    }

    #[test]
    fn a_cut_off_rock_ledge_falls_until_its_lowest_part_lands() {
        // A stone pillar (x = 7, y 3..=8) carrying a granite ledge at y 9, and
        // a short stone column at x = 8 (y 3..=4) under the ledge.
        let mut cells = vec![((8, 3, 8), BlockType::Stone), ((8, 4, 8), BlockType::Stone)];
        for y in 3..=8 {
            cells.push(((7, y, 8), BlockType::Stone));
        }
        for x in 7..=10 {
            cells.push(((x, 9, 8), BlockType::Granite));
        }
        let mut w = world(&cells);
        let mut falling = detach(&mut w, (7, 4, 8)); // knock out the pillar near its base
        assert_eq!(falling.bodies.len(), 1);
        let Outcome::Placed { cells, .. } = run(&mut falling, &w, &[]).pop().unwrap().1 else { panic!() };
        // The pillar's bottom meets the stump after one cell: everything,
        // ledge included, lands exactly one cell lower in the same shape.
        assert!(cells.contains(&((7, 4, 8), BlockType::Stone)));
        for x in 7..=10 {
            assert!(cells.contains(&((x, 8, 8), BlockType::Granite)), "{cells:?}");
        }
        assert_eq!(cells.len(), 4 + 4);
    }

    #[test]
    fn hills_and_overhangs_connected_to_the_ground_never_fall() {
        let mut cells = Vec::new();
        for x in 2..12 {
            for y in 3..7 {
                cells.push(((x, y, 4), BlockType::Stone));
            }
        }
        cells.push(((12, 6, 4), BlockType::Stone)); // an overhang
        let mut w = world(&cells);
        assert!(detach(&mut w, (6, 3, 4)).bodies.is_empty(), "digging under a hill drops nothing");
        assert!(detach(&mut w, (11, 5, 4)).bodies.is_empty(), "the overhang is still held from its side");
        assert_eq!(detach(&mut w, (11, 6, 4)).bodies.len(), 1, "its last link cut, the overhang falls");
    }

    #[test]
    fn plants_ride_on_top_and_are_crushed_underneath() {
        let cells = vec![
            ((5, 3, 5), BlockType::Stone),
            ((5, 6, 5), BlockType::Stone),
            ((5, 7, 5), BlockType::Fern),     // rides the falling block
            ((5, 4, 5), BlockType::RedPoppy), // in the landing spot
        ];
        let w = world(&cells);
        let mut w = w;
        let mut falling = detach(&mut w, (5, 5, 5));
        let Outcome::Placed { cells, crushed, .. } = run(&mut falling, &w, &[]).pop().unwrap().1 else { panic!() };
        assert!(cells.contains(&((5, 4, 5), BlockType::Stone)));
        assert!(cells.contains(&((5, 5, 5), BlockType::Fern)));
        assert_eq!(crushed, vec![(BlockType::RedPoppy, 1)]);
    }

    #[test]
    fn a_player_underneath_holds_the_block_until_they_move() {
        let mut w = world(&[((5, 3, 5), BlockType::Stone), ((5, 9, 5), BlockType::Stone)]);
        let mut falling = detach(&mut w, (5, 8, 5));
        let player = [Vec3::new(5.5, 4.0, 5.5)];
        for _ in 0..120 {
            assert!(falling.step(&w, 1.0 / 60.0, &player).is_empty());
        }
        assert!((falling.bodies[0].fallen - 3.0).abs() < 1e-6, "rests on the player's head");
        let Outcome::Placed { cells, .. } = run(&mut falling, &w, &[]).pop().unwrap().1 else { panic!() };
        assert_eq!(cells, vec![((5, 4, 5), BlockType::Stone)]);
    }

    #[test]
    fn nothing_below_becomes_an_exact_drop_and_save_lands_everything_now() {
        let mut w = world(&[]);
        for y in 0..3 {
            w.chunks.get_mut(&(0, 0)).unwrap().set_local(3, y, 3, BlockType::Air);
        }
        let mut falling = Falling::default();
        falling.spawn(&[((3, 5, 3), BlockType::Stone), ((3, 6, 3), BlockType::OakLeaves)]);
        falling.spawn(&[((9, 8, 9), BlockType::Stone)]);
        let outcomes = falling.resolve_all(&w, &[]);
        assert!(falling.bodies.is_empty());
        assert!(matches!(&outcomes[0].1, Outcome::Dropped(_, c) if *c == vec![(BlockType::Stone, 1), (BlockType::OakLeaves, 1)]));
        assert!(matches!(&outcomes[1].1, Outcome::Placed { cells, .. } if *cells == vec![((9, 3, 9), BlockType::Stone)]));
    }

    #[test]
    fn two_bodies_landing_together_never_overlap() {
        let mut w = world(&[]);
        let mut falling = Falling::default();
        falling.spawn(&[((5, 6, 5), BlockType::Stone)]);
        falling.spawn(&[((5, 8, 5), BlockType::Granite)]);
        let mut placed = HashSet::new();
        for (_, o) in falling.resolve_all(&w, &[]) {
            let Outcome::Placed { cells, .. } = o else { panic!() };
            for (p, b) in cells {
                assert!(placed.insert(p), "overlap at {p:?}");
                w.set_block(p.0, p.1, p.2, b);
            }
        }
        assert_eq!(w.get_block(5, 3, 5), BlockType::Stone);
        assert_eq!(w.get_block(5, 4, 5), BlockType::Granite);
    }

    #[test]
    fn falling_bodies_render_without_wind_and_fit_one_packet() {
        let mut falling = Falling::default();
        let body = falling.spawn(&tree()).clone();
        let mesh = body_mesh(&body.info(), true);
        assert!(!mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| v.wind == 0.0));
        let big = BodyInfo { id: 1, origin: [0; 3], cells: vec![([0, 0, 0], BlockType::Stone); MAX_COMPONENT + 64] };
        let msg = crate::net::Packet::Reliable {
            id: 1,
            msg: crate::net::ReliableMsg::FallingSpawn { body: big, pos: [0.0; 3], rot: [0.0; 4] },
        };
        assert!(crate::net::encode(&msg).len() < crate::transport::MAX_PACKET_BYTES);
    }
}
