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
    pub revision: u64,
    pub(crate) sky_cache: std::cell::RefCell<Option<crate::shelter::SkyField>>,
    roof: [u8; 256],
    sky_roof: [u8; 256],
}

impl Chunk {
    pub fn new(cx: i32, cz: i32) -> Self {
        Self {
            cx,
            cz,
            blocks: Vec::with_capacity((CHUNK_X * TERRAIN_HEIGHT * CHUNK_Z) as usize),
            dirty: true,
            revision: 0,
            sky_cache: Default::default(),
            roof: [0; 256],
            sky_roof: [0; 256],
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
        if self.blocks[index] == block {
            return;
        }
        self.blocks[index] = block;
        self.revision = self.revision.wrapping_add(1);
        *self.sky_cache.get_mut() = None;
        let col = (lz * CHUNK_X + lx) as usize;
        if block.is_solid() {
            self.roof[col] = self.roof[col].max((ly + 1) as u8);
        } else if self.roof[col] as i32 == ly + 1 {
            self.roof[col] = (0..ly)
                .rev()
                .find(|&y| self.get_local(lx, y, lz).is_solid())
                .map_or(0, |y| (y + 1) as u8);
        }
        if block.is_opaque() {
            self.sky_roof[col] = self.sky_roof[col].max((ly + 1) as u8);
        } else if self.sky_roof[col] as i32 == ly + 1 {
            self.sky_roof[col] = (0..ly)
                .rev()
                .find(|&y| self.get_local(lx, y, lz).is_opaque())
                .map_or(0, |y| (y + 1) as u8);
        }
        self.dirty = true;
    }

    pub fn stored_height(&self) -> i32 {
        (self.blocks.len() / (CHUNK_X * CHUNK_Z) as usize) as i32
    }
    pub fn roof_height(&self, x: i32, z: i32) -> i32 {
        self.roof[(z * 16 + x) as usize] as i32
    }
    pub fn sky_height(&self, x: i32, z: i32) -> i32 {
        self.sky_roof[(z * 16 + x) as usize] as i32
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
    fn roof_cache_tracks_replaced_roofs_at_build_ceiling() {
        let mut c = Chunk::new(0, 0);
        c.set_local(3, 127, 4, BlockType::OakLeaves);
        assert_eq!(c.roof_height(3, 4), 128);
        assert_eq!(c.sky_height(3, 4), 0);
        c.set_local(3, 20, 4, BlockType::Stone);
        assert_eq!(c.sky_height(3, 4), 21);
        c.set_local(3, 127, 4, BlockType::Air);
        assert_eq!(c.roof_height(3, 4), 21);
        c.set_local(3, 20, 4, BlockType::Air);
        assert_eq!(c.roof_height(3, 4), 0);
    }
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
