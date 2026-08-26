use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::block::BlockType;
use super::chunk::{world_to_chunk, world_to_local, Chunk, CHUNK_X, CHUNK_Y, CHUNK_Z};
use super::noise::{column_rand, fbm};

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
                    let block = if ly > height {
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
                        BlockType::Dirt
                    } else {
                        BlockType::Stone
                    };
                    chunk.set_local(lx, ly, lz, block);
                }

                // Simple tree scattering, away from the shoreline.
                if height > SEA_LEVEL + 2 && column_rand(wx, wz, self.seed, 0xA11CE) < 0.006 {
                    self.place_tree(&mut chunk, lx, height, lz, wx, wz);
                }

                // Rare crystal outcrops on dry land, for the "carrying a
                // crystal" rule condition. Sits on top of the surface block.
                if height > SEA_LEVEL + 2 && column_rand(wx, wz, self.seed, 0xC4157A1) < 0.0015 {
                    chunk.set_local(lx, height + 1, lz, BlockType::Crystal);
                }
            }
        }

        chunk.dirty = true;
        chunk
    }

    fn place_tree(&self, chunk: &mut Chunk, lx: i32, ground_y: i32, lz: i32, wx: i32, wz: i32) {
        let trunk_height = 4 + (column_rand(wx, wz, self.seed, 0xBEEF) * 3.0) as i32;
        for i in 1..=trunk_height {
            chunk.set_local(lx, ground_y + i, lz, BlockType::Wood);
        }
        let top = ground_y + trunk_height;
        for dx in -2i32..=2 {
            for dz in -2i32..=2 {
                for dy in -1..=2 {
                    if dx.abs() == 2 && dz.abs() == 2 {
                        continue;
                    }
                    let bx = lx + dx;
                    let by = top + dy;
                    let bz = lz + dz;
                    if Chunk::in_bounds(bx, by, bz) && chunk.get_local(bx, by, bz) == BlockType::Air
                    {
                        chunk.set_local(bx, by, bz, BlockType::Leaves);
                    }
                }
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
