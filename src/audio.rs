//! Sound playback -- footsteps, creature attacks/deaths/ambient calls, and
//! weather/night ambience. All clips are `include_bytes!`-compiled in (same
//! pattern `model.rs` uses for the bundled `.glb` files), decoded fresh
//! each time they're played via `rodio`.
//!
//! Deliberately non-spatial: every sound is a flat 2D "play this clip"
//! fire-and-forget, the same complexity level as the rest of the engine's
//! effects (no listener-relative panning/attenuation infrastructure exists
//! here, unlike a fuller game audio system).

use std::io::Cursor;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

use crate::creature::CreatureKind;
use crate::daynight::is_night;
use crate::weather::Weather;

const ANIMAL_STEP: &[u8] = include_bytes!("../sounds/animal_step.mp3");
const BIRDS_FLOCK: &[u8] = include_bytes!("../sounds/birds_flock.mp3");
const COW: &[u8] = include_bytes!("../sounds/cow.mp3");
const CREATURE_DEATH: &[u8] = include_bytes!("../sounds/creature_generic_death.mp3");
const GOBLIN_ATTACK: &[u8] = include_bytes!("../sounds/goblin_attack.mp3");
const HUMAN_STEP: &[u8] = include_bytes!("../sounds/human_step.mp3");
const HUMAN_STEP_2: &[u8] = include_bytes!("../sounds/human_step_2.mp3");
const LIGHTNING: &[u8] = include_bytes!("../sounds/lightning.mp3");
const PLAYER_ATTACK: &[u8] = include_bytes!("../sounds/player_attack.mp3");
const SHEEP: &[u8] = include_bytes!("../sounds/sheep.mp3");
const STINGER_ATTACK: &[u8] = include_bytes!("../sounds/stinger_attack.mp3");
const STONE_GOLEM_ATTACK: &[u8] = include_bytes!("../sounds/stone_golem_attack.mp3");
const SUNSCORCH_ATTACK: &[u8] = include_bytes!("../sounds/sunscorch_attack.mp3");
const WEATHER_CLEAR_NIGHT: &[u8] = include_bytes!("../sounds/weather_clear_night.mp3");
const WEATHER_MYST: &[u8] = include_bytes!("../sounds/weather_myst.mp3");
const WEATHER_NIGHT: &[u8] = include_bytes!("../sounds/weather_night.mp3");
const WEATHER_RAIN: &[u8] = include_bytes!("../sounds/weather_rain.mp3");
const WEATHER_STORM: &[u8] = include_bytes!("../sounds/weather_storm.mp3");
const WOLF_ATTACK: &[u8] = include_bytes!("../sounds/wolf_attack.mp3");

/// Which hostile kind's attack clip to play -- `None` for a kind that
/// never attacks (see `CreatureKind::is_hostile`).
fn attack_sound(kind: CreatureKind) -> Option<&'static [u8]> {
    match kind {
        CreatureKind::Wolf => Some(WOLF_ATTACK),
        CreatureKind::Goblin => Some(GOBLIN_ATTACK),
        CreatureKind::Stinger => Some(STINGER_ATTACK),
        CreatureKind::StoneGolem => Some(STONE_GOLEM_ATTACK),
        CreatureKind::Sunscorch => Some(SUNSCORCH_ATTACK),
        CreatureKind::Sheep | CreatureKind::Chicken | CreatureKind::Cow => None,
    }
}

/// Idle vocalization clip for a kind that has one -- only cow/sheep ship a
/// sound file for this (see `CreatureKind::has_ambient_call`).
fn ambient_sound(kind: CreatureKind) -> Option<&'static [u8]> {
    match kind {
        CreatureKind::Cow => Some(COW),
        CreatureKind::Sheep => Some(SHEEP),
        _ => None,
    }
}

/// Which looping ambience track (if any) should be playing right now --
/// pure function of weather + time of day, so it's independently testable
/// without touching real audio output. Rain/storm/mist have their own
/// track regardless of time of day (weather sounds the same at 3pm or
/// 3am); sunny and windy have no weather-specific track, so they fall back
/// to a night-only ambience (sunny nights get the calmer `ClearNight`
/// track; windy nights get the generic `Night` track) and silence during
/// the day, on the assumption that daytime bird-flock one-shots (see
/// `AudioEngine::play_bird_flock`) already carry daytime atmosphere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmbientTrack {
    Rain,
    Storm,
    Myst,
    ClearNight,
    Night,
    Silence,
}

pub fn ambient_track_for(weather: Weather, time_of_day: f32) -> AmbientTrack {
    match weather {
        Weather::Rain => AmbientTrack::Rain,
        Weather::Storm => AmbientTrack::Storm,
        Weather::Mist => AmbientTrack::Myst,
        Weather::Sunny if is_night(time_of_day) => AmbientTrack::ClearNight,
        Weather::Windy if is_night(time_of_day) => AmbientTrack::Night,
        Weather::Sunny | Weather::Windy => AmbientTrack::Silence,
    }
}

impl AmbientTrack {
    fn bytes(self) -> Option<&'static [u8]> {
        match self {
            AmbientTrack::Rain => Some(WEATHER_RAIN),
            AmbientTrack::Storm => Some(WEATHER_STORM),
            AmbientTrack::Myst => Some(WEATHER_MYST),
            AmbientTrack::ClearNight => Some(WEATHER_CLEAR_NIGHT),
            AmbientTrack::Night => Some(WEATHER_NIGHT),
            AmbientTrack::Silence => None,
        }
    }
}

/// Volume for the single looping ambience sink -- kept well under one-shot
/// effect volumes so it reads as background atmosphere, not a foreground
/// sound competing with steps/attacks.
const AMBIENT_VOLUME: f32 = 0.35;

/// Cheap xorshift RNG for pitch/volume jitter -- same style as
/// `creature.rs`'s `SimpleRng`, not shared with it since this lives in a
/// different module and doesn't need anything creature-specific.
struct Rng(u64);

impl Rng {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A random multiplier in `[1 - spread, 1 + spread]`.
    fn jitter(&mut self, spread: f32) -> f32 {
        1.0 + (self.next_f32() * 2.0 - 1.0) * spread
    }
}

/// Owns the audio output device and the one active looping ambience sink.
/// If no output device is available (e.g. a headless CI box), every method
/// silently no-ops rather than panicking -- sound is a nice-to-have, not
/// something that should ever crash or block the game.
pub struct AudioEngine {
    /// Kept alive for as long as the engine exists -- dropping it stops
    /// all playback. Never read otherwise, hence the leading underscore.
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    rng: Rng,
    ambient_sink: Option<Sink>,
    ambient_track: AmbientTrack,
}

impl AudioEngine {
    pub fn new() -> Self {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Self {
                _stream: Some(stream),
                handle: Some(handle),
                rng: Rng(0x9E3779B97F4A7C15),
                ambient_sink: None,
                ambient_track: AmbientTrack::Silence,
            },
            Err(err) => {
                log::warn!("No audio output device available, sounds disabled: {err}");
                Self {
                    _stream: None,
                    handle: None,
                    rng: Rng(1),
                    ambient_sink: None,
                    ambient_track: AmbientTrack::Silence,
                }
            }
        }
    }

    /// Fire-and-forget one-shot playback at an exact volume/pitch. A no-op
    /// if there's no output device, or (defensively) if decoding somehow
    /// fails -- a bad sound should never crash the game.
    fn play(&self, bytes: &'static [u8], volume: f32, pitch: f32) {
        let Some(handle) = &self.handle else { return };
        let Ok(decoder) = Decoder::new(Cursor::new(bytes)) else { return };
        let Ok(sink) = Sink::try_new(handle) else { return };
        sink.set_volume(volume.max(0.0));
        sink.set_speed(pitch.max(0.05));
        sink.append(decoder);
        sink.detach();
    }

    /// `play`, but with small random pitch/volume variance so repeated
    /// plays of the same clip (footsteps in particular) don't sound
    /// identically robotic every time.
    fn play_varied(&mut self, bytes: &'static [u8], base_volume: f32, pitch_spread: f32, volume_spread: f32) {
        let pitch = self.rng.jitter(pitch_spread);
        let volume = base_volume * self.rng.jitter(volume_spread);
        self.play(bytes, volume, pitch);
    }

    /// One player footstep -- alternates between the two human step clips
    /// (not just repeating one) on top of the usual pitch/volume jitter.
    pub fn play_player_step(&mut self) {
        let bytes = if self.rng.next_f32() < 0.5 { HUMAN_STEP } else { HUMAN_STEP_2 };
        self.play_varied(bytes, 0.45, 0.08, 0.18);
    }

    /// One creature footstep, any kind -- a single generic clip (see
    /// `ANIMAL_STEP`), since the engine doesn't ship per-kind step sounds.
    pub fn play_creature_step(&mut self) {
        self.play_varied(ANIMAL_STEP, 0.3, 0.12, 0.22);
    }

    /// A hostile creature's attack landing on a player. A no-op for a kind
    /// with no attack sound (sheep/chicken/cow never attack).
    pub fn play_creature_attack(&mut self, kind: CreatureKind) {
        if let Some(bytes) = attack_sound(kind) {
            self.play_varied(bytes, 0.6, 0.05, 0.1);
        }
    }

    /// The player's own mining swing landing a hit -- the closest existing
    /// action to "attack" the player currently has (there's no player-vs-
    /// creature melee yet).
    pub fn play_player_attack(&mut self) {
        self.play_varied(PLAYER_ATTACK, 0.5, 0.06, 0.12);
    }

    /// Any creature's death -- one generic clip regardless of kind.
    pub fn play_creature_death(&mut self) {
        self.play_varied(CREATURE_DEATH, 0.55, 0.05, 0.1);
    }

    /// A cow/sheep's idle vocalization. A no-op for any other kind.
    pub fn play_creature_ambient(&mut self, kind: CreatureKind) {
        if let Some(bytes) = ambient_sound(kind) {
            self.play_varied(bytes, 0.4, 0.08, 0.15);
        }
    }

    /// A lightning strike (see `App::update_lightning`).
    pub fn play_lightning(&mut self) {
        self.play_varied(LIGHTNING, 0.7, 0.03, 0.1);
    }

    /// A new bird flock spawning (see `App::update_birds`).
    pub fn play_bird_flock(&mut self) {
        self.play_varied(BIRDS_FLOCK, 0.25, 0.03, 0.08);
    }

    /// Starts/stops/swaps the single looping ambience sink to match the
    /// current weather + time of day -- see `ambient_track_for`. Cheap to
    /// call every frame: it's a no-op unless the desired track actually
    /// changed since the last call.
    pub fn update_ambience(&mut self, weather: Weather, time_of_day: f32) {
        let Some(handle) = &self.handle else { return };
        let desired = ambient_track_for(weather, time_of_day);
        if desired == self.ambient_track && (self.ambient_sink.is_some() || desired == AmbientTrack::Silence) {
            return;
        }
        if let Some(sink) = self.ambient_sink.take() {
            sink.stop();
        }
        self.ambient_track = desired;
        let Some(bytes) = desired.bytes() else { return };
        let Ok(decoder) = Decoder::new(Cursor::new(bytes)) else { return };
        let Ok(sink) = Sink::try_new(handle) else { return };
        sink.set_volume(AMBIENT_VOLUME);
        sink.append(decoder.repeat_infinite());
        self.ambient_sink = Some(sink);
    }
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hostile_kind_has_an_attack_sound_and_every_passive_kind_has_none() {
        for kind in [
            CreatureKind::StoneGolem,
            CreatureKind::Wolf,
            CreatureKind::Stinger,
            CreatureKind::Goblin,
            CreatureKind::Sunscorch,
        ] {
            assert!(attack_sound(kind).is_some(), "expected a hostile kind to have an attack sound");
        }
        for kind in [CreatureKind::Sheep, CreatureKind::Chicken, CreatureKind::Cow] {
            assert!(attack_sound(kind).is_none(), "a passive kind should have no attack sound");
        }
    }

    #[test]
    fn only_cow_and_sheep_have_an_ambient_sound() {
        assert!(ambient_sound(CreatureKind::Cow).is_some());
        assert!(ambient_sound(CreatureKind::Sheep).is_some());
        for kind in [
            CreatureKind::Chicken,
            CreatureKind::StoneGolem,
            CreatureKind::Wolf,
            CreatureKind::Stinger,
            CreatureKind::Goblin,
            CreatureKind::Sunscorch,
        ] {
            assert!(ambient_sound(kind).is_none(), "only cow/sheep should have an ambient call sound");
        }
    }

    #[test]
    fn rain_storm_and_mist_pick_their_own_track_regardless_of_time_of_day() {
        for &t in &[0.0, 0.25, 0.5, 0.75] {
            assert_eq!(ambient_track_for(Weather::Rain, t), AmbientTrack::Rain);
            assert_eq!(ambient_track_for(Weather::Storm, t), AmbientTrack::Storm);
            assert_eq!(ambient_track_for(Weather::Mist, t), AmbientTrack::Myst);
        }
    }

    #[test]
    fn sunny_is_silent_by_day_and_clear_night_by_night() {
        assert_eq!(ambient_track_for(Weather::Sunny, 0.25), AmbientTrack::Silence, "noon");
        assert_eq!(ambient_track_for(Weather::Sunny, 0.75), AmbientTrack::ClearNight, "midnight");
    }

    #[test]
    fn windy_is_silent_by_day_and_generic_night_by_night() {
        assert_eq!(ambient_track_for(Weather::Windy, 0.25), AmbientTrack::Silence, "noon");
        assert_eq!(ambient_track_for(Weather::Windy, 0.75), AmbientTrack::Night, "midnight");
    }

    #[test]
    fn every_ambient_track_except_silence_has_bytes() {
        for track in [
            AmbientTrack::Rain,
            AmbientTrack::Storm,
            AmbientTrack::Myst,
            AmbientTrack::ClearNight,
            AmbientTrack::Night,
        ] {
            assert!(track.bytes().is_some());
        }
        assert!(AmbientTrack::Silence.bytes().is_none());
    }

    /// A degenerate/missing audio device (as in a headless test run) must
    /// never panic -- every public method should just silently no-op.
    #[test]
    fn every_playback_method_is_a_harmless_no_op_without_an_output_device() {
        let mut engine = AudioEngine { _stream: None, handle: None, rng: Rng(42), ambient_sink: None, ambient_track: AmbientTrack::Silence };
        engine.play_player_step();
        engine.play_creature_step();
        engine.play_creature_attack(CreatureKind::Wolf);
        engine.play_player_attack();
        engine.play_creature_death();
        engine.play_creature_ambient(CreatureKind::Cow);
        engine.play_lightning();
        engine.play_bird_flock();
        engine.update_ambience(Weather::Storm, 0.75);
    }

    #[test]
    fn rng_jitter_stays_within_the_requested_spread() {
        let mut rng = Rng(12345);
        for _ in 0..1000 {
            let j = rng.jitter(0.2);
            assert!((0.8..=1.2).contains(&j), "jitter {j} escaped its [0.8, 1.2] spread");
        }
    }
}
