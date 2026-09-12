//! Cosmetic player poses shared by the local controller and multiplayer peers.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clip {
    #[default]
    Idle,
    Walk,
    Run,
    Attack,
    Work,
    Jump,
    Dance,
    Angry,
}

impl Clip {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle", Self::Walk => "walk", Self::Run => "run",
            Self::Attack => "attack", Self::Work => "work", Self::Jump => "jump",
            Self::Dance => "dance", Self::Angry => "angry",
        }
    }

    pub fn duration(self) -> Option<f32> {
        match self {
            Self::Attack => Some(0.9), Self::Work | Self::Jump => Some(1.2),
            Self::Dance => Some(3.2), Self::Angry => Some(2.0),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub clip: Clip,
    pub time: f32,
    /// Changes even when a player repeats the same action, restarting its pose.
    pub sequence: u64,
}

impl Animation {
    pub fn start(&mut self, clip: Clip) {
        self.clip = clip;
        self.time = 0.0;
        self.sequence = self.sequence.wrapping_add(1);
    }

    pub fn advance(&mut self, dt: f32, speed: f32, grounded: bool) {
        self.time += dt.clamp(0.0, 0.1);
        let moving = speed > 0.2;
        let gesture = matches!(self.clip, Clip::Dance | Clip::Angry);
        let active = self.clip.duration().is_some_and(|duration| self.time < duration);
        let next = if !grounded { Clip::Jump }
            else if active && !(gesture && moving) { self.clip }
            else if speed > 5.0 { Clip::Run }
            else if moving { Clip::Walk }
            else { Clip::Idle };
        if next != self.clip || self.time > 3600.0 { self.start(next); }
    }

    /// Ignore malformed or reordered cosmetic state; this never changes physics.
    pub fn accept(&mut self, incoming: Self) -> bool {
        if !incoming.time.is_finite() || !(0.0..=3600.0).contains(&incoming.time) {
            return false;
        }
        if incoming.sequence < self.sequence
            || (incoming.sequence == self.sequence &&
                (incoming.time < self.time || incoming.clip != self.clip)) {
            return false;
        }
        *self = incoming;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actions_finish_gestures_cancel_on_movement_and_airborne_players_jump() {
        let mut a = Animation::default();
        a.start(Clip::Dance);
        a.advance(0.1, 0.0, true);
        assert_eq!(a.clip, Clip::Dance);
        a.advance(0.1, 3.0, true);
        assert_eq!(a.clip, Clip::Walk);
        a.start(Clip::Work);
        a.advance(0.1, 3.0, true);
        assert_eq!(a.clip, Clip::Work);
        for _ in 0..13 { a.advance(0.1, 3.0, true); }
        assert_eq!(a.clip, Clip::Walk);
        a.advance(0.1, 3.0, false);
        assert_eq!(a.clip, Clip::Jump);
        for _ in 0..13 { a.advance(0.1, 0.0, true); }
        assert_eq!(a.clip, Clip::Idle);
    }
    #[test]
    fn replicated_actions_restart_and_reordered_or_invalid_states_are_ignored() {
        let mut sender = Animation::default();
        sender.start(Clip::Attack);
        sender.advance(0.1,0.0,true);
        let old = sender;
        let mut receiver = Animation::default();
        assert!(receiver.accept(sender));
        sender.start(Clip::Attack);
        assert!(receiver.accept(sender));
        assert_eq!(receiver.time,0.0);
        assert!(!receiver.accept(old));
        assert!(!receiver.accept(Animation { time:f32::NAN, ..sender }));
        assert!(!receiver.accept(Animation { time:-1.0, ..sender }));
    }
}
