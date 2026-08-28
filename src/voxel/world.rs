use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::block::BlockType;
use super::chunk::{world_to_chunk, world_to_local, Chunk, CHUNK_X, CHUNK_Y, CHUNK_Z};
use super::noise::{block_rand, block_rand_range, column_rand, fbm};

pub const SEA_LEVEL: i32 = 18;

/// 6-connected neighbor offsets, used by `World::flood_from`.
const NEIGHBOR_OFFSETS: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

/// Hard cap on how many blocks a single dig can flood in one go. Not real
/// fluid dynamics -- just enough that tunneling into a lake fills the hole
/// you made instead of leaving it as a dry pocket, without a single unlucky
/// break being able to drain an entire lake or hang the game on a huge cave.
const MAX_FLOOD_BLOCKS: usize = 48;

/// Bottom-most layers of every chunk are unbreakable bedrock (hardness 0),
/// regardless of terrain height, so the world always has a floor.
const BEDROCK_DEPTH: i32 = 2;

/// Wood/leaves pairs for the four tree species in `textures/blocks.csv`;
/// `place_tree` picks one per tree via `column_rand`.
const TREE_SPECIES: [(BlockType, BlockType); 4] = [
    (BlockType::OakWood, BlockType::OakLeaves),
    (BlockType::SpruceWood, BlockType::SpruceLeaves),
    (BlockType::BirchWood, BlockType::BirchLeaves),
    (BlockType::CherryWood, BlockType::CherryLeaves),
];

/// One kind of underground deposit grown by `World::scatter_veins`: a small
/// cluster of `block` replacing `Stone`, seeded by a per-chunk-attempt roll.
struct VeinConfig {
    block: BlockType,
    salt: u32,
    attempts_per_chunk: u32,
    spawn_chance: f32,
    min_y: i32,
    max_y: i32,
    min_size: i32,
    max_size: i32,
}

/// Ore/stone-variant veins, roughly ordered shallow-and-common to
/// deep-and-rare like the CSV's hardness column suggests. Basalt and
/// cobblestone aren't ores but generate the same way -- small underground
/// pockets replacing plain stone.
const VEINS: [VeinConfig; 6] = [
    VeinConfig {
        block: BlockType::CopperOre,
        salt: 0xC099_7E01,
        attempts_per_chunk: 6,
        spawn_chance: 0.55,
        min_y: 4,
        max_y: 34,
        min_size: 3,
        max_size: 6,
    },
    VeinConfig {
        block: BlockType::Cobblestone,
        salt: 0xC0BB_1E02,
        attempts_per_chunk: 5,
        spawn_chance: 0.4,
        min_y: 2,
        max_y: 30,
        min_size: 3,
        max_size: 6,
    },
    VeinConfig {
        block: BlockType::GoldOre,
        salt: 0x6014_D003,
        attempts_per_chunk: 3,
        spawn_chance: 0.4,
        min_y: 3,
        max_y: 22,
        min_size: 2,
        max_size: 4,
    },
    VeinConfig {
        block: BlockType::Basalt,
        salt: 0xBA5A_1704,
        attempts_per_chunk: 4,
        spawn_chance: 0.35,
        min_y: 2,
        max_y: 20,
        min_size: 4,
        max_size: 7,
    },
    VeinConfig {
        block: BlockType::EmeraldOre,
        salt: 0xE3E7_A105,
        attempts_per_chunk: 2,
        spawn_chance: 0.18,
        min_y: 3,
        max_y: 16,
        min_size: 2,
        max_size: 3,
    },
    VeinConfig {
        block: BlockType::DiamondOre,
        salt: 0xD1A5_0906,
        attempts_per_chunk: 2,
        spawn_chance: 0.22,
        min_y: 2,
        max_y: 10,
        min_size: 2,
        max_size: 3,
    },
];

pub struct World {
    pub seed: u32,
    pub chunks: HashMap<(i32, i32), Chunk>,
    /// Blocks that differ from the procedurally generated terrain. Kept
    /// separately so the whole world doesn't need to be serialized.
    pub edits: HashMap<(i32, i32, i32), BlockType>,
    /// Positions of every RedStone block currently placed, maintained
    /// incrementally by `set_block` so the passive "heal nearby creatures"
    /// effect doesn't need to rescan the world every tick.
    pub redstone_positions: Vec<(i32, i32, i32)>,
}

impl World {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            chunks: HashMap::new(),
            edits: HashMap::new(),
            redstone_positions: Vec::new(),
        }
    }

    /// Rebuilds `redstone_positions` from `edits` -- call once after bulk
    /// loading edits (e.g. from a save file), since those bypass
    /// `set_block`'s incremental tracking.
    pub fn rebuild_redstone_positions(&mut self) {
        self.redstone_positions = self
            .edits
            .iter()
            .filter(|(_, &b)| b == BlockType::RedStone)
            .map(|(&pos, _)| pos)
            .collect();
    }

    pub fn terrain_height(&self, wx: i32, wz: i32) -> i32 {
        // `fbm` returns [0, 1], not [-1, 1] -- recenter both octaves around 0
        // so terrain actually varies both above *and* below the mean instead
        // of only ever adding height. Without this, height never dropped
        // below 14 and the world generated no water at all despite the
        // sea-level logic below.
        let base = fbm(wx as f32 * 0.01, wz as f32 * 0.01, self.seed, 4, 2.0, 0.5) * 2.0 - 1.0;
        let hills = fbm(
            wx as f32 * 0.04,
            wz as f32 * 0.04,
            self.seed ^ 0x51ed,
            3,
            2.0,
            0.5,
        ) * 2.0
            - 1.0;
        // Mean height sits a bit above SEA_LEVEL (18) so most land stays
        // dry, but low-lying dips (large-scale `base` swings) fall below it
        // and fill with water, forming lakes/ponds/coastline.
        let h = 24.0 + base * 14.0 + hills * 5.0;
        h.clamp(2.0, (CHUNK_Y - 6) as f32) as i32
    }

    fn generate_chunk(&self, cx: i32, cz: i32) -> Chunk {
        let mut chunk = Chunk::new(cx, cz);
        let (ox, oz) = chunk.world_origin();

        for lx in 0..CHUNK_X {
            for lz in 0..CHUNK_Z {
                let wx = ox + lx;
                let wz = oz + lz;
                let height = self.terrain_height(wx, wz);

                for ly in 0..CHUNK_Y {
                    let block = if ly < BEDROCK_DEPTH {
                        BlockType::Bedrock
                    } else if ly > height {
                        if ly <= SEA_LEVEL {
                            BlockType::Water
                        } else {
                            BlockType::Air
                        }
                    } else if ly == height {
                        if height <= SEA_LEVEL + 1 {
                            BlockType::Sand
                        } else {
                            BlockType::Grass
                        }
                    } else if ly > height - 4 {
                        BlockType::Soil
                    } else {
                        BlockType::Stone
                    };
                    chunk.set_local(lx, ly, lz, block);
                }

                // Simple tree scattering, away from the shoreline. Species
                // is picked per-tree so all four wood types show up.
                let on_dry_land = height > SEA_LEVEL + 2;
                let mut placed_topper = false;
                if on_dry_land && column_rand(wx, wz, self.seed, 0xA11CE) < 0.006 {
                    let species_roll = column_rand(wx, wz, self.seed, 0x5FEC1E5);
                    let idx = ((species_roll * TREE_SPECIES.len() as f32) as usize)
                        .min(TREE_SPECIES.len() - 1);
                    let (wood, leaves) = TREE_SPECIES[idx];
                    self.place_tree(&mut chunk, lx, height, lz, wx, wz, wood, leaves);
                    placed_topper = true;
                }

                // Rare pumpkin patches on open grass, never on a tree's own
                // trunk cell.
                if on_dry_land
                    && !placed_topper
                    && column_rand(wx, wz, self.seed, 0x9A9E27) < 0.0025
                {
                    chunk.set_local(lx, height + 1, lz, BlockType::Pumpkin);
                    placed_topper = true;
                }

                // Short grass tufts -- a common, purely decorative "only on
                // top" cover, so it should never overwrite a tree/pumpkin.
                if on_dry_land
                    && !placed_topper
                    && column_rand(wx, wz, self.seed, 0x5407A55) < 0.06
                {
                    chunk.set_local(lx, height + 1, lz, BlockType::ShortGrass);
                }

                // Rare crystal outcrops on dry land, for the "carrying a
                // crystal" rule condition. Sits on top of the surface block.
                if on_dry_land && column_rand(wx, wz, self.seed, 0xC4157A1) < 0.0015 {
                    chunk.set_local(lx, height + 1, lz, BlockType::Crystal);
                }
            }
        }

        self.scatter_veins(&mut chunk, cx, cz);
        self.scatter_brick_ruins(&mut chunk, cx, cz);

        chunk.dirty = true;
        chunk
    }

    #[allow(clippy::too_many_arguments)]
    fn place_tree(
        &self,
        chunk: &mut Chunk,
        lx: i32,
        ground_y: i32,
        lz: i32,
        wx: i32,
        wz: i32,
        wood: BlockType,
        leaves: BlockType,
    ) {
        let trunk_height = 4 + (column_rand(wx, wz, self.seed, 0xBEEF) * 3.0) as i32;
        for i in 1..=trunk_height {
            chunk.set_local(lx, ground_y + i, lz, wood);
        }
        let top = ground_y + trunk_height;

        // Canopy layers relative to the treetop, bottom to top, each with
        // its own radius -- tapered so the crown reads as round-ish rather
        // than a flat box, with a narrow fringe above and below the full
        // middle spread.
        const CANOPY_LAYERS: [(i32, i32); 4] = [(-1, 1), (0, 2), (1, 2), (2, 1)];
        // Per-tree roll for how full the outer ring of each layer is, so
        // canopies vary a little instead of all being an identical cutout.
        let fullness = column_rand(wx, wz, self.seed, 0x7EAF01);

        for &(dy, radius) in CANOPY_LAYERS.iter() {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let dist = dx.abs().max(dz.abs());
                    if dist > radius {
                        continue;
                    }
                    // Corners of every layer's square are cut, including
                    // the radius-1 top/bottom fringes -- a diamond/octagon
                    // cross-section reads far less like a rectangular box
                    // than a full square.
                    if dx.abs() == radius && dz.abs() == radius {
                        continue;
                    }
                    // Per-cell jitter on the outer ring, so the edge isn't
                    // a perfectly even circle either.
                    if dist == radius {
                        let jitter =
                            column_rand(wx + dx, wz + dz, self.seed, 0x1EAF ^ (dy as u32));
                        if jitter > 0.55 + fullness * 0.3 {
                            continue;
                        }
                    }
                    let bx = lx + dx;
                    let by = top + dy;
                    let bz = lz + dz;
                    if Chunk::in_bounds(bx, by, bz) && chunk.get_local(bx, by, bz) == BlockType::Air
                    {
                        chunk.set_local(bx, by, bz, leaves);
                    }
                }
            }
        }
    }

    /// Grows small underground deposits (ores, basalt, cobblestone) by
    /// replacing `Stone` cells with `VEINS` entries. Each vein starts from a
    /// deterministic per-chunk-attempt roll and grows via a short random
    /// walk, so results are reproducible from `(seed, cx, cz)` alone like
    /// the rest of chunk generation.
    fn scatter_veins(&self, chunk: &mut Chunk, cx: i32, cz: i32) {
        for vein in VEINS.iter() {
            for attempt in 0..vein.attempts_per_chunk as i32 {
                if block_rand(cx, attempt, cz, self.seed, vein.salt) >= vein.spawn_chance {
                    continue;
                }
                let lx = block_rand_range(cx, attempt, cz, self.seed, vein.salt ^ 0xA1, 0, CHUNK_X - 1);
                let ly = block_rand_range(
                    cx,
                    attempt,
                    cz,
                    self.seed,
                    vein.salt ^ 0xB2,
                    vein.min_y,
                    vein.max_y,
                );
                let lz = block_rand_range(cx, attempt, cz, self.seed, vein.salt ^ 0xC3, 0, CHUNK_Z - 1);
                let size =
                    block_rand_range(cx, attempt, cz, self.seed, vein.salt ^ 0xD4, vein.min_size, vein.max_size);

                let (mut x, mut y, mut z) = (lx, ly, lz);
                for step in 0..size {
                    if Chunk::in_bounds(x, y, z) && chunk.get_local(x, y, z) == BlockType::Stone {
                        chunk.set_local(x, y, z, vein.block);
                    }
                    let dir = block_rand_range(x, y, z, self.seed, vein.salt ^ (step as u32), 0, 5) as usize;
                    let (dx, dy, dz) = NEIGHBOR_OFFSETS[dir];
                    x += dx;
                    y += dy;
                    z += dz;
                }
            }
        }
    }

    /// Extremely rare single brick blocks buried in stone, as if leftover
    /// ruins -- "single items" per the CSV, so unlike `scatter_veins` this
    /// never clusters: exactly one `Stone` cell becomes `Bricks` per roll.
    fn scatter_brick_ruins(&self, chunk: &mut Chunk, cx: i32, cz: i32) {
        const ATTEMPTS: i32 = 2;
        const SPAWN_CHANCE: f32 = 0.05;
        const SALT: u32 = 0xB61C_5301;
        const MIN_Y: i32 = 3;
        const MAX_Y: i32 = 25;

        for attempt in 0..ATTEMPTS {
            if block_rand(cx, attempt, cz, self.seed, SALT) >= SPAWN_CHANCE {
                continue;
            }
            let lx = block_rand_range(cx, attempt, cz, self.seed, SALT ^ 0xA1, 0, CHUNK_X - 1);
            let ly = block_rand_range(cx, attempt, cz, self.seed, SALT ^ 0xB2, MIN_Y, MAX_Y);
            let lz = block_rand_range(cx, attempt, cz, self.seed, SALT ^ 0xC3, 0, CHUNK_Z - 1);
            if chunk.get_local(lx, ly, lz) == BlockType::Stone {
                chunk.set_local(lx, ly, lz, BlockType::Bricks);
            }
        }
    }

    pub fn ensure_chunk_loaded(&mut self, cx: i32, cz: i32) {
        if self.chunks.contains_key(&(cx, cz)) {
            return;
        }
        let mut chunk = self.generate_chunk(cx, cz);
        self.apply_edits_to_chunk(&mut chunk);
        self.chunks.insert((cx, cz), chunk);
    }

    fn apply_edits_to_chunk(&self, chunk: &mut Chunk) {
        if self.edits.is_empty() {
            return;
        }
        let (ox, oz) = chunk.world_origin();
        for lx in 0..CHUNK_X {
            for lz in 0..CHUNK_Z {
                for ly in 0..CHUNK_Y {
                    if let Some(&b) = self.edits.get(&(ox + lx, ly, oz + lz)) {
                        chunk.set_local(lx, ly, lz, b);
                    }
                }
            }
        }
    }

    pub fn unload_chunk(&mut self, cx: i32, cz: i32) {
        self.chunks.remove(&(cx, cz));
    }

    pub fn get_block(&self, wx: i32, wy: i32, wz: i32) -> BlockType {
        if wy < 0 || wy >= CHUNK_Y {
            return BlockType::Air;
        }
        let (cx, cz) = world_to_chunk(wx, wz);
        let (lx, lz) = world_to_local(wx, wz);
        match self.chunks.get(&(cx, cz)) {
            Some(chunk) => chunk.get_local(lx, wy, lz),
            None => BlockType::Air,
        }
    }

    pub fn set_block(&mut self, wx: i32, wy: i32, wz: i32, block: BlockType) {
        if wy < 0 || wy >= CHUNK_Y {
            return;
        }
        let was = self.get_block(wx, wy, wz);
        if was == BlockType::RedStone && block != BlockType::RedStone {
            self.redstone_positions.retain(|&p| p != (wx, wy, wz));
        } else if block == BlockType::RedStone && was != BlockType::RedStone {
            self.redstone_positions.push((wx, wy, wz));
        }
        self.edits.insert((wx, wy, wz), block);
        let (cx, cz) = world_to_chunk(wx, wz);
        let (lx, lz) = world_to_local(wx, wz);
        if let Some(chunk) = self.chunks.get_mut(&(cx, cz)) {
            chunk.set_local(lx, wy, lz, block);
        }
        // Mark neighboring chunks dirty too if we edited on a boundary, so
        // their meshes drop/regain the face against this block.
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            if lx + dx < 0 || lx + dx >= CHUNK_X || lz + dz < 0 || lz + dz >= CHUNK_Z {
                if let Some(neighbor) = self.chunks.get_mut(&(cx + dx, cz + dz)) {
                    neighbor.dirty = true;
                }
            }
        }
    }

    pub fn is_solid(&self, wx: i32, wy: i32, wz: i32) -> bool {
        self.get_block(wx, wy, wz).is_solid()
    }

    /// Call after a block at `start` has just been broken to `Air`. Returns
    /// every additional position (bounded by `MAX_FLOOD_BLOCKS`) that should
    /// become `Water` because it's connected to an existing water source --
    /// the caller is responsible for actually setting those blocks (so it
    /// can replicate each one the same way as any other edit).
    ///
    /// This is a simple bounded flood fill, not real fluid simulation: it
    /// only spreads sideways and downward (never climbing back up through an
    /// open shaft) from a cell that directly touches water in any direction
    /// -- water sitting right above `start` falls in; a newly-dug tunnel
    /// that reaches sideways into a lake wall gets flooded from that point
    /// on.
    pub fn flood_from(&self, start: (i32, i32, i32)) -> Vec<(i32, i32, i32)> {
        if self.get_block(start.0, start.1, start.2) != BlockType::Air {
            return Vec::new();
        }
        let touches_water = NEIGHBOR_OFFSETS.iter().any(|&(dx, dy, dz)| {
            self.get_block(start.0 + dx, start.1 + dy, start.2 + dz) == BlockType::Water
        });
        if !touches_water {
            return Vec::new();
        }

        let mut visited: HashSet<(i32, i32, i32)> = HashSet::new();
        let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
        let mut result = Vec::new();
        visited.insert(start);
        queue.push_back(start);

        while let Some(pos) = queue.pop_front() {
            if result.len() >= MAX_FLOOD_BLOCKS {
                break;
            }
            result.push(pos);
            for &(dx, dy, dz) in NEIGHBOR_OFFSETS.iter() {
                if dy > 0 {
                    continue; // water doesn't climb back up an open shaft
                }
                let next = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
                if visited.insert(next) && self.get_block(next.0, next.1, next.2) == BlockType::Air
                {
                    queue.push_back(next);
                }
            }
        }
        result
    }
}

#[derive(Serialize, Deserialize)]
pub struct WorldSave {
    pub seed: u32,
    pub player_pos: [f32; 3],
    pub player_yaw: f32,
    pub player_pitch: f32,
    pub time_of_day: f32,
    pub edits: Vec<((i32, i32, i32), BlockType)>,
    pub modules: Vec<crate::scripting::ModuleSaveEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flood_from_fills_an_isolated_air_pocket_touching_water() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        // Wall off (6,10,5) on every side except toward the water, so the
        // flood has nowhere else to spread to -- isolates exactly the case
        // under test (one pocket, one water source, nothing beyond it).
        for &(dx, dy, dz) in &[(1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            chunk.set_local(6 + dx, 10 + dy, 5 + dz, BlockType::Stone);
        }
        chunk.set_local(5, 10, 5, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        let result = world.flood_from((6, 10, 5));
        assert_eq!(result, vec![(6, 10, 5)]);
    }

    #[test]
    fn flood_from_does_nothing_without_an_adjacent_water_source() {
        let world = World::new(1);
        // No chunk loaded at all -- get_block reads Air everywhere, so the
        // start cell passes the "is Air" check but never finds water.
        let result = world.flood_from((6, 10, 5));
        assert!(result.is_empty());
    }

    #[test]
    fn flood_from_refuses_to_start_on_a_non_air_cell() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::Water);
        chunk.set_local(6, 10, 5, BlockType::Stone); // not actually broken
        world.chunks.insert((0, 0), chunk);

        assert!(world.flood_from((6, 10, 5)).is_empty());
    }

    #[test]
    fn flood_from_lets_water_fall_from_directly_above() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        // Isolate the pocket again, this time leaving only "up" open --
        // that's where the water sits.
        for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            chunk.set_local(5 + dx, 10 + dy, 5 + dz, BlockType::Stone);
        }
        chunk.set_local(5, 11, 5, BlockType::Water); // sits right above the dig
        world.chunks.insert((0, 0), chunk);

        let result = world.flood_from((5, 10, 5));
        assert_eq!(result, vec![(5, 10, 5)]);
    }

    #[test]
    fn flood_from_spreads_through_a_connected_air_tunnel() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(0, 10, 0, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        // (1,10,0) touches the water directly; (2,10,0) is only reachable
        // by continuing sideways through the already-flooded cell.
        let result = world.flood_from((1, 10, 0));
        assert!(result.contains(&(1, 10, 0)));
        assert!(
            result.contains(&(2, 10, 0)),
            "flood should spread sideways through connected air: {result:?}"
        );
    }

    #[test]
    fn flood_from_does_not_climb_upward_through_an_open_shaft() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        // (6,10,5) touches the water sideways; (6,11,5) is open air directly
        // above it -- water shouldn't defy gravity to climb up into it.
        let result = world.flood_from((6, 10, 5));
        assert!(result.contains(&(6, 10, 5)));
        assert!(
            !result.contains(&(6, 11, 5)),
            "flood shouldn't climb upward into an open shaft: {result:?}"
        );
    }

    #[test]
    fn flood_from_caps_at_max_flood_blocks() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        // The rest of the chunk is Air by default -- a huge connected
        // pocket, far more than MAX_FLOOD_BLOCKS, all reachable from one
        // water source.
        chunk.set_local(0, 10, 0, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        let result = world.flood_from((1, 10, 0));
        assert_eq!(
            result.len(),
            MAX_FLOOD_BLOCKS,
            "flood should stop at the cap instead of filling the whole cavity"
        );
    }
}
