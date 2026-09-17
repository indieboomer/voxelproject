//! Simple upright bodies, voxel stepping, and host-side body separation.
use super::*;

pub(super) fn body(kind: CreatureKind) -> (f32, f32) {
    match kind {
        CreatureKind::DragonGreen | CreatureKind::DragonRed => (4.1, 5.0),
        CreatureKind::StoneGolem => (0.95, 2.8),
        CreatureKind::Cow => (0.9, 1.6),
        CreatureKind::Sheep | CreatureKind::Wolf => (0.7, 1.2),
        CreatureKind::Chicken => (0.45, 0.8),
        // Include the insectoid's forward-reaching attack pose.
        CreatureKind::Stinger => (0.8, 1.2),
        CreatureKind::Fish => (0.8, 0.7),
        _ => (0.55, 1.8),
    }
}

fn solid(world: &World, x: i32, y: i32, z: i32) -> bool {
    if world
        .chunks
        .contains_key(&crate::voxel::chunk::world_to_chunk(x, z))
    {
        world.is_solid(x, y, z)
    } else {
        y <= world.terrain_height(x, z)
    }
}

fn clear(world: &World, pos: Vec3, kind: CreatureKind) -> bool {
    let (r, h) = body(kind);
    let bottom = if kind == CreatureKind::Fish {
        pos.y - h * 0.5
    } else {
        pos.y
    };
    for x in (pos.x - r).floor() as i32..=(pos.x + r - 0.001).floor() as i32 {
        for z in (pos.z - r).floor() as i32..=(pos.z + r - 0.001).floor() as i32 {
            for y in (bottom + 0.001).floor() as i32..=(bottom + h - 0.001).floor() as i32 {
                if solid(world, x, y, z) {
                    return false;
                }
            }
        }
    }
    true
}

pub(super) fn ground_move(world: &World, start: Vec3, delta: Vec3, kind: CreatureKind) -> Vec3 {
    let mut pos = start;
    let steps = (delta.length() / 0.2).ceil().clamp(1.0, 128.0) as usize;
    let delta = delta / steps as f32;
    for _ in 0..steps {
        for axis in [Vec3::X * delta.x, Vec3::Z * delta.z] {
            if axis.length_squared() == 0.0 {
                continue;
            }
            let next = pos + axis;
            if clear(world, next, kind) {
                pos = next;
            } else {
                let raised = Vec3::new(next.x, pos.y.floor() + 1.0, next.z);
                if clear(world, raised, kind) {
                    pos = raised;
                }
            }
        }
    }
    // Settle onto actual blocks, including placed blocks and excavated terrain.
    for _ in 0..crate::voxel::chunk::CHUNK_Y {
        let below = Vec3::new(pos.x, pos.y.ceil() - 1.0, pos.z);
        if below.y < 0.0 || !clear(world, below, kind) {
            break;
        }
        pos = below;
    }
    pos
}

impl Creatures {
    pub(super) fn separate_bodies(&mut self, world: &World, players: &[(PlayerId, Vec3)]) {
        let mut bodies: Vec<_> = self
            .ecs
            .query::<(&Pos, &Kind, &CreatureId)>()
            .iter()
            .map(|(e, (p, k, id))| (e, p.0, k.0, id.0))
            .collect();
        bodies.sort_by_key(|b| b.3);
        for _ in 0..6 {
            for i in 0..bodies.len() {
                let (_, mut p, kind, _) = bodies[i];
                let (r, h) = body(kind);
                let bottom = if kind == CreatureKind::Fish {
                    p.y - h * 0.5
                } else {
                    p.y
                };
                let obstacles = players.iter().map(|(_, p)| (*p, 0.3, 1.8)).chain(
                    bodies
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .map(|(_, b)| {
                            let (r, h) = body(b.2);
                            let mut p = b.1;
                            if b.2 == CreatureKind::Fish {
                                p.y -= h * 0.5;
                            }
                            (p, r, h)
                        }),
                );
                for (other, radius, height) in obstacles {
                    if bottom >= other.y + height || bottom + h <= other.y {
                        continue;
                    }
                    let delta = Vec3::new(p.x - other.x, 0.0, p.z - other.z);
                    let distance = delta.length();
                    if distance >= r + radius {
                        continue;
                    }
                    let direction = if distance > 0.0001 {
                        delta / distance
                    } else if bodies[i].3 % 2 == 0 {
                        -Vec3::X
                    } else {
                        Vec3::X
                    };
                    let next = p + direction * (r + radius - distance + 0.001);
                    if clear(world, next, kind) {
                        p = next;
                    }
                }
                bodies[i].1 = p;
            }
        }
        for (e, p, _, _) in bodies {
            if let Ok(mut pos) = self.ecs.get::<&mut Pos>(e) {
                pos.0 = p;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> World {
        let mut w = World::new(71);
        w.generation.shape = crate::worldgen::Shape::Flat;
        w.generation.relief = 0;
        w.generation.trees = 0;
        w
    }
    #[test]
    fn ground_bodies_step_blocks_but_cannot_cross_walls() {
        let mut w = world();
        for cx in -1..=1 {
            for cz in -1..=1 {
                w.ensure_chunk_loaded(cx, cz);
            }
        }
        for z in -3..=3 {
            for y in 26..30 {
                w.set_block(3, y, z, crate::voxel::BlockType::Stone);
            }
        }
        let start = Vec3::new(0.5, 26.0, 0.5);
        let end = ground_move(&w, start, Vec3::X * 8.0, CreatureKind::Sheep);
        assert!(end.x < 2.41, "{end:?}");
        w.set_block(1, 26, 0, crate::voxel::BlockType::Stone);
        let end = ground_move(&w, start, Vec3::X, CreatureKind::Chicken);
        assert_eq!(end.y, 27.0);
    }
    #[test]
    fn all_species_separate_from_players_and_each_other() {
        let w = world();
        for id in 0..=12 {
            let kind = CreatureKind::from_u8(id);
            let p = Vec3::new(0.5, 40.0, 0.5);
            let mut c = Creatures::new();
            c.spawn_one(kind, p, 1);
            c.spawn_one(kind, p, 2);
            c.separate_bodies(&w, &[(0, p)]);
            let snapshot = c.snapshot_with_ids();
            let a = Vec3::from_array(snapshot[0].2);
            let b = Vec3::from_array(snapshot[1].2);
            assert!(a.distance(p) >= body(kind).0 + 0.3);
            assert!(b.distance(p) >= body(kind).0 + 0.3);
            assert!(a.distance(b) >= body(kind).0 * 2.0, "{kind:?}");
        }
    }
    #[test]
    fn natural_spawns_respect_species_settings() {
        let mut w = world();
        w.generation.creatures.insert("sheep".into(), 1000);
        w.generation.creatures.insert("cow".into(), 0);
        let mut rng = SimpleRng::new(7);
        let mut sheep = 0;
        for _ in 0..10000 {
            let kind = pick_world_kind(&mut rng, &w).unwrap();
            assert_ne!(kind, CreatureKind::Cow);
            if kind == CreatureKind::Sheep {
                sheep += 1;
            }
        }
        assert!(sheep > 7500);
        for species in crate::worldgen::CREATURE_SPECIES {
            w.generation.creatures.insert((*species).into(), 0);
        }
        assert!(pick_world_kind(&mut rng, &w).is_none());
        let mut c = Creatures::new();
        c.spawn_around(&w, Vec3::ZERO, 30, 7);
        assert!(c.snapshot_with_ids().is_empty());
    }
}
