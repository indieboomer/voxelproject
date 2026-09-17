use glam::Vec3;
use crate::voxel::{BlockType, World};
/// Deterministic wild hive locations: a small subset of mature oak trees.
pub fn nearby(world: &World, center: Vec3) -> Vec<Vec3> {
    let cx = center.x.floor() as i32;
    let cz = center.z.floor() as i32;
    let mut out = Vec::new();
    for x in (cx - 48)..=(cx + 48) { for z in (cz - 48)..=(cz + 48) {
        let y = world.terrain_height(x, z);
        let trunk_kind = world.get_block(x, y + 1, z);
        let (trunk, leaves) = match trunk_kind {
            BlockType::OakWood => (true, BlockType::OakLeaves),
            BlockType::BirchWood => (true, BlockType::BirchLeaves),
            BlockType::CherryWood => (true, BlockType::CherryLeaves),
            _ => (false, BlockType::Air),
        };
        let leaf = (-2..=2).flat_map(|dx| (-2..=2).map(move |dz| (dx, dz)))
            .find(|(dx, dz)| world.get_block(x + dx, y + 3, z + dz) == leaves);
        // Roughly one hive for every five mature oak, birch, or cherry trees.
        // The first leaf found is replaced visually by the hive model.
        // found is replaced visually by the hive model at the canopy edge.
        if trunk && leaf.is_some()
            && crate::voxel::noise::block_rand(x, y, z, world.seed, 0xbee5) < 0.20 {
            let (dx, dz) = leaf.unwrap();
            out.push(Vec3::new(x as f32 + dx as f32 + 0.5, y as f32 + 3.0, z as f32 + dz as f32 + 0.5));
        }
    }}
    out
}
pub fn particles(positions: &[Vec3], time: f32) -> crate::voxel::mesher::MeshData {
    let mut mesh = crate::voxel::mesher::MeshData { vertices: vec![], indices: vec![] };
    for (hive, &origin) in positions.iter().enumerate() {
        for i in 0..3 {
            let phase = time * 1.8 + hive as f32 * 1.7 + i as f32 * 2.1;
            let pos = origin + Vec3::new(phase.cos() * 0.55, 0.12 + (phase * 1.7).sin() * 0.12, phase.sin() * 0.4);
            let size = Vec3::splat(0.035);
            crate::voxel::mesher::push_cuboid(&mut mesh.vertices, &mut mesh.indices, pos - size, pos + size, [0.95, 0.7, 0.08], crate::voxel::atlas::white_uv());
        }
    }
    mesh
}
pub fn key(pos: Vec3) -> u64 { ((pos.x.to_bits() as u64) << 32) ^ pos.z.to_bits() as u64 }
