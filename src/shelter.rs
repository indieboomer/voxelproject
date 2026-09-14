//! Edited, loaded roof columns shared by precipitation and mesh skylight.
use crate::voxel::{
    chunk::{world_to_chunk, CHUNK_Y},
    mesher::MeshData,
    World,
};
use glam::Vec3;
use std::collections::HashMap;

#[derive(Default)]
pub struct Roofs(HashMap<(i32, i32), f32>);
impl Roofs {
    pub fn height(&mut self, world: &World, x: i32, z: i32) -> f32 {
        *self.0.entry((x, z)).or_insert_with(|| {
            // Unknown columns must not leak precipitation into unloaded terrain.
            let Some(chunk) = world.chunks.get(&world_to_chunk(x, z)) else {
                return CHUNK_Y as f32;
            };
            (0..chunk.stored_height())
                .rev()
                .find(|&y| world.get_block(x, y, z).is_solid())
                .map_or(0., |y| (y + 1) as f32)
        })
    }
    pub fn covered(&mut self, world: &World, p: Vec3) -> bool {
        p.y < self.height(world, p.x.floor() as i32, p.z.floor() as i32) - 0.01
    }
    pub fn rain_segment(
        &mut self,
        world: &World,
        x: f32,
        z: f32,
        bottom: f32,
        top: f32,
    ) -> Option<(f32, f32)> {
        let bottom = bottom.max(self.height(world, x.floor() as i32, z.floor() as i32));
        (top > bottom).then_some((bottom, top))
    }
    pub fn shade(&mut self, world: &World, mesh: &mut MeshData) {
        for v in &mut mesh.vertices {
            let p = Vec3::from_array(v.position) + Vec3::from_array(v.normal) * 0.02;
            // AO remains intact for local lights; the vertex shader unpacks shelter.
            if self.covered(world, p) {
                v.ao = -v.ao.abs();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{chunk::Chunk, BlockType};
    #[test]
    fn roofs_clip_rain_and_edits_reopen_the_column() {
        let mut w = World::new(1);
        w.chunks.insert((0, 0), Chunk::new(0, 0));
        w.set_block(3, 10, 3, BlockType::Glass);
        let mut roofs = Roofs::default();
        assert_eq!(roofs.rain_segment(&w, 3.5, 3.5, 5., 9.), None);
        assert_eq!(roofs.rain_segment(&w, 3.5, 3.5, 10., 12.), Some((11., 12.)));
        assert!(roofs.covered(&w, Vec3::new(3.5, 5., 3.5)));
        assert_eq!(roofs.rain_segment(&w, 4.5, 3.5, 5., 9.), Some((5., 9.)));
        w.set_block(3, 10, 3, BlockType::Air);
        assert_eq!(
            Roofs::default().rain_segment(&w, 3.5, 3.5, 5., 9.),
            Some((5., 9.))
        );
    }
}
