use super::block::BlockType;

pub const CHUNK_X: i32 = 16;
pub const CHUNK_Z: i32 = 16;
pub const CHUNK_Y: i32 = 48;

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
            blocks: vec![BlockType::Air; (CHUNK_X * CHUNK_Y * CHUNK_Z) as usize],
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
        self.blocks[Self::index(lx, ly, lz)]
    }

    pub fn set_local(&mut self, lx: i32, ly: i32, lz: i32, block: BlockType) {
        if !Self::in_bounds(lx, ly, lz) {
            return;
        }
        self.blocks[Self::index(lx, ly, lz)] = block;
        self.dirty = true;
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
