use glam::Vec3;

use crate::input::Input;
use crate::voxel::{BlockType, World};
use winit::keyboard::KeyCode;

const HALF_WIDTH: f32 = 0.3;
const HEIGHT: f32 = 1.8;
const GRAVITY: f32 = -24.0;
const JUMP_SPEED: f32 = 8.0;
const WALK_SPEED: f32 = 4.5;
const SPRINT_SPEED: f32 = 7.5;
/// Speed multiplier while standing on a Mud block (see `world.rs`'s
/// `replace_block` API -- rules turn soil to mud, and this is what makes
/// that actually slow the player rather than just being cosmetic).
const MUD_SPEED_MULTIPLIER: f32 = 0.45;

pub struct Player {
    /// Feet position (bottom-center of the collision box).
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Set by breaking a Crystal block; exposed to Lua rules via
    /// `api.players()` so a rule can react to "players carrying a crystal".
    pub carrying_crystal: bool,
}

impl Player {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            on_ground: false,
            carrying_crystal: false,
        }
    }

    pub fn update(&mut self, world: &World, input: &Input, forward: Vec3, right: Vec3, dt: f32) {
        let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

        let mut wish = Vec3::ZERO;
        if input.is_down(KeyCode::KeyW) {
            wish += forward_flat;
        }
        if input.is_down(KeyCode::KeyS) {
            wish -= forward_flat;
        }
        if input.is_down(KeyCode::KeyD) {
            wish += right_flat;
        }
        if input.is_down(KeyCode::KeyA) {
            wish -= right_flat;
        }
        wish = wish.normalize_or_zero();

        let mut speed = if input.is_down(KeyCode::ShiftLeft) {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        let below = world.get_block(
            self.position.x.floor() as i32,
            (self.position.y - 0.05).floor() as i32,
            self.position.z.floor() as i32,
        );
        if below == BlockType::Mud {
            speed *= MUD_SPEED_MULTIPLIER;
        }

        self.velocity.x = wish.x * speed;
        self.velocity.z = wish.z * speed;

        if input.is_down(KeyCode::Space) && self.on_ground {
            self.velocity.y = JUMP_SPEED;
            self.on_ground = false;
        }

        self.velocity.y += GRAVITY * dt;
        self.velocity.y = self.velocity.y.max(-50.0);

        let delta = self.velocity * dt;
        self.move_and_collide(world, delta);
    }

    fn move_and_collide(&mut self, world: &World, delta: Vec3) {
        // X axis
        let attempt = self.position + Vec3::new(delta.x, 0.0, 0.0);
        if !Self::collides(world, attempt) {
            self.position.x = attempt.x;
        } else {
            self.velocity.x = 0.0;
        }

        // Z axis
        let attempt = self.position + Vec3::new(0.0, 0.0, delta.z);
        if !Self::collides(world, attempt) {
            self.position.z = attempt.z;
        } else {
            self.velocity.z = 0.0;
        }

        // Y axis
        let attempt = self.position + Vec3::new(0.0, delta.y, 0.0);
        if !Self::collides(world, attempt) {
            self.position.y = attempt.y;
            if delta.y < 0.0 {
                self.on_ground = false;
            }
        } else {
            if delta.y < 0.0 {
                self.on_ground = true;
            }
            self.velocity.y = 0.0;
        }
    }

    fn collides(world: &World, feet: Vec3) -> bool {
        let min_x = feet.x - HALF_WIDTH;
        let max_x = feet.x + HALF_WIDTH;
        let min_y = feet.y;
        let max_y = feet.y + HEIGHT;
        let min_z = feet.z - HALF_WIDTH;
        let max_z = feet.z + HALF_WIDTH;

        let bx0 = min_x.floor() as i32;
        let bx1 = (max_x - 1e-4).floor() as i32;
        let by0 = min_y.floor() as i32;
        let by1 = (max_y - 1e-4).floor() as i32;
        let bz0 = min_z.floor() as i32;
        let bz1 = (max_z - 1e-4).floor() as i32;

        for bx in bx0..=bx1 {
            for by in by0..=by1 {
                for bz in bz0..=bz1 {
                    if world.is_solid(bx, by, bz) {
                        return true;
                    }
                }
            }
        }
        false
    }
}
