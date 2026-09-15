//! Stage 1A: short-lived targeting resolved against the authoritative world.
use crate::{
    creature::Creatures,
    voxel::{BlockType, World},
};
use glam::Vec3;

pub const CAST_RANGE: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Creature {
        id: u32,
    },
    Block {
        position: (i32, i32, i32),
        material: BlockType,
    },
}

/// Never saved or queued across game updates. Durable world identity/revisions and
/// network targeting requests belong to the persistent Spellbook stages.
#[derive(Clone, Copy, Debug)]
pub struct TargetContext {
    pub origin: Vec3,
    pub facing: Vec3,
    pub hit_position: Vec3,
    pub hit_normal: Vec3,
    pub target: Target,
}

impl TargetContext {
    pub fn resolve(
        world: &World,
        creatures: &Creatures,
        origin: Vec3,
        facing: Vec3,
    ) -> Option<Self> {
        if !origin.is_finite() || origin.abs().max_element() > 1_000_000.0 || !facing.is_finite() {
            return None;
        }
        let facing = facing.normalize_or_zero();
        if facing == Vec3::ZERO {
            return None;
        }
        if let Some((id, distance)) = creatures.aimed_body(world, origin, facing, CAST_RANGE) {
            if !loaded_path(world, origin, facing, distance) {
                return None;
            }
            return Some(Self {
                origin,
                facing,
                hit_position: origin + facing * distance,
                // Creature picking uses the existing body proxy, not a surface mesh.
                hit_normal: Vec3::ZERO,
                target: Target::Creature { id },
            });
        }
        let hit = crate::raycast::raycast(world, origin, facing, CAST_RANGE)?;
        let (x, y, z) = hit.target;
        let low = Vec3::new(x as f32, y as f32, z as f32);
        let mut distance = 0.0f32;
        let mut normal = Vec3::ZERO;
        // Recover the entry face exactly, including diagonal ray hits.
        for axis in 0..3 {
            if facing[axis].abs() > 1e-6 {
                let face = if facing[axis] > 0.0 {
                    low[axis]
                } else {
                    low[axis] + 1.0
                };
                let t = (face - origin[axis]) / facing[axis];
                if t > distance {
                    distance = t;
                    normal = Vec3::ZERO;
                    normal[axis] = -facing[axis].signum();
                }
            }
        }
        if !loaded_path(world, origin, facing, distance) {
            return None;
        }
        Some(Self {
            origin,
            facing,
            hit_position: origin + facing * distance,
            hit_normal: normal,
            target: Target::Block {
                position: hit.target,
                material: world.get_block(x, y, z),
            },
        })
    }

    /// Re-resolve identity, material, range and obstruction; never trust supplied
    /// hit coordinates. Origin must agree with the authoritative caster's eyes.
    pub fn validate(self, world: &World, creatures: &Creatures, eye: Vec3) -> Result<Self, String> {
        if !eye.is_finite() || !self.origin.is_finite() || self.origin.distance(eye) > 0.1 {
            return Err("The caster moved; aim again".into());
        }
        let current = Self::resolve(world, creatures, eye, self.facing)
            .ok_or("Aim at a creature or block within 18 blocks")?;
        if current.target != self.target {
            return Err("The target changed or is obstructed; aim again".into());
        }
        Ok(current)
    }
}

fn loaded_path(world: &World, origin: Vec3, facing: Vec3, distance: f32) -> bool {
    // Unloaded chunks read as air in World; they must not establish visibility.
    for step in 0..=(distance / 0.25).ceil() as u32 {
        let point = origin + facing * (step as f32 * 0.25).min(distance);
        let chunk =
            crate::voxel::chunk::world_to_chunk(point.x.floor() as i32, point.z.floor() as i32);
        if !world.chunks.contains_key(&chunk) {
            return false;
        }
    }
    true
}
