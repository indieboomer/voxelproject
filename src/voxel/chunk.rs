use super::block::BlockType;

pub const CHUNK_X: i32 = 16;
pub const CHUNK_Z: i32 = 16;
pub const CHUNK_Y: i32 = 128;
/// Preserve the existing seeded landscape independently of the build ceiling.
pub const TERRAIN_HEIGHT: i32 = 48;

pub struct Chunk {
    pub cx: i32,
    pub cz: i32,
    blocks: Vec<BlockType>,
    pub dirty: bool,
}

impl Chunk {
    pub fn new(cx: i32, cz: i32) -> Self {
        Self {
            cx,
            cz,
            blocks: Vec::with_capacity((CHUNK_X * TERRAIN_HEIGHT * CHUNK_Z) as usize),
            dirty: true,
        }
    }

    #[inline]
    fn index(lx: i32, ly: i32, lz: i32) -> usize {
        ((ly * CHUNK_Z + lz) * CHUNK_X + lx) as usize
    }

    #[inline]
    pub fn in_bounds(lx: i32, ly: i32, lz: i32) -> bool {
        lx >= 0 && lx < CHUNK_X && ly >= 0 && ly < CHUNK_Y && lz >= 0 && lz < CHUNK_Z
    }

    pub fn get_local(&self, lx: i32, ly: i32, lz: i32) -> BlockType {
        if !Self::in_bounds(lx, ly, lz) {
            return BlockType::Air;
        }
        self.blocks
            .get(Self::index(lx, ly, lz))
            .copied()
            .unwrap_or(BlockType::Air)
    }

    pub fn set_local(&mut self, lx: i32, ly: i32, lz: i32, block: BlockType) {
        if !Self::in_bounds(lx, ly, lz) {
            return;
        }
        let index = Self::index(lx, ly, lz);
        if index >= self.blocks.len() {
            if block == BlockType::Air {
                return;
            }
            let new_len = ((ly + 1) * CHUNK_X * CHUNK_Z) as usize;
            self.blocks.reserve_exact(new_len - self.blocks.len());
            self.blocks.resize(new_len, BlockType::Air);
        }
        self.blocks[index] = block;
        self.dirty = true;
    }

    pub fn stored_height(&self) -> i32 {
        (self.blocks.len() / (CHUNK_X * CHUNK_Z) as usize) as i32
    }

    pub fn world_origin(&self) -> (i32, i32) {
        (self.cx * CHUNK_X, self.cz * CHUNK_Z)
    }
}

/// Splits a world block coordinate into (chunk coord, local coord).
pub fn world_to_chunk(wx: i32, wz: i32) -> (i32, i32) {
    (wx.div_euclid(CHUNK_X), wz.div_euclid(CHUNK_Z))
}

pub fn world_to_local(wx: i32, wz: i32) -> (i32, i32) {
    (wx.rem_euclid(CHUNK_X), wz.rem_euclid(CHUNK_Z))
}

#[cfg(test)]
mod height_tests {
    use super::*;
    #[test]
    fn upper_air_is_free_and_ceiling_blocks_mesh_and_survive_reload() {
        let mut chunk = Chunk::new(0, 0);
        assert_eq!(chunk.stored_height(), 0);
        chunk.set_local(0, 127, 0, BlockType::Air);
        assert_eq!(chunk.stored_height(), 0);
        chunk.set_local(0, 20, 0, BlockType::Stone);
        assert_eq!(chunk.stored_height(), 21);
        assert_eq!(chunk.get_local(0, 127, 0), BlockType::Air);
        let mut world = crate::voxel::World::new(42);
        world.ensure_chunk_loaded(0, 0);
        assert!(world.chunks[&(0, 0)].stored_height() <= TERRAIN_HEIGHT);
        world.set_block(4, 127, 4, BlockType::Bricks);
        world.unload_chunk(0, 0);
        world.ensure_chunk_loaded(0, 0);
        assert_eq!(world.get_block(4, 127, 4), BlockType::Bricks);
        let mesh = crate::voxel::mesher::build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
        assert!(mesh.vertices.iter().any(|v| v.position[1] == 128.0));
        world.set_block(4, 128, 4, BlockType::Bricks);
        assert_eq!(world.get_block(4, 128, 4), BlockType::Air);
    }
}
