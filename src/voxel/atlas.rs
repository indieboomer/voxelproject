//! Maps block types (and faces) to tiles in `assets/textures/atlas.png`.
//! The atlas itself, and the `atlas_tiles` tile-index constants, are built
//! by `tools/build_atlas.py` from `textures/blocks.csv` and the flat
//! texture images under `textures/` -- re-run that script and rebuild if
//! either changes.

use super::atlas_tiles::{ATLAS_COLS, ATLAS_ROWS, TILE_WHITE};
use super::block::BlockType;

pub const ATLAS_BYTES: &[u8] = include_bytes!("../../assets/textures/atlas.png");

/// Which atlas tile a block shows on a given face. Face index matches
/// `mesher::FACE_NORMALS`: 0=+X, 1=-X, 2=+Y (top), 3=-Y (bottom), 4=+Z, 5=-Z.
pub fn tile_for(block: BlockType, face: usize) -> u8 {
    let def = block.def();
    match face {
        2 => def.tile_top,
        3 => def.tile_bottom,
        _ => def.tile_side,
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

/// A constant UV at the center of the white swatch, in rect form for
/// `push_cuboid`. Flat-colored meshes must not sample a tile boundary:
/// interpolation rounding can otherwise select a neighboring colored or
/// transparent texel and make entire faces flicker as the camera moves.
pub fn white_uv() -> [f32; 4] {
    let [u0, v0, u1, v1] = uv_rect(TILE_WHITE);
    let u = (u0 + u1) * 0.5;
    let v = (v0 + v1) * 0.5;
    [u, v, u, v]
}

/// Whether this block's faces should be alpha-tested (cutout) rather than
/// treated as fully solid -- leaves, whose atlas tiles have real
/// transparency baked in from the source texture.
pub fn is_cutout(block: BlockType) -> bool {
    block.def().cutout
}
