//! Six peaceful quest givers, simulated by the host with bounded, collision-aware patrols.
use crate::{
    adventure::{self, Cell},
    voxel::World,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Npc {
    pub kind: u8,
    pub home: Cell,
    pub position: [f32; 3],
    pub facing: f32,
    pub walking: bool,
    pub phase: f32,
    pub wait: f32,
    pub step: u32,
    pub target: Option<Cell>,
}
impl Npc {
    pub fn valid(&self) -> bool {
        self.kind < 6
            && Vec3::from_array(self.position).is_finite()
            && self.facing.is_finite()
            && self.phase.is_finite()
            && self.wait.is_finite()
            && Vec3::from_array(self.position).distance(adventure::feet(self.home)) < 16.
            && self.home.0.unsigned_abs() < 1_000_000
            && self.home.2.unsigned_abs() < 1_000_000
            && (1..crate::voxel::chunk::CHUNK_Y - 2).contains(&self.home.1)
            && self
                .target
                .is_none_or(|p| adventure::feet(p).distance(adventure::feet(self.home)) < 10.)
    }
    pub fn tick(&mut self, world: &World, players: &[Vec3], dt: f32) {
        let dt = dt.clamp(0., 0.1);
        self.phase = (self.phase + dt) % 1000.;
        self.walking = false;
        let mut pos = Vec3::from_array(self.position);
        if let Some(p) = players.iter().find(|p| p.distance_squared(pos) < 9.) {
            let d = *p - pos;
            self.facing = d.z.atan2(d.x);
            return;
        }
        // Builders can obstruct a patrol; never walk through their blocks.
        if !adventure::clear_feet(world, adventure::cell(pos)) {
            self.target = None;
            return;
        }
        self.wait = (self.wait - dt).max(0.);
        if self.wait > 0. {
            return;
        }
        if let Some(target) = self.target {
            if !adventure::clear_feet(world, target) {
                self.target = None;
                self.wait = 1.;
                return;
            }
            let end = adventure::feet(target);
            let delta = end - pos;
            self.facing = delta.z.atan2(delta.x);
            if delta.length() < dt * 0.8 {
                pos = end;
                self.target = None;
                self.wait = 1.5 + (self.step % 3) as f32;
            } else {
                pos += delta.normalize_or_zero() * dt * 0.8;
                self.walking = true;
            }
            self.position = pos.to_array();
            return;
        }
        let p = adventure::cell(pos);
        self.step = self.step.wrapping_add(1);
        for i in 0..4 {
            let (dx, dz) = [(1, 0), (0, 1), (-1, 0), (0, -1)]
                [((self.step / 3 + self.kind as u32 + i) % 4) as usize];
            let next = (p.0 + dx, p.1, p.2 + dz);
            // A flat cardinal step keeps the complete body on supported, dry ground.
            if (next.0 - self.home.0).abs() <= 4
                && (next.2 - self.home.2).abs() <= 4
                && adventure::clear_feet(world, next)
                && players
                    .iter()
                    .all(|p| p.distance(adventure::feet(next)) > 1.2)
            {
                self.target = Some(next);
                return;
            }
        }
        self.wait = 2.;
    }
}
pub fn populate(world: &mut World, center: Vec3) -> Vec<Npc> {
    crate::crafting::load_interaction_area(world, center);
    let mut result: Vec<Npc> = Vec::new();
    for kind in 0..6u8 {
        let angle = kind as f32 * std::f32::consts::TAU / 6.;
        let desired = center + Vec3::new(angle.cos() * 9., 0., angle.sin() * 9.);
        let mut found = None;
        'search: for radius in 0i32..=18 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if dx.abs().max(dz.abs()) != radius {
                        continue;
                    }
                    let x = desired.x.floor() as i32 + dx;
                    let z = desired.z.floor() as i32 + dz;
                    world.ensure_chunk_loaded(x.div_euclid(16), z.div_euclid(16));
                    for y in (1..crate::voxel::chunk::CHUNK_Y - 2).rev() {
                        let p = (x, y, z);
                        if adventure::clear_feet(world, p)
                            && (y as f32 - center.y).abs() < 12.
                            && !matches!(
                                world.get_block(x, y - 1, z),
                                crate::voxel::BlockType::OakLeaves
                                    | crate::voxel::BlockType::BirchLeaves
                                    | crate::voxel::BlockType::CherryLeaves
                                    | crate::voxel::BlockType::SpruceLeaves
                            )
                            && result.iter().all(|n| {
                                Vec3::from_array(n.position).distance(adventure::feet(p)) > 4.
                            })
                        {
                            found = Some(p);
                            break 'search;
                        }
                    }
                }
            }
        }
        if let Some(home) = found {
            result.push(Npc {
                kind,
                home,
                position: adventure::feet(home).to_array(),
                facing: angle + std::f32::consts::PI,
                walking: false,
                phase: kind as f32,
                wait: kind as f32,
                step: 0,
                target: None,
            });
        }
    }
    result
}
pub fn can_talk(world: &World, npc: &Npc, feet: Vec3) -> bool {
    let eye = feet + Vec3::Y * 1.62;
    let target = Vec3::from_array(npc.position) + Vec3::Y;
    let distance = eye.distance(target);
    feet.is_finite()
        && distance < 6.
        && crate::raycast::raycast(world, eye, target - eye, (distance - 0.4).max(0.)).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_six_spawn_on_dry_ground_and_patrol_without_crossing_obstructions() {
        let mut world = World::new(42);
        let center = Vec3::new(8.5, 40., 8.5);
        for x in -16..32 {
            for z in -16..32 {
                world.set_block(x, 39, z, crate::voxel::BlockType::Stone);
                for y in 40..44 {
                    world.set_block(x, y, z, crate::voxel::BlockType::Air);
                }
            }
        }
        let mut npcs = populate(&mut world, center);
        assert_eq!(npcs.len(), 6);
        let before = npcs.iter().map(|n| n.position).collect::<Vec<_>>();
        for _ in 0..1000 {
            for npc in &mut npcs {
                npc.tick(&world, &[], 0.1);
                assert!(npc.valid());
                assert!(adventure::clear_feet(
                    &world,
                    adventure::cell(Vec3::from_array(npc.position))
                ));
            }
        }
        assert!(npcs.iter().zip(before).any(|(n, p)| n.position != p));
        let n = &mut npcs[0];
        let feet = Vec3::from_array(n.position) + Vec3::X * 2.;
        let p = n.position;
        n.tick(&world, &[feet], 0.1);
        assert_eq!(n.position, p);
        assert!(!n.walking);
        assert!(can_talk(&world, n, feet));
        assert!(!can_talk(&world, n, feet + Vec3::X * 20.));
        let saved: Vec<Npc> = serde_json::from_slice(&serde_json::to_vec(&npcs).unwrap()).unwrap();
        assert!(saved.iter().all(Npc::valid));
    }
}
