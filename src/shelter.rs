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
            chunk.roof_height(x.rem_euclid(16), z.rem_euclid(16)) as f32
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
            // Keep contact AO independent from light arriving through openings.
            if self.covered(world, p) {
                v.skylight = skylight(world, p);
            }
        }
    }
}

/// Static skylight floods through air, fading by two levels per block away from sky.
/// Eight halo cells cover its full reach, so adjacent chunks agree at their seams.
pub struct SkyField {
    origin: (i32, i32),
    height: usize,
    levels: Vec<u8>,
}
impl SkyField {
    fn build(world: &World, cx: i32, cz: i32) -> Self {
        let origin = (cx * 16 - 8, cz * 16 - 8);
        let height = (world
            .chunks
            .iter()
            .filter(|((x, z), _)| (x - cx).abs() <= 1 && (z - cz).abs() <= 1)
            .map(|(_, c)| c.stored_height())
            .max()
            .unwrap_or(1)
            + 1)
        .min(CHUNK_Y) as usize;
        let mut levels = vec![255u8; 32 * 32 * height];
        for z in 0..32 {
            for x in 0..32 {
                let wx = origin.0 + x as i32;
                let wz = origin.1 + z as i32;
                let Some(c) = world.chunks.get(&world_to_chunk(wx, wz)) else {
                    continue;
                };
                let lx = wx.rem_euclid(16);
                let lz = wz.rem_euclid(16);
                let sky = c.sky_height(lx, lz);
                for y in 0..height {
                    levels[(y * 32 + z) * 32 + x] = if y as i32 >= sky {
                        15
                    } else if c.get_local(lx, y as i32, lz).is_opaque() {
                        255
                    } else {
                        0
                    };
                }
            }
        }
        let mut queue = std::collections::VecDeque::new();
        let neighbors = |i: usize| {
            let x = i % 32;
            let z = (i / 32) % 32;
            let y = i / 1024;
            [
                (x > 0).then(|| i - 1),
                (x < 31).then(|| i + 1),
                (z > 0).then(|| i - 32),
                (z < 31).then(|| i + 32),
                (y > 0).then(|| i - 1024),
                (y + 1 < height).then(|| i + 1024),
            ]
        };
        for i in 0..levels.len() {
            if levels[i] == 15 && neighbors(i).into_iter().flatten().any(|j| levels[j] == 0) {
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            let next = levels[i].saturating_sub(2);
            if next == 0 {
                continue;
            }
            for j in neighbors(i).into_iter().flatten() {
                if levels[j] < next {
                    levels[j] = next;
                    queue.push_back(j);
                }
            }
        }
        // The halo is needed only during propagation, not in the resident cache.
        let mut interior = Vec::with_capacity(16 * 16 * height);
        for y in 0..height {
            for z in 8..24 {
                interior.extend_from_slice(&levels[(y * 32 + z) * 32 + 8..(y * 32 + z) * 32 + 24]);
            }
        }
        Self {
            origin: (cx * 16, cz * 16),
            height,
            levels: interior,
        }
    }
    fn sample(&self, x: i32, y: i32, z: i32) -> f32 {
        if y < 0 {
            return 0.;
        }
        if y as usize >= self.height {
            return 1.;
        }
        let x = (x - self.origin.0) as usize;
        let z = (z - self.origin.1) as usize;
        let n = self.levels[(y as usize * 16 + z) * 16 + x];
        if n == 255 {
            0.
        } else {
            (n as f32 / 15.).powi(2)
        }
    }
}
pub fn skylight(world: &World, p: Vec3) -> f32 {
    let x = p.x.floor() as i32;
    let z = p.z.floor() as i32;
    let (cx, cz) = world_to_chunk(x, z);
    let Some(c) = world.chunks.get(&(cx, cz)) else {
        return 0.;
    };
    let mut cache = c.sky_cache.borrow_mut();
    cache
        .get_or_insert_with(|| SkyField::build(world, cx, cz))
        .sample(x, p.y.floor() as i32, z)
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
    #[test]
    fn skylight_fades_through_door_and_edit_invalidates_cached_room() {
        let mut w = World::new(1);
        w.chunks.insert((0, 0), Chunk::new(0, 0));
        for x in 1..15 {
            for z in 1..15 {
                for y in 0..6 {
                    if x == 1 || x == 14 || z == 1 || z == 14 || y == 0 || y == 5 {
                        w.set_block(x, y, z, BlockType::Stone);
                    }
                }
            }
        }
        assert_eq!(skylight(&w, Vec3::new(8.5, 2.5, 8.5)), 0.);
        for y in 1..5 {
            w.set_block(7, y, 1, BlockType::Air);
        }
        let near = skylight(&w, Vec3::new(7.5, 2.5, 2.5));
        let far = skylight(&w, Vec3::new(7.5, 2.5, 7.5));
        assert!(near > far && near > 0. && near < 1.);
        for y in 1..5 {
            w.set_block(7, y, 1, BlockType::Stone);
        }
        assert_eq!(skylight(&w, Vec3::new(7.5, 2.5, 2.5)), 0.);
        w.set_block(8, 5, 8, BlockType::Air);
        assert_eq!(skylight(&w, Vec3::new(8.5, 2.5, 8.5)), 1.);
    }
}
