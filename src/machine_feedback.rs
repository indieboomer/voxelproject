//! Local, bounded presentation of authoritative machine events. Never changes stock.
use crate::automation::{center, Activity, Cell, Device, Kind, State};
use crate::voxel::{
    atlas::white_uv,
    mesher::{push_cuboid, MeshData},
};
use glam::Vec3;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cue {
    Transfer,
    Changed,
    Blocked,
    Built,
    Removed,
    Produced,
    DarkAltar,
    Shrine,
}
impl Cue {
    fn color(self) -> [f32; 3] {
        match self {
            Self::Transfer => [0.18, 0.7, 0.95],
            Self::Changed => [0.7, 0.35, 0.95],
            Self::Blocked => [1.0, 0.25, 0.08],
            Self::Built => [0.25, 0.95, 0.4],
            Self::Removed => [0.65, 0.5, 0.3],
            Self::Produced => [1.0, 0.72, 0.16],
            Self::DarkAltar => [0.12, 0.015, 0.22],
            Self::Shrine => [0.35, 0.9, 0.65],
        }
    }
}
#[derive(Clone, Copy)]
struct Stamp {
    events: [u64; 3],
    revision: u64,
    kind: Kind,
}
impl From<&Device> for Stamp {
    fn from(d: &Device) -> Self {
        Self {
            events: d.feedback_events,
            revision: d.config_revision,
            kind: d.kind,
        }
    }
}
#[derive(Clone, Copy)]
struct Pulse {
    cue: Cue,
    age: f32,
}
#[derive(Default)]
pub struct Feedback {
    previous: BTreeMap<Cell, Stamp>,
    pulses: BTreeMap<Cell, Pulse>,
    cooldowns: BTreeMap<Cell, f32>,
    sound_delay: f32,
    pending_sound: Option<(Cell, Cue)>,
    ready: bool,
    smelter_heat: BTreeMap<Cell, f32>,
    time: f32,
    active_props: BTreeMap<Cell, Kind>,
}
impl Feedback {
    /// Loading/joining observes a baseline, not a replay of historical events.
    pub fn synchronize(&mut self, state: &State) {
        self.previous = state
            .devices
            .iter()
            .map(|(&p, d)| (p, Stamp::from(d)))
            .collect();
        self.pulses.clear();
        self.smelter_heat.clear();
        self.active_props.clear();
        self.cooldowns.clear();
        self.pending_sound = None;
        self.ready = true;
    }
    pub fn update(&mut self, state: &State, dt: f32, listener: Vec3) -> Vec<(Cell, Cue)> {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.sound_delay = (self.sound_delay - dt).max(0.0);
        self.pulses.retain(|_, p| {
            p.age += dt;
            p.age < 0.8
        });
        self.cooldowns.retain(|_, v| {
            *v -= dt;
            *v > 0.0
        });
        if !self.ready {
            self.synchronize(state);
            return Vec::new();
        }
        self.time = (self.time + dt) % 3600.0;
        self.active_props = state
            .devices
            .iter()
            .filter(|(p, d)| {
                d.kind.sustained()
                    && d.config.enabled
                    && d.activity == Activity::Working
                    && center(**p).distance_squared(listener) < 32.0 * 32.0
            })
            .map(|(&p, d)| (p, d.kind))
            .collect();
        self.smelter_heat.retain(|p, heat| {
            *heat -= dt;
            *heat > 0.0
                && state
                    .devices
                    .get(p)
                    .is_some_and(|d| d.kind == Kind::Smelter && d.config.enabled)
                && center(*p).distance_squared(listener) <= 24.0 * 24.0
        });
        for (&p, d) in &state.devices {
            // Instant production can already be Idle after ejecting its output.
            let produced = self.previous.get(&p).is_some_and(|old| {
                old.kind == Kind::Smelter && old.events[0] != d.feedback_events[0]
            });
            if d.kind == Kind::Smelter
                && d.config.enabled
                && (produced || d.activity == Activity::Working)
                && center(p).distance_squared(listener) <= 24.0 * 24.0
            {
                self.smelter_heat.insert(p, 1.5);
            }
        }
        let mut changes = Vec::new();
        for (&p, d) in &state.devices {
            let cue = match self.previous.get(&p) {
                None => Some(Cue::Built),
                Some(old) if old.kind != d.kind => Some(Cue::Built),
                Some(old) if old.events[0] != d.feedback_events[0] => Some(match d.kind {
                    Kind::DarkAltar => Cue::DarkAltar,
                    Kind::Shrine => Cue::Shrine,
                    _ => Cue::Produced,
                }),
                Some(old)
                    if old.revision != d.config_revision
                        || old.events[2] != d.feedback_events[2] =>
                {
                    Some(
                        if matches!(
                            d.activity,
                            Activity::BlockedOutput
                                | Activity::InsufficientMana
                                | Activity::MissingIngredients
                                | Activity::MissingFuel
                                | Activity::Closed
                                | Activity::Disabled
                        ) {
                            Cue::Blocked
                        } else {
                            Cue::Changed
                        },
                    )
                }
                Some(old) if old.events[1] != d.feedback_events[1] => Some(Cue::Transfer),
                _ => None,
            };
            if let Some(cue) = cue {
                changes.push((p, cue));
            }
        }
        for &p in self.previous.keys() {
            if !state.devices.contains_key(&p) {
                changes.push((p, Cue::Removed));
            }
        }
        self.previous = state
            .devices
            .iter()
            .map(|(&p, d)| (p, Stamp::from(d)))
            .collect();
        changes.retain(|(p, _)| center(*p).distance_squared(listener) <= 24.0 * 24.0);
        // Production takes priority over busy channels; nearest sounds win ties.
        changes.sort_by(|a, b| {
            b.1.cmp(&a.1).then_with(|| {
                center(a.0)
                    .distance_squared(listener)
                    .total_cmp(&center(b.0).distance_squared(listener))
            })
        });
        let mut sounds = Vec::new();
        for (p, cue) in changes {
            let important = matches!(
                cue,
                Cue::Produced | Cue::DarkAltar | Cue::Shrine | Cue::Built | Cue::Removed
            ) || (matches!(cue, Cue::Changed | Cue::Blocked)
                && self.pulses.get(&p).is_none_or(|pulse| pulse.cue != cue));
            if self.cooldowns.contains_key(&p) && !important {
                continue;
            }
            // At most one brief pulse per device; activity cannot accumulate particles.
            self.pulses.insert(p, Pulse { cue, age: 0.0 });
            self.cooldowns.insert(p, 0.7);
            if !matches!(cue, Cue::DarkAltar | Cue::Shrine)
                && self.pending_sound.is_none_or(|(_, waiting)| cue > waiting)
            {
                self.pending_sound = Some((p, cue));
            }
        }
        if self.sound_delay == 0.0 {
            if let Some((p, cue)) = self.pending_sound.take() {
                if center(p).distance_squared(listener) <= 24.0 * 24.0 {
                    sounds.push((p, cue));
                    self.sound_delay = 0.25;
                }
            }
        }
        sounds
    }
    pub fn decorate(&self, cell: Cell, mesh: &mut MeshData) {
        if let Some(&heat) = self.smelter_heat.get(&cell) {
            for v in &mut mesh.vertices {
                if v.color == [0.95, 0.25, 0.045] {
                    v.emission = self.fire_strength(cell, heat) * 0.32;
                }
            }
        }
        let Some(p) = self.pulses.get(&cell) else {
            return;
        };
        let strength = (1.0 - p.age / 0.8).max(0.0);
        let color = p.cue.color();
        for v in &mut mesh.vertices {
            for i in 0..3 {
                v.color[i] = v.color[i] * (1.0 - 0.3 * strength) + color[i] * 0.3 * strength;
            }
            v.emission = v.emission.max(0.35 * strength);
        }
    }
    pub fn particles(&self) -> MeshData {
        let mut mesh = MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        // Fixed five puffs per nearby furnace; repeated production never accumulates particles.
        for (&cell, &heat) in &self.smelter_heat {
            let phase = cell.0 as f32 * 0.37 + cell.2 as f32 * 0.61;
            for i in 0..5 {
                let age = (self.time * 0.65 + i as f32 / 5.0 + phase).rem_euclid(1.0);
                let pos = center(cell)
                    + Vec3::new(
                        age * 0.35 + (phase + age * 5.0).sin() * age * 0.09,
                        0.52 + age * 1.65,
                        age * 0.15,
                    );
                let size = (0.07 + age * 0.13) * (1.0 - age).sqrt() * heat.min(1.0);
                let shade = 0.025 + age * 0.025;
                push_cuboid(
                    &mut mesh.vertices,
                    &mut mesh.indices,
                    pos - Vec3::splat(size),
                    pos + Vec3::splat(size),
                    [shade; 3],
                    white_uv(),
                );
            }
        }
        for (&cell, &kind) in &self.active_props {
            if kind == Kind::Lantern {
                continue;
            }
            for i in 0..8 {
                let age = (self.time * 0.5 + i as f32 / 8.0).fract();
                let angle = self.time * 0.7 + i as f32 * 2.4;
                let pos = center(cell)
                    + Vec3::new(angle.cos() * 0.32, 0.8 + age * 1.2, angle.sin() * 0.32);
                let size = Vec3::splat(0.065 * (1.0 - age));
                let color = if kind == Kind::DarkAltar {
                    [0.045, 0.008, 0.075]
                } else {
                    [0.35, 0.85, 0.65]
                };
                let start = mesh.vertices.len();
                push_cuboid(
                    &mut mesh.vertices,
                    &mut mesh.indices,
                    pos - size,
                    pos + size,
                    color,
                    white_uv(),
                );
                for v in &mut mesh.vertices[start..] {
                    v.emission = if kind == Kind::Shrine { 0.5 } else { 0.05 };
                }
            }
        }
        for (&cell, p) in &self.pulses {
            let progress = p.age / 0.8;
            let color = p.cue.color();
            let n = if p.cue == Cue::Produced { 8 } else { 4 };
            for i in 0..n {
                let a = i as f32 * std::f32::consts::TAU / n as f32;
                let radius = if p.cue == Cue::Transfer {
                    0.25
                } else {
                    0.35 + progress * 0.3
                };
                let height = if p.cue == Cue::Produced {
                    0.35 + progress * 0.7
                } else {
                    progress * 0.35
                };
                let pos = center(cell) + Vec3::new(a.cos() * radius, height, a.sin() * radius);
                let size = Vec3::splat(0.045 * (1.0 - progress));
                let start = mesh.vertices.len();
                push_cuboid(
                    &mut mesh.vertices,
                    &mut mesh.indices,
                    pos - size,
                    pos + size,
                    color,
                    white_uv(),
                );
                for v in &mut mesh.vertices[start..] {
                    v.emission = 0.65 * (1.0 - progress);
                }
            }
        }
        mesh
    }
    fn fire_strength(&self, cell: Cell, heat: f32) -> f32 {
        heat.min(1.0) * (0.9 + 0.1 * (self.time * 8.0 + cell.0 as f32 + cell.2 as f32).sin())
    }
    /// Share the existing four warm point lights with campfires, nearest first.
    pub fn lights(&self, campfires: &[Vec3], eye: Vec3) -> [[f32; 4]; 4] {
        if self.smelter_heat.is_empty() && !self.active_props.values().any(|k| *k == Kind::Lantern)
        {
            return crate::campfire::lights(campfires, eye);
        }
        let mut lights: Vec<_> = campfires
            .iter()
            .map(|p| (*p + Vec3::Y * 0.7, 8.0))
            .chain(
                self.smelter_heat
                    .iter()
                    .map(|(&cell, &heat)| (center(cell), 2.8 * self.fire_strength(cell, heat))),
            )
            // Negative radius marks a steady cool light in the existing light buffer.
            .chain(
                self.active_props
                    .iter()
                    .filter(|(_, k)| **k == Kind::Lantern)
                    .map(|(&cell, _)| (center(cell) + Vec3::Y * 1.95, -8.0)),
            )
            .filter(|(p, _)| p.distance_squared(eye) < 32.0 * 32.0)
            .collect();
        lights.sort_by(|a, b| {
            a.0.distance_squared(eye)
                .total_cmp(&b.0.distance_squared(eye))
        });
        let mut result = [[0.0; 4]; 4];
        for (out, (pos, radius)) in result.iter_mut().zip(lights) {
            *out = pos.extend(radius).to_array();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn powered_lantern_is_cool_and_aura_particles_stop_with_power() {
        let p = (0, 30, 0);
        let mut s = State::default();
        let mut f = Feedback::default();
        s.devices.insert(p, Device::new(Kind::Lantern, p, 0));
        s.devices
            .insert((2, 30, 0), Device::new(Kind::DarkAltar, (2, 30, 0), 0));
        f.synchronize(&s);
        for d in s.devices.values_mut() {
            d.activity = Activity::Working;
        }
        f.update(&s, 0.1, center(p));
        let light = f.lights(&[], center(p))[0];
        assert_eq!(light, [0.5, 32.45, 0.5, -8.0]);
        let particles = f.particles();
        assert!(!particles.vertices.is_empty());
        assert!(particles
            .vertices
            .iter()
            .all(|v| v.color[0] < 0.1 && v.color[2] > v.color[0]));
        for d in s.devices.values_mut() {
            d.activity = Activity::InsufficientMana;
        }
        f.update(&s, 0.1, center(p));
        assert_eq!(f.lights(&[], center(p)), [[0.0; 4]; 4]);
        assert!(f.particles().vertices.is_empty());
    }
    #[test]
    fn smelter_production_smokes_even_after_ejection_then_fades() {
        let p = (0, 30, 0);
        let mut s = State::default();
        let mut d = Device::new(Kind::Smelter, p, 0);
        d.fuel_heat = 8;
        d.activity = Activity::Idle;
        s.devices.insert(p, d);
        let mut f = Feedback::default();
        f.synchronize(&s);
        f.update(&s, 0.1, center(p));
        assert!(f.particles().vertices.is_empty());
        assert_eq!(f.lights(&[], center(p)), [[0.0; 4]; 4]);
        s.devices.get_mut(&p).unwrap().feedback(0);
        f.update(&s, 0.1, center(p));
        f.update(&s, 0.9, center(p)); // Generic production sparkle has expired.
        let smoke = f.particles();
        assert!(!smoke.vertices.is_empty());
        assert!(smoke.vertices.iter().all(|v| v.emission == 0.0
            && v.color.iter().all(|c| *c < 0.06)
            && v.position.iter().all(|c| c.is_finite())
            && v.position[1] > 30.9));
        assert!(f.lights(&[], center(p))[0][3] > 0.0);
        let mut prop = crate::automation_mesh::device(&s.devices[&p], 0.0, None, &s);
        f.decorate(p, &mut prop);
        assert!(prop
            .vertices
            .iter()
            .any(|v| v.color == [0.95, 0.25, 0.045] && v.emission > 0.0));
        f.update(&s, 0.7, center(p));
        assert!(f.particles().vertices.is_empty());
        assert_eq!(f.lights(&[], center(p)), [[0.0; 4]; 4]);
    }
    #[test]
    fn smelter_effect_is_bounded_and_clears_when_disabled_or_removed() {
        let p = (0, 30, 0);
        let mut s = State::default();
        s.devices.insert(p, Device::new(Kind::Smelter, p, 0));
        let mut f = Feedback::default();
        f.synchronize(&s);
        s.devices.get_mut(&p).unwrap().activity = Activity::Working;
        f.update(&s, 0.1, center(p));
        let count = f.particles().vertices.len();
        for _ in 0..50 {
            f.update(&s, 0.1, center(p));
        }
        assert_eq!(f.particles().vertices.len(), count);
        let camps = vec![center(p) + Vec3::X * 10.0; 8];
        let lights = f.lights(&camps, center(p));
        assert!(lights[0][3] <= 2.8);
        assert_eq!(lights[1][3], 8.0);
        s.devices.get_mut(&p).unwrap().config.enabled = false;
        f.update(&s, 0.1, center(p));
        assert!(f.smelter_heat.is_empty());
        s.devices.get_mut(&p).unwrap().config.enabled = true;
        f.update(&s, 0.1, center(p));
        assert!(!f.smelter_heat.is_empty());
        s.devices.clear();
        f.update(&s, 0.1, center(p));
        assert!(f.smelter_heat.is_empty());
    }
    #[test]
    fn blocked_state_overrides_transfer_pulse_and_sound_waits_for_rate_limit() {
        let p = (0, 30, 0);
        let mut s = State::default();
        s.devices.insert(p, Device::new(Kind::Workshop, p, 0));
        let mut f = Feedback::default();
        f.synchronize(&s);
        s.devices.get_mut(&p).unwrap().feedback(1);
        assert_eq!(f.update(&s, 0.1, center(p)), vec![(p, Cue::Transfer)]);
        let d = s.devices.get_mut(&p).unwrap();
        d.activity = Activity::BlockedOutput;
        d.feedback(2);
        assert!(f.update(&s, 0.1, center(p)).is_empty());
        assert_eq!(f.pulses[&p].cue, Cue::Blocked);
        assert_eq!(f.update(&s, 0.2, center(p)), vec![(p, Cue::Blocked)]);
    }
    #[test]
    fn baseline_duplicates_net_zero_transfer_and_production_are_handled() {
        let p = (0, 30, 0);
        let mut s = State::default();
        s.devices.insert(p, Device::new(Kind::Channel, p, 0));
        let mut f = Feedback::default();
        assert!(f.update(&s, 0.1, center(p)).is_empty());
        // The stock remains empty, but a transfer happened between snapshots.
        s.devices.get_mut(&p).unwrap().feedback(1);
        assert_eq!(f.update(&s, 0.1, center(p)), vec![(p, Cue::Transfer)]);
        assert!(f.update(&s, 1.0, center(p)).is_empty());
        assert!(f.particles().vertices.is_empty());
        s.devices.get_mut(&p).unwrap().feedback(0);
        assert_eq!(f.update(&s, 0.1, center(p)), vec![(p, Cue::Produced)]);
        let particles = f.particles();
        assert!(!particles.vertices.is_empty());
        assert!(particles
            .vertices
            .iter()
            .all(|v| v.position.iter().all(|x| x.is_finite())));
        f.synchronize(&s);
        assert!(f.update(&s, 0.1, center(p)).is_empty());
    }
    #[test]
    fn distant_changes_are_not_replayed_and_dense_updates_are_throttled() {
        let mut s = State::default();
        let mut f = Feedback::default();
        f.synchronize(&s);
        for x in 0..64 {
            s.devices
                .insert((x, 30, 0), Device::new(Kind::Vessel, (x, 30, 0), 0));
        }
        assert_eq!(f.update(&s, 0.1, center((0, 30, 0))).len(), 1);
        assert!(f.pulses.len() <= 25);
        assert!(f.update(&s, 1.0, center((63, 30, 0))).is_empty());
        s.devices.remove(&(63, 30, 0));
        assert_eq!(
            f.update(&s, 0.1, center((63, 30, 0))),
            vec![((63, 30, 0), Cue::Removed)]
        );
    }
}
