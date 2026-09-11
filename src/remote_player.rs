use std::collections::HashMap;
use std::time::Instant;

use glam::{Vec3, Vec3Swizzles};

use crate::net::PlayerId;
use crate::voxel::atlas::white_uv;
use crate::voxel::mesher::{push_cuboid, MeshData, Vertex};

const CRYSTAL_MARKER_COLOR: [f32; 3] = [0.55, 0.85, 0.95];

/// Host-selected combination. Model/hat indices are zero-based; None is bareheaded.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Appearance {
    pub model: u8,
    pub hat: Option<u8>,
}

impl Appearance {
    pub fn choose(seed: u32, used: impl IntoIterator<Item = Self>) -> Self {
        let used: Vec<_> = used.into_iter().collect();
        let available: Vec<_> = (0..20).map(|i| Self {
            model: i / 5,
            hat: if i % 5 == 0 { None } else { Some(i % 5 - 1) },
        }).filter(|a| !used.contains(a)).collect();
        assert!(!available.is_empty(), "all player appearances occupied");
        available[seed as usize % available.len()]
    }
}
pub struct RemotePlayer {
    pub appearance: Appearance,
    pub animation_started: Instant,
    pub held: Option<crate::equipment::Entry>,
    pub pos: Vec3,
    pub yaw: f32,
    pub carrying_crystal: bool,
    pub last_seen: Instant,
    /// Approximated on the host from consecutive `PlayerState` updates
    /// (position delta / time since the last one) since the network
    /// protocol doesn't carry a client's real physics velocity -- good
    /// enough for exposing `speed`/`running` to Lua rules via `api.players`.
    pub velocity: Vec3,
    /// Only meaningful on the host, which learns it from the client's
    /// `Hello` and uses it to format chat/join messages -- a joined
    /// client's own `remote_players` entries leave this empty since they
    /// never need it (no in-world name tags yet).
    pub nickname: String,
    pub display_name: String,
    /// The host's authoritative record of this player's health/poison/
    /// movement-attribute state, set via the World API (`api.damage_player`
    /// et al) and the poison DoT timer, then broadcast to everyone in the
    /// same `Snapshot` `carrying_crystal` already rides. Like `nickname`,
    /// only meaningful on the host -- a joined client's entries for *other*
    /// players are never read for anything beyond mesh rendering, since
    /// each client only applies these to its own local `Player`.
    pub health: f32,
    pub poisoned: bool,
    pub speed_multiplier: f32,
    pub jump_multiplier: f32,
    /// Same host-authoritative story as `health` above, updated every
    /// frame by `App`'s oxygen pass (drains while submerged, regenerates
    /// otherwise -- see `player::OXYGEN_DRAIN_PER_SEC`/`_REGEN_PER_SEC`).
    pub oxygen: f32,
}

impl RemotePlayer {
    /// Constructs a fresh entry with the same defaults `Player::new` uses --
    /// full health, unpoisoned, 1.0 multipliers, full oxygen -- so a newly
    /// joined player starts identically whether the host is looking at its
    /// own `Player` or this `RemotePlayer` record of someone else's.
    pub fn new(pos: Vec3, yaw: f32, carrying_crystal: bool, nickname: String) -> Self {
        Self {
            appearance: Appearance::default(),
            animation_started: Instant::now(),
            held:None,
            pos,
            yaw,
            carrying_crystal,
            last_seen: Instant::now(),
            velocity: Vec3::ZERO,
            display_name:nickname.clone(),
            nickname,
            health: crate::player::MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: crate::player::MAX_OXYGEN,
        }
    }
}

/// Builds a mesh for every tracked remote player except `exclude` (the
/// local player, if it appears in the same map).
pub fn build_mesh(players: &HashMap<PlayerId, RemotePlayer>, exclude: PlayerId, models: &crate::model::Models) -> MeshData {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let uv = white_uv();
    for (&id, rp) in players.iter() {
        if id == exclude {
            continue;
        }
        let held=crate::held_item::mesh(rp.held,rp.pos+Vec3::new(0.4,0.65,0.0),glam::Mat3::from_rotation_y(-rp.yaw),0.55);
        let offset=vertices.len() as u32;vertices.extend(held.vertices);indices.extend(held.indices.into_iter().map(|i|i+offset));
        let feet = rp.pos;
        models.push_player(&mut vertices, &mut indices, rp.appearance, feet, rp.yaw,
            if rp.last_seen.elapsed().as_secs_f32() < 0.25 { rp.velocity.xz().length() } else { 0.0 },
            rp.animation_started.elapsed().as_secs_f32());
        // Small floating marker above crystal carriers, so "sheep hunt
        // players carrying a crystal" is something you can actually see
        // happening, not just trust the log for.
        if rp.carrying_crystal {
            let marker_min = Vec3::new(feet.x - 0.1, feet.y + 2.2, feet.z - 0.1);
            let marker_max = Vec3::new(feet.x + 0.1, feet.y + 2.4, feet.z + 0.1);
            push_cuboid(
                &mut vertices,
                &mut indices,
                marker_min,
                marker_max,
                CRYSTAL_MARKER_COLOR,
                uv,
            );
        }
    }

    MeshData { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignments_are_unique_including_host_and_bare_heads() {
        for seed in 0..100 {
            let mut assigned = Vec::new();
            for _ in 0..20 {
                let next = Appearance::choose(seed, assigned.iter().copied());
                assert!(!assigned.contains(&next));
                assigned.push(next);
            }
            assert_eq!(assigned.iter().filter(|a| a.hat.is_none()).count(), 4);
            // A replacement guest can only receive a currently free combination.
            assigned.remove(2);
            let replacement = Appearance::choose(seed, assigned.iter().copied());
            assert!(!assigned.contains(&replacement));
        }
    }
}
