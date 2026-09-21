use crate::voxel::{BlockType, World};
use glam::Vec3;
/// Deterministic wild hive locations: a small subset of mature oak trees.
pub fn nearby(world: &World, center: Vec3) -> Vec<Vec3> {
    let cx = center.x.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut out = Vec::new();
    for x in (cx - 48)..=(cx + 48) {
        for z in (cz - 48)..=(cz + 48) {
            let y = world.terrain_height(x, z);
            let trunk_kind = world.get_block(x, y + 1, z);
            let (trunk, leaves) = match trunk_kind {
                BlockType::OakWood => (true, BlockType::OakLeaves),
                BlockType::BirchWood => (true, BlockType::BirchLeaves),
                BlockType::CherryWood => (true, BlockType::CherryLeaves),
                _ => (false, BlockType::Air),
            };
            if !trunk {
                continue;
            }
            let leaf = (3..=8)
                .flat_map(|dy| (-2..=2).flat_map(move |dx| (-2..=2).map(move |dz| (dx, dy, dz))))
                .find(|(dx, dy, dz)| world.get_block(x + dx, y + dy, z + dz) == leaves);
            // Roughly one hive for every five mature oak, birch, or cherry trees.
            // Hang the voxel below the lowest canopy leaf, in an empty cell.
            if trunk
                && leaf.is_some()
                && crate::voxel::noise::block_rand(x, y, z, world.seed, 0xbee5) < 0.20
            {
                let (dx, dy, dz) = leaf.unwrap();
                if world.get_block(x + dx, y + dy - 1, z + dz) == BlockType::Air {
                    out.push(Vec3::new(
                        (x + dx) as f32 + 0.5,
                        (y + dy - 1) as f32,
                        (z + dz) as f32 + 0.5,
                    ));
                }
            }
        }
    }
    out
}
pub fn particles(positions: &[Vec3], time: f32) -> crate::voxel::mesher::MeshData {
    let mut mesh = crate::voxel::mesher::MeshData {
        vertices: vec![],
        indices: vec![],
    };
    for (hive, &origin) in positions.iter().enumerate() {
        for i in 0..8 {
            let phase = time * (1.7 + i as f32 * 0.09) + hive as f32 * 1.7 + i as f32 * 2.1;
            let pos = origin
                + Vec3::new(
                    phase.cos() * (0.85 + i as f32 * 0.04),
                    0.5 + (phase * 1.7).sin() * 0.35,
                    phase.sin() * (0.85 + i as f32 * 0.03),
                );
            let size = Vec3::splat(0.035);
            crate::voxel::mesher::push_cuboid(
                &mut mesh.vertices,
                &mut mesh.indices,
                pos - size,
                pos + size,
                [0.95, 0.7, 0.08],
                crate::voxel::atlas::white_uv(),
            );
        }
    }
    mesh
}
pub fn key(pos: Vec3) -> u64 {
    ((pos.x.to_bits() as u64) << 32) ^ pos.z.to_bits() as u64
}

pub const DESTROY_DAMAGE: f32 = 35.0;
pub const TEXTURE_LAYER: f32 = 44.0;

/// Code-generated 16px honeycomb: crisp amber cells, golden wax, dark entrance.
pub fn texture() -> &'static image::RgbaImage {
    static TEXTURE: std::sync::OnceLock<image::RgbaImage> = std::sync::OnceLock::new();
    TEXTURE.get_or_init(|| {
        image::RgbaImage::from_fn(16, 16, |x, y| {
            let row = y / 4;
            let col = (x + (row % 2) * 2) % 4;
            let color = if (6..=9).contains(&x) && (10..=12).contains(&y) {
                [67, 31, 9, 255]
            } else if y % 4 == 0 || col == 0 {
                [185, 92, 15, 255]
            } else if (x * 7 + y * 11) % 5 == 0 {
                [255, 207, 65, 255]
            } else if y % 4 == 1 {
                [247, 174, 34, 255]
            } else {
                [228, 133, 20, 255]
            };
            image::Rgba(color)
        })
    })
}

pub fn mesh(positions: &[Vec3]) -> crate::voxel::mesher::MeshData {
    let mut mesh = crate::voxel::mesher::MeshData {
        vertices: vec![],
        indices: vec![],
    };
    for &pos in positions {
        crate::voxel::mesher::push_cuboid(
            &mut mesh.vertices,
            &mut mesh.indices,
            pos - Vec3::new(0.5, 0.0, 0.5),
            pos + Vec3::new(0.5, 1.0, 0.5),
            [1.0; 3],
            [0.0, 0.0, 1.0, 1.0],
        );
    }
    for v in &mut mesh.vertices {
        v.tex_layer = TEXTURE_LAYER;
    }
    mesh
}

/// Exact cube targeting, with solid terrain blocking harvesting through walls.
pub fn aimed(world: &World, eye: Vec3, direction: Vec3, positions: &[Vec3]) -> Option<Vec3> {
    let direction = direction.normalize_or_zero();
    positions
        .iter()
        .copied()
        .filter_map(|pos| {
            let lo = pos - Vec3::new(0.5, 0.0, 0.5);
            let hi = lo + Vec3::ONE;
            let mut near = 0.0f32;
            let mut far = 6.0f32;
            for axis in 0..3 {
                if direction[axis].abs() < 0.00001 {
                    if eye[axis] < lo[axis] || eye[axis] > hi[axis] {
                        return None;
                    }
                } else {
                    let a = (lo[axis] - eye[axis]) / direction[axis];
                    let b = (hi[axis] - eye[axis]) / direction[axis];
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                }
            }
            if near > far || crate::raycast::raycast(world, eye, direction, near).is_some() {
                None
            } else {
                Some((near, pos))
            }
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn beehive_is_one_textured_voxel_with_eight_orbiting_bees() {
        let p = Vec3::new(0.5, 40.0, 0.5);
        let m = mesh(&[p]);
        assert_eq!(m.vertices.len(), 24);
        assert_eq!(m.indices.len(), 36);
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in m.vertices {
            let p = Vec3::from_array(v.position);
            lo = lo.min(p);
            hi = hi.max(p);
            assert_eq!(v.tex_layer, TEXTURE_LAYER);
        }
        assert_eq!(hi - lo, Vec3::ONE);
        assert_eq!(texture().dimensions(), (16, 16));
        assert_eq!(particles(&[p], 0.0).vertices.len(), 8 * 24);
        assert!(DESTROY_DAMAGE >= crate::player::MAX_HEALTH / 3.0);
    }
    #[test]
    fn beehive_targeting_hits_cube_but_not_near_misses_or_walls() {
        let mut world = World::new(42);
        let p = Vec3::new(0.5, 40.0, 0.5);
        let eye = Vec3::new(0.5, 40.5, 3.5);
        assert_eq!(aimed(&world, eye, -Vec3::Z, &[p]), Some(p));
        assert_eq!(aimed(&world, eye + Vec3::X, -Vec3::Z, &[p]), None);
        world.ensure_chunk_loaded(0, 0);
        world.set_block(0, 40, 2, BlockType::Stone);
        assert_eq!(aimed(&world, eye, -Vec3::Z, &[p]), None);
    }
}
