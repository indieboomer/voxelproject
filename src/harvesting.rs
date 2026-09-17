//! Renewable animal harvest definitions and cooldown policy.
use crate::creature::CreatureKind;

pub const COOLDOWN_SECONDS: f32 = 300.0;
pub const fn output(kind: CreatureKind) -> Option<(&'static str, u32)> {
    match kind {
        CreatureKind::Sheep => Some(("harvest:wool", 2)),
        CreatureKind::Chicken => Some(("harvest:egg", 1)),
        CreatureKind::Cow => Some(("harvest:milk", 1)),
        _ => None,
    }
}
pub fn output_random(kind: CreatureKind, roll: u64) -> Option<(&'static str, u32)> {
    if kind == CreatureKind::Sheep { return Some(if roll & 1 == 0 { ("harvest:wool", 2) } else { ("harvest:milk", 1) }); }
    output(kind)
}
pub const HIVE_COOLDOWN_SECONDS: f32 = 240.0;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passive_animals_have_distinct_renewable_outputs() {
        assert_eq!(output(CreatureKind::Sheep), Some(("harvest:wool", 2)));
        assert_eq!(output(CreatureKind::Chicken), Some(("harvest:egg", 1)));
        assert_eq!(output(CreatureKind::Cow), Some(("harvest:milk", 1)));
        assert_eq!(output(CreatureKind::Wolf), None);
        assert_eq!(output_random(CreatureKind::Sheep, 0), Some(("harvest:wool", 2)));
        assert_eq!(output_random(CreatureKind::Sheep, 1), Some(("harvest:milk", 1)));
    }
}
