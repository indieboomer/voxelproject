use glam::Vec3;

use crate::input::Input;
use crate::voxel::{BlockType, World, COLLECTIBLE_BLOCKS};
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

/// Every player starts here every session -- health is deliberately NOT
/// saved with the world (see world_api/schema.yaml's
/// `persistence.player_state_not_saved`), the same way the Resources
/// inventory already wasn't before this existed.
pub const MAX_HEALTH: f32 = 100.0;
/// Clamp range for `speed_multiplier`/`jump_multiplier` -- see
/// world_api/schema.yaml's `attribute_multiplier_min`/`_max`.
pub const MIN_ATTRIBUTE_MULTIPLIER: f32 = 0.1;
pub const MAX_ATTRIBUTE_MULTIPLIER: f32 = 5.0;

pub struct Player {
    /// Feet position (bottom-center of the collision box).
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Set by breaking a Crystal block; exposed to Lua rules via
    /// `api.players()` so a rule can react to "players carrying a crystal".
    pub carrying_crystal: bool,
    /// Holding sprint (shift) while actually trying to move -- exposed to
    /// Lua rules as `running` so a rule can tell walking from sprinting.
    pub sprinting: bool,
    /// Count of each `COLLECTIBLE_BLOCKS` type gathered so far, indexed the
    /// same way -- incremented on breaking, decremented on placing. Shown
    /// in the Resources HUD panel.
    resources: [u32; COLLECTIBLE_BLOCKS.len()],
    /// 0..=MAX_HEALTH. Set via `api.damage_player`/`api.heal_player`; there
    /// is no death/respawn system yet, so it just clamps at 0 and stays.
    pub health: f32,
    /// While true, `App`'s poison timer drains 1 health every
    /// `POISON_TICK_INTERVAL` seconds -- see `api.set_poisoned`.
    pub poisoned: bool,
    /// Multiplies `WALK_SPEED`/`SPRINT_SPEED` in `update` -- see
    /// `api.set_player_speed`.
    pub speed_multiplier: f32,
    /// Multiplies `JUMP_SPEED` in `update` -- see `api.set_player_jump`.
    pub jump_multiplier: f32,
}

impl Player {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            on_ground: false,
            carrying_crystal: false,
            sprinting: false,
            resources: [0; COLLECTIBLE_BLOCKS.len()],
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
        }
    }

    /// Clamped, symmetric with `heal` (a negative amount here is
    /// equivalent to `heal`'s negative case, but `api.damage_player`/
    /// `api.heal_player` each call the one matching their own sign so a
    /// rule author never has to think about which one to use for which
    /// direction).
    pub fn damage(&mut self, amount: f32) {
        self.health = (self.health - amount).clamp(0.0, MAX_HEALTH);
    }

    pub fn heal(&mut self, amount: f32) {
        self.health = (self.health + amount).clamp(0.0, MAX_HEALTH);
    }

    pub fn set_speed_multiplier(&mut self, multiplier: f32) {
        self.speed_multiplier = multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
    }

    pub fn set_jump_multiplier(&mut self, multiplier: f32) {
        self.jump_multiplier = multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
    }

    /// A snapshot of every `COLLECTIBLE_BLOCKS` count, for the World API's
    /// `api.get_resource_count`/`api.take_item` -- see their host-only
    /// caveat in world_api/schema.yaml (only the host's own `Player` has
    /// this; a remote player's counts live on their own machine).
    pub fn resources_snapshot(&self) -> [u32; COLLECTIBLE_BLOCKS.len()] {
        self.resources
    }

    /// Adds one to the gathered count for `block`, if it's collectible.
    /// A no-op for anything not in `COLLECTIBLE_BLOCKS` (e.g. Crystal).
    pub fn add_resource(&mut self, block: BlockType) {
        self.add_resources(block, 1);
    }

    /// Batch form of `add_resource`, for `api.give_item` granting more than
    /// one at a time without a caller-side loop. Same no-op-for-
    /// uncollectible-kinds behavior.
    pub fn add_resources(&mut self, block: BlockType, amount: u32) {
        if let Some(i) = COLLECTIBLE_BLOCKS.iter().position(|&b| b == block) {
            self.resources[i] = self.resources[i].saturating_add(amount);
        }
    }

    pub fn resource_count(&self, block: BlockType) -> u32 {
        COLLECTIBLE_BLOCKS
            .iter()
            .position(|&b| b == block)
            .map_or(0, |i| self.resources[i])
    }

    /// Consumes one gathered `block`, if any remain. Returns whether it
    /// succeeded -- the caller should only place the block on success.
    pub fn take_resource(&mut self, block: BlockType) -> bool {
        self.take_resources(block, 1)
    }

    /// Batch form of `take_resource`, for `api.take_item` removing more
    /// than one at a time. Atomic -- either the whole `amount` is removed,
    /// or (not holding enough, or `block` isn't collectible) none of it is.
    pub fn take_resources(&mut self, block: BlockType, amount: u32) -> bool {
        match COLLECTIBLE_BLOCKS.iter().position(|&b| b == block) {
            Some(i) if self.resources[i] >= amount => {
                self.resources[i] -= amount;
                true
            }
            _ => false,
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

        self.sprinting = input.is_down(KeyCode::ShiftLeft) && wish != Vec3::ZERO;
        let mut speed = if self.sprinting {
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
        speed *= self.speed_multiplier;

        self.velocity.x = wish.x * speed;
        self.velocity.z = wish.z * speed;

        if input.is_down(KeyCode::Space) && self.on_ground {
            self.velocity.y = JUMP_SPEED * self.jump_multiplier;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_player_starts_at_full_health_unpoisoned_with_default_multipliers() {
        let player = Player::new(Vec3::ZERO);
        assert_eq!(player.health, MAX_HEALTH);
        assert!(!player.poisoned);
        assert_eq!(player.speed_multiplier, 1.0);
        assert_eq!(player.jump_multiplier, 1.0);
    }

    #[test]
    fn damage_and_heal_clamp_health_into_0_to_max() {
        let mut player = Player::new(Vec3::ZERO);
        player.damage(30.0);
        assert_eq!(player.health, 70.0);
        player.heal(10.0);
        assert_eq!(player.health, 80.0);

        player.damage(1000.0);
        assert_eq!(player.health, 0.0, "damage should clamp at 0, not go negative");

        player.heal(1000.0);
        assert_eq!(player.health, MAX_HEALTH, "heal should clamp at max_health");
    }

    #[test]
    fn attribute_multipliers_clamp_into_the_allowed_range() {
        let mut player = Player::new(Vec3::ZERO);
        player.set_speed_multiplier(0.0);
        assert_eq!(player.speed_multiplier, MIN_ATTRIBUTE_MULTIPLIER);
        player.set_speed_multiplier(100.0);
        assert_eq!(player.speed_multiplier, MAX_ATTRIBUTE_MULTIPLIER);

        player.set_jump_multiplier(-5.0);
        assert_eq!(player.jump_multiplier, MIN_ATTRIBUTE_MULTIPLIER);
        player.set_jump_multiplier(2.5);
        assert_eq!(player.jump_multiplier, 2.5);
    }

    #[test]
    fn take_resources_removes_the_whole_amount_atomically_or_none_of_it() {
        let mut player = Player::new(Vec3::ZERO);
        player.add_resources(BlockType::Stone, 100);

        assert!(player.take_resources(BlockType::Stone, 40));
        assert_eq!(player.resource_count(BlockType::Stone), 60);

        assert!(
            !player.take_resources(BlockType::Stone, 61),
            "should fail rather than partially take when short by 1"
        );
        assert_eq!(
            player.resource_count(BlockType::Stone),
            60,
            "a failed take must not remove anything"
        );
    }

    #[test]
    fn add_and_take_resource_round_trip_through_the_gathered_count() {
        let mut player = Player::new(Vec3::ZERO);
        assert_eq!(player.resource_count(BlockType::Stone), 0);

        player.add_resource(BlockType::Stone);
        player.add_resource(BlockType::Stone);
        assert_eq!(player.resource_count(BlockType::Stone), 2);

        assert!(player.take_resource(BlockType::Stone));
        assert_eq!(player.resource_count(BlockType::Stone), 1);
        assert!(player.take_resource(BlockType::Stone));
        assert_eq!(player.resource_count(BlockType::Stone), 0);

        assert!(
            !player.take_resource(BlockType::Stone),
            "taking from an empty count should fail, not underflow"
        );
    }

    #[test]
    fn add_resources_grants_a_batch_in_one_call() {
        let mut player = Player::new(Vec3::ZERO);
        player.add_resources(BlockType::Stone, 100);
        assert_eq!(player.resource_count(BlockType::Stone), 100);
    }

    #[test]
    fn add_resources_is_a_no_op_for_an_uncollectible_kind() {
        let mut player = Player::new(Vec3::ZERO);
        player.add_resources(BlockType::Crystal, 50);
        assert_eq!(player.resource_count(BlockType::Crystal), 0);
    }

    #[test]
    fn crystal_is_not_a_stackable_resource() {
        // Crystal is deliberately excluded from COLLECTIBLE_BLOCKS -- it's
        // tracked separately via `carrying_crystal`.
        let mut player = Player::new(Vec3::ZERO);
        player.add_resource(BlockType::Crystal);
        assert_eq!(player.resource_count(BlockType::Crystal), 0);
        assert!(!player.take_resource(BlockType::Crystal));
    }

    #[test]
    fn different_block_types_are_tracked_independently() {
        let mut player = Player::new(Vec3::ZERO);
        player.add_resource(BlockType::OakWood);
        player.add_resource(BlockType::OakWood);
        player.add_resource(BlockType::Sand);

        assert_eq!(player.resource_count(BlockType::OakWood), 2);
        assert_eq!(player.resource_count(BlockType::Sand), 1);
        assert_eq!(player.resource_count(BlockType::Soil), 0);
    }
}
