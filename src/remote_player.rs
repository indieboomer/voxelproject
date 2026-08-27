use std::collections::HashMap;
use std::time::Instant;

use glam::Vec3;

use crate::net::PlayerId;
use crate::voxel::atlas::white_uv;
use crate::voxel::mesher::{push_cuboid, MeshData, Vertex};

const PLAYER_COLORS: [[f32; 3]; 4] = [
    [0.85, 0.25, 0.25],
    [0.25, 0.5, 0.85],
    [0.85, 0.75, 0.2],
    [0.5, 0.85, 0.3],
];
const CRYSTAL_MARKER_COLOR: [f32; 3] = [0.55, 0.85, 0.95];

fn color_for(id: PlayerId) -> [f32; 3] {
    PLAYER_COLORS[id as usize % PLAYER_COLORS.len()]
}

pub struct RemotePlayer {
    pub pos: Vec3,
    pub yaw: f32,
    pub carrying_crystal: bool,
    pub last_seen: Instant,
    /// Approximated on the host from consecutive `PlayerState` updates
    /// (position delta / time since the last one) since the network
    /// protocol doesn't carry a client's real physics velocity -- good
    /// enough for exposing `speed`/`running` to Lua rules via `api.players`.
    pub velocity: Vec3,
}

/// Builds a mesh for every tracked remote player except `exclude` (the
/// local player, if it appears in the same map).
pub fn build_mesh(players: &HashMap<PlayerId, RemotePlayer>, exclude: PlayerId) -> MeshData {
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let uv = white_uv();
    for (&id, rp) in players.iter() {
        if id == exclude {
            continue;
        }
        let color = color_for(id);
        let feet = rp.pos;
        let body_min = Vec3::new(feet.x - 0.3, feet.y, feet.z - 0.3);
        let body_max = Vec3::new(feet.x + 0.3, feet.y + 1.4, feet.z + 0.3);
        push_cuboid(&mut vertices, &mut indices, body_min, body_max, color, uv);

        let head_min = Vec3::new(feet.x - 0.22, body_max.y, feet.z - 0.22);
        let head_max = Vec3::new(feet.x + 0.22, body_max.y + 0.4, feet.z + 0.22);
        push_cuboid(&mut vertices, &mut indices, head_min, head_max, color, uv);

        // Small floating marker above crystal carriers, so "sheep hunt
        // players carrying a crystal" is something you can actually see
        // happening, not just trust the log for.
        if rp.carrying_crystal {
            let marker_min = Vec3::new(feet.x - 0.1, head_max.y + 0.15, feet.z - 0.1);
            let marker_max = Vec3::new(feet.x + 0.1, head_max.y + 0.35, feet.z + 0.1);
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
