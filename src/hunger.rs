//! Gentle, host-authoritative hunger shared by local and remote players.
pub const HUNGRY: f32 = 35.0;
pub const STARVING: f32 = 8.0;
const DRAIN_PER_SEC: f32 = 100.0 / 1500.0;
const DAMAGE_PER_SEC: f32 = 1.0 / 30.0;
const HEALTH_FLOOR: f32 = 50.0;

pub fn tick(satiety: &mut f32, health: &mut f32, dt: f32) {
    if *health <= 0.0 { return; }
    let dt = dt.max(0.0);
    let grace = ((*satiety - STARVING) / DRAIN_PER_SEC).max(0.0);
    *satiety = (*satiety - dt * DRAIN_PER_SEC).max(0.0);
    // Never heal an already injured player or let hunger become lethal.
    if *health > HEALTH_FLOOR {
        *health = (*health - (dt - grace).max(0.0) * DAMAGE_PER_SEC).max(HEALTH_FLOOR);
    }
}

/// Every successful meal clears hunger, including food with no nutrition bonus.
/// Richer meals add more fullness and therefore postpone the next hungry state.
pub fn eat(satiety: &mut f32, nutrition: f32) {
    *satiety = (satiety.max(HUNGRY + 10.0) + nutrition.max(0.0))
        .min(crate::player::MAX_SATIETY);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunger_has_two_long_grace_periods_then_gentle_damage() {
        let (mut food, mut health) = (100.0, 100.0);
        tick(&mut food, &mut health, 975.0);
        assert!((food - HUNGRY).abs() < 0.001);
        assert_eq!(health, 100.0);
        tick(&mut food, &mut health, 405.0);
        assert!((food - STARVING).abs() < 0.001);
        assert_eq!(health, 100.0);
        tick(&mut food, &mut health, 30.0);
        assert!((health - 99.0).abs() < 0.001);
    }

    #[test]
    fn hunger_stops_at_half_health_without_healing_injuries() {
        for initial in [100.0, 50.0, 20.0, 0.0] {
            let (mut food, mut health) = (0.0, initial);
            tick(&mut food, &mut health, 10000.0);
            assert_eq!(health, initial.min(HEALTH_FLOOR));
        }
    }

    #[test]
    fn every_meal_clears_hunger_and_cooked_food_lasts_longer() {
        for nutrition in [0.0, 10.0, 16.0, 32.0] {
            let (mut food, mut health) = (0.0, 70.0);
            eat(&mut food, nutrition);
            assert!(food > HUNGRY);
            tick(&mut food, &mut health, 120.0);
            assert!(food > HUNGRY);
            assert_eq!(health, 70.0);
        }
        let (mut raw, mut cooked) = (0.0, 0.0);
        eat(&mut raw, 16.0);
        eat(&mut cooked, 32.0);
        assert!(cooked > raw);
        eat(&mut cooked, 100.0);
        assert_eq!(cooked, crate::player::MAX_SATIETY);
    }
}
