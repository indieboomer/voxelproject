//! Connected room graphs, spiral entrances and persistent host-owned rewards.
#[path = "underground_legacy.rs"]
mod legacy;
use crate::voxel::noise::column_rand;
use crate::voxel::{
    chunk::{Chunk, CHUNK_X, TERRAIN_HEIGHT as CHUNK_Y, CHUNK_Z},
    BlockType, World,
};
#[cfg(test)]
pub use legacy::Site;
use std::sync::Arc;
const SIZE: i32 = 96;
const FLOOR: i32 = 3;
type Cell = (i32, i32, i32);
pub struct Layout {
    origin: (i32, i32),
    pub mouth: Cell,
    rooms: Vec<Cell>,
    dungeon: bool,
    compact: bool,
    corridor_width: i32,
    corridor_height: i32,
    air: Vec<bool>,
    steps: Vec<Cell>,
}
impl Layout {
    fn index(x: i32, y: i32, z: i32) -> Option<usize> {
        ((0..SIZE).contains(&x) && (2..CHUNK_Y).contains(&y) && (0..SIZE).contains(&z))
            .then_some(((x.max(0) * SIZE + z.max(0)) * CHUNK_Y + y.max(0)) as usize)
    }
    fn hollow(&self, x: i32, y: i32, z: i32) -> bool {
        Self::index(x - self.origin.0, y, z - self.origin.1).is_some_and(|i| self.air[i])
    }
    fn clear(&mut self, x: i32, y: i32, z: i32) {
        if let Some(i) = Self::index(x - self.origin.0, y, z - self.origin.1) {
            self.air[i] = true;
        }
    }
    fn passage(&mut self, p: Cell) {
        self.passage_size(p, if self.compact { 1 } else { 3 }, 4);
    }
    fn passage_size(&mut self, p: Cell, width: i32, height: i32) {
        let start = -(width - 1) / 2;
        for dx in start..start + width {
            for dz in start..start + width {
                for dy in 1..=height {
                    self.clear(p.0 + dx, p.1 + dy, p.2 + dz);
                }
            }
        }
    }
    fn connect(&mut self, a: Cell, b: Cell) {
        let mut p = a;
        self.passage_size(p, self.corridor_width, self.corridor_height);
        while p.0 != b.0 {
            p.0 += (b.0 - p.0).signum();
            self.passage_size(p, self.corridor_width, self.corridor_height);
        }
        while p.2 != b.2 {
            p.2 += (b.2 - p.2).signum();
            self.passage_size(p, self.corridor_width, self.corridor_height);
        }
    }
}
fn plan(world:&World,rx:i32,rz:i32)->Option<(Vec<Cell>,usize,Cell)> {
    let ox = rx * SIZE;
    let oz = rz * SIZE;
    let rooms: Vec<_> = (0..3)
        .flat_map(|x| (0..3).map(move |z| (ox + 20 + x * 28, FLOOR, oz + 20 + z * 28)))
        .collect();
    // Search nine possible entrances instead of rejecting an entire region on one roll.
    let entry = rooms
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let min = [(-8, -8), (-8, 8), (8, -8), (8, 8), (0, 0)]
                .into_iter()
                .map(|(x, z)| world.terrain_height(p.0 + x, p.2 + z))
                .min()
                .unwrap();
            (min > crate::voxel::world::SEA_LEVEL + 1).then_some((i, min))
        })
        .max_by_key(|&(i, h)| {
            (
                h,
                (column_rand(rx + i as i32, rz, world.seed, 0xDA01) * 1000.) as i32,
            )
        })?
        .0;
    let center = rooms[entry];
    let mouth = (
        center.0 + 6,
        world.terrain_height(center.0 + 6, center.2 - 6),
        center.2 - 6,
    );
    Some((rooms,entry,mouth))
}
/// Analytic map markers; never allocate cave layouts or load chunks.
pub fn entrances(world:&World,min:(i32,i32),max:(i32,i32))->Vec<Cell> {
    if !world.generation.underground {return Vec::new();}
    let mut result=Vec::new();
    for x in min.0.div_euclid(SIZE)..=max.0.div_euclid(SIZE) {for z in min.1.div_euclid(SIZE)..=max.1.div_euclid(SIZE) {
        let mouth=if world.generation.cave_version==0 {legacy::site(world,x,z).filter(|s|s.open).map(|s|(s.x+30,s.entrance_floor,s.z))}
            else {plan(world,x,z).map(|p|p.2)};
        if let Some(p)=mouth.filter(|p|p.0>=min.0&&p.0<=max.0&&p.2>=min.1&&p.2<=max.1) {result.push(p);}
    }}result
}
fn layout(world: &World, rx: i32, rz: i32) -> Option<Arc<Layout>> {
    if !world.generation.underground {
        return None;
    }
    if let Some(l) = world.cave_layouts.borrow().get(&(rx, rz)) {
        return Some(l.clone());
    }
    let (rooms,entry,mouth)=plan(world,rx,rz)?;
    let (ox,oz)=(rx*SIZE,rz*SIZE);
    let center=rooms[entry];
    let mut l = Layout {
        origin: (ox, oz),
        mouth,
        rooms,
        dungeon: column_rand(rx, rz, world.seed, 0xDA02) < 0.5,
        compact: world.generation.cave_version >= 2,
        corridor_width: if world.generation.cave_version >= 2 {
            1 + (column_rand(rx, rz, world.seed, 0xDA06) * 2.) as i32
        } else { 3 },
        corridor_height: if world.generation.cave_version >= 2 {
            3 + (column_rand(rx, rz, world.seed, 0xDA07) * 2.) as i32
        } else { 4 },
        air: vec![false; (SIZE * SIZE * CHUNK_Y) as usize],
        steps: Vec::new(),
    };
    for i in 0..l.rooms.len() {
        let p = l.rooms[i];
        if l.compact {
            let width = 4 + (column_rand(p.0, p.2, world.seed, 0xDA03) * 2.) as i32;
            let depth = 4 + (column_rand(p.0, p.2, world.seed, 0xDA08) * 2.) as i32;
            let height = 3 + (column_rand(p.0, p.2, world.seed, 0xDA09) * 2.) as i32;
            let x0 = -(width - 1) / 2;
            let z0 = -(depth - 1) / 2;
            for x in x0..x0 + width {
                for z in z0..z0 + depth {
                    // Natural chambers retain a low, uneven ceiling; dungeons are level.
                    let corner = (x == x0 || x == x0 + width - 1)
                        && (z == z0 || z == z0 + depth - 1);
                    let ceiling = if !l.dungeon && corner { 3 } else { height };
                    for y in 1..=ceiling { l.clear(p.0 + x, p.1 + y, p.2 + z); }
                }
            }
            continue;
        }
        let r = if i == entry {
            3
        } else {
            5 + (column_rand(p.0, p.2, world.seed, 0xDA03) * 3.) as i32
        };
        for x in -r..=r {
            for z in -r..=r {
                let radius = (x * x + z * z) as f32 / (r * r) as f32;
                let ceiling = if l.dungeon {
                    6
                } else {
                    (8.0 - radius * 4.0) as i32
                };
                if l.dungeon || radius < 1.0 {
                    for y in 1..=ceiling {
                        l.clear(p.0 + x, p.1 + y, p.2 + z);
                    }
                }
            }
        }
    }
    // Randomized depth-first spanning tree: all nine rooms are reachable.
    let mut seen = [false; 9];
    seen[entry] = true;
    let mut stack = vec![entry];
    let mut edges = Vec::new();
    while let Some(&i) = stack.last() {
        let (x, z) = (i / 3, i % 3);
        let mut choices: Vec<_> = (0..9)
            .filter(|&j| !seen[j] && (x.abs_diff(j / 3) + z.abs_diff(j % 3) == 1))
            .collect();
        choices.sort_by_key(|&j| {
            (column_rand(
                i as i32,
                j as i32,
                world.seed ^ (rx as u32) ^ (rz as u32),
                0xDA04,
            ) * 1_000_000.) as u32
        });
        if let Some(&j) = choices.first() {
            l.connect(l.rooms[i], l.rooms[j]);
            edges.push((i.min(j), i.max(j)));
            seen[j] = true;
            stack.push(j);
        } else {
            stack.pop();
        }
    }
    // A loop offers an alternate route; each system has at least 224 blocks of corridor.
    let (a, b) = (0usize..9)
        .flat_map(|a| (a + 1..9).map(move |b| (a, b)))
        .find(|&(a, b)| {
            a.div_euclid(3).abs_diff(b / 3) + a.rem_euclid(3).abs_diff(b % 3) == 1
                && !edges.contains(&(a, b))
        })
        .unwrap();
    l.connect(l.rooms[a], l.rooms[b]);
    // A compact spiral descends one block per two horizontal steps to near bedrock.
    let steps = (mouth.1 - FLOOR) * 2;
    let mut end = mouth;
    for step in 0..=steps {
        let t = step % 48;
        let (x, z) = match t {
            0..=11 => (6, -6 + t),
            12..=23 => (6 - (t - 12), 6),
            24..=35 => (-6, 6 - (t - 24)),
            _ => (-6 + (t - 36), -6),
        };
        end = (center.0 + x, mouth.1 - step / 2, center.2 + z);
        l.passage(end);
        l.steps.push(end);
    }
    let final_leg = steps % 48;
    let arrived_along_z = (1..=12).contains(&final_leg) || (25..=36).contains(&final_leg);
    if l.compact && !arrived_along_z {
        // Turn inward before heading to the room: retracing the final stair run
        // would put its solid treads across the ceiling of a one-block passage.
        let inward = (end.0, end.1, center.2);
        l.connect(end, inward);
        l.connect(inward, center);
    } else {
        l.connect(end, center);
    }
    // Clear overhead foliage at the mouth so the start is visible from the surface.
    let approach_radius = if l.compact { 0 } else { 2 };
    for dx in -approach_radius..=approach_radius {
        for dz in -3..=1 {
            for y in mouth.1 + 1..CHUNK_Y {
                l.clear(mouth.0 + dx, y, mouth.2 + dz);
            }
        }
    }
    if l.compact {
        // One-block approach stays level through the three-block-wide gate.
        for dz in -3..0 { l.steps.push((mouth.0, mouth.1, mouth.2 + dz)); }
    }
    for &(x, y, z) in &l.steps {
        if let Some(i) = Layout::index(x - ox, y, z - oz) {
            l.air[i] = false;
        }
    }
    let l = Arc::new(l);
    let mut cache = world.cave_layouts.borrow_mut();
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert((rx, rz), l.clone());
    Some(l)
}
pub fn nearby_entrance(world: &World, pos: glam::Vec3) -> Option<Cell> {
    if !world.generation.underground || world.generation.cave_version == 0 {
        return None;
    }
    let rx = (pos.x.floor() as i32).div_euclid(SIZE);
    let rz = (pos.z.floor() as i32).div_euclid(SIZE);
    (-1..=1)
        .flat_map(|dx| (-1..=1).map(move |dz| (rx + dx, rz + dz)))
        .filter_map(|(x, z)| layout(world, x, z).map(|l| l.mouth))
        .min_by_key(|p| ((p.0 as f32 - pos.x).powi(2) + (p.2 as f32 - pos.z).powi(2)) as u32)
}
#[cfg(test)]
pub fn site(world: &World, rx: i32, rz: i32) -> Option<Site> {
    if world.generation.cave_version == 0 {
        return legacy::site(world, rx, rz);
    }
    let l = layout(world, rx, rz)?;
    let p = l.rooms[4];
    Some(Site {
        region: (rx, rz),
        x: p.0,
        z: p.2,
        floor: p.1,
        dungeon: l.dungeon,
        open: true,
        entrance_floor: l.mouth.1,
    })
}
pub fn carve(world: &World, chunk: &mut Chunk) {
    if world.generation.cave_version == 0 {
        legacy::carve(world, chunk);
        return;
    }
    let (ox, oz) = chunk.world_origin();
    let Some(l) = layout(world, ox.div_euclid(SIZE), oz.div_euclid(SIZE)) else {
        return;
    };
    for x in 0..CHUNK_X {
        for z in 0..CHUNK_Z {
            let (wx, wz) = (ox + x, oz + z);
            let height = world.terrain_height(wx, wz);
            let step_heights: Vec<_> = l
                .steps
                .iter()
                .filter(|p| p.0 == wx && p.2 == wz)
                .map(|p| p.1)
                .collect();
            for y in 2..CHUNK_Y {
                if step_heights.contains(&y) {
                    chunk.set_local(x, y, z, BlockType::Stone);
                } else if l.hollow(wx, y, wz) {
                    chunk.set_local(x, y, z, BlockType::Air);
                } else {
                    let shell = [
                        (1, 0, 0),
                        (-1, 0, 0),
                        (0, 1, 0),
                        (0, -1, 0),
                        (0, 0, 1),
                        (0, 0, -1),
                    ]
                    .into_iter()
                    .any(|(a, b, c)| l.hollow(wx + a, y + b, wz + c));
                    if shell && y < height {
                        let block = wall_material(world.seed, wx, y, wz,
                            chunk.get_local(x, y, z), l.dungeon);
                        chunk.set_local(x, y, z, block);
                    } else if shell && y <= 10 {
                        // A solid lining keeps deep passages dry beneath low riverbeds.
                        chunk.set_local(x, y, z, BlockType::Stone);
                    }
                }
            }
            for y in 2..CHUNK_Y {
                if chunk.get_local(x, y, z).def().cross && !chunk.get_local(x, y - 1, z).is_solid()
                {
                    chunk.set_local(x, y, z, BlockType::Air);
                }
            }
            // Keep trees and surface props from hiding the doorway and its approach.
            if (wx - l.mouth.0).abs() <= 5 && (-8..=1).contains(&(wz - l.mouth.2)) {
                for y in height + 1..CHUNK_Y {
                    chunk.set_local(x, y, z, BlockType::Air);
                }
            }
            // Stone doorway marks every entrance without sealing the walking opening.
            let dx = wx - l.mouth.0;
            let dz = wz - l.mouth.2;
            let gate_radius = if l.compact { 1 } else { 3 };
            let gate_height = if l.compact { 4 } else { 5 };
            if dz == -2 && dx.abs() <= gate_radius {
                for y in l.mouth.1 + 1..=(l.mouth.1 + gate_height).min(CHUNK_Y - 1) {
                    if dx.abs() == gate_radius || y == l.mouth.1 + gate_height {
                        chunk.set_local(x, y, z, BlockType::Cobblestone);
                    }
                }
            }
        }
    }
}

/// Keep deposits exposed by excavation; the old lining erased every resource
/// except copper and gold. World-coordinate patches also cross chunk seams.
fn wall_material(seed: u32, x: i32, y: i32, z: i32, original: BlockType, dungeon: bool) -> BlockType {
    let material = wall_deposit(seed, x, y, z, original, dungeon);
    // Keep half the deposits, using a separate roll so every resource retains
    // its relative rarity and depth limits. Include natural ore exposed by caves.
    // Roll per two-block patch to retain small clusters rather than loose specks.
    if (material.id().ends_with("_ore") || matches!(material,
        BlockType::Coal | BlockType::Sulfur | BlockType::RockSalt | BlockType::Quartz |
        BlockType::Amethyst | BlockType::Moonstone | BlockType::Amber | BlockType::Crystal))
        && crate::voxel::noise::block_rand(x.div_euclid(2), y.div_euclid(2), z.div_euclid(2), seed, 0xDA09) >= 0.5
    {
        if dungeon { BlockType::Bricks } else { BlockType::Stone }
    } else { material }
}
fn wall_deposit(seed: u32, x: i32, y: i32, z: i32, original: BlockType, dungeon: bool) -> BlockType {
    use BlockType::*;
    if original.id().ends_with("_ore") || matches!(original,
        Coal | Sulfur | RockSalt | Quartz | Amethyst | Moonstone | Amber | Crystal |
        Clay | Limestone | Marble | Granite | Obsidian | Basalt) {
        return original;
    }
    let fallback = if dungeon { Bricks } else { Stone };
    // Two-block cells give small patches; thinning their edges avoids solid
    // checkerboard deposits. The additional retention roll above halves this
    // original 14% density to approximately 7% of otherwise plain lining.
    let roll = crate::voxel::noise::block_rand(x.div_euclid(2), y.div_euclid(2), z.div_euclid(2), seed, 0xDA06);
    if roll >= 0.20 || crate::voxel::noise::block_rand(x,y,z,seed,0xDA07) > 0.7 { return fallback; }
    let kind = crate::voxel::noise::block_rand(x.div_euclid(2), y.div_euclid(2), z.div_euclid(2), seed, 0xDA08);
    match kind {
        r if r < 0.23 => IronOre,
        r if r < 0.43 => CopperOre,
        r if r < 0.61 => Coal,
        r if r < 0.72 => TinOre,
        r if r < 0.78 => SilverOre,
        r if r < 0.83 => GoldOre,
        r if r < 0.87 => Sulfur,
        r if r < 0.91 => RockSalt,
        r if r < 0.95 => Quartz,
        _ if y > 16 => Quartz,
        r if r < 0.965 => EmeraldOre,
        r if r < 0.975 => Amethyst,
        r if r < 0.983 => SapphireOre,
        r if r < 0.991 => RubyOre,
        _ if y > 10 => SilverOre,
        r if r < 0.995 => DiamondOre,
        r if r < 0.998 => MithrilOre,
        _ => Moonstone,
    }
}
pub fn discover(world: &mut World, creatures: &mut crate::creature::Creatures) {
    if world.generation.cave_version == 0 {
        legacy::discover(world, creatures);
        return;
    }
    let regions: std::collections::BTreeSet<_> = world
        .chunks
        .keys()
        .map(|&(x, z)| {
            (
                (x * CHUNK_X).div_euclid(SIZE),
                (z * CHUNK_Z).div_euclid(SIZE),
            )
        })
        .collect();
    for (rx, rz) in regions {
        let Some(l) = layout(world, rx, rz) else {
            continue;
        };
        for i in [0, 4, 8] {
            let marker = (rx * 3 + i / 4, rz);
            if world.underground_discovered.contains(&marker) {
                continue;
            }
            let room = l.rooms[i as usize];
            let p = (room.0 - 1, room.1 + 1, room.2 + 1);
            if !world
                .chunks
                .contains_key(&crate::voxel::chunk::world_to_chunk(p.0, p.2))
                || world.automation.devices.len() >= crate::automation::balance().max_devices
            {
                continue;
            }
            world.underground_discovered.insert(marker);
            if world.get_block(p.0, p.1, p.2) != BlockType::Air {
                continue;
            }
            let mut chest = crate::automation::Device::new(crate::automation::Kind::Chest, p, 0);
            chest
                .items
                .insert("resource:copper_ore".into(), 8 + i as u32);
            chest.items.insert("resource:gold_ore".into(), 3);
            chest.items.insert("resource:crystal".into(), 2);
            world.automation.devices.insert(p, chest);
            for dx in [0, if l.compact { 1 } else { 3 }] {
                let p = (room.0 + dx, room.1 + 1, room.2);
                if world.get_block(p.0, p.1, p.2) == BlockType::Air
                    && world.get_block(p.0, p.1 + 1, p.2) == BlockType::Air
                {
                    creatures.spawn_one(
                        if dx == 0 { crate::creature::CreatureKind::SkeletonSorcerer } else { crate::creature::CreatureKind::Skeleton },
                        glam::Vec3::new(p.0 as f32 + 0.5, p.1 as f32, p.2 as f32 + 0.5),
                        world.seed as u64 ^ p.0 as u64,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cave_resource_deposits_are_halved_without_changing_common_stone() {
        use BlockType::*;
        for block in [IronOre, CopperOre, Coal, TinOre, SilverOre, GoldOre, Sulfur,
            RockSalt, Quartz, EmeraldOre, Amethyst, SapphireOre, RubyOre, DiamondOre,
            MithrilOre, Moonstone, Amber, Crystal] {
            for dungeon in [false, true] {
                let retained = (-100..100).flat_map(|x| (-100..100).map(move |z| (x*2,z*2)))
                    .filter(|&(x,z)| wall_material(42,x,6,z,block,dungeon)==block).count();
                assert!((19000..21000).contains(&retained), "{block:?}: {retained}/40000");
            }
        }
        for block in [Clay, Limestone, Marble, Granite, Obsidian, Basalt] {
            for x in -100..100 { assert_eq!(wall_material(42,x,6,0,block,false),block); }
        }
    }
    use std::collections::{HashSet, VecDeque};
    #[test]
    fn cave_lining_preserves_deposits_and_keeps_rare_ores_deep() {
        use BlockType::*;
        for block in [IronOre, TinOre, Coal, SilverOre, DiamondOre, MithrilOre, Quartz, Sulfur] {
            for dungeon in [false,true] {
                assert_eq!(wall_deposit(42, 4, 6, -8, block, dungeon), block);
            }
        }
        let mut counts = std::collections::HashMap::<&str,usize>::new();
        for x in -100..100 { for z in -100..100 { for y in [6, 22] {
            let block = wall_material(42, x, y, z, Stone, false);
            if y == 22 { assert!(!matches!(block, DiamondOre | MithrilOre | RubyOre | SapphireOre | EmeraldOre)); }
            *counts.entry(block.id()).or_default() += 1;
        }}}
        for block in [IronOre, CopperOre, Coal, TinOre, SilverOre, GoldOre, Sulfur, RockSalt,
            Quartz, EmeraldOre, Amethyst, SapphireOre, RubyOre, DiamondOre, MithrilOre, Moonstone] {
            assert!(counts.get(block.id()).copied().unwrap_or(0) > 0, "missing {block:?}");
        }
        assert!(counts[IronOre.id()] > counts[MithrilOre.id()] * 20);
    }

    #[test]
    fn generated_cave_walls_expose_a_variety_and_mined_ore_stays_mined() {
        use BlockType::*;
        for seed in [7,42,2026] {
            let mut world = World::new(seed);
            let mut found = HashSet::new();
            let mut ore_cell = None;
            for rx in -1..=1 { for rz in -1..=1 {
                let Some(l) = layout(&world,rx,rz) else { continue; };
                for &(cx,cy,cz) in &l.rooms {
                    for x in cx-4..=cx+4 { for z in cz-4..=cz+4 { for y in cy..=cy+5 {
                        if l.hollow(x,y,z) || ![(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)]
                            .into_iter().any(|(a,b,c)| l.hollow(x+a,y+b,z+c)) { continue; }
                        world.ensure_chunk_loaded(x.div_euclid(CHUNK_X), z.div_euclid(CHUNK_Z));
                        let block = world.get_block(x,y,z);
                        found.insert(block.id());
                        if block == IronOre { ore_cell = Some((x,y,z)); }
                    }}}
                }
            }}
            for block in [CopperOre, IronOre, Coal, TinOre, SilverOre, GoldOre] {
                assert!(found.contains(block.id()), "seed {seed} missing exposed {block:?}");
            }
            let (x,y,z) = ore_cell.unwrap();
            world.set_block(x,y,z,Air);
            world.unload_chunk(x.div_euclid(CHUNK_X),z.div_euclid(CHUNK_Z));
            world.ensure_chunk_loaded(x.div_euclid(CHUNK_X),z.div_euclid(CHUNK_Z));
            assert_eq!(world.get_block(x,y,z),Air);
        }
    }
    #[test]
    fn entrances_are_common_and_every_room_is_reachable_from_the_surface() {
        for seed in [7, 42, 2026] {
            let mut world = World::new(seed);
            let mut count = 0;
            let mut nearest = i32::MAX;
            for rx in -2..=2 {
                for rz in -2..=2 {
                    let Some(l) = layout(&world, rx, rz) else {
                        continue;
                    };
                    count += 1;
                    nearest = nearest.min(l.mouth.0.abs().max(l.mouth.2.abs()));
                    assert!((1..=2).contains(&l.corridor_width));
                    assert!((3..=4).contains(&l.corridor_height));
                    for room in &l.rooms {
                        // The entrance chamber also joins the spiral; measure the other rooms
                        // off the corridor axes so connected doorways don't inflate their width.
                        if (room.0 + 6, room.2 - 6) == (l.mouth.0, l.mouth.2) { continue; }
                        let width = (-4..=4).filter(|dx| l.hollow(room.0 + dx, FLOOR + 1, room.2 - 1)).count();
                        let depth = (-4..=4).filter(|dz| l.hollow(room.0 - 1, FLOOR + 1, room.2 + dz)).count();
                        assert!((4..=5).contains(&width), "room width {width}");
                        assert!((4..=5).contains(&depth), "room depth {depth}");
                        assert!(l.hollow(room.0, FLOOR + 3, room.2));
                        assert!(!l.hollow(room.0, FLOOR + 5, room.2));
                    }
                    assert!(l
                        .rooms
                        .iter()
                        .any(|p| (p.0 - l.mouth.0).abs() + (p.2 - l.mouth.2).abs() >= 50));

                    for x in 0..6 {
                        for z in 0..6 {
                            world.ensure_chunk_loaded(rx * 6 + x, rz * 6 + z);
                        }
                    }
                    let start = (l.mouth.0, l.mouth.1 + 1, l.mouth.2);
                    for dy in 1..=3 {
                        assert_eq!(world.get_block(l.mouth.0, l.mouth.1 + dy, l.mouth.2 - 2), BlockType::Air);
                        for dx in [-1, 1] {
                            assert_eq!(world.get_block(l.mouth.0 + dx, l.mouth.1 + dy, l.mouth.2 - 2), BlockType::Cobblestone);
                        }
                    }
                    for dx in -1..=1 {
                        assert_eq!(world.get_block(l.mouth.0 + dx, l.mouth.1 + 4, l.mouth.2 - 2), BlockType::Cobblestone);
                    }
                    let walkable = |p: Cell| {
                        world.get_block(p.0, p.1 - 1, p.2).is_solid()
                            && world.get_block(p.0, p.1, p.2) == BlockType::Air
                            && world.get_block(p.0, p.1 + 1, p.2) == BlockType::Air
                    };
                    assert!(walkable(start), "blocked mouth {seed} {start:?}");
                    let mut reached = HashSet::from([start]);
                    let mut queue = VecDeque::from([start]);
                    while let Some(p) = queue.pop_front() {
                        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            for dy in -1..=1 {
                                let q = (p.0 + dx, p.1 + dy, p.2 + dz);
                                if q.0 < rx * SIZE
                                    || q.0 >= (rx + 1) * SIZE
                                    || q.2 < rz * SIZE
                                    || q.2 >= (rz + 1) * SIZE
                                {
                                    continue;
                                }
                                if walkable(q)
                                    && world.get_block(q.0, p.1.max(q.1) + 1, q.2) == BlockType::Air
                                    && reached.insert(q)
                                {
                                    queue.push_back(q);
                                }
                            }
                        }
                    }
                    for room in &l.rooms {
                        assert!(
                            reached.contains(&(room.0, room.1 + 1, room.2)),
                            "unreachable room seed={seed} mouth={start:?} room={room:?}"
                        );
                    }
                    let mut creatures = crate::creature::Creatures::new();
                    discover(&mut world, &mut creatures);
                    let guardians = creatures.snapshot_with_ids();
                    assert!(guardians.iter().any(|entry| entry.1 == crate::creature::CreatureKind::SkeletonSorcerer.to_u8()));
                    assert!(guardians.iter().filter(|entry| entry.1 == crate::creature::CreatureKind::SkeletonSorcerer.to_u8()).all(|entry| entry.3 == 54.0));
                    let p = (l.rooms[4].0 - 1, FLOOR + 1, l.rooms[4].2 + 1);
                    assert!(world.automation.devices.remove(&p).is_some());
                    discover(&mut world, &mut creatures);
                    assert_eq!(creatures.snapshot_with_ids().len(), guardians.len());
                    assert!(!world.automation.devices.contains_key(&p));
                }
            }
            println!("seed={seed}: {count}/25 regions have open systems; nearest entrance within {nearest} blocks per axis");
            assert!(count >= 15, "too sparse: seed={seed}, {count}/25");
            assert!(nearest <= 150);
        }
    }
}
