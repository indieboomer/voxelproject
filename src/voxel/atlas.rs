//! Maps block types (and faces) to tiles in `assets/textures/atlas.png`.
//! The atlas itself is built by `tools/build_atlas.py` from the Minecraft
//! resource pack under `textures/` -- re-run that script and rebuild if the
//! source textures change. Tile indices here must match its layout exactly.

use super::block::BlockType;

pub const ATLAS_BYTES: &[u8] = include_bytes!("../../assets/textures/atlas.png");
pub const ATLAS_COLS: u32 = 4;
pub const ATLAS_ROWS: u32 = 4;

const TILE_GRASS_TOP: u8 = 0;
const TILE_GRASS_SIDE: u8 = 1;
const TILE_DIRT: u8 = 2;
const TILE_STONE: u8 = 3;
const TILE_SAND: u8 = 4;
const TILE_WOOD_SIDE: u8 = 5;
const TILE_WOOD_TOP: u8 = 6;
const TILE_LEAVES: u8 = 7;
const TILE_CRYSTAL: u8 = 8;
const TILE_MUD: u8 = 9;
const TILE_REDSTONE: u8 = 10;
const TILE_WATER: u8 = 11;
/// Flat white swatch. Entities (creatures/players) sample this and are
/// tinted entirely by per-vertex color, so they share the same textured
/// pipeline as world geometry without needing real entity skins.
pub const TILE_WHITE: u8 = 12;

/// Which atlas tile a block shows on a given face. Face index matches
/// `mesher::FACE_NORMALS`: 0=+X, 1=-X, 2=+Y (top), 3=-Y (bottom), 4=+Z, 5=-Z.
pub fn tile_for(block: BlockType, face: usize) -> u8 {
    match block {
        BlockType::Grass => match face {
            2 => TILE_GRASS_TOP,
            3 => TILE_DIRT,
            _ => TILE_GRASS_SIDE,
        },
        BlockType::Dirt => TILE_DIRT,
        BlockType::Stone => TILE_STONE,
        BlockType::Sand => TILE_SAND,
        BlockType::Wood => match face {
            2 | 3 => TILE_WOOD_TOP,
            _ => TILE_WOOD_SIDE,
        },
        BlockType::Leaves => TILE_LEAVES,
        BlockType::Water => TILE_WATER,
        BlockType::Crystal => TILE_CRYSTAL,
        BlockType::Mud => TILE_MUD,
        BlockType::RedStone => TILE_REDSTONE,
        BlockType::Air => TILE_WHITE,
    }
}

/// UV rect `[u0, v0, u1, v1]` for a tile index.
pub fn uv_rect(tile: u8) -> [f32; 4] {
    let col = (tile as u32 % ATLAS_COLS) as f32;
    let row = (tile as u32 / ATLAS_COLS) as f32;
    let u0 = col / ATLAS_COLS as f32;
    let v0 = row / ATLAS_ROWS as f32;
    let u1 = (col + 1.0) / ATLAS_COLS as f32;
    let v1 = (row + 1.0) / ATLAS_ROWS as f32;
    [u0, v0, u1, v1]
}

pub fn white_uv() -> [f32; 4] {
    uv_rect(TILE_WHITE)
}

/// Whether this block's faces should be alpha-tested (cutout) rather than
/// treated as fully solid -- currently only Leaves, whose atlas tile has
/// real transparency baked in from the source texture.
pub fn is_cutout(block: BlockType) -> bool {
    matches!(block, BlockType::Leaves)
}
