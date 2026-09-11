//! Persistent campfire blocks with derived, bounded visual effects and local lights.
use crate::voxel::{
    atlas,
    chunk::{Chunk, CHUNK_Y},
    mesher::{push_cuboid, MeshData, Vertex},
    noise::column_rand,
    BlockType,
};
use glam::Vec3;

pub fn generate(chunk: &mut Chunk, seed: u32) {
    // At most one fire per 48x48 region, with eight candidate clearings.
    if chunk.cx.rem_euclid(3) != 1
        || chunk.cz.rem_euclid(3) != 1
        || column_rand(chunk.cx, chunk.cz, seed, 0xCAAF) > 0.7
    {
        return;
    }
    for attempt in 0..8 {
        let x = 3 + (column_rand(chunk.cx, chunk.cz, seed, 0xCAA1 + attempt * 2) * 9.0) as i32;
        let z = 3 + (column_rand(chunk.cx, chunk.cz, seed, 0xCAA2 + attempt * 2) * 9.0) as i32;
        let Some(y) = (20..CHUNK_Y - 5)
            .rev()
            .find(|&y| chunk.get_local(x, y, z).is_solid())
        else {
            continue;
        };
        let suitable = (-1..=1).all(|dx| {
            (-1..=1).all(|dz| {
                matches!(
                    chunk.get_local(x + dx, y, z + dz),
                    BlockType::Grass | BlockType::Soil | BlockType::Sand | BlockType::Stone
                ) && (1..=4).all(|dy| {
                    let b = chunk.get_local(x + dx, y + dy, z + dz);
                    b == BlockType::Air
                        || (dy == 1 && b.def().only_on_top && b != BlockType::Campfire)
                })
            })
        });
        if suitable {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    chunk.set_local(x + dx, y + 1, z + dz, BlockType::Air);
                }
            }
            chunk.set_local(x, y + 1, z, BlockType::Campfire);
            return;
        }
    }
}

pub fn positions(chunk: &Chunk) -> Vec<Vec3> {
    let (ox, oz) = chunk.world_origin();
    let mut result = Vec::new();
    for x in 0..16 {
        for z in 0..16 {
            for y in 1..CHUNK_Y {
                if chunk.get_local(x, y, z) == BlockType::Campfire
                    && chunk.get_local(x, y - 1, z).is_solid()
                {
                    result.push(Vec3::new(
                        (ox + x) as f32 + 0.5,
                        y as f32,
                        (oz + z) as f32 + 0.5,
                    ));
                }
            }
        }
    }
    result
}

pub fn base_mesh(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, origin: Vec3) {
    let stone = atlas::uv_rect(atlas::tile_for(BlockType::Cobblestone, 0));
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::TAU / 8.0;
        let center = origin + Vec3::new(0.5 + angle.cos() * 0.38, 0.08, 0.5 + angle.sin() * 0.38);
        push_cuboid(
            vertices,
            indices,
            center - Vec3::new(0.11, 0.08, 0.11),
            center + Vec3::new(0.11, 0.08, 0.11),
            [0.8; 3],
            stone,
        );
    }
    let wood = atlas::uv_rect(atlas::tile_for(BlockType::OakWood, 0));
    for z in [0.28, 0.58] {
        push_cuboid(
            vertices,
            indices,
            origin + Vec3::new(0.12, 0.08, z),
            origin + Vec3::new(0.88, 0.23, z + 0.14),
            [0.65; 3],
            wood,
        );
    }
    for x in [0.3, 0.56] {
        push_cuboid(
            vertices,
            indices,
            origin + Vec3::new(x, 0.23, 0.13),
            origin + Vec3::new(x + 0.14, 0.36, 0.87),
            [0.55; 3],
            wood,
        );
    }
}

pub fn lights(positions: &[Vec3], eye: Vec3) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for (out, pos) in result.iter_mut().zip(
        positions
            .iter()
            .filter(|p| p.distance_squared(eye) < 32.0 * 32.0),
    ) {
        *out = (*pos + Vec3::Y * 0.7).extend(8.0).to_array();
    }
    result
}

pub fn effects(positions: &[Vec3], eye: Vec3, time: f32) -> MeshData {
    let mut mesh = MeshData {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for &pos in positions.iter().take(8) {
        let phase = pos.x * 0.37 + pos.z * 0.71;
        let mut right = Vec3::new(eye.z - pos.z, 0.0, pos.x - eye.x).normalize_or_zero();
        if right.length_squared() < 0.5 {
            right = Vec3::X;
        }
        for i in 0..3 {
            let offset = (i as f32 - 1.0) * 0.18;
            let height = 0.65 + 0.13 * (time * 6.0 + phase + i as f32 * 2.0).sin();
            card(
                &mut mesh,
                pos + right * offset + Vec3::Y * 0.28,
                right,
                0.45,
                height,
                -1.0,
            );
        }
        for i in 0..5 {
            let age = (time * 0.25 + i as f32 / 5.0 + phase.fract()).rem_euclid(1.0);
            let base = pos
                + Vec3::new(
                    age * 0.55 + (time + phase).sin() * age * 0.12,
                    0.9 + age * 2.7,
                    age * 0.2,
                );
            card(
                &mut mesh,
                base,
                right,
                0.3 + age * 0.8,
                0.4 + age * 0.7,
                -2.0 - age,
            );
        }
    }
    mesh
}

fn card(mesh: &mut MeshData, base: Vec3, right: Vec3, width: f32, height: f32, kind: f32) {
    let start = mesh.vertices.len() as u32;
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        mesh.vertices.push(Vertex {
            position: (base + right * (u - 0.5) * width + Vec3::Y * v * height).to_array(),
            color: [1.0; 3],
            normal: [0.0, 1.0, 0.0],
            uv: [u, v],
            ao: 1.0,
            reflectivity: 0.0,
            emission: 0.0,
            wind: 0.0,
            tex_layer: kind,
            glimmer: 0.0,
        });
    }
    // Double sided; effects are excluded from the sun shadow pass.
    for index in [0, 1, 2, 0, 2, 3, 2, 1, 0, 3, 2, 0] {
        mesh.indices.push(start + index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clearing(cx: i32, cz: i32) -> Chunk {
        let mut chunk = Chunk::new(cx, cz);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_local(x, 24, z, BlockType::Grass);
            }
        }
        chunk
    }
    #[test]
    fn generated_fires_are_sparse_deterministic_and_require_dry_open_ground() {
        let mut count = 0;
        for cx in -5..6 {
            for cz in -5..6 {
                let mut a = clearing(cx, cz);
                let mut b = clearing(cx, cz);
                generate(&mut a, 42);
                generate(&mut b, 42);
                let fires = positions(&a);
                assert_eq!(fires, positions(&b));
                assert!(fires.len() <= 1);
                count += fires.len();
                if !fires.is_empty() {
                    for obstacle in [BlockType::Water, BlockType::OakLeaves] {
                        let mut blocked = clearing(cx, cz);
                        for bx in 0..16 {
                            for bz in 0..16 {
                                blocked.set_local(bx, 25, bz, obstacle);
                            }
                        }
                        generate(&mut blocked, 42);
                        assert!(positions(&blocked).is_empty());
                    }
                    let mut slope = clearing(cx, cz);
                    for bx in (0..16).step_by(2) {
                        for bz in 0..16 {
                            slope.set_local(bx, 24, bz, BlockType::Air);
                        }
                    }
                    generate(&mut slope, 42);
                    assert!(positions(&slope).is_empty());
                }
            }
        }
        assert!(count > 0 && count < 20);
    }
    #[test]
    fn ordinary_world_generation_produces_campfires() {
        let mut survey=String::new();
        for seed in [7,42,2026] {
        let mut world = crate::voxel::World::new(seed);
        let mut found=Vec::new();
        for cx in -6..=6 {
            for cz in -6..=6 {
                let key = (cx * 3 + 1, cz * 3 + 1);
                world.ensure_chunk_loaded(key.0, key.1);
                found.extend(positions(&world.chunks[&key]));
            }
        }
        assert!(found.len() >= 3, "too few campfires in natural terrain survey: seed {seed}, count {}",found.len());
        found.sort_by(|a,b|a.length_squared().total_cmp(&b.length_squared()));
        survey.push_str(&format!("seed={seed}: {} campfires; nearest to origin {:?}\n",found.len(),found[0]));
        }
        println!("{survey}");
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/campfire-generation.txt",survey).unwrap();
    }

    #[test]
    fn campfire_roundtrips_and_has_no_inventory_or_recipe_entry() {
        let block = BlockType::Campfire;
        assert_eq!(BlockType::from_name(block.id()), Some(block));
        let bytes = bincode::serialize(&block).unwrap();
        assert_eq!(bincode::deserialize::<BlockType>(&bytes).unwrap(), block);
        assert!(!crate::voxel::COLLECTIBLE_BLOCKS.contains(&block));
        assert!(!block.is_solid() && !block.is_opaque());
        let mut world = crate::voxel::World::new(42);
        world.ensure_chunk_loaded(0, 0);
        world.set_block(8, 24, 8, BlockType::Stone);
        world.set_block(8, 25, 8, block);
        world.unload_chunk(0, 0);
        world.ensure_chunk_loaded(0, 0);
        assert!(positions(&world.chunks[&(0, 0)]).contains(&Vec3::new(8.5, 25.0, 8.5)));
        world.set_block(8, 25, 8, BlockType::Air);
        assert!(positions(&world.chunks[&(0, 0)]).is_empty());
    }
    #[test]
    fn effects_animate_and_remain_bounded_and_lights_have_local_range() {
        let positions = vec![Vec3::ZERO; 20];
        let a = effects(&positions, Vec3::new(3.0, 2.0, 3.0), 0.0);
        let b = effects(&positions, Vec3::new(3.0, 2.0, 3.0), 1.0);
        assert_eq!(a.vertices.len(), 8 * 8 * 4);
        assert_ne!(a.vertices[2].position, b.vertices[2].position);
        assert!(a
            .vertices
            .iter()
            .all(|v| Vec3::from_array(v.position).is_finite()));
        assert_eq!(
            lights(&positions, Vec3::ZERO)
                .iter()
                .filter(|p| p[3] > 0.0)
                .count(),
            4
        );
        assert_eq!(lights(&positions, Vec3::splat(100.0)), [[0.0; 4]; 4]);
    }
}
