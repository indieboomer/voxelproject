//! Seeded underground landmarks. Terrain is pure; rewards are initialized once by the host.
use crate::voxel::noise::column_rand;
use crate::voxel::{
    chunk::{Chunk, CHUNK_X, CHUNK_Z, TERRAIN_HEIGHT as CHUNK_Y},
    BlockType, World,
};

const SPACING: i32 = 96;
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub region: (i32, i32),
    pub x: i32,
    pub z: i32,
    pub floor: i32,
    pub dungeon: bool,
    pub open: bool,
    pub(super) entrance_floor: i32,
}
pub fn site(world: &World, rx: i32, rz: i32) -> Option<Site> {
    if !world.generation.underground || column_rand(rx, rz, world.seed, 0xCA01) > 0.68 {
        return None;
    }
    let x = rx * SPACING + 32;
    let z = rz * SPACING + 40;
    let h = [(0, 0), (-14, 0), (14, 0), (0, -10), (0, 10)]
        .into_iter()
        .map(|(dx, dz)| world.terrain_height(x + dx, z + dz))
        .min()?;
    if h < 23 || world.terrain_height(x + 30, z) - (h - 12).max(3) > 24 {
        return None;
    }
    Some(Site {
        region: (rx, rz),
        x,
        z,
        floor: (h - 12).max(3),
        dungeon: column_rand(rx, rz, world.seed, 0xCA02) < 0.5,
        open: column_rand(rx, rz, world.seed, 0xCA03) < 0.55
            && (6..=32)
                .all(|dx| world.terrain_height(x + dx, z) > crate::voxel::world::SEA_LEVEL + 1),
        entrance_floor: world.terrain_height(x + 30, z),
    })
}
impl Site {
    pub fn chest(self) -> (i32, i32, i32) {
        (self.x - 3, self.floor + 1, self.z + 2)
    }
    fn hollow(self, x: i32, y: i32, z: i32) -> bool {
        let dx = x - self.x;
        let dz = z - self.z;
        let dy = y - self.floor;
        let chamber = if self.dungeon {
            dx.abs() <= 6 && dz.abs() <= 5 && (1..=5).contains(&dy)
        } else {
            let r = (dx as f32 / 13.0).powi(2) + (dz as f32 / 9.0).powi(2);
            dy >= 1 && (dy as f32) <= 2.0 + (1.0 - r) * 6.0 && r < 1.0
        };
        // A walkable stair passage ends at the surface or behind a short excavation plug.
        let end = if self.open { 32 } else { 27 };
        let stair = self.floor + ((dx - 6).clamp(0, 24) * (self.entrance_floor - self.floor)) / 24;
        chamber || ((5..=end).contains(&dx) && dz.abs() <= 1 && y > stair && y <= stair + 3)
    }
}
pub fn carve(world: &World, chunk: &mut Chunk) {
    let (ox, oz) = chunk.world_origin();
    let Some(s) = site(world, ox.div_euclid(SPACING), oz.div_euclid(SPACING)) else {
        return;
    };
    for lx in 0..CHUNK_X {
        for lz in 0..CHUNK_Z {
            let (x, z) = (ox + lx, oz + lz);
            if (x - s.x).abs() > 33 || (z - s.z).abs() > 11 {
                continue;
            }
            let height = world.terrain_height(x, z);
            // Keep rivers and lakes sealed off from underground rooms.
            if height <= crate::voxel::world::SEA_LEVEL + 1 {
                continue;
            }
            for y in 2..CHUNK_Y - 1 {
                let block = chunk.get_local(lx, y, lz);
                if s.hollow(x, y, z) {
                    if block != BlockType::Water {
                        chunk.set_local(lx, y, lz, BlockType::Air);
                    }
                } else if y <= height && block.is_solid() {
                    let wall = [
                        (1, 0, 0),
                        (-1, 0, 0),
                        (0, 1, 0),
                        (0, -1, 0),
                        (0, 0, 1),
                        (0, 0, -1),
                    ]
                    .into_iter()
                    .any(|(a, b, c)| s.hollow(x + a, y + b, z + c));
                    if wall {
                        let material = if y >= height {
                            block
                        } else {
                            super::wall_material(world.seed, x, y, z, block, s.dungeon)
                        };
                        chunk.set_local(lx, y, lz, material);
                    }
                }
            }
            // Entrance excavation can remove a surface prop's support.
            for y in 2..CHUNK_Y {
                if chunk.get_local(lx, y, lz).def().cross
                    && !chunk.get_local(lx, y - 1, lz).is_solid()
                {
                    chunk.set_local(lx, y, lz, BlockType::Air);
                }
            }
        }
    }
}

/// Called only on the authoritative host. Persistent region markers prevent loot farming.
pub fn discover(world: &mut World, creatures: &mut crate::creature::Creatures) {
    if !world.generation.underground {
        return;
    }
    let regions: std::collections::BTreeSet<_> = world
        .chunks
        .keys()
        .map(|&(x, z)| {
            (
                (x * CHUNK_X).div_euclid(SPACING),
                (z * CHUNK_Z).div_euclid(SPACING),
            )
        })
        .collect();
    for region in regions {
        if world.underground_discovered.contains(&region) {
            continue;
        }
        let Some(s) = site(world, region.0, region.1) else {
            continue;
        };
        let p = s.chest();
        // Wait until the chamber's chunk is loaded, without generating distant chunks.
        if !world
            .chunks
            .contains_key(&crate::voxel::chunk::world_to_chunk(p.0, p.2))
        {
            continue;
        }
        if world.automation.devices.len() >= crate::automation::balance().max_devices {
            continue;
        }
        world.underground_discovered.insert(s.region);
        if world.get_block(p.0, p.1, p.2) != BlockType::Air {
            continue;
        }
        let mut chest = crate::automation::Device::new(crate::automation::Kind::Chest, p, 0);
        chest.items.insert("resource:copper_ore".into(), 8);
        chest.items.insert("resource:gold_ore".into(), 3);
        chest.items.insert("resource:crystal".into(), 2);
        world.automation.devices.insert(p, chest);
        for dx in [0, 3] {
            let pos = glam::Vec3::new(
                (s.x + dx) as f32 + 0.5,
                (s.floor + 1) as f32,
                s.z as f32 + 0.5,
            );
            if world.get_block(s.x + dx, s.floor + 1, s.z) == BlockType::Air
                && world.get_block(s.x + dx, s.floor + 2, s.z) == BlockType::Air
            {
                creatures.spawn_one(
                    if dx == 0 {
                        crate::creature::CreatureKind::SkeletonSorcerer
                    } else {
                        crate::creature::CreatureKind::Skeleton
                    },
                    pos,
                    (world.seed ^ (s.x as u32) ^ (dx as u32)) as u64,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn old_world(seed: u32) -> World {
        let mut w = World::new(seed);
        w.generation.cave_version = 0;
        w
    }
    #[test]
    fn terrain_matches_reverse_chunk_order_and_preserves_excavation() {
        let mut a = old_world(42);
        let mut b = old_world(42);
        let s = (-5..5)
            .flat_map(|x| (-5..5).map(move |z| (x, z)))
            .find_map(|(x, z)| site(&a, x, z).filter(|s| s.open))
            .unwrap();
        let (cx, cz) = crate::voxel::chunk::world_to_chunk(s.x, s.z);
        let coords: Vec<_> = (-1..=2)
            .flat_map(|dx| (-1..=1).map(move |dz| (cx + dx, cz + dz)))
            .collect();
        for &(x, z) in &coords {
            a.ensure_chunk_loaded(x, z);
        }
        for &(x, z) in coords.iter().rev() {
            b.ensure_chunk_loaded(x, z);
        }
        for x in s.x - 14..=s.x + 32 {
            for z in s.z - 10..=s.z + 10 {
                for y in 0..CHUNK_Y {
                    assert_eq!(a.get_block(x, y, z), b.get_block(x, y, z));
                }
            }
        }
        assert_eq!(
            a.get_block(s.x + 30, s.entrance_floor + 1, s.z),
            BlockType::Air
        );
        assert!(a.get_block(s.x + 30, s.entrance_floor, s.z).is_solid());
        for dx in 6..=30 {
            let y = s.floor + ((dx - 6) * (s.entrance_floor - s.floor)) / 24;
            assert_eq!(a.get_block(s.x + dx, y + 1, s.z), BlockType::Air);
            assert_eq!(a.get_block(s.x + dx, y + 2, s.z), BlockType::Air);
        }
        a.set_block(s.x, s.floor, s.z, BlockType::Air);
        a.unload_chunk(cx, cz);
        a.ensure_chunk_loaded(cx, cz);
        assert_eq!(a.get_block(s.x, s.floor, s.z), BlockType::Air);
    }
    #[test]
    fn chambers_have_loot_enriched_walls_and_do_not_restock_after_unloading() {
        let mut world = old_world(42);
        let mut kinds = [false; 2];
        let mut entrances = [false; 2];
        for x in -5..5 {
            for z in -5..5 {
                let Some(s) = site(&world, x, z) else {
                    continue;
                };
                kinds[usize::from(s.dungeon)] = true;
                entrances[usize::from(s.open)] = true;
                let (cx, cz) = crate::voxel::chunk::world_to_chunk(s.x, s.z);
                for dx in -1..=2 {
                    for dz in -1..=1 {
                        world.ensure_chunk_loaded(cx + dx, cz + dz);
                    }
                }
                assert_eq!(world.get_block(s.x, s.floor + 2, s.z), BlockType::Air);
                assert!(world.get_block(s.x, s.floor, s.z).is_solid());
                let mut creatures = crate::creature::Creatures::new();
                discover(&mut world, &mut creatures);
                assert!(world.automation.devices.contains_key(&s.chest()));
                world.automation.devices.remove(&s.chest());
                world.unload_chunk(cx, cz);
                world.ensure_chunk_loaded(cx, cz);
                discover(&mut world, &mut creatures);
                assert!(!world.automation.devices.contains_key(&s.chest()));
            }
        }
        assert_eq!(kinds, [true, true]);
        assert_eq!(entrances, [true, true]);
    }
}
