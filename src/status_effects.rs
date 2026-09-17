//! Shared timed status effects used by survival, equipment, and spells.
use std::collections::BTreeMap;

/// The full Phase 2 status vocabulary. Some entries are intentionally exposed
/// before their gameplay producers are migrated from legacy fields.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind { Hungry, Starving, Wet, Cold, Poisoned, Regeneration, Sheltered, Burning }

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub remaining: f32,
    pub period: f32,
    pub health_per_tick: f32,
    pub source: String,
    elapsed: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Effects { active: BTreeMap<Kind, Effect> }

impl Effects {
    pub fn apply(&mut self, kind: Kind, duration: f32, period: f32, health_per_tick: f32, source: impl Into<String>) {
        let duration = duration.max(0.0);
        let source = source.into();
        let entry = self.active.entry(kind).or_insert_with(|| Effect { remaining: 0.0, period: period.max(0.01), health_per_tick, source: source.clone(), elapsed: 0.0 });
        entry.remaining = entry.remaining.max(duration);
        entry.period = period.max(0.01);
        entry.health_per_tick = health_per_tick;
        entry.source = source;
    }
    pub fn remove(&mut self, kind: Kind) { self.active.remove(&kind); }
    #[allow(dead_code)]
    pub fn contains(&self, kind: Kind) -> bool { self.active.contains_key(&kind) }
    pub fn tick(&mut self, dt: f32) -> f32 {
        let dt = dt.max(0.0);
        let mut damage = 0.0;
        for effect in self.active.values_mut() {
            effect.remaining -= dt;
            effect.elapsed += dt;
            while effect.elapsed >= effect.period {
                effect.elapsed -= effect.period;
                damage += effect.health_per_tick;
            }
        }
        self.active.retain(|_, e| e.remaining > 0.0);
        damage
    }
    #[allow(dead_code)]
    pub fn get(&self, kind: Kind) -> Option<&Effect> { self.active.get(&kind) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effects_refresh_and_tick_without_stacking_duplicates() {
        let mut effects = Effects::default();
        effects.apply(Kind::Poisoned, 3.0, 1.0, 2.0, "mushroom");
        effects.apply(Kind::Poisoned, 5.0, 1.0, 2.0, "mushroom");
        assert!(effects.contains(Kind::Poisoned));
        assert_eq!(effects.tick(2.1), 4.0);
        assert!(effects.get(Kind::Poisoned).unwrap().remaining > 2.0);
        assert_eq!(effects.tick(5.0), 10.0);
        assert!(!effects.contains(Kind::Poisoned));
    }
}
