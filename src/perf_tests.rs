//! Shaped-mode cost breakdown (run manually, in the profile you play with):
//! `cargo test perf_breakdown -- --ignored --nocapture` (dev build) or with
//! `--release`. Compares shaping off and on on the same generated terrain.
use crate::voxel::{mesher::{build_chunk_mesh, build_chunk_mesh_with, Detail}, World};
use glam::Vec3;
use std::time::Instant;

#[test]
#[ignore = "timing report, run manually"]
fn perf_breakdown() {
    for shaping in [false, true] {
        let mut world = World::new(42);
        for x in -5..=5 {
            for z in -5..=5 {
                world.ensure_chunk_loaded(x, z);
            }
        }
        world.terrain_shaping = shaping;
        // Whole render radius: meshing cost and the GPU's triangle load.
        let start = Instant::now();
        let (mut triangles, mut vertices) = (0, 0);
        // As the game meshes them: full detail within two chunks, far beyond.
        for (key, chunk) in &world.chunks {
            let detail = if shaping && (key.0.abs() > 2 || key.1.abs() > 2) { Detail::Far } else { Detail::Full };
            let mesh = build_chunk_mesh_with(&world, chunk, detail);
            triangles += mesh.indices.len() / 3;
            vertices += mesh.vertices.len();
        }
        let mesh_ms = start.elapsed().as_secs_f64() * 1000.0 / world.chunks.len() as f64;
        // Triangles by feature, classified from vertex attributes.
        let mut kinds = std::collections::BTreeMap::<&str, (usize, usize)>::new();
        for chunk in world.chunks.values() {
            let mesh = build_chunk_mesh(&world, chunk);
            for tri in mesh.indices.chunks(3) {
                let v = &mesh.vertices[tri[0] as usize];
                let kind = if (v.tex_layer - crate::voxel::natural::NATURAL_FOLIAGE).abs() < 1e-3 {
                    "foliage clumps"
                } else if v.tex_layer > 0.01 && v.tex_layer < 0.1 {
                    "leaf cubes"
                } else if v.wind > 0.0 {
                    "plant cards and tufts"
                } else if v.color == [0.95; 3] {
                    "tree tubes"
                } else if v.normal[1] > 0.01 && v.normal[1] < 0.999 {
                    "shaped/sloped tops"
                } else {
                    "cube faces and side bands"
                };
                let e = kinds.entry(kind).or_default();
                e.0 += 1;
            }
            let distinct: std::collections::HashSet<[u32; 3]> = mesh.vertices.iter().map(|v| v.position.map(f32::to_bits)).collect();
            kinds.entry("~ (distinct vertex positions)").or_default().1 += distinct.len();
        }
        println!("  shaping={shaping} triangles by feature: {kinds:?}");
        // Per-frame work that reads shapes: player physics, aim ray, creatures.
        let feet = Vec3::new(8.5, world.terrain_height(8, 8) as f32 + 1.0, 8.5);
        let mut player = crate::player::Player::new(feet);
        let input = crate::input::Input::new();
        let start = Instant::now();
        for _ in 0..100 {
            player.update(&world, &input, Vec3::X, Vec3::Z, 1.0 / 60.0);
        }
        let player_ms = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
        let start = Instant::now();
        for i in 0..100 {
            let dir = Vec3::new((i as f32 * 0.1).cos(), -0.6, (i as f32 * 0.1).sin());
            let _ = crate::raycast::raycast(&world, feet + Vec3::Y * 1.6, dir, 6.0);
        }
        let ray_ms = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
        let mut creatures: Vec<([f32; 3], u8, f32, u8, f32)> = (0..40)
            .map(|i| {
                let (x, z) = (i * 7 % 160 - 80, i * 13 % 160 - 80);
                ([x as f32 + 0.5, world.terrain_height(x, z) as f32 + 1.0, z as f32 + 0.5], 1, 0.0, 0, 0.0)
            })
            .collect();
        let start = Instant::now();
        for _ in 0..20 {
            crate::creature::settle_on_shaped_terrain(&world, &mut creatures);
        }
        let creature_ms = start.elapsed().as_secs_f64() * 1000.0 / 20.0;
        println!(
            "shaping={shaping}: mesh {mesh_ms:.1} ms/chunk; {} chunks -> {triangles} triangles, {vertices} vertices ({:.1} MB); \
             per frame: player {player_ms:.3} ms, aim ray {ray_ms:.3} ms, 40 creatures {creature_ms:.3} ms",
            world.chunks.len(),
            vertices as f64 * std::mem::size_of::<crate::voxel::mesher::Vertex>() as f64 / 1e6
        );
    }
}
