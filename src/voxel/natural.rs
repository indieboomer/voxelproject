//! Natural-coast presentation for shaped mode (docs/NATURAL_COAST.md):
//! world-space material palettes, water depth/shore data for the shallows and
//! foam shader, and foliage shaping. Visual only: nothing here changes
//! occupancy, picking, collision or saves.

use super::block::BlockType;
use super::noise::fbm;

/// `Vertex::tex_layer` marking terrain whose atlas detail the shader softens
/// under a world-space palette carried in the vertex color.
pub const NATURAL_TEX: f32 = 0.25;
/// `Vertex::tex_layer` for natural foliage: coarse cutout, softened detail.
pub const NATURAL_FOLIAGE: f32 = 0.375;
/// Natural water top vertices carry `wet = [WATER_DATA + depth, shore]`.
pub const WATER_DATA: f32 = 10.0;
/// Height of the grass lip drawn over an earth side face.
pub const GRASS_LIP: f32 = 0.22;
const SEED: u32 = 0x4A70_0001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Grass,
    Soil,
    Rock,
    Sand,
}

pub fn palette(block: BlockType) -> Option<Palette> {
    match block {
        BlockType::Grass => Some(Palette::Grass),
        BlockType::Soil => Some(Palette::Soil),
        BlockType::Stone => Some(Palette::Rock),
        BlockType::Sand => Some(Palette::Sand),
        _ => None,
    }
}

fn linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| v.powf(2.2))
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// Albedo at world point `p`: large coherent areas from low-frequency noise,
/// never an independent color per cell.
pub fn tint(palette: Palette, p: [f32; 3]) -> [f32; 3] {
    let broad = fbm(p[0] * 0.045, p[2] * 0.045, SEED, 3, 2.0, 0.5);
    let mid = fbm(p[0] * 0.19 + p[1] * 0.11, p[2] * 0.19, SEED ^ 0x55, 2, 2.0, 0.5);
    let t = ((broad - 0.5) * 2.2 + 0.5).clamp(0.0, 1.0);
    match palette {
        Palette::Grass => {
            let lush = mix(linear([0.60, 0.71, 0.28]), linear([0.42, 0.58, 0.22]), t);
            mix(lush, linear([0.72, 0.71, 0.38]), (mid - 0.6) * 2.5)
        }
        Palette::Soil => mix(linear([0.64, 0.46, 0.27]), linear([0.52, 0.37, 0.23]), t),
        Palette::Rock => {
            // Faint horizontal strata so cliff faces read as layered stone.
            let strata = fbm(p[1] * 0.7, p[0] * 0.05 + p[2] * 0.05, SEED ^ 0x77, 2, 2.0, 0.5);
            mix(linear([0.71, 0.67, 0.60]), linear([0.57, 0.54, 0.49]), t * 0.6 + strata * 0.4)
        }
        Palette::Sand => mix(linear([0.88, 0.80, 0.60]), linear([0.80, 0.71, 0.52]), t),
    }
}

/// Seabed color for water shading: the palette of the first non-water block
/// below, or a generic sandy floor.
fn seabed(sample: &impl Fn(i32, i32, i32) -> BlockType, x: i32, y: i32, z: i32) -> [f32; 3] {
    let floor = (1..12).map(|d| sample(x, y - d, z)).find(|b| *b != BlockType::Water);
    let p = [x as f32, y as f32, z as f32];
    match floor.and_then(palette) {
        Some(Palette::Grass) => tint(Palette::Soil, p),
        Some(found) => tint(found, p),
        None => tint(Palette::Sand, p),
    }
}

/// Water data at the lattice corner `(x, y + 1, z)` of surface-layer `y`:
/// mean depth in cells, shore proximity (1 at land contact, 0 three cells
/// out, narrower at steep rock), and mean seabed color. Derived from blocks
/// only, so coastal edits update it with the normal chunk remesh.
pub fn water_corner(sample: &impl Fn(i32, i32, i32) -> BlockType, x: i32, y: i32, z: i32) -> (f32, f32, [f32; 3]) {
    let cells = [(x - 1, z - 1), (x, z - 1), (x - 1, z), (x, z)];
    let mut depth = 0.0;
    let mut color = [0.0; 3];
    let mut wet = 0.0;
    for &(cx, cz) in &cells {
        if sample(cx, y, cz) == BlockType::Water {
            depth += (0..8).take_while(|d| sample(cx, y - d, cz) == BlockType::Water).count() as f32;
            let c = seabed(sample, cx, y, cz);
            color = std::array::from_fn(|k| color[k] + c[k]);
            wet += 1.0;
        }
    }
    let wet = f32::max(wet, 1.0);
    let mut nearest = f32::INFINITY;
    for dx in -3..3 {
        for dz in -3..3 {
            let (cx, cz) = (x + dx, z + dz);
            let block = sample(cx, y, cz);
            if !block.is_solid() {
                continue;
            }
            let gap = |p: f32, lo: f32| (lo - p).max(p - (lo + 1.0)).max(0.0);
            let mut d = gap(x as f32, cx as f32).hypot(gap(z as f32, cz as f32));
            if sample(cx, y + 1, cz).is_solid() {
                d *= 1.6; // rock walls: a tighter foam band
            }
            nearest = nearest.min(d);
        }
    }
    let shore = 1.0 - (nearest / 3.0).clamp(0.0, 1.0);
    (depth / wet, shore, color.map(|c| c / wet))
}

/// Leaf corner placement: exposed corners pull toward the leaves around
/// them, rounding cube crowns into lobes, plus a small lattice-keyed jitter.
/// Every face sharing a corner computes the same point, so nothing cracks.
pub fn leaf_corner(sample: &impl Fn(i32, i32, i32) -> BlockType, [x, y, z]: [i32; 3]) -> ([f32; 3], f32) {
    let mut sum = [0.0f32; 3];
    let mut count = 0.0f32;
    for dx in [-1, 0] {
        for dy in [-1, 0] {
            for dz in [-1, 0] {
                if sample(x + dx, y + dy, z + dz).def().cutout {
                    sum[0] += dx as f32 + 0.5;
                    sum[1] += dy as f32 + 0.5;
                    sum[2] += dz as f32 + 0.5;
                    count += 1.0;
                }
            }
        }
    }
    let pull = if count > 0.0 && count < 8.0 { 0.6 / count } else { 0.0 };
    let jitter = super::mesher::leaf_offset([x, y, z]);
    // Corners buried among more leaves sit deeper in the crown: darker.
    let shade = 1.0 - 0.5 * ((count - 1.0).max(0.0) / 7.0);
    (std::array::from_fn(|k| [x, y, z][k] as f32 + sum[k] * pull + jitter[k]), shade)
}

/// Foliage albedo: warm yellow-green crowns (deeper for spruce, blossom for
/// cherry), varying per crown rather than per block.
pub fn foliage(block: BlockType, p: [f32; 3]) -> [f32; 3] {
    let t = fbm(p[0] * 0.09 + p[1] * 0.05, p[2] * 0.09, SEED ^ 0x99, 2, 2.0, 0.5);
    let t = ((t - 0.5) * 2.4 + 0.5).clamp(0.0, 1.0);
    match block {
        BlockType::SpruceLeaves => mix(linear([0.30, 0.46, 0.24]), linear([0.22, 0.38, 0.22]), t),
        BlockType::CherryLeaves => mix(linear([0.94, 0.66, 0.74]), linear([0.86, 0.56, 0.66]), t),
        BlockType::BirchLeaves => mix(linear([0.60, 0.70, 0.26]), linear([0.44, 0.58, 0.20]), t),
        _ => mix(linear([0.50, 0.62, 0.20]), linear([0.30, 0.46, 0.15]), t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_are_continuous_in_world_space() {
        for palette in [Palette::Grass, Palette::Soil, Palette::Rock, Palette::Sand] {
            for i in 0..200 {
                let p = [i as f32 * 0.37, 20.0, i as f32 * 0.21];
                let q = [p[0] + 0.01, p[1], p[2] + 0.01];
                let (a, b) = (tint(palette, p), tint(palette, q));
                assert!((0..3).all(|k| (a[k] - b[k]).abs() < 0.01), "{palette:?}");
                assert!(a.iter().all(|c| (0.0..=1.0).contains(c)));
            }
        }
    }

    #[test]
    fn shore_and_depth_follow_the_coast() {
        // Land for x < 4, water deepening toward +x.
        let sample = |x: i32, y: i32, _z: i32| {
            let floor = 10 - (x - 4).clamp(0, 4);
            if x < 4 { if y <= 10 { BlockType::Stone } else { BlockType::Air } }
            else if y <= floor { BlockType::Sand } else if y <= 10 { BlockType::Water } else { BlockType::Air }
        };
        let (d_near, s_near, _) = water_corner(&sample, 5, 10, 0);
        let (d_far, s_far, _) = water_corner(&sample, 9, 10, 0);
        assert!(s_near > s_far && s_far == 0.0, "{s_near} {s_far}");
        assert!(d_far > d_near);
    }

    fn world() -> crate::voxel::World {
        let mut world = crate::voxel::World::new(5);
        world.chunks.insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        world.terrain_shaping = true;
        world
    }

    #[test]
    fn only_growing_leaves_sway_in_both_modes() {
        for shaping in [false, true] {
            let mut world = world();
            world.terrain_shaping = shaping;
            // Generated foliage is chunk data; a placed leaf is a recorded edit.
            world.chunks.get_mut(&(0, 0)).unwrap().set_local(3, 10, 3, BlockType::OakLeaves);
            world.set_block(9, 10, 9, BlockType::OakLeaves);
            world.set_block(4, 10, 3, BlockType::OakLeaves); // placed against the tree
            let mesh = crate::voxel::mesher::build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
            let sway = |x: f32| mesh.vertices.iter().filter(|v| (v.position[0] - x).abs() < 0.5 + 0.3 && v.position[2] < 5.0)
                .any(|v| v.wind > 0.0);
            assert!(mesh.vertices.iter().filter(|v| v.position[0] > 8.5).all(|v| v.wind == 0.0), "placed leaf moved");
            assert!(sway(3.0), "growing leaf should sway (shaping {shaping})");
            assert!(mesh.vertices.iter().filter(|v| v.position[0] > 4.6 && v.position[0] < 5.4 && v.position[2] < 5.0)
                .all(|v| v.wind == 0.0), "placed leaf beside the tree moved");
        }
    }

    #[test]
    fn coastal_edits_update_shore_data_and_ground_decorations() {
        let mut world = world();
        // Sea at x >= 8 (surface layer 9), land at x < 8 up to layer 10.
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..=10 {
                    let block = if x < 8 { if y == 10 { BlockType::Grass } else { BlockType::Soil } }
                        else if y <= 6 { BlockType::Sand } else if y <= 9 { BlockType::Water } else { BlockType::Air };
                    world.set_block(x, y, z, block);
                }
            }
        }
        let shore_at = |world: &crate::voxel::World, x: f32, z: f32| {
            let mesh = crate::voxel::mesher::build_chunk_mesh(world, &world.chunks[&(0, 0)]);
            mesh.vertices.iter()
                .filter(|v| v.wet[0] > WATER_DATA - 0.5 && (v.position[0] - x).abs() < 0.01 && (v.position[2] - z).abs() < 0.01)
                .map(|v| v.wet[1]).fold(f32::NAN, f32::max)
        };
        let before = shore_at(&world, 9.0, 8.0);
        assert!(before > 0.3, "near a rock coast (narrow band): {before}");
        // Open a channel: dig land beside that point down to sea level.
        for x in 5..8 {
            for z in 6..11 {
                for y in 7..=10 {
                    world.set_block(x, y, z, if y <= 9 { BlockType::Water } else { BlockType::Air });
                }
            }
        }
        let after = shore_at(&world, 9.0, 8.0);
        assert!(after < before, "{before} -> {after}");

        // Tufts sit only on exposed grass: covering their cell removes them.
        let tufts = |world: &crate::voxel::World| {
            let mesh = crate::voxel::mesher::build_chunk_mesh(world, &world.chunks[&(0, 0)]);
            mesh.vertices.iter().filter(|v| v.wind > 0.0 && v.position[0] < 4.5).count()
        };
        let had = tufts(&world);
        assert!(had > 0, "the patch noise should place some tufts on this land");
        for x in 0..5 {
            for z in 0..16 {
                world.set_block(x, 11, z, BlockType::Stone);
            }
        }
        assert_eq!(tufts(&world), 0, "no tufts left under new blocks");
    }
}
