//! Versioned tree pass. Geometry uses existing species blocks and coordinate RNG.
use super::{
    block::BlockType,
    chunk::{Chunk, CHUNK_X, CHUNK_Z},
    noise::{block_rand_range, column_rand},
    world::{World, SEA_LEVEL},
};
use std::collections::BTreeMap;

type Pos = (i32, i32, i32);
const REACH: i32 = 6;
const CELL: i32 = 8;
const MAX_BLOCKS: usize = 640;
const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

#[derive(Clone, Copy)]
struct BranchConfig {
    height: (i32, i32),
    min_height: i32,
    count: (i32, i32),
    // Origin fractions in tenths of trunk height, with four-block clearance.
    origin: (i32, i32),
    length: (i32, i32),
    rise_percent: i32,
    descend_percent: i32,
    base_thickness: i32,
    turn_percent: i32,
    leaf_radius: i32,
    leaf_lift: i32,
}

const CONFIGS: [(BlockType, BlockType, BranchConfig); 4] = [
    (
        BlockType::OakWood,
        BlockType::OakLeaves,
        BranchConfig {
            height: (7, 9),
            min_height: 6,
            count: (3, 5),
            origin: (5, 8),
            length: (2, 4),
            rise_percent: 45,
            descend_percent: 10,
            base_thickness: 2,
            turn_percent: 45,
            leaf_radius: 2,
            leaf_lift: 1,
        },
    ),
    (
        BlockType::SpruceWood,
        BlockType::SpruceLeaves,
        BranchConfig {
            height: (8, 10),
            min_height: 6,
            count: (3, 5),
            origin: (5, 8),
            length: (2, 3),
            rise_percent: 65,
            descend_percent: 0,
            base_thickness: 1,
            turn_percent: 20,
            leaf_radius: 2,
            leaf_lift: 1,
        },
    ),
    (
        BlockType::BirchWood,
        BlockType::BirchLeaves,
        BranchConfig {
            height: (6, 8),
            min_height: 6,
            count: (1, 3),
            origin: (6, 8),
            length: (2, 3),
            rise_percent: 65,
            descend_percent: 0,
            base_thickness: 1,
            turn_percent: 30,
            leaf_radius: 2,
            leaf_lift: 1,
        },
    ),
    (
        BlockType::CherryWood,
        BlockType::CherryLeaves,
        BranchConfig {
            height: (6, 8),
            min_height: 6,
            count: (3, 5),
            origin: (6, 9),
            length: (2, 4),
            rise_percent: 25,
            descend_percent: 15,
            base_thickness: 1,
            turn_percent: 55,
            leaf_radius: 2,
            leaf_lift: 1,
        },
    ),
];

fn cluster(blocks: &mut BTreeMap<Pos, BlockType>, p: Pos, radius: i32, leaves: BlockType) {
    for dx in -radius..=radius {
        for dy in -1..=1 {
            for dz in -radius..=radius {
                if dx * dx + dz * dz + 2 * dy * dy <= radius * radius + 1 {
                    blocks
                        .entry((p.0 + dx, p.1 + dy, p.2 + dz))
                        .or_insert(leaves);
                }
            }
        }
    }
}

fn geometry(seed: u32, wx: i32, wz: i32, species: usize) -> BTreeMap<Pos, BlockType> {
    let (wood, leaves, c) = CONFIGS[species];
    let roll = |n, salt, lo, hi| block_rand_range(wx, n, wz, seed, salt, lo, hi);
    let h = roll(0, 0xBEEF, c.height.0, c.height.1);
    // Narrow, balanced, asymmetric, broad, dominant-side, high crown.
    let pattern = roll(0, 0xB001, 0, 5);
    let rotation = roll(0, 0xB002, 0, 3);
    let mut blocks = BTreeMap::new();
    for y in 1..=h {
        blocks.insert((0, y, 0), wood);
    }
    cluster(
        &mut blocks,
        (0, h, 0),
        if pattern == 0 { 1 } else { 2 },
        leaves,
    );
    cluster(&mut blocks, (0, h + 1, 0), 1, leaves);
    if h < c.min_height {
        return blocks;
    }
    let count = roll(0, 0xB003, c.count.0, c.count.1);
    let mut origins = std::collections::BTreeSet::new();
    for n in 0..count {
        let low = (h * c.origin.0 / 10).max(4);
        let high = (h * c.origin.1 / 10).max(low).min(h - 1);
        let preferred_y = if pattern == 5 {
            high
        } else {
            roll(n, 0xB004, low, high)
        };
        let preferred_direction = (rotation
            + if pattern == 2 || pattern == 4 {
                roll(n, 0xB005, 0, 1)
            } else {
                roll(n, 0xB005, 0, 3)
            })
            % 4;
        let levels = high - low + 1;
        let (y, direction) = (0..4 * levels)
            .map(|offset| {
                (
                    low + (preferred_y - low + offset).rem_euclid(levels),
                    (preferred_direction + offset / levels) % 4,
                )
            })
            .find(|origin| !origins.contains(origin))
            .expect("branch count fits the configured origin slots");
        origins.insert((y, direction));
        let mut length = roll(n, 0xB006, c.length.0, c.length.1);
        if pattern == 0 {
            length = 2;
        }
        if pattern == 3 || (pattern == 4 && n == 0) {
            length = c.length.1;
        }
        if pattern == 4 && n > 0 {
            length = c.length.0;
        }
        let slope = roll(n, 0xB007, 0, 99);
        let dy = if slope < c.rise_percent {
            1
        } else if slope < c.rise_percent + c.descend_percent && y > 4 {
            -1
        } else {
            0
        };
        let turn = roll(n, 0xB008, 0, 99) < c.turn_percent;
        let mut dir = direction;
        let mut p = (0, y, 0);
        // Overlapping upper clusters link each limb to the central crown,
        // leaving its underside and inner wood visible.
        for cy in y + 1..h {
            cluster(&mut blocks, (0, cy, 0), 1, leaves);
        }
        for step in 1..=length {
            if step == length && turn {
                dir = (dir + if roll(n, 0xB009, 0, 1) == 0 { 1 } else { 3 }) % 4;
            }
            let (dx, dz) = DIRS[dir as usize];
            p.0 += dx;
            p.2 += dz;
            blocks.insert(p, wood);
            if step == 1 && c.base_thickness > 1 && h >= 8 {
                blocks.insert((p.0, p.1 + 1, p.2), wood);
            }
            if step == length - 1 && dy != 0 {
                // Separate horizontal and vertical steps keep face connectivity.
                p.1 += dy;
                blocks.insert(p, wood);
            }
            cluster(
                &mut blocks,
                (p.0, p.1 + c.leaf_lift, p.2),
                if step == length { c.leaf_radius } else { 1 },
                leaves,
            );
        }
    }
    // Intersecting limbs can cut off a small fringe. Keep only the crown's
    // face-connected leaf mass; this bounded flood never grows extra geometry.
    let root = (0, h + 1, 0);
    let mut connected = std::collections::HashSet::from([root]);
    let mut pending = vec![root];
    while let Some((x, y, z)) = pending.pop() {
        for (dx, dy, dz) in [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            let p = (x + dx, y + dy, z + dz);
            if blocks.get(&p) == Some(&leaves) && connected.insert(p) {
                pending.push(p);
            }
        }
    }
    blocks.retain(|p, b| *b == wood || connected.contains(p));
    // Fixed loops bound geometry; never truncate a connected limb to meet a cap.
    debug_assert!(blocks.len() <= MAX_BLOCKS);
    blocks
}

impl World {
    pub(super) fn scatter_branched_trees(&self, chunk: &mut Chunk) {
        if self.generation.trees == 0 {
            return;
        }
        let (ox, oz) = (chunk.cx * CHUNK_X, chunk.cz * CHUNK_Z);
        // At most 25 candidate cells per chunk, including the six-block halo.
        // Owner-cell RNG and world-space ordering are independent of load order.
        for gx in (ox - REACH).div_euclid(CELL)..=(ox + CHUNK_X - 1 + REACH).div_euclid(CELL) {
            for gz in (oz - REACH).div_euclid(CELL)..=(oz + CHUNK_Z - 1 + REACH).div_euclid(CELL) {
                if column_rand(gx, gz, self.seed, 0xA11CE)
                    >= (0.384 * self.generation.trees as f32 / 100.).min(0.95)
                {
                    continue;
                }
                let wx = gx * CELL + block_rand_range(gx, 0, gz, self.seed, 0xBA01, 0, 7);
                let wz = gz * CELL + block_rand_range(gx, 0, gz, self.seed, 0xBA02, 0, 7);
                let ground = self.terrain_height(wx, wz);
                let water = if self.generation.shape == crate::worldgen::Shape::Mainland {
                    super::terrain::tributary(wx, wz, self.seed).map_or(SEA_LEVEL, |p| p.1)
                } else {
                    SEA_LEVEL
                };
                if ground <= water + 2
                    || self.generation.surface == crate::worldgen::Surface::Stone
                    || ground
                        >= if self.generation.surface == crate::worldgen::Surface::Natural {
                            super::terrain::ALPINE_LINE
                        } else {
                            super::chunk::TERRAIN_HEIGHT - 12
                        }
                {
                    continue;
                }
                let species = block_rand_range(wx, 0, wz, self.seed, 0x5FEC1E5, 0, 3) as usize;
                let blocks = geometry(self.seed, wx, wz, species);
                // Check the entire footprint even outside this chunk. Reject a tree
                // on steep terrain rather than clipping limbs or blocking paths.
                let mut heights = BTreeMap::new();
                let fits = blocks.keys().all(|&(x, y, z)| {
                    let terrain = *heights
                        .entry((x, z))
                        .or_insert_with(|| self.terrain_height(wx + x, wz + z));
                    (x == 0 && z == 0) || ground + y >= terrain + 3
                });
                if !fits {
                    continue;
                }
                for ((x, y, z), block) in blocks {
                    let block = if self.generation.tree_version >= 2 && (x != 0 || z != 0) {
                        block.branch()
                    } else { block };
                    let (x, y, z) = (wx + x - ox, ground + y, wz + z - oz);
                    if !Chunk::in_bounds(x, y, z) {
                        continue;
                    }
                    let old = chunk.get_local(x, y, z);
                    if old == BlockType::Air
                        || old == BlockType::ShortGrass
                        || (block.is_wood()
                            && matches!(old, BlockType::Pumpkin | BlockType::Crystal))
                        || (block.is_wood() && CONFIGS.iter().any(|s| old == s.1))
                    {
                        chunk.set_local(x, y, z, block);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, VecDeque};

    fn connected(cells: &BTreeSet<Pos>, root: Pos) -> bool {
        let mut seen = BTreeSet::from([root]);
        let mut queue = VecDeque::from([root]);
        while let Some((x, y, z)) = queue.pop_front() {
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let p = (x + dx, y + dy, z + dz);
                if cells.contains(&p) && seen.insert(p) {
                    queue.push_back(p);
                }
            }
        }
        seen.len() == cells.len()
    }

    #[test]
    fn trees_are_seeded_connected_bounded_and_have_visible_species_wood() {
        for species in 0..CONFIGS.len() {
            let mut silhouettes = BTreeSet::new();
            for seed in 0..256 {
                let tree = geometry(seed, -17, 31, species);
                assert_eq!(tree, geometry(seed, -17, 31, species));
                assert!(tree.len() <= MAX_BLOCKS);
                let wood: BTreeSet<_> = tree
                    .iter()
                    .filter(|(_, b)| b.is_wood())
                    .map(|(&p, &b)| {
                        assert_eq!(b, CONFIGS[species].0);
                        p
                    })
                    .collect();
                assert!(
                    connected(&wood, (0, 1, 0)),
                    "wood species={species} seed={seed}"
                );
                assert!(
                    connected(&tree.keys().copied().collect(), (0, 1, 0)),
                    "canopy species={species} seed={seed}"
                );
                let leaves: BTreeSet<_> = tree
                    .iter()
                    .filter(|(_, b)| !b.is_wood())
                    .map(|(&p, _)| p)
                    .collect();
                assert!(
                    connected(&leaves, *leaves.first().unwrap()),
                    "leaf mass species={species} seed={seed}"
                );
                let limbs: Vec<_> = wood
                    .iter()
                    .filter(|p| p.0 != 0 || p.2 != 0)
                    .copied()
                    .collect();
                assert!(!limbs.is_empty());
                assert!(limbs.iter().all(|p| p.1 >= 4));
                assert!(
                    limbs
                        .iter()
                        .any(|p| !tree.contains_key(&(p.0, p.1 - 1, p.2))),
                    "visible underside"
                );
                assert!(tree
                    .keys()
                    .all(|p| p.0.abs() <= REACH && p.2.abs() <= REACH));
                silhouettes.insert(limbs);
            }
            assert!(silhouettes.len() > 32);
        }
    }

    #[test]
    fn trees_cross_chunk_edges_and_ignore_generation_order() {
        let mut world = World::new(42);
        world.generation.shape = crate::worldgen::Shape::Flat;
        world.generation.trees = 300;
        let coords: Vec<_> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        let build = |order: Vec<(i32, i32)>| {
            let mut result = BTreeMap::new();
            for (cx, cz) in order {
                let mut chunk = Chunk::new(cx, cz);
                world.scatter_branched_trees(&mut chunk);
                for x in 0..CHUNK_X {
                    for z in 0..CHUNK_Z {
                        for y in 0..chunk.stored_height() {
                            let b = chunk.get_local(x, y, z);
                            if b != BlockType::Air {
                                result.insert((cx * CHUNK_X + x, y, cz * CHUNK_Z + z), b);
                            }
                        }
                    }
                }
            }
            result
        };
        let forward = build(coords.clone());
        assert!(forward.values().any(|b| b.is_branch()));
        assert_eq!(forward, build(coords.into_iter().rev().collect()));
        assert!(forward.iter().any(|(&(x, y, z), b)| x == 15
            && b.is_wood()
            && forward.get(&(16, y, z)).is_some_and(|b| b.is_wood())));
        // Every interior wood voxel reaches a trunk through wood, including seams.
        let wood: BTreeSet<_> = forward
            .iter()
            .filter(|(_, b)| b.is_wood())
            .map(|(&p, _)| p)
            .collect();
        for &start in wood
            .iter()
            .filter(|p| (0..16).contains(&p.0) && (0..16).contains(&p.2))
        {
            let mut seen = BTreeSet::from([start]);
            let mut queue = VecDeque::from([start]);
            let mut rooted = false;
            while let Some((x, y, z)) = queue.pop_front() {
                if y == world.terrain_height(x, z) + 1 {
                    rooted = true;
                    break;
                }
                for (dx, dy, dz) in [
                    (1, 0, 0),
                    (-1, 0, 0),
                    (0, 1, 0),
                    (0, -1, 0),
                    (0, 0, 1),
                    (0, 0, -1),
                ] {
                    let p = (x + dx, y + dy, z + dz);
                    if wood.contains(&p) && seen.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
            assert!(rooted, "unrooted wood at {start:?}");
        }
    }
}
