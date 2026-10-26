//! Falling trees (docs/FALLING_TREES.md): unsupported tree parts detach as one
//! rigid body, fall, and settle back onto the voxel grid as resources.
//!
//! Lifecycle of every source cell, owned by exactly one place at a time:
//! attached (a world block) -> detached (a `Body`; the world cell is cleared
//! by an ordinary edit) -> settled (world blocks again, as resource roles via
//! ordinary edits) or, when no safe placement exists, one loot drop holding
//! exactly the same item counts. The host owns all of it; clients only draw
//! bodies from spawn messages and snapshot poses.
//!
//! Physics sits behind `FallPhysics` so a full engine (for example Rapier)
//! can replace `SimplePhysics` without touching detection or settlement.

use glam::{IVec3, Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::voxel::{organic, BlockType, World};

type Pos = (i32, i32, i32);

/// Largest tree component considered; bigger ones are treated as anchored
/// rather than risking a huge unexpected collapse.
pub const MAX_COMPONENT: usize = 1500;
/// Simultaneous falling bodies; further detachments wait in a queue with
/// their cells still in the world.
pub const MAX_BODIES: usize = 6;
/// Leaves farther than this (in leaf steps) from detached wood stay put.
const LEAF_REACH: i32 = 4;
const GRAVITY: f32 = 20.0;
const WOOD_MASS: f32 = 1.0;
const LEAF_MASS: f32 = 0.12;
/// Seconds of calm, supported rest before settling; a hard cap after that.
const CALM_TIME: f32 = 0.6;
const MAX_FALL_TIME: f32 = 12.0;

const DIRS: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];

fn at(p: Pos, d: IVec3) -> Pos {
    (p.0 + d.x, p.1 + d.y, p.2 + d.z)
}

fn living_wood(world: &World, p: Pos) -> Option<BlockType> {
    let b = world.get_block(p.0, p.1, p.2);
    (b.is_wood() && organic::is_living(world, b, p)).then_some(b)
}

fn living_leaf(world: &World, p: Pos) -> bool {
    let b = world.get_block(p.0, p.1, p.2);
    b.def().cutout && world.edits.get(&p) != Some(&b)
}

/// A trunk cell standing on real ground (not wood, not leaves) roots its tree.
fn rooted(world: &World, p: Pos, block: BlockType) -> bool {
    if block.is_branch() {
        return false;
    }
    let below = world.get_block(p.0, p.1 - 1, p.2);
    below.is_solid() && !below.def().cutout && living_wood(world, (p.0, p.1 - 1, p.2)).is_none()
}

/// Living wood connected to `start` through trunk and branch links of the
/// same species, and whether any of it is rooted. `None` if over the cap.
fn wood_component(world: &World, start: Pos) -> Option<(Vec<(Pos, BlockType)>, bool)> {
    let species = living_wood(world, start)?.wood_resource();
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    let mut cells = Vec::new();
    let mut anchored = false;
    while let Some(p) = queue.pop_front() {
        let block = living_wood(world, p)?;
        anchored |= rooted(world, p, block);
        cells.push((p, block));
        if cells.len() > MAX_COMPONENT {
            return None;
        }
        for d in DIRS {
            let n = at(p, d);
            if !seen.contains(&n) && living_wood(world, n).is_some_and(|b| b.wood_resource() == species) {
                seen.insert(n);
                queue.push_back(n);
            }
        }
    }
    Some((cells, anchored))
}

/// Unsupported tree components next to `around` (a cell that just lost its
/// living wood), each with its associated foliage. Leaves are payload: a leaf
/// joins a component only if it is reachable from the component's wood
/// through leaves, within `LEAF_REACH`, and strictly closer to it than to any
/// living wood that stays attached, so touching crowns never hold each other
/// up or get dragged along.
pub fn detached_components(world: &World, around: Pos) -> Vec<Vec<(Pos, BlockType)>> {
    let mut done = HashSet::new();
    let mut result = Vec::new();
    for d in DIRS {
        let seed = at(around, d);
        if done.contains(&seed) || living_wood(world, seed).is_none() {
            continue;
        }
        let Some((wood, anchored)) = wood_component(world, seed) else {
            continue; // too large: treat as anchored
        };
        done.extend(wood.iter().map(|(p, _)| *p));
        if anchored {
            continue;
        }
        let mut cells = wood.clone();
        cells.extend(associated_leaves(world, &wood));
        result.push(cells);
    }
    result
}

fn associated_leaves(world: &World, wood: &[(Pos, BlockType)]) -> Vec<(Pos, BlockType)> {
    let own: HashSet<Pos> = wood.iter().map(|(p, _)| *p).collect();
    let (mut lo, mut hi) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
    for (p, _) in wood {
        let v = IVec3::new(p.0, p.1, p.2);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let pad = IVec3::splat(LEAF_REACH + 1);
    let (lo, hi) = (lo - pad, hi + pad);
    // Multi-source BFS over leaves: detached wood (true) against attached
    // living wood (false). Attached seeds go first, so ties stay attached.
    let mut owner: HashMap<Pos, (bool, i32)> = HashMap::new();
    let mut queue = VecDeque::new();
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                let p = (x, y, z);
                if !own.contains(&p) && living_wood(world, p).is_some() {
                    queue.push_back((p, false, 0));
                }
            }
        }
    }
    for (p, _) in wood {
        queue.push_back((*p, true, 0));
    }
    while let Some((p, detached, dist)) = queue.pop_front() {
        if dist >= LEAF_REACH {
            continue;
        }
        for d in DIRS {
            let n = at(p, d);
            let inside = n.0 >= lo.x && n.0 <= hi.x && n.1 >= lo.y && n.1 <= hi.y && n.2 >= lo.z && n.2 <= hi.z;
            if inside && !owner.contains_key(&n) && living_leaf(world, n) {
                owner.insert(n, (detached, dist + 1));
                queue.push_back((n, detached, dist + 1));
            }
        }
    }
    let mut leaves: Vec<(Pos, BlockType)> = owner
        .into_iter()
        .filter(|(_, (detached, _))| *detached)
        .map(|(p, _)| (p, world.get_block(p.0, p.1, p.2)))
        .collect();
    leaves.sort_by_key(|(p, _)| *p);
    leaves
}

/// A detached tree part in flight. Cell offsets are relative to `origin`
/// (the cell whose center `pos` tracks); `rot` turns offsets into the world.
#[derive(Clone, Debug)]
pub struct Body {
    pub id: u32,
    pub origin: [i32; 3],
    pub cells: Vec<([i32; 3], BlockType)>,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub spin: Vec3,
    com: Vec3,
    inv_mass: f32,
    inv_inertia: Mat3,
    calm: f32,
    pub age: f32,
}

/// What clients need to draw a body: sent once, poses follow in snapshots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyInfo {
    pub id: u32,
    pub origin: [i32; 3],
    pub cells: Vec<([i8; 3], BlockType)>,
}

impl Body {
    fn new(id: u32, cells: &[(Pos, BlockType)], push: Vec3) -> Self {
        let origin = cells[0].0;
        let cells: Vec<([i32; 3], BlockType)> =
            cells.iter().map(|(p, b)| ([p.0 - origin.0, p.1 - origin.1, p.2 - origin.2], *b)).collect();
        let mass_of = |b: BlockType| if b.def().cutout { LEAF_MASS } else { WOOD_MASS };
        let mass: f32 = cells.iter().map(|(_, b)| mass_of(*b)).sum();
        let com = cells.iter().map(|(o, b)| Vec3::from(o.map(|c| c as f32)) * mass_of(*b)).sum::<Vec3>() / mass;
        let mut inertia = Mat3::ZERO;
        for (o, b) in &cells {
            let r = Vec3::from(o.map(|c| c as f32)) - com;
            let m = mass_of(*b);
            // Point mass plus a unit cube's own inertia.
            inertia += (Mat3::IDENTITY * r.length_squared() - outer(r, r)) * m + Mat3::IDENTITY * (m / 6.0);
        }
        let origin_center = Vec3::new(origin.0 as f32 + 0.5, origin.1 as f32 + 0.5, origin.2 as f32 + 0.5);
        // A gentle start so a balanced trunk still tips away from the cut.
        let axis = Vec3::Y.cross(push).normalize_or_zero();
        Self {
            id,
            origin: [origin.0, origin.1, origin.2],
            cells,
            pos: origin_center,
            rot: Quat::IDENTITY,
            vel: push * 0.3,
            spin: axis * 0.9,
            com,
            inv_mass: 1.0 / mass,
            inv_inertia: inertia.inverse(),
            calm: 0.0,
            age: 0.0,
        }
    }

    pub fn info(&self) -> BodyInfo {
        BodyInfo {
            id: self.id,
            origin: self.origin,
            cells: self.cells.iter().map(|(o, b)| (o.map(|c| c.clamp(-127, 127) as i8), *b)).collect(),
        }
    }

    /// World position of cell `offset`'s center.
    pub fn cell_center(&self, offset: [i32; 3]) -> Vec3 {
        self.pos + self.rot * Vec3::from(offset.map(|c| c as f32))
    }

    fn com_world(&self) -> Vec3 {
        self.pos + self.rot * self.com
    }

    fn inv_inertia_world(&self) -> Mat3 {
        let r = Mat3::from_quat(self.rot);
        r * self.inv_inertia * r.transpose()
    }

    /// Item counts this body is worth: one existing yield per source cell.
    pub fn contents(&self) -> Vec<(BlockType, u32)> {
        tally(self.cells.iter().map(|(_, b)| *b))
    }
}

fn outer(a: Vec3, b: Vec3) -> Mat3 {
    Mat3::from_cols(a * b.x, a * b.y, a * b.z)
}

/// Rigid-body integration and contact response for one body.
pub trait FallPhysics {
    /// Advances `body` by `dt`; returns whether it touched static ground.
    fn step(&self, world: &World, body: &mut Body, dt: f32) -> bool;
}

/// A small impulse solver: every cell is a contact sphere against the static
/// voxel world (full cubes), with restitution, friction and positional
/// correction. Enough for trees toppling and coming to rest.
pub struct SimplePhysics;

impl FallPhysics for SimplePhysics {
    fn step(&self, world: &World, body: &mut Body, dt: f32) -> bool {
        let mut touched = false;
        let steps = 4;
        let h = dt / steps as f32;
        for _ in 0..steps {
            let com = body.com_world();
            let wet = world.get_block(com.x.floor() as i32, com.y.floor() as i32, com.z.floor() as i32) == BlockType::Water;
            body.vel.y -= GRAVITY * if wet { 0.25 } else { 1.0 } * h;
            if wet {
                body.vel *= 0.94;
                body.spin *= 0.94;
            }
            body.pos += body.vel * h;
            body.rot = (Quat::from_scaled_axis(body.spin * h) * body.rot).normalize();
            // Gather contacts.
            let com = body.com_world();
            let inv_i = body.inv_inertia_world();
            let mut deepest: Option<(Vec3, f32)> = None;
            // Foliage contacts are smaller and softer than wood: a fallen
            // crown props the tree up a little, as real crowns do.
            for (offset, block) in body.cells.clone() {
                let radius = if block.def().cutout { 0.12 } else { 0.45 };
                let c = body.cell_center(offset);
                let base = c.floor();
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            let v = base + Vec3::new(dx as f32, dy as f32, dz as f32);
                            let b = world.get_block(v.x as i32, v.y as i32, v.z as i32);
                            if !b.is_solid() {
                                continue;
                            }
                            let closest = c.clamp(v, v + Vec3::ONE);
                            let delta = c - closest;
                            let dist = delta.length();
                            if dist >= radius {
                                continue;
                            }
                            let (n, depth) = if dist < 1e-4 { (Vec3::Y, radius) } else { (delta / dist, radius - dist) };
                            touched = true;
                            if deepest.map_or(true, |(_, d)| depth > d) {
                                deepest = Some((n, depth));
                            }
                            // Impulse along the normal, then Coulomb friction.
                            let r = c - com;
                            let rel = body.vel + body.spin.cross(r);
                            let vn = rel.dot(n);
                            if vn >= 0.0 {
                                continue;
                            }
                            let k = body.inv_mass + n.dot((inv_i * r.cross(n)).cross(r));
                            let j = -(1.0 + 0.1) * vn / k;
                            body.vel += n * j * body.inv_mass;
                            body.spin += inv_i * r.cross(n * j);
                            let rel = body.vel + body.spin.cross(r);
                            let tangent = rel - n * rel.dot(n);
                            if tangent.length_squared() > 1e-8 {
                                let t = tangent.normalize();
                                let kt = body.inv_mass + t.dot((inv_i * r.cross(t)).cross(r));
                                let jt = (-rel.dot(t) / kt).max(-0.6 * j);
                                body.vel += t * jt * body.inv_mass;
                                body.spin += inv_i * r.cross(t * jt);
                            }
                        }
                    }
                }
            }
            if let Some((n, depth)) = deepest {
                body.pos += n * depth * 0.6;
                body.spin *= 0.985;
            }
        }
        touched
    }
}

/// The 24 proper rotations of the cubic grid.
pub fn grid_rotations() -> Vec<[[i32; 3]; 3]> {
    let mut out = Vec::new();
    for perm in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        for signs in 0..8 {
            let mut m = [[0; 3]; 3];
            for (row, &col) in perm.iter().enumerate() {
                m[row][col] = if signs & (1 << row) != 0 { -1 } else { 1 };
            }
            let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            if det == 1 {
                out.push(m);
            }
        }
    }
    out
}

fn apply(m: &[[i32; 3]; 3], o: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|r| m[r][0] * o[0] + m[r][1] * o[1] + m[r][2] * o[2])
}

fn to_quat(m: &[[i32; 3]; 3]) -> Quat {
    let col = |c: usize| Vec3::new(m[0][c] as f32, m[1][c] as f32, m[2][c] as f32);
    Quat::from_mat3(&Mat3::from_cols(col(0), col(1), col(2)))
}

/// How a body ended: placed back on the grid (leaves that would land inside
/// existing blocks are crushed into an item drop), or converted entirely to
/// one item drop. Every source cell is accounted for exactly once.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Placed { cells: Vec<(Pos, BlockType)>, crushed: Vec<(BlockType, u32)>, at: Vec3 },
    Dropped(Vec3, Vec<(BlockType, u32)>),
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

/// Grid placement for a resting body: the grid rotations nearest its pose
/// (by trunk axis, then twist), tried closest first (the first rotation with any valid translation wins),
/// with a bounded search of integer translations (up to four cells down,
/// since a fallen tree rests on its crown and limbs). Each cell offset is
/// transformed exactly (never rounded per cell), so the mapping is
/// one-to-one. Every trunk cell needs an air destination outside every
/// player, and at least one placed log must rest on real ground (never on
/// leaves). Limbs and leaves whose destination is occupied are crushed into
/// items rather than overwriting anything. Among valid candidates the one
/// placing the most wood, then the most leaves, then the smallest shift wins.
/// Returns `(placed cells, crushed source blocks)`.
#[allow(clippy::type_complexity)]
pub fn placement(world: &World, body: &Body, players: &[Vec3]) -> Option<(Vec<(Pos, BlockType)>, Vec<BlockType>)> {
    // Rank by where the trunk axis (the body's local up) ends up, so a
    // tree lying at 50 degrees settles lying rather than snapping upright;
    // overall closeness breaks ties between twists about that axis.
    let up = body.rot * Vec3::Y;
    let mut rotations = grid_rotations();
    rotations.sort_by(|a, b| {
        let key = |m: &[[i32; 3]; 3]| {
            let q = to_quat(m);
            ((q * Vec3::Y).dot(up), q.dot(body.rot).abs())
        };
        let (ka, kb) = (key(a), key(b));
        kb.0.total_cmp(&ka.0).then(kb.1.total_cmp(&ka.1))
    });
    let base = (body.pos - Vec3::splat(0.5)).round();
    let mut shifts: Vec<IVec3> = Vec::new();
    for dy in [0, -1, -2, -3, -4, 1, 2] {
        for dx in [0, 1, -1] {
            for dz in [0, 1, -1] {
                shifts.push(IVec3::new(dx, dy, dz));
            }
        }
    }
    let blocked_by_player = |p: Pos| {
        players.iter().any(|f| {
            let (lo, hi) = (*f - Vec3::new(0.3, 0.0, 0.3), *f + Vec3::new(0.3, 1.8, 0.3));
            let (cl, ch) = (Vec3::new(p.0 as f32, p.1 as f32, p.2 as f32), Vec3::new(p.0 as f32 + 1.0, p.1 as f32 + 1.0, p.2 as f32 + 1.0));
            lo.x < ch.x && hi.x > cl.x && lo.y < ch.y && hi.y > cl.y && lo.z < ch.z && hi.z > cl.z
        })
    };
    let free = |p: &Pos| {
        (0..crate::voxel::chunk::CHUNK_Y).contains(&p.1)
            && world.chunks.contains_key(&crate::voxel::chunk::world_to_chunk(p.0, p.2))
            && world.get_block(p.0, p.1, p.2) == BlockType::Air
            && world.automation.device_at(*p).is_none()
            && !blocked_by_player(*p)
    };
    let trunk = |b: &BlockType| b.is_wood() && !b.is_branch();
    #[allow(clippy::type_complexity)]
    let mut best: Option<((usize, usize, i32), Vec<(Pos, BlockType)>, Vec<BlockType>)> = None;
    for m in rotations.iter().take(4) {
        if best.is_some() {
            break; // a closer rotation already fits
        }
        for shift in &shifts {
            let t = IVec3::new(base.x as i32, base.y as i32, base.z as i32) + *shift;
            let cells: Vec<(Pos, BlockType)> = body
                .cells
                .iter()
                .map(|(o, b)| {
                    let r = apply(m, *o);
                    ((t.x + r[0], t.y + r[1], t.z + r[2]), *b)
                })
                .collect();
            if !cells.iter().filter(|(_, b)| trunk(b)).all(|(p, _)| free(p)) {
                continue;
            }
            let (placed, crushed): (Vec<_>, Vec<_>) = cells.into_iter().partition(|(p, b)| trunk(b) || free(p));
            let logs: HashSet<Pos> = placed.iter().filter(|(_, b)| !b.def().cutout).map(|(p, _)| *p).collect();
            let supported = logs.iter().any(|p| {
                let below = (p.0, p.1 - 1, p.2);
                let ground = world.get_block(below.0, below.1, below.2);
                !logs.contains(&below) && ground.is_solid() && !ground.def().cutout
            });
            if !supported {
                continue;
            }
            let score = (logs.len(), placed.len() - logs.len(), -(shift.x.abs() + shift.y.abs() + shift.z.abs()));
            if best.as_ref().map_or(true, |(s, _, _)| score > *s) {
                best = Some((score, placed, crushed.into_iter().map(|(_, b)| b).collect()));
            }
        }
    }
    best.map(|(_, placed, crushed)| (placed.into_iter().map(|(p, b)| (p, settled_role(b))).collect(), crushed))
}

/// Living wood settles as its species' wood resource (a log); growing leaves
/// as static leaf resources. Placed through edits, both stay non-living.
pub fn settled_role(block: BlockType) -> BlockType {
    block.wood_resource()
}

/// All falling bodies and pending support checks (host-side state).
pub struct Falling {
    pub bodies: Vec<Body>,
    queued: VecDeque<(Vec<(Pos, BlockType)>, Vec3)>,
    checks: Vec<(Pos, Vec3)>,
    next_id: u32,
    physics: Box<dyn FallPhysics>,
}

impl Default for Falling {
    fn default() -> Self {
        Self {
            bodies: Vec::new(),
            queued: VecDeque::new(),
            checks: Vec::new(),
            next_id: 1,
            physics: Box::new(SimplePhysics),
        }
    }
}

impl Falling {
    /// Record that living wood at `pos` was removed; `push` is the horizontal
    /// direction the cut should tip the tree (away from the cutter).
    pub fn note_cut(&mut self, pos: Pos, push: Vec3) {
        self.checks.push((pos, push));
    }

    /// Support checks for recorded cuts: returns components to detach now
    /// (the caller clears their cells with edits, then calls `spawn`).
    pub fn take_detached(&mut self, world: &World) -> Vec<(Vec<(Pos, BlockType)>, Vec3)> {
        let mut out: Vec<(Vec<(Pos, BlockType)>, Vec3)> = Vec::new();
        for (pos, push) in std::mem::take(&mut self.checks) {
            for cells in detached_components(world, pos) {
                // Skip anything already claimed this frame.
                if out.iter().any(|(c, _)| c.iter().any(|x| cells.contains(x))) {
                    continue;
                }
                out.push((cells, push));
            }
        }
        // Over budget: queue the rest; their cells stay in the world.
        let mut ready = Vec::new();
        let pending: Vec<_> = self.queued.drain(..).chain(out).collect();
        for item in pending {
            if self.bodies.len() + ready.len() < MAX_BODIES {
                ready.push(item);
            } else {
                self.queued.push_back(item);
            }
        }
        // A queued component may have been edited meanwhile: keep only cells
        // that are still exactly what was recorded.
        ready
            .into_iter()
            .filter_map(|(cells, push)| {
                let cells: Vec<_> = cells.into_iter().filter(|(p, b)| world.get_block(p.0, p.1, p.2) == *b).collect();
                (!cells.is_empty()).then_some((cells, push))
            })
            .collect()
    }

    pub fn spawn(&mut self, cells: &[(Pos, BlockType)], push: Vec3) -> &Body {
        let id = self.next_id;
        self.next_id += 1;
        self.bodies.push(Body::new(id, cells, push));
        self.bodies.last().unwrap()
    }

    /// Advances every body; finished ones are removed and returned with
    /// their outcome for the caller to apply as edits or a drop.
    pub fn step(&mut self, world: &World, dt: f32, players: &[Vec3]) -> Vec<(u32, Outcome)> {
        let dt = dt.min(1.0 / 30.0);
        let mut finished = Vec::new();
        let mut index = 0;
        while index < self.bodies.len() {
            let body = &mut self.bodies[index];
            body.age += dt;
            let touched = self.physics.step(world, body, dt);
            let calm = body.vel.length() < 0.2 && body.spin.length() < 0.15;
            body.calm = if touched && calm { body.calm + dt } else { 0.0 };
            let lost = body.pos.y < -16.0;
            if body.calm >= CALM_TIME || body.age >= MAX_FALL_TIME || lost {
                let body = self.bodies.remove(index);
                let outcome = Self::finish(world, &body, players);
                finished.push((body.id, outcome));
            } else {
                index += 1;
            }
        }
        finished
    }

    /// Settles or converts every body now (before saving, or on shutdown).
    pub fn resolve_all(&mut self, world: &World, players: &[Vec3]) -> Vec<(u32, Outcome)> {
        self.bodies.drain(..).map(|b| (b.id, Self::finish(world, &b, players))).collect()
    }

    fn finish(world: &World, body: &Body, players: &[Vec3]) -> Outcome {
        match placement(world, body, players) {
            Some((cells, crushed)) => Outcome::Placed { cells, crushed: tally(crushed.into_iter()), at: body.com_world() },
            None => Outcome::Dropped(body.com_world(), body.contents()),
        }
    }
}

/// The body's look in local space (relative to its origin cell's center):
/// the part is rebuilt as vegetation in a scratch world so trunks, limbs and
/// crowns keep their shapes in flight. Wind is zeroed: detached foliage only
/// moves with the body.
pub fn body_mesh(info: &BodyInfo, shaping: bool) -> crate::voxel::mesher::MeshData {
    use crate::voxel::chunk::{world_to_chunk, Chunk, CHUNK_Y};
    let mut scratch = World::new(0);
    scratch.terrain_shaping = shaping;
    let low = info.cells.iter().fold(IVec3::splat(i32::MAX), |m, (o, _)| m.min(IVec3::new(o[0] as i32, o[1] as i32, o[2] as i32)));
    // Origin cell placement in the scratch world, keeping every cell inside.
    let shift = IVec3::new(2 - low.x, (CHUNK_Y / 2).max(2 - low.y), 2 - low.z);
    for (o, block) in &info.cells {
        let p = shift + IVec3::new(o[0] as i32, o[1] as i32, o[2] as i32);
        if !(0..CHUNK_Y).contains(&p.y) {
            continue;
        }
        let key = world_to_chunk(p.x, p.z);
        scratch
            .chunks
            .entry(key)
            .or_insert_with(|| Chunk::new(key.0, key.1))
            .set_local(p.x.rem_euclid(16), p.y, p.z.rem_euclid(16), *block);
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

    /// Generated (living) blocks are chunk data; ground everywhere at y = 0.
    fn world(cells: &[(Pos, BlockType)]) -> World {
        let mut world = World::new(4);
        for (cx, cz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let mut chunk = Chunk::new(cx, cz);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_local(x, 0, z, BlockType::Grass);
                }
            }
            world.chunks.insert((cx, cz), chunk);
        }
        for &((x, y, z), b) in cells {
            let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
            world.chunks.get_mut(&(cx, cz)).unwrap().set_local(x.rem_euclid(16), y, z.rem_euclid(16), b);
        }
        world
    }

    /// A trunk at (x, z) with a limb and a leaf ball on top.
    fn tree(x: i32, z: i32, height: i32) -> Vec<(Pos, BlockType)> {
        let mut cells: Vec<(Pos, BlockType)> = (1..=height).map(|y| ((x, y, z), BlockType::OakWood)).collect();
        cells.push(((x + 1, height - 1, z), BlockType::OakBranch));
        for dx in -1..=1 {
            for dz in -1..=1 {
                for dy in 1..=2 {
                    let p = (x + dx, height + dy, z + dz);
                    cells.push((p, BlockType::OakLeaves));
                }
            }
        }
        cells.push(((x + 2, height - 1, z), BlockType::OakLeaves));
        cells
    }

    #[test]
    fn cutting_a_trunk_releases_the_crown_and_keeps_the_stump() {
        let mut w = world(&tree(8, 8, 6));
        w.set_block(8, 2, 8, BlockType::Air); // the cut
        let parts = detached_components(&w, (8, 2, 8));
        assert_eq!(parts.len(), 1);
        let part = &parts[0];
        assert!(part.iter().any(|(p, _)| *p == (8, 3, 8)));
        assert!(!part.iter().any(|(p, _)| *p == (8, 1, 8)), "the stump stays");
        assert_eq!(part.iter().filter(|(_, b)| b.is_wood()).count(), 4 + 1, "trunk above the cut and the limb");
        assert_eq!(part.iter().filter(|(_, b)| b.def().cutout).count(), 19, "the crown and the limb's leaf");
        // The stump below is still anchored: nothing else detaches.
        assert!(detached_components(&w, (8, 0, 8)).is_empty());
    }

    #[test]
    fn cutting_a_limb_releases_only_the_limb_and_its_leaves() {
        let mut cells = tree(8, 8, 6);
        cells.push(((10, 5, 8), BlockType::OakBranch));
        cells.push(((11, 5, 8), BlockType::OakLeaves));
        let mut w = world(&cells);
        w.set_block(9, 5, 8, BlockType::Air);
        let parts = detached_components(&w, (9, 5, 8));
        assert_eq!(parts.len(), 1);
        let part: HashSet<Pos> = parts[0].iter().map(|(p, _)| *p).collect();
        assert!(part.contains(&(10, 5, 8)) && part.contains(&(11, 5, 8)));
        assert!(!part.contains(&(8, 7, 8)), "the trunk's crown stays");
    }

    #[test]
    fn touching_crowns_do_not_hold_or_drag_each_other() {
        let mut cells = tree(6, 8, 6);
        cells.extend(tree(9, 8, 6)); // crowns touch at x = 7..8
        let mut w = world(&cells);
        w.set_block(6, 1, 8, BlockType::Air);
        let parts = detached_components(&w, (6, 1, 8));
        assert_eq!(parts.len(), 1, "tree A falls although its leaves touch tree B");
        let part: HashSet<Pos> = parts[0].iter().map(|(p, _)| *p).collect();
        assert!(!part.contains(&(9, 3, 8)), "tree B's wood stays");
        assert!(!part.contains(&(9, 7, 8)) && !part.contains(&(10, 7, 8)), "tree B's crown stays");
    }

    #[test]
    fn placed_wood_never_detaches_or_supports() {
        let mut w = world(&tree(8, 8, 6));
        // A player's log beside the trunk is not part of the tree.
        w.set_block(9, 3, 8, BlockType::OakWood);
        w.set_block(8, 2, 8, BlockType::Air);
        let parts = detached_components(&w, (8, 2, 8));
        assert_eq!(parts.len(), 1);
        assert!(!parts[0].iter().any(|(p, _)| *p == (9, 3, 8)));
    }

    #[test]
    fn grid_rotations_are_the_24_proper_ones_and_map_one_to_one() {
        let rotations = grid_rotations();
        assert_eq!(rotations.len(), 24);
        let offsets: Vec<[i32; 3]> = (-2..=2).flat_map(|x| (-2..=2).flat_map(move |y| (-2..=2).map(move |z| [x, y, z]))).collect();
        for m in &rotations {
            let mapped: HashSet<[i32; 3]> = offsets.iter().map(|o| apply(m, *o)).collect();
            assert_eq!(mapped.len(), offsets.len());
        }
    }

    fn fall(w: &World, cells: &[(Pos, BlockType)], push: Vec3) -> Outcome {
        let mut falling = Falling::default();
        falling.spawn(cells, push);
        for _ in 0..(20.0 * 60.0) as usize {
            if let Some((_, outcome)) = falling.step(w, 1.0 / 60.0, &[]).pop() {
                return outcome;
            }
        }
        panic!("never finished");
    }

    #[test]
    fn a_cut_tree_falls_settles_on_the_grid_and_conserves_material() {
        let mut w = world(&tree(8, 8, 6));
        w.set_block(8, 1, 8, BlockType::Air);
        let parts = detached_components(&w, (8, 1, 8));
        let part = parts[0].clone();
        for (p, _) in &part {
            w.set_block(p.0, p.1, p.2, BlockType::Air);
        }
        match fall(&w, &part, Vec3::X) {
            Outcome::Placed { cells, crushed, .. } => {
                let set: HashSet<Pos> = cells.iter().map(|(p, _)| *p).collect();
                assert_eq!(set.len(), cells.len(), "one-to-one");
                assert!(cells.iter().all(|(p, _)| w.get_block(p.0, p.1, p.2) == BlockType::Air));
                // Material: exactly one resource per source cell, by role;
                // crushed leaves become items instead of overwriting blocks.
                let count = |c: &[(Pos, BlockType)], b: BlockType| c.iter().filter(|(_, x)| *x == b).count();
                let crushed_leaves = crushed.iter().find(|(b, _)| *b == BlockType::OakLeaves).map_or(0, |(_, n)| *n as usize);
                assert_eq!(count(&cells, BlockType::OakLeaves) + crushed_leaves, count(&part, BlockType::OakLeaves));
                let crushed_wood = crushed.iter().find(|(b, _)| *b == BlockType::OakWood).map_or(0, |(_, n)| *n as usize);
                assert_eq!(count(&cells, BlockType::OakWood) + crushed_wood, part.iter().filter(|(_, b)| b.is_wood()).count());
                // It toppled: lying down, every log is at most two cells up,
                // and nothing went below the world.
                assert!(cells.iter().filter(|(_, b)| b.is_wood()).all(|(p, _)| p.1 <= 2), "{cells:?}");
                assert!(cells.iter().all(|(p, _)| p.1 >= 1));
            }
            other => panic!("expected a placement, got {other:?}"),
        }
    }

    #[test]
    fn no_landing_space_falls_back_to_exact_item_counts() {
        // Solid stone all around the body: its own cells are the only air,
        // and a player stands in them.
        let mut cells = Vec::new();
        for x in 4..=12 {
            for z in 4..=12 {
                for y in 1..=8 {
                    if (x, z) != (8, 8) || !(3..=4).contains(&y) {
                        cells.push(((x, y, z), BlockType::Stone));
                    }
                }
            }
        }
        let w = world(&cells);
        let body_cells = vec![((8, 3, 8), BlockType::OakWood), ((8, 4, 8), BlockType::OakLeaves)];
        let mut falling = Falling::default();
        falling.spawn(&body_cells, Vec3::ZERO);
        // A player stands where it would land.
        let outcome = falling.resolve_all(&w, &[Vec3::new(8.5, 3.0, 8.5)]).pop().unwrap().1;
        match outcome {
            Outcome::Dropped(_, contents) => {
                assert_eq!(contents, vec![(BlockType::OakWood, 1), (BlockType::OakLeaves, 1)]);
            }
            other => panic!("expected a drop, got {other:?}"),
        }
    }

    #[test]
    fn leaves_alone_cannot_hold_a_settled_log_up() {
        // Only leaves under the body: no placement is supported.
        let mut cells = Vec::new();
        for x in 4..13 {
            for z in 4..13 {
                cells.push(((x, 1, z), BlockType::OakLeaves));
            }
        }
        let mut w = world(&cells);
        for x in 4..13 {
            for z in 4..13 {
                w.set_block(x, 1, z, BlockType::OakLeaves); // placed leaf resources
            }
        }
        let mut falling = Falling::default();
        falling.spawn(&[((8, 2, 8), BlockType::OakWood)], Vec3::ZERO);
        assert!(matches!(falling.resolve_all(&w, &[]).pop().unwrap().1, Outcome::Dropped(..)));
    }

    #[test]
    #[ignore = "debug trace"]
    fn trace_fall() {
        let mut w = world(&tree(8, 8, 6));
        w.set_block(8, 1, 8, BlockType::Air);
        let part = detached_components(&w, (8, 1, 8))[0].clone();
        for (p, _) in &part {
            w.set_block(p.0, p.1, p.2, BlockType::Air);
        }
        let mut falling = Falling::default();
        falling.spawn(&part, Vec3::X);
        for i in 0..1200 {
            if !falling.step(&w, 1.0 / 60.0, &[]).is_empty() { println!("done at {i}"); break; }
            let b = &falling.bodies[0];
            if i % 10 == 0 { println!("{i} pos {:.2} up {:.2} vel {:.2} spin {:.2} calm {:.2}", b.pos, b.rot * Vec3::Y, b.vel, b.spin, b.calm); }
        }
    }

    #[test]
    fn falling_bodies_keep_their_tree_look_without_wind_and_fit_one_packet() {
        let cells = tree(8, 8, 6);
        let mut falling = Falling::default();
        let body = falling.spawn(&cells, Vec3::X).clone();
        for shaping in [false, true] {
            let mesh = body_mesh(&body.info(), shaping);
            assert!(!mesh.vertices.is_empty());
            assert!(mesh.vertices.iter().all(|v| v.wind == 0.0), "detached foliage never sways");
            // Local space: centered on the origin cell, spanning the tree.
            let top = mesh.vertices.iter().map(|v| v.position[1]).fold(f32::MIN, f32::max);
            assert!((6.0..9.0).contains(&top), "{top}");
            let posed = posed(&mesh, Vec3::new(8.5, 1.5, 8.5), Quat::IDENTITY);
            assert_eq!(posed.vertices.len(), mesh.vertices.len());
        }
        // The largest allowed component still fits one reliable packet.
        let big = BodyInfo { id: 1, origin: [0; 3], cells: vec![([0, 0, 0], BlockType::OakLeaves); MAX_COMPONENT * 2] };
        let msg = crate::net::Packet::Reliable { id: 1, msg: crate::net::ReliableMsg::FallingSpawn { body: big, pos: [0.0; 3], rot: [0.0; 4] } };
        assert!(crate::net::encode(&msg).len() < crate::transport::MAX_PACKET_BYTES);
    }

    #[test]
    #[ignore = "debug trace of the grove oak"]
    fn trace_grove() {
        let mut w = World::new(0);
        for cx in -2..2 { for cz in -2..2 { w.chunks.insert((cx, cz), Chunk::new(cx, cz)); } }
        let (generated, placed) = crate::voxel::coast::tree_grove((-6, 7, -6));
        for (x, y, z, b) in generated {
            if let Some(c) = w.chunks.get_mut(&crate::voxel::chunk::world_to_chunk(x, z)) { c.set_local(x.rem_euclid(16), y, z.rem_euclid(16), b); }
        }
        for (x, y, z, b) in placed { w.set_block(x, y, z, b); }
        w.set_block(-4, 7, -3, BlockType::Air);
        let mut f = Falling::default();
        f.note_cut((-4, 7, -3), Vec3::new(1.0, 0.0, -0.4).normalize());
        for (cells, push) in f.take_detached(&w) {
            for &((x, y, z), _) in &cells { w.set_block(x, y, z, BlockType::Air); }
            println!("detached {} cells", cells.len());
            f.spawn(&cells, push);
        }
        let started = std::time::Instant::now();
        let mut frames = 0;
        for i in 0..720 {
            frames += 1;
            let done = f.step(&w, 1.0 / 60.0, &[]);
            if !done.is_empty() { println!("physics+settle: {:.3} ms/frame over {frames} frames", started.elapsed().as_secs_f64() * 1000.0 / frames as f64); }
            if let Some(b) = f.bodies.first() { if i % 30 == 0 { println!("{i} pos {:.2} up {:.2} vel {:.2} spin {:.2}", b.pos, b.rot * Vec3::Y, b.vel, b.spin); } }
            if !done.is_empty() { println!("done {i}: {:?}", done[0].1.clone()); break; }
        }
    }
}
