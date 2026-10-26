//! Organic living trees for shaped mode (docs/TREES.md).
//!
//! A living wood cell (generated trunk wood, or any branch block) renders as
//! tapered, slightly irregular tube pieces instead of a cube. Pieces run from
//! the cell's center to rings on each face shared with a connected living
//! neighbor; both neighbors build that face ring from the same key, so tubes
//! join without gaps across cells and chunks. Trunks flare at the ground and
//! taper upward, limbs leave the trunk at a rising angle through a collar, and
//! dead ends (tips and fresh cuts) close with end-grain caps.
//!
//! Wood the player placed is construction material: it is a recorded edit and
//! keeps its cube. Everything here is derived from blocks and edits, so saves
//! and networking are unchanged; collision and picking use the same pieces.

use glam::Vec3;

use super::block::BlockType;
use super::world::World;

const SEED: u32 = 0x0A6A_7001;
/// Ring sides for trunks and limbs.
const TRUNK_SIDES: usize = 10;
const LIMB_SIDES: usize = 6;
/// Keep every ring this far inside its owner cell.
const MARGIN: f32 = 0.02;
/// Largest ring irregularity (see `ring_point`); radii leave room for it.
const WOBBLE: f32 = 0.09;

/// The six axis directions, in mesher face order (+X, -X, +Y, -Y, +Z, -Z).
const DIRS: [[i32; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

/// Whether the block at `pos` is part of a growing tree: generated trunk wood
/// (not a recorded edit) or a branch block (generation only).
pub fn is_living(world: &World, block: BlockType, pos: (i32, i32, i32)) -> bool {
    block.is_wood() && (block.is_branch() || world.edits.get(&pos) != Some(&block))
}

fn trunk_radius(block: BlockType) -> f32 {
    match block.wood_resource() {
        BlockType::OakWood => 0.44,
        BlockType::SpruceWood => 0.33,
        BlockType::BirchWood => 0.27,
        _ => 0.34, // cherry
    }
}

fn limb_radius(block: BlockType) -> f32 {
    trunk_radius(block) * 0.42
}

fn rand(key: [i32; 3], salt: u32) -> f32 {
    super::noise::block_rand(key[0], key[1], key[2], SEED, salt)
}

/// One tube piece between two rings, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub a: Vec3,
    pub b: Vec3,
    pub ra: f32,
    pub rb: f32,
    /// Ring axes (0 = x, 1 = y, 2 = z) at each end.
    pub axis_a: usize,
    pub axis_b: usize,
    /// Irregularity keys for each ring (shared rings use the same key).
    pub key_a: [i32; 3],
    pub key_b: [i32; 3],
    /// Close end `b` with an end-grain disc (tips and fresh cuts).
    pub cap_b: bool,
    pub sides: usize,
}

/// A living wood cell's derived geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeCell {
    pub pieces: Vec<Piece>,
    /// A rounded knot closing joints that are not straight-through.
    pub knot: Option<(Vec3, f32)>,
}

impl TreeCell {
    /// Whether world point `p` is inside the visible wood.
    pub fn contains(&self, p: Vec3) -> bool {
        self.knot.is_some_and(|(c, r)| p.distance(c) <= r)
            || self.pieces.iter().any(|piece| {
                let ab = piece.b - piece.a;
                let t = ((p - piece.a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                p.distance(piece.a + ab * t) <= piece.ra + (piece.rb - piece.ra) * t
            })
    }

    /// Axis-aligned boxes bounding the wood, for player collision.
    pub fn boxes(&self) -> Vec<(Vec3, Vec3)> {
        let mut boxes: Vec<(Vec3, Vec3)> = self
            .pieces
            .iter()
            .map(|p| {
                let r = Vec3::splat(p.ra.max(p.rb));
                (p.a.min(p.b) - r, p.a.max(p.b) + r)
            })
            .collect();
        if let Some((c, r)) = self.knot {
            boxes.push((c - Vec3::splat(r), c + Vec3::splat(r)));
        }
        boxes
    }
}

/// Organic geometry for the cell at `(x, y, z)`, or `None` when it keeps its
/// cube (shaping off, not wood, or placed construction wood).
pub fn tree_cell(world: &World, x: i32, y: i32, z: i32) -> Option<TreeCell> {
    if !world.terrain_shaping {
        return None;
    }
    let sample = |x, y, z| world.get_block(x, y, z);
    let living = |p: (i32, i32, i32), b: BlockType| is_living(world, b, p);
    cell_with(&sample, &living, x, y, z)
}

/// `tree_cell` against an explicit block sampler and living-wood test.
pub fn cell_with(
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    living: &impl Fn((i32, i32, i32), BlockType) -> bool,
    x: i32,
    y: i32,
    z: i32,
) -> Option<TreeCell> {
    let block = sample(x, y, z);
    if !living((x, y, z), block) {
        return None;
    }
    let species = block.wood_resource();
    let connected = DIRS.map(|d| {
        let p = (x + d[0], y + d[1], z + d[2]);
        let n = sample(p.0, p.1, p.2);
        n.wood_resource() == species && living(p, n)
    });
    let trunk = !block.is_branch();
    // A trunk standing on ground roots into it: a flared ring on its floor.
    let rooted = trunk && !connected[3] && sample(x, y - 1, z).is_solid();
    // Ground level of this trunk, for taper and flare.
    let ground = if trunk {
        let mut g = y;
        while g > y - 24 {
            let below = sample(x, g - 1, z);
            if !(below.wood_resource() == species && !below.is_branch() && living((x, g - 1, z), below)) {
                break;
            }
            g -= 1;
        }
        g
    } else {
        y
    };
    let base = trunk_radius(block);
    let trunk_at = |height: f32| {
        let h = (height - ground as f32).max(0.0);
        base * (1.0 - 0.055 * h).max(0.55) + base * 0.35 * (-h * 2.5).exp()
    };
    let cell = Vec3::new(x as f32, y as f32, z as f32);
    let center_key = [x, y, z];
    // Mild bends: a small lattice-keyed lean of each cell's center.
    let lean = if trunk { 0.07 } else { 0.05 };
    let center = cell + Vec3::new(
        0.5 + (rand(center_key, 1) * 2.0 - 1.0) * lean,
        0.5 + if trunk { 0.0 } else { (rand(center_key, 2) * 2.0 - 1.0) * lean },
        0.5 + (rand(center_key, 3) * 2.0 - 1.0) * lean,
    );
    let room = (0.5 - lean - MARGIN) / (1.0 + WOBBLE);
    let face_center = |d: [i32; 3]| cell + Vec3::new(0.5 + d[0] as f32 * 0.5, 0.5 + d[1] as f32 * 0.5, 0.5 + d[2] as f32 * 0.5);
    let face_key = |d: [i32; 3]| [2 * x + 1 + d[0], 2 * y + 1 + d[1], 2 * z + 1 + d[2]];
    // Ring radius on a shared face: both cells compute the same value.
    let face_radius = |i: usize| -> f32 {
        let d = DIRS[i];
        let n = sample(x + d[0], y + d[1], z + d[2]);
        let other_trunk = !n.is_branch() && connected[i];
        let r = if trunk && (other_trunk || (i == 3 && rooted)) {
            trunk_at((y as f32 + 0.5 + d[1] as f32 * 0.5).floor())
        } else if trunk || (!n.is_branch() && connected[i]) {
            limb_radius(block) * 1.35 // collar where a limb leaves the trunk
        } else {
            limb_radius(block) * (0.9 + 0.2 * rand(face_key(d), 4))
        };
        r.min((0.5 - MARGIN) / (1.0 + WOBBLE))
    };
    let axis = |i: usize| i / 2;
    let sides = if trunk { TRUNK_SIDES } else { LIMB_SIDES };
    let piece = |a: Vec3, b: Vec3, ra: f32, rb: f32, axis_a, axis_b, key_a, key_b, cap_b| Piece {
        a,
        b,
        ra,
        rb,
        axis_a,
        axis_b,
        key_a,
        key_b,
        cap_b,
        sides,
    };
    let mut links: Vec<usize> = (0..6).filter(|&i| connected[i] || (i == 3 && rooted)).collect();
    let mut pieces = Vec::new();
    let mut knot = None;
    let center_r = |r: f32| r.min(room);
    match links.as_slice() {
        // Straight through: one smooth tube with a ring at the center.
        [i, j] if i / 2 == j / 2 => {
            let (ri, rj) = (face_radius(*i), face_radius(*j));
            let rc = center_r((ri + rj) * 0.5);
            pieces.push(piece(face_center(DIRS[*i]), center, ri, rc, axis(*i), axis(*i), face_key(DIRS[*i]), center_key, false));
            pieces.push(piece(center, face_center(DIRS[*j]), rc, rj, axis(*i), axis(*i), center_key, face_key(DIRS[*j]), false));
        }
        // A single link (tip or fresh cut) or none: continue straight across
        // the cell, tapering, and close the far end with end grain.
        [] | [_] => {
            let i = links.first().copied().unwrap_or(3);
            let far = i ^ 1;
            let ri = if links.is_empty() { center_r(base * 0.8) } else { face_radius(i) };
            let tip = if trunk { (ri * 0.8).min(face_radius_cap(trunk_at((y + 1) as f32))) } else { ri * 0.7 };
            let rc = center_r((ri + tip) * 0.5);
            pieces.push(piece(face_center(DIRS[i]), center, ri, rc, axis(i), axis(i), face_key(DIRS[i]), center_key, false));
            pieces.push(piece(center, face_center(DIRS[far]), rc, tip, axis(i), axis(i), center_key, face_key(DIRS[far]), true));
            if links.is_empty() {
                // An isolated piece: close the near end too.
                links.push(i);
                pieces[0].cap_b = false;
                let flipped = piece(center, face_center(DIRS[i]), rc, ri, axis(i), axis(i), center_key, face_key(DIRS[i]), true);
                pieces[0] = flipped;
            }
        }
        // A trunk with limbs leaving it: the trunk stays straight and each
        // limb starts low inside it, rising out through a collar.
        _ if trunk && connected[2] && (connected[3] || rooted) => {
            let (rd, ru) = (face_radius(3), face_radius(2));
            let rc = center_r((rd + ru) * 0.5);
            pieces.push(piece(face_center(DIRS[3]), center, rd, rc, 1, 1, face_key(DIRS[3]), center_key, false));
            pieces.push(piece(center, face_center(DIRS[2]), rc, ru, 1, 1, center_key, face_key(DIRS[2]), false));
            for &i in links.iter().filter(|&&i| axis(i) != 1) {
                let start = center - Vec3::Y * 0.28;
                let r = face_radius(i);
                pieces.push(piece(start, face_center(DIRS[i]), r * 1.1, r, axis(i), axis(i), center_key, face_key(DIRS[i]), false));
            }
        }
        // Elbows: one angled chord between the two face rings, so a stepped
        // limb reads as a bending bough rather than a square hook.
        [i, j] => {
            let (ri, rj) = (face_radius(*i), face_radius(*j));
            pieces.push(piece(face_center(DIRS[*i]), face_center(DIRS[*j]), ri, rj, axis(*i), axis(*j), face_key(DIRS[*i]), face_key(DIRS[*j]), false));
        }
        // Forks: pieces meet in a rounded knot at the center.
        _ => {
            let rk = center_r(links.iter().map(|&i| face_radius(i)).fold(0.0, f32::max));
            for &i in &links {
                let r = face_radius(i);
                pieces.push(piece(face_center(DIRS[i]), center, r, rk, axis(i), axis(i), face_key(DIRS[i]), center_key, false));
            }
            knot = Some((center, rk));
        }
    }
    Some(TreeCell { pieces, knot })
}

fn face_radius_cap(r: f32) -> f32 {
    r.min((0.5 - MARGIN) / (1.0 + WOBBLE))
}

/// Ring vertex `k` of `sides` around `axis` at `center`: a slightly irregular
/// cross-section keyed by `key`, identical for both cells sharing the ring.
pub fn ring_point(center: Vec3, axis: usize, radius: f32, key: [i32; 3], k: usize, sides: usize) -> (Vec3, Vec3) {
    let angle = k as f32 / sides as f32 * std::f32::consts::TAU;
    let wobble = 1.0 + WOBBLE * (rand(key, 10 + (k % sides) as u32) * 2.0 - 1.0);
    let (u, v) = match axis {
        0 => (Vec3::Z, Vec3::Y),
        1 => (Vec3::X, Vec3::Z),
        _ => (Vec3::X, Vec3::Y),
    };
    let out = u * angle.cos() + v * angle.sin();
    (center + out * radius * wobble, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::chunk::Chunk;

    /// Generated (not edited) blocks: chunk data written directly.
    fn world(cells: &[((i32, i32, i32), BlockType)]) -> World {
        let mut world = World::new(9);
        let mut chunk = Chunk::new(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_local(x, 0, z, BlockType::Grass);
            }
        }
        for &((x, y, z), b) in cells {
            chunk.set_local(x, y, z, b);
        }
        world.chunks.insert((0, 0), chunk);
        world.terrain_shaping = true;
        world
    }

    fn trunk(x: i32, z: i32, height: i32) -> Vec<((i32, i32, i32), BlockType)> {
        (1..=height).map(|y| ((x, y, z), BlockType::OakWood)).collect()
    }

    #[test]
    fn trunks_taper_flare_and_stay_inside_their_cells() {
        let w = world(&trunk(5, 5, 6));
        let radius_at = |y: i32| tree_cell(&w, 5, y, 5).unwrap().pieces[0].ra;
        assert!(radius_at(1) > radius_at(2), "flared base");
        assert!(radius_at(2) > radius_at(5), "tapers upward");
        for y in 1..=6 {
            let cell = tree_cell(&w, 5, y, 5).unwrap();
            for p in &cell.pieces {
                for (end, r) in [(p.a, p.ra), (p.b, p.rb)] {
                    for k in 0..p.sides {
                        let (q, _) = ring_point(end, p.axis_a, r, [0; 3], k, p.sides);
                        let local = q - Vec3::new(5.0, y as f32, 5.0);
                        assert!(local.min_element() >= -1e-4 && local.max_element() <= 1.0 + 1e-4, "{local}");
                    }
                }
            }
        }
        // The top is a fresh end: capped with end grain.
        assert!(tree_cell(&w, 5, 6, 5).unwrap().pieces.iter().any(|p| p.cap_b));
        assert!(!tree_cell(&w, 5, 3, 5).unwrap().pieces.iter().any(|p| p.cap_b));
    }

    #[test]
    fn shared_rings_match_between_neighbors() {
        let w = world(&trunk(5, 5, 4));
        let lower = tree_cell(&w, 5, 2, 5).unwrap();
        let upper = tree_cell(&w, 5, 3, 5).unwrap();
        // The ring on the shared face (y = 3) is the same point, radius and key.
        let ring_on_face = |cell: &TreeCell| {
            cell.pieces.iter().flat_map(|p| [(p.a, p.ra, p.key_a), (p.b, p.rb, p.key_b)])
                .find(|(c, _, _)| (c.y - 3.0).abs() < 1e-6).unwrap()
        };
        assert_eq!(ring_on_face(&lower), ring_on_face(&upper));
    }

    #[test]
    fn limbs_fork_from_the_trunk_and_cuts_get_caps() {
        let mut cells = trunk(5, 5, 6);
        cells.push(((6, 4, 5), BlockType::OakBranch));
        cells.push(((7, 4, 5), BlockType::OakBranch));
        let mut w = world(&cells);
        let fork = tree_cell(&w, 5, 4, 5).unwrap();
        assert_eq!(fork.pieces.len(), 3, "trunk down, trunk up, limb");
        let limb = fork.pieces[2];
        assert!(limb.a.y < limb.b.y, "the limb rises out of the trunk");
        assert!(limb.ra > limb_radius(BlockType::OakWood), "collar");
        // The limb's far cell is a tip with an end cap.
        assert!(tree_cell(&w, 7, 4, 5).unwrap().pieces.iter().any(|p| p.cap_b));
        // Removing the limb leaves a straight trunk; removing the trunk above
        // the fork leaves a capped cut.
        w.set_block(6, 4, 5, BlockType::Air);
        assert_eq!(tree_cell(&w, 5, 4, 5).unwrap().pieces.len(), 2);
        w.set_block(5, 5, 5, BlockType::Air);
        assert!(tree_cell(&w, 5, 4, 5).unwrap().pieces.iter().any(|p| p.cap_b));
    }

    #[test]
    fn placed_wood_stays_a_cube_even_against_a_tree() {
        let mut w = world(&trunk(5, 5, 4));
        w.set_block(6, 2, 5, BlockType::OakWood); // a player's post beside the trunk
        assert!(tree_cell(&w, 6, 2, 5).is_none());
        assert!(tree_cell(&w, 5, 2, 5).is_some());
        // The living trunk does not connect to the construction.
        assert_eq!(tree_cell(&w, 5, 2, 5).unwrap().pieces.len(), 2);
        w.terrain_shaping = false;
        assert!(tree_cell(&w, 5, 2, 5).is_none());
    }

    #[test]
    fn picking_and_collision_follow_the_round_trunk() {
        let w = world(&trunk(5, 5, 6));
        let cell = tree_cell(&w, 5, 4, 5).unwrap();
        assert!(cell.contains(Vec3::new(5.5, 4.5, 5.5)));
        assert!(!cell.contains(Vec3::new(5.05, 4.5, 5.05)), "the cube's corner is empty");
        let hit = crate::raycast::raycast(&w, Vec3::new(5.03, 4.5, 3.0), Vec3::Z, 4.0);
        assert!(hit.is_none() || hit.unwrap().target != (5, 4, 5), "a ray past the trunk's side misses it");
        let hit = crate::raycast::raycast(&w, Vec3::new(5.5, 4.5, 3.0), Vec3::Z, 4.0).unwrap();
        assert_eq!(hit.target, (5, 4, 5));
    }
}
