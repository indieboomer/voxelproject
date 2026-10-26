use glam::Vec3;

use crate::voxel::shaping::{self, CellShape};
use crate::voxel::World;

pub struct RaycastHit {
    /// The solid block that was hit.
    pub target: (i32, i32, i32),
    /// The empty voxel just before the hit, i.e. where a new block would go.
    pub place: (i32, i32, i32),
}

/// Simple fixed-step raycast against the voxel grid. Step size is small
/// relative to a block so it won't tunnel through walls at typical reach
/// distances.
///
/// With terrain shaping on, a shaped cell only counts once the ray reaches
/// its visible solid, so rays through a ramp's empty part pass on to what is
/// behind it. A hit reached from inside an occupied cell's empty part places
/// above that cell, never into it.
pub fn raycast(world: &World, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<RaycastHit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    const STEP: f32 = 0.05;
    let mut t = 0.0f32;
    let mut last_voxel: Option<(i32, i32, i32)> = None;
    let mut cached: Option<((i32, i32, i32), Option<CellShape>)> = None;

    while t < max_dist {
        let p = origin + dir * t;
        let voxel = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        // Targetable, not just solid: non-solid decorations like short
        // grass are still breakable even though the player walks through
        // them (see `BlockType::is_targetable`).
        if world.get_block(voxel.0, voxel.1, voxel.2).is_targetable() {
            let shape = match cached {
                Some((v, shape)) if v == voxel => shape,
                _ => {
                    let shape = shaping::shape_at(world, voxel.0, voxel.1, voxel.2);
                    cached = Some((voxel, shape));
                    shape
                }
            };
            let inside = match crate::voxel::organic::tree_cell(world, voxel.0, voxel.1, voxel.2) {
                Some(tree) => tree.contains(p),
                None => shaping::cell_contains(shape.as_ref(), voxel, p),
            };
            if inside {
                let place = match last_voxel {
                    Some(v) if world.get_block(v.0, v.1, v.2).is_targetable() => (v.0, v.1 + 1, v.2),
                    Some(v) => v,
                    None => voxel,
                };
                return Some(RaycastHit { target: voxel, place });
            }
        }
        last_voxel = Some(voxel);
        t += STEP;
    }
    None
}
