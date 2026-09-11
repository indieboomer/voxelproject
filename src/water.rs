//! Derived visual spills, not fluid simulation. Rebuilt with dirty chunk meshes.
use crate::voxel::{
    chunk::{world_to_chunk, Chunk, CHUNK_X, CHUNK_Y, CHUNK_Z},
    BlockType, World,
};
use glam::Vec3;

pub const MAX_VISIBLE_FALLS: usize = 32;
pub const LINES_PER_FALL: usize = 80;
pub const MAX_VERTICES: usize = MAX_VISIBLE_FALLS * LINES_PER_FALL * 2;

/// Shared by simulation and Lua queries, including staged block edits.
pub fn fish_spawn_clear(get: impl Fn(i32, i32, i32) -> BlockType, pos: Vec3) -> bool {
    if !pos.is_finite() || pos.abs().max_element() > 1_000_000.0 {
        return false;
    }
    for x in (pos.x - 0.7).floor() as i32..=(pos.x + 0.7).floor() as i32 {
        for y in (pos.y - 0.35).floor() as i32..=(pos.y + 0.35).floor() as i32 {
            for z in (pos.z - 0.7).floor() as i32..=(pos.z + 0.7).floor() as i32 {
                if get(x, y, z) != BlockType::Water {
                    return false;
                }
            }
        }
    }
    let (x, y, z) = (
        pos.x.floor() as i32,
        pos.y.floor() as i32,
        pos.z.floor() as i32,
    );
    (-3..=3).all(|dx| (-3..=3).all(|dz| get(x + dx, y, z + dz) == BlockType::Water))
}

pub fn falls_at(
    get: impl Fn(i32, i32, i32) -> BlockType,
    loaded: impl Fn(i32, i32) -> bool,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<Waterfall> {
    let mut falls = Vec::new();
    if !(1..CHUNK_Y - 1).contains(&y)
        || !loaded(x, z)
        || get(x, y, z) != BlockType::Water
        || (y + 1..CHUNK_Y).any(|above| get(x, above, z) != BlockType::Air)
    {
        return falls;
    }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let (nx, nz) = (x + dx, z + dz);
        if !loaded(nx, nz) || get(nx, y, nz) != BlockType::Air {
            continue;
        }
        for below in (1.max(y - 16)..y).rev() {
            match get(nx, below, nz) {
                BlockType::Air => continue,
                BlockType::Water if y - below >= 2 => {
                    falls.push(Waterfall {
                        lip: Vec3::new(
                            x as f32 + 0.5 + dx as f32 * 0.51,
                            y as f32 + 1.0,
                            z as f32 + 0.5 + dz as f32 * 0.51,
                        ),
                        bottom: below as f32 + 1.0,
                        direction: Vec3::new(dx as f32, 0.0, dz as f32),
                    });
                    break;
                }
                _ => break,
            }
        }
    }
    falls
}

#[derive(Clone, Copy, Debug)]
pub struct Waterfall {
    pub lip: Vec3,
    pub bottom: f32,
    pub direction: Vec3,
}
impl Waterfall {
    pub fn sound_position(self) -> Vec3 {
        Vec3::new(self.lip.x, (self.lip.y + self.bottom) * 0.5, self.lip.z)
    }
    /// Stateless falling streaks and short ballistic spray at the impact.
    /// Returns exactly LINES_PER_FALL segments, clipped to the water column.
    pub fn lines(self, time: f32) -> impl Iterator<Item = (Vec3, Vec3, f32)> {
        let height = self.lip.y - self.bottom;
        let tangent = Vec3::new(-self.direction.z, 0.0, self.direction.x);
        (0..LINES_PER_FALL).map(move |i| {
            let random = |salt: u32| {
                let mut n = (i as u32).wrapping_mul(747796405).wrapping_add(salt);
                n = (n ^ (n >> 16)).wrapping_mul(2246822519);
                (n & 65535) as f32 / 65535.0
            };
            let offset = tangent * (random(17) - 0.5) * 0.92;
            let phase = (time * 0.8 + random(123)).fract();
            if i < 64 {
                let y = height * phase * phase;
                let p = self.lip + offset + self.direction * 0.12 - Vec3::Y * y;
                let q = Vec3::new(p.x, p.y - (0.35 + phase * 0.7).min(p.y - self.bottom), p.z);
                (p, q, 0.48)
            } else {
                let start = Vec3::new(self.lip.x, self.bottom + 0.03, self.lip.z) + offset;
                let velocity = tangent * (random(41) - 0.5) * 1.2 + self.direction * 0.65;
                let p = start + velocity * phase + Vec3::Y * (phase * (1.0 - phase) * 1.4);
                (p, p + Vec3::Y * 0.10, 0.5 * (1.0 - phase))
            }
        })
    }
}

pub fn scan_chunk(world: &World, chunk: &Chunk) -> Vec<Waterfall> {
    let (ox, oz) = chunk.world_origin();
    let mut falls = Vec::new();
    for x in 0..CHUNK_X {
        for z in 0..CHUNK_Z {
            let Some(y) = (1..CHUNK_Y - 1)
                .rev()
                .find(|&y| chunk.get_local(x, y, z) != BlockType::Air)
            else {
                continue;
            };
            if chunk.get_local(x, y, z) == BlockType::Water {
                falls.extend(falls_at(
                    |x, y, z| world.get_block(x, y, z),
                    |x, z| world.chunks.contains_key(&world_to_chunk(x, z)),
                    ox + x,
                    y,
                    oz + z,
                ));
            }
        }
    }
    falls
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spills_require_open_drop_into_lower_water_and_loaded_neighbors() {
        let mut world = World::new(42);
        let mut upper = Chunk::new(0, 0);
        upper.set_local(15, 25, 8, BlockType::Water);
        world.chunks.insert((0, 0), upper);
        assert!(scan_chunk(&world, &world.chunks[&(0, 0)]).is_empty());
        let mut lower = Chunk::new(1, 0);
        lower.set_local(0, 18, 8, BlockType::Water);
        world.chunks.insert((1, 0), lower);
        let falls = scan_chunk(&world, &world.chunks[&(0, 0)]);
        assert_eq!(falls.len(), 1);
        for t in [0.0, 1.0, 99.0] {
            assert_eq!(falls[0].lines(t).count(), LINES_PER_FALL);
            assert!(falls[0]
                .lines(t)
                .all(|(p, q, _)| p.is_finite() && q.y >= 19.0 && p.y <= 26.0));
        }
        world
            .chunks
            .get_mut(&(1, 0))
            .unwrap()
            .set_local(0, 22, 8, BlockType::Stone);
        assert!(scan_chunk(&world, &world.chunks[&(0, 0)]).is_empty());
        world
            .chunks
            .get_mut(&(1, 0))
            .unwrap()
            .set_local(0, 22, 8, BlockType::Air);
        world
            .chunks
            .get_mut(&(0, 0))
            .unwrap()
            .set_local(15, 25, 8, BlockType::Air);
        assert!(scan_chunk(&world, &world.chunks[&(0, 0)]).is_empty());
    }
}
