//! Host-only ambient population. Recycle entities without death events or loot.
use super::*;
use crate::voxel::{chunk::{world_to_chunk, CHUNK_Y}, BlockType};

impl Creatures {
    /// Natural discovery stops at this shared ceiling, including scripted residents.
    pub(super) fn has_population_room(&self) -> bool { self.ecs.len() < 128 }

    pub fn populate_wildlife(&mut self, world: &World, players: &[(PlayerId, Vec3)], dt: f32) {
        self.population_timer -= dt.max(0.0);
        if self.population_timer > 0.0 { return; }
        self.population_timer = 2.0;
        let players: Vec<_> = players.iter().copied().filter(|(_, p)| p.is_finite()).collect();
        if players.is_empty() { return; }
        let distance = |a: Vec3, b: Vec3| (a.x-b.x).powi(2)+(a.z-b.z).powi(2);
        let snapshot = self.snapshot_with_ids();
        let mut remove = Vec::new();
        self.wildlife.retain(|id, cell| {
            let Some(c) = snapshot.iter().find(|c| c.0 == *id) else { return false; };
            let kind = CreatureKind::from_u8(c.1);
            let custom = self.behaviors.get(id).is_some_and(|b|
                b.mode != BehaviorMode::Auto || b.target.is_some() || b.aggressive != kind.is_hostile());
            if custom { return false; } // Modified wildlife becomes persistent.
            if players.iter().all(|(_, p)| distance(*p, Vec3::from_array(c.2)) > 128.0*128.0) {
                remove.push((*id, *cell));
                return false;
            }
            true
        });
        for (id, cell) in remove {
            let entity = self.ecs.query::<&CreatureId>().iter().find(|(_, c)| c.0 == id).map(|(e, _)| e);
            if let Some(entity) = entity { let _ = self.ecs.despawn(entity); }
            self.behaviors.remove(&id);
            if let Some(cell) = cell { self.fish_regions.remove(&cell); }
        }
        self.population_sequence = self.population_sequence.wrapping_add(1);
        let mut rng = SimpleRng::new(world.seed as u64 ^ self.population_sequence.wrapping_mul(0x9E3779B185EBCA87));
        // Rotate player priority to avoid starving distant guests at the global cap.
        for offset in 0..players.len() {
            let (_, center) = players[(offset + self.population_sequence as usize % players.len()) % players.len()];
            let mut nearby = self.snapshot_with_ids().iter().filter(|c|
                !CreatureKind::from_u8(c.1).is_dragon() && CreatureKind::from_u8(c.1) != CreatureKind::Fish
                && distance(center, Vec3::from_array(c.2)) < 88.0*88.0).count();
            for _ in 0..2 {
                if nearby >= 20 || !self.has_population_room() { break; }
                let kind = pick_starter_kind(&mut rng);
                for _ in 0..12 {
                    let angle = rng.next_f32()*std::f32::consts::TAU;
                    let radius = 48.0 + rng.next_f32()*24.0;
                    let x = (center.x+angle.cos()*radius).floor() as i32;
                    let z = (center.z+angle.sin()*radius).floor() as i32;
                    if !world.chunks.contains_key(&world_to_chunk(x,z)) { continue; }
                    let Some(y) = (1..CHUNK_Y-4).rev().find(|&y|
                        matches!(world.get_block(x,y-1,z), BlockType::Grass|BlockType::Soil|BlockType::Sand|BlockType::Stone)
                        && (0..4).all(|dy|world.get_block(x,y+dy,z)==BlockType::Air)) else {continue;};
                    let pos = Vec3::new(x as f32+0.5,y as f32,z as f32+0.5);
                    if players.iter().any(|(_,p)|distance(*p,pos)<48.0*48.0)
                        || self.ecs.query::<&Pos>().iter().any(|(_,p)|distance(p.0,pos)<8.0*8.0) {continue;}
                    let id = self.spawn_one(kind,pos,self.population_sequence ^ x as u64 ^ (z as u64).rotate_left(32));
                    self.wildlife.insert(id,None);
                    nearby += 1;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ground(center: i32) -> World {
        let mut world = World::new(71);
        for cx in center-5..=center+5 { for cz in -5..=5 {
            let mut chunk = crate::voxel::chunk::Chunk::new(cx,cz);
            for x in 0..16 {for z in 0..16 {chunk.set_local(x,24,z,BlockType::Stone);}}
            world.chunks.insert((cx,cz),chunk);
        }}
        world
    }
    #[test]
    fn exploration_recycles_only_natural_wildlife_and_stays_bounded() {
        let mut creatures = Creatures::new();
        let scripted = creatures.spawn_one(CreatureKind::Sheep, Vec3::ZERO, 1);
        for region in 0..12 {
            let center = region*32;
            let world = ground(center);
            let player = Vec3::new(center as f32*16.0+8.0,25.0,8.0);
            for _ in 0..15 { creatures.populate_wildlife(&world,&[(7,player)],2.0); }
            let snapshot = creatures.snapshot_with_ids();
            assert!(snapshot.iter().any(|c|c.0==scripted));
            assert_eq!(creatures.wildlife.len(),if region == 0 {19} else {20});
            assert!(snapshot.len() <= 22);
            for c in snapshot.iter().filter(|c| creatures.wildlife.contains_key(&c.0)) {
                let p = Vec3::from_array(c.2);
                assert!(p.distance(player)>=48.0 && p.distance(player)<=73.0);
                assert_eq!(world.get_block(p.x.floor() as i32,24,p.z.floor() as i32),BlockType::Stone);
            }
            assert!(creatures.take_audio_events().deaths.is_empty());
        }
        let id = *creatures.wildlife.keys().next().unwrap();
        creatures.behaviors.get_mut(&id).unwrap().mode = BehaviorMode::Ignore;
        creatures.populate_wildlife(&ground(600), &[(0,Vec3::new(9608.0,25.0,8.0))],2.0);
        assert!(creatures.snapshot_with_ids().iter().any(|c|c.0==id));
        assert!(!creatures.wildlife.contains_key(&id));
    }
    #[test]
    fn discovery_respects_global_cap_and_unloaded_ground() {
        let mut creatures = Creatures::new();
        creatures.populate_wildlife(&World::new(1),&[(0,Vec3::ZERO)],2.0);
        assert!(creatures.wildlife.is_empty());
        for i in 0..128 {creatures.spawn_one(CreatureKind::Sheep,Vec3::new(-1000.0,25.0,i as f32),i);}
        creatures.populate_wildlife(&ground(0),&[(0,Vec3::new(8.0,25.0,8.0))],2.0);
        assert_eq!(creatures.ecs.len(),128);
        assert!(creatures.wildlife.is_empty());
        let old: crate::save::CraftingSave = serde_json::from_str("{}").unwrap();
        assert!(old.wildlife.is_empty());
        let mut save = crate::save::CraftingSave::default();
        save.wildlife.insert(1,Some((-3,2)));
        let restored: crate::save::CraftingSave = serde_json::from_str(&serde_json::to_string(&save).unwrap()).unwrap();
        assert_eq!(save.wildlife,restored.wildlife);
    }
    #[test]
    fn distant_guest_keeps_wildlife_and_recycled_fish_release_pool() {
        let mut creatures = Creatures::new();
        let fish = creatures.spawn_one(CreatureKind::Fish,Vec3::new(8.0,25.0,8.0),1);
        creatures.wildlife.insert(fish,Some((0,0)));
        creatures.fish_regions.insert((0,0));
        let host = (0,Vec3::new(1000.0,25.0,8.0));
        let guest = (7,Vec3::new(8.0,25.0,8.0));
        let world = World::new(1);
        creatures.populate_wildlife(&world,&[host,guest],2.0);
        assert!(creatures.wildlife.contains_key(&fish));
        creatures.populate_wildlife(&world,&[host],2.0);
        assert!(!creatures.wildlife.contains_key(&fish));
        assert!(!creatures.fish_regions.contains(&(0,0)));
        assert!(creatures.snapshot_with_ids().is_empty());
        assert!(creatures.take_audio_events().deaths.is_empty());
    }
}
