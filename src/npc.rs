//! Seeded traveler encounters, discovered as players explore a 100-block world grid.
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
    spawn_near(
        world,
        center,
        (site_hash(world.seed, (0, 0)) % 6) as u8,
        true,
    )
    .into_iter()
    .collect()
}

fn spawn_near(world: &mut World, center: Vec3, kind: u8, starting: bool) -> Option<Npc> {
    crate::crafting::load_interaction_area(world, center);
    let angle = kind as f32 * std::f32::consts::TAU / 6.;
    let desired = center
        + if starting {
            Vec3::new(angle.cos() * 9., 0., angle.sin() * 9.)
        } else {
            Vec3::ZERO
        };
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
                    let ground = world.get_block(x, y - 1, z);
                    if ground == crate::voxel::BlockType::Water {
                        break;
                    }
                    if !ground.is_solid() {
                        continue;
                    }
                    if adventure::clear_feet(world, p)
                        && (!starting || (y as f32 - center.y).abs() < 12.)
                        && !matches!(
                            world.get_block(x, y - 1, z),
                            crate::voxel::BlockType::OakLeaves
                                | crate::voxel::BlockType::BirchLeaves
                                | crate::voxel::BlockType::CherryLeaves
                                | crate::voxel::BlockType::SpruceLeaves
                        )
                    {
                        found = Some(p);
                        break 'search;
                    }
                    // Never search underneath a canopy, roof, water or obstructed surface.
                    break;
                }
            }
        }
    }
    if let Some(home) = found {
        return Some(Npc {
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
    None
}

pub const SPACING: i32 = 100;
pub const ACTIVE_RADIUS: f32 = 140.;
pub const MAX_VISIBLE: usize = 32;
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Distribution {
    pub origin: Option<Cell>,
    pub visited: std::collections::BTreeSet<(i32, i32)>,
}
fn site_hash(seed: u32, region: (i32, i32)) -> u64 {
    let mut n = u64::from(seed)
        ^ (region.0 as i64 as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (region.1 as i64 as u64).wrapping_mul(0xbf58476d1ce4e5b9);
    n = n.wrapping_add(0x9e3779b97f4a7c15);
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}
impl Distribution {
    pub fn initialize(&mut self, world: &mut World, npcs: &mut Vec<Npc>, center: Vec3) {
        if self.origin.is_some_and(crate::automation::valid_cell) {
            return;
        }
        self.origin = Some(adventure::cell(center));
        self.visited.clear();
        self.visited.insert((0, 0));
        // Migrate the old six-person starting group without touching player quest progress.
        npcs.clear();
        npcs.extend(populate(world, center));
    }
    fn site(&self, seed: u32, region: (i32, i32)) -> Vec3 {
        let origin = self.origin.unwrap();
        let h = site_hash(seed, region);
        Vec3::new(
            (origin.0 + region.0 * SPACING + (h % 21) as i32 - 10) as f32,
            origin.1 as f32,
            (origin.2 + region.1 * SPACING + ((h >> 8) % 21) as i32 - 10) as f32,
        )
    }
    /// At most one terrain search per call. All players contribute, including distant guests.
    pub fn discover(&mut self, world: &mut World, npcs: &mut Vec<Npc>, players: &[Vec3]) {
        let Some(origin) = self.origin else {
            return;
        };
        let mut candidates = Vec::new();
        for player in players
            .iter()
            .filter(|p| p.is_finite() && p.abs().max_element() < 999_000.)
        {
            let rx = ((player.x - origin.0 as f32) / SPACING as f32).round() as i32;
            let rz = ((player.z - origin.2 as f32) / SPACING as f32).round() as i32;
            for x in rx - 1..=rx + 1 {
                for z in rz - 1..=rz + 1 {
                    let region = (x, z);
                    if self.visited.contains(&region) {
                        continue;
                    }
                    let site = self.site(world.seed, region);
                    let delta = site - *player;
                    let distance = delta.x * delta.x + delta.z * delta.z;
                    if distance < 90. * 90. {
                        candidates.push((distance, region, site));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        if let Some((_, region, site)) = candidates.first().copied() {
            self.visited.insert(region);
            if site.x.abs() > 998_900. || site.z.abs() > 998_900. {
                return;
            }
            if let Some(npc) = spawn_near(
                world,
                site,
                (site_hash(world.seed, region) % 6) as u8,
                false,
            ) {
                npcs.push(npc);
            }
        }
    }
}
pub fn nearby(npcs: &[Npc], player: Vec3) -> Vec<Npc> {
    let mut found: Vec<_> = npcs
        .iter()
        .filter(|n| {
            Vec3::from_array(n.position).distance_squared(player) < ACTIVE_RADIUS * ACTIVE_RADIUS
        })
        .cloned()
        .collect();
    found.sort_by(|a, b| {
        Vec3::from_array(a.position)
            .distance_squared(player)
            .total_cmp(&Vec3::from_array(b.position).distance_squared(player))
    });
    found.truncate(MAX_VISIBLE);
    found
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
    fn flat_world() -> World {
        let mut world = World::new(42);
        for cx in -10..35 {
            for cz in -4..5 {
                let mut chunk = crate::voxel::chunk::Chunk::new(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set_local(x, 39, z, crate::voxel::BlockType::Stone);
                    }
                }
                world.chunks.insert((cx, cz), chunk);
            }
        }
        world
    }
    #[test]
    fn exploration_spaces_encounters_and_reload_preserves_them_without_duplicates() {
        let mut world = flat_world();
        let start = Vec3::new(8.5, 40., 8.5);
        let mut distribution = Distribution::default();
        let mut npcs = Vec::new();
        distribution.initialize(&mut world, &mut npcs, start);
        assert_eq!(npcs.len(), 1);
        let original_kind = npcs[0].kind;
        for x in [-100., 100., 200., 300., 400.] {
            // Host remains at camp while a distant guest explores.
            for _ in 0..12 {
                distribution.discover(&mut world, &mut npcs, &[start, start + Vec3::X * x]);
            }
        }
        assert_eq!(npcs.len(), 6);
        assert!(npcs
            .iter()
            .all(|n| n.valid() && adventure::clear_feet(&world, n.home)));
        let mut homes: Vec<_> = npcs.iter().map(|n| n.home.0).collect();
        homes.sort();
        assert!(
            homes
                .windows(2)
                .all(|p| (60..=140).contains(&(p[1] - p[0]))),
            "{homes:?}"
        );
        let mut save = crate::save::CraftingSave::default();
        save.npcs = npcs;
        save.npc_distribution = distribution;
        let mut restored: crate::save::CraftingSave =
            serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
        restored.npc_distribution.initialize(
            &mut world,
            &mut restored.npcs,
            start + Vec3::X * 200.,
        );
        assert_eq!(
            restored.npc_distribution.origin,
            save.npc_distribution.origin
        );
        for x in [-100., 100., 200., 300., 400.] {
            restored.npc_distribution.discover(
                &mut world,
                &mut restored.npcs,
                &[start + Vec3::X * x],
            );
        }
        assert_eq!(restored.npcs.len(), 6);
        assert_eq!(restored.npcs[0].kind, original_kind);
        assert!(nearby(&restored.npcs, start).len() < restored.npcs.len());
        let guest = nearby(&restored.npcs, start + Vec3::X * 400.);
        assert!(!guest.is_empty());
        assert!(guest.iter().all(|n| n.home.0 > 200));
    }
    #[test]
    fn legacy_starting_group_migrates_once_and_seeded_roles_vary() {
        let mut world = flat_world();
        let start = Vec3::new(8.5, 40., 8.5);
        let npc = populate(&mut world, start).remove(0);
        let mut old = vec![npc; 6];
        let mut distribution = Distribution::default();
        distribution.initialize(&mut world, &mut old, start);
        assert_eq!(old.len(), 1);
        let original = old[0].home;
        distribution.initialize(&mut world, &mut old, start + Vec3::X * 300.);
        assert_eq!(old[0].home, original);
        let kinds: std::collections::BTreeSet<_> =
            (-30..30).map(|x| site_hash(42, (x, 2)) % 6).collect();
        assert_eq!(kinds.len(), 6);
        assert_ne!(site_hash(42, (2, -3)), site_hash(43, (2, -3)));
    }
    #[test]
    fn one_starting_traveler_spawns_on_dry_ground_and_patrols() {
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
        assert_eq!(npcs.len(), 1);
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
