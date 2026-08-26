use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::block::BlockType;
use super::chunk::{world_to_chunk, world_to_local, Chunk, CHUNK_X, CHUNK_Y, CHUNK_Z};
use super::noise::{column_rand, fbm};

pub const SEA_LEVEL: i32 = 18;

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
        let base = fbm(wx as f32 * 0.01, wz as f32 * 0.01, self.seed, 4, 2.0, 0.5);
        let hills = fbm(
            wx as f32 * 0.04,
            wz as f32 * 0.04,
            self.seed ^ 0x51ed,
            3,
            2.0,
            0.5,
        );
        let h = base * 22.0 + hills * 6.0 + 14.0;
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
