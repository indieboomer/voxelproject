//! Reproducible natural-coast test scene (docs/NATURAL_COAST.md): broad grassy
//! terraces, rock faces at larger drops, a sandy bay, graded shallows with
//! submerged rocks, small islets, trees and flower patches. Built only from
//! ordinary blocks, so it is editable, saved and replicated like any terrain.

use super::block::BlockType;
use super::noise::{column_rand, fbm};

/// Scene footprint in cells (x, z). The sea lies toward -z.
pub const SIZE: (i32, i32) = (64, 48);
/// Cells of ground kept below the water surface everywhere.
const FLOOR: i32 = 8;
/// Clear air above the water surface (tallest trees fit inside).
const CLEAR: i32 = 24;
const SEED: u32 = 0xC0A5_7001;

/// Coastline position (local z) for column x: wobbly, with a sandy bay.
fn coast_z(x: i32) -> f32 {
    let x = x as f32;
    let wobble = (fbm(x * 0.09, 3.7, SEED, 3, 2.0, 0.5) - 0.5) * 9.0;
    let bay = 7.0 * (-((x - 22.0) / 6.5).powi(2)).exp();
    let point = -4.0 * (-((x - 46.0) / 4.0).powi(2)).exp();
    15.0 + wobble + bay + point
}

/// Land top level above the water surface (>= 1), or seabed depth as a
/// non-positive value (0 = one cell of water, -3 = four cells).
fn level(x: i32, z: i32) -> i32 {
    let inland = z as f32 - coast_z(x);
    let n = fbm(x as f32 * 0.07, z as f32 * 0.07, SEED ^ 0x11, 3, 2.0, 0.5);
    if inland < 0.0 {
        // Gently shelving seabed: turquoise shallows, deeper blue offshore.
        return -((-inland * 0.28 + n * 1.5) as i32).clamp(0, 4);
    }
    let in_bay = (x - 22).abs() < 8;
    // Broad terraces about five cells deep. Each terrace edge alternates
    // along the coast between a one-cell grass shoulder and a two-cell rock
    // face, so slopes and cliffs interleave instead of forming stripes.
    let steps = ((inland + n * 6.0 - 1.0) / 5.0).floor().max(0.0) as i32;
    let mut terrace = 1;
    for i in 0..steps.min(5) {
        let rocky = fbm(x as f32 * 0.09, i as f32 * 3.7, SEED ^ 0x44, 2, 2.0, 0.5) > 0.5;
        terrace += 1 + rocky as i32;
    }
    // Away from the bay, the first land rises straight out of the water as rock.
    if !in_bay && inland < 4.0 {
        return terrace.max(2 + (n * 2.0) as i32);
    }
    terrace
}

fn islet(x: i32, z: i32) -> Option<i32> {
    for (cx, cz, r, h) in [(8, 5, 2.4f32, 2), (40, 3, 1.7, 1), (55, 7, 2.2, 3)] {
        let d = (((x - cx).pow(2) + (z - cz).pow(2)) as f32).sqrt();
        if d <= r {
            return Some(h - (d > r - 1.0) as i32);
        }
    }
    None
}

/// Every edit for the scene, with the water surface at `origin.1` (the top of
/// the highest water layer) and the footprint starting at `origin` (x, z).
pub fn coast_scene(origin: (i32, i32, i32)) -> Vec<(i32, i32, i32, BlockType)> {
    let (ox, w, oz) = origin;
    let (sx, sz) = SIZE;
    let levels: Vec<Vec<i32>> = (0..sx)
        .map(|x| (0..sz).map(|z| islet(x, z).unwrap_or_else(|| level(x, z))).collect())
        .collect();
    let at = |x: i32, z: i32| {
        if (0..sx).contains(&x) && (0..sz).contains(&z) {
            levels[x as usize][z as usize]
        } else {
            0
        }
    };
    let mut edits = Vec::new();
    let mut tops = Vec::new();
    for x in 0..sx {
        for z in 0..sz {
            let l = at(x, z);
            let rand = |salt| column_rand(x, z, SEED, salt);
            // Column top layer (water surface layer is w - 1).
            let top = if l >= 1 { w + l - 1 } else { w - 2 + l };
            let lowest_neighbor = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|(dx, dz)| {
                    let n = at(x + dx, z + dz);
                    if n >= 1 { w + n - 1 } else { w - 1 }
                })
                .min()
                .unwrap();
            let cliff = l >= 1 && top - lowest_neighbor >= 2;
            let beach = l == 1 && (x - 22).abs() < 8;
            for y in w - FLOOR..=w + CLEAR {
                let block = if y > top {
                    if y < w && l < 1 { BlockType::Water } else { BlockType::Air }
                } else if l < 1 {
                    // Seabed: sand in the shallows, rock patches and stones.
                    let rocky = fbm(x as f32 * 0.21, z as f32 * 0.21, SEED ^ 0x22, 2, 2.0, 0.5) > 0.62;
                    if y == top && !rocky && l >= -2 { BlockType::Sand } else { BlockType::Stone }
                } else if y == top {
                    if beach { BlockType::Sand } else if cliff && rand(3) < 0.25 { BlockType::Stone } else { BlockType::Grass }
                } else if cliff && y > lowest_neighbor - 1 {
                    BlockType::Stone // exposed rock face
                } else if beach || (l >= 1 && y >= top - 2 && y >= w - 1) {
                    if beach { BlockType::Sand } else { BlockType::Soil }
                } else {
                    BlockType::Stone
                };
                edits.push((ox + x, y, oz + z, block));
            }
            // Submerged boulders reaching one cell below the surface.
            if l <= -1 && rand(5) < 0.05 {
                for y in top + 1..w - 1 {
                    edits.push((ox + x, y, oz + z, BlockType::Stone));
                }
            }
            if l >= 2 && !cliff {
                tops.push((x, z, top));
            }
        }
    }
    // Trees on terrace grass, spaced apart; flower and fern patches between.
    let mut trees: Vec<(i32, i32)> = Vec::new();
    for &(x, z, top) in &tops {
        let r = column_rand(x, z, SEED, 7);
        let clear = (2..sx - 2).contains(&x) && (2..sz - 2).contains(&z);
        if clear && r < 0.03 && trees.iter().all(|&(tx, tz)| (tx - x).pow(2) + (tz - z).pow(2) > 60) {
            trees.push((x, z));
            let species = [0, 0, 0, 2, 1, 3][(column_rand(x, z, SEED, 8) * 6.0) as usize % 6];
            for ((dx, dy, dz), block) in super::trees::tree_blocks(SEED, ox + x, oz + z, species) {
                if top + dy <= w + CLEAR {
                    edits.push((ox + x + dx, top + dy, oz + z + dz, block));
                }
            }
        } else if fbm(x as f32 * 0.15, z as f32 * 0.15, SEED ^ 0x33, 2, 2.0, 0.5) > 0.58 && r > 0.4 {
            let plants = [BlockType::Fern, BlockType::Lavender, BlockType::Clover, BlockType::Bluebell, BlockType::ShortGrass];
            edits.push((ox + x, top + 1, oz + z, plants[(r * 50.0) as usize % plants.len()]));
        }
    }
    // Later entries win: dedupe keeping the last edit per cell.
    let mut cells = std::collections::BTreeMap::new();
    for (x, y, z, b) in edits {
        cells.insert((x, y, z), b);
    }
    cells.into_iter().map(|((x, y, z), b)| (x, y, z, b)).collect()
}

/// Steep-bank test scene (docs/TERRAIN_SHAPING.md): one hillside, 24 x 20
/// cells, whose rows climb at different steepness: one cell per column (45
/// degrees), two per column (steep bank), three per column (cliff steps), and
/// a two-per-column bank broken by a one-cell ledge. Grass over soil, ground
/// layer at `origin.1 - 1`.
pub fn steep_bank(origin: (i32, i32, i32)) -> Vec<(i32, i32, i32, BlockType)> {
    let (ox, y0, oz) = origin;
    let mut edits = Vec::new();
    for x in 0..24 {
        for z in 0..20 {
            let step = (x - 4).clamp(0, 8);
            let rise = match z / 5 {
                0 => step,
                1 => 2 * step,
                2 => 3 * step.min(5),
                // A ledge: the bank pauses for a column halfway up.
                _ => 2 * step.min(3) + 2 * (step - 4).max(0),
            };
            for y in -3..20 {
                let block = if y >= rise {
                    BlockType::Air
                } else if y == rise - 1 {
                    BlockType::Grass
                } else if y >= rise - 4 {
                    BlockType::Soil
                } else {
                    BlockType::Stone
                };
                edits.push((ox + x, y0 + y, oz + z, block));
            }
        }
    }
    edits
}

/// Tree test grove (docs/TREES.md): grass ground at layer `origin.1 - 1`,
/// four generated trees of different species, and a player-built oak post
/// and beam touching one trunk. Returns `(generated, placed)` blocks: the
/// generated ones must be written as chunk data so they count as living.
#[cfg(test)]
pub fn tree_grove(origin: (i32, i32, i32)) -> (Vec<(i32, i32, i32, BlockType)>, Vec<(i32, i32, i32, BlockType)>) {
    let (ox, y0, oz) = origin;
    let mut generated = Vec::new();
    for x in -6..18 {
        for z in -6..18 {
            for y in y0 - 3..y0 + 16 {
                let block = if y >= y0 { BlockType::Air } else if y == y0 - 1 { BlockType::Grass } else { BlockType::Soil };
                generated.push((ox + x, y, oz + z, block));
            }
        }
    }
    for (x, z, species) in [(2, 3, 0), (10, 2, 3), (4, 11, 2), (12, 10, 1)] {
        for ((dx, dy, dz), block) in super::trees::tree_blocks(SEED, ox + x, oz + z, species) {
            generated.push((ox + x + dx, y0 - 1 + dy, oz + z + dz, block));
        }
    }
    // A post and beam built against the oak's trunk: construction, not tree.
    let placed = vec![
        (ox + 3, y0, oz + 3, BlockType::OakWood),
        (ox + 3, y0 + 1, oz + 3, BlockType::OakWood),
        (ox + 3, y0 + 2, oz + 3, BlockType::OakWood),
        (ox + 4, y0 + 2, oz + 3, BlockType::OakWood),
        (ox + 5, y0 + 2, oz + 3, BlockType::OakWood),
    ];
    (generated, placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coast_scene_is_deterministic_and_has_every_landform() {
        let a = coast_scene((0, 30, 0));
        assert_eq!(a, coast_scene((0, 30, 0)));
        let count = |b: BlockType| a.iter().filter(|e| e.3 == b).count();
        for block in [BlockType::Water, BlockType::Sand, BlockType::Stone, BlockType::Grass, BlockType::OakLeaves] {
            assert!(count(block) > 20, "{block:?}: {}", count(block));
        }
        // Water only at or below the surface layer.
        assert!(a.iter().filter(|e| e.3 == BlockType::Water).all(|e| e.1 < 30));
    }
}
