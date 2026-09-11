//! Sound playback -- footsteps, creature attacks/deaths/ambient calls,
//! weather/night ambience, and a looping background-music theme. All clips
//! are `include_bytes!`-compiled in (same pattern `model.rs` uses for the
//! bundled `.glb` files), decoded fresh each time they're played via
//! `rodio`.
//!
//! Creature-originated sounds (steps/attacks/deaths/ambient calls) are
//! spatialized -- distance falloff and left/right panning relative to the
//! listener (the local player's camera) -- via `rodio`'s `SpatialSink`; see
//! `spatial_positions`. Everything else (the player's own footsteps/mining
//! swing, weather ambience, lightning, bird flocks, background music) plays
//! centered/flat, since it either originates at the listener itself or is
//! meant to read as diffuse/omnipresent atmosphere rather than coming from
//! one point.
//!
//! The weather/night ambience track and the background-music theme both
//! crossfade rather than cut -- see `update_ambience`, `update_music`, and
//! the shared `approach` ease helper -- so a weather or day/night change
//! never snaps audibly.

use std::io::Cursor;

use glam::Vec3;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, SpatialSink, Source};

use crate::creature::CreatureKind;
use crate::daynight::is_night;
use crate::weather::Weather;

const ANIMAL_STEP: &[u8] = include_bytes!("../sounds/animal_step.mp3");
const BACKGROUND_MUSIC: &[u8] = include_bytes!("../sounds/background_music.mp3");
const BIRDS_FLOCK: &[u8] = include_bytes!("../sounds/birds_flock.mp3");
const COW: &[u8] = include_bytes!("../sounds/cow.mp3");
const CREATURE_DEATH: &[u8] = include_bytes!("../sounds/creature_generic_death.mp3");
const DRAGON_ATTACK: &[u8] = include_bytes!("../sounds/dragon_attack.mp3");
const DRAGON_FLY: &[u8] = include_bytes!("../sounds/dragon_fly.mp3");
const WATERFALL: &[u8] = include_bytes!("../sounds/waterfall.mp3");
const GOBLIN_ATTACK: &[u8] = include_bytes!("../sounds/goblin_attack.mp3");
const HUMAN_STEP: &[u8] = include_bytes!("../sounds/human_step.mp3");
const HUMAN_STEP_2: &[u8] = include_bytes!("../sounds/human_step_2.mp3");
const LIGHTNING: &[u8] = include_bytes!("../sounds/lightning.mp3");
const PLAYER_ATTACK: &[u8] = include_bytes!("../sounds/player_attack.mp3");
const SHEEP: &[u8] = include_bytes!("../sounds/sheep.mp3");
const SKELETON: &[u8] = include_bytes!("../sounds/skeleton.mp3");
const STINGER_ATTACK: &[u8] = include_bytes!("../sounds/stinger_attack.mp3");
const STONE_GOLEM_ATTACK: &[u8] = include_bytes!("../sounds/stone_golem_attack.mp3");
const SUNSCORCH_ATTACK: &[u8] = include_bytes!("../sounds/sunscorch_attack.mp3");
const WEATHER_CLEAR_NIGHT: &[u8] = include_bytes!("../sounds/weather_clear_night.mp3");
const WEATHER_MYST: &[u8] = include_bytes!("../sounds/weather_myst.mp3");
const WEATHER_NIGHT: &[u8] = include_bytes!("../sounds/weather_night.mp3");
const WEATHER_RAIN: &[u8] = include_bytes!("../sounds/weather_rain.mp3");
const WEATHER_STORM: &[u8] = include_bytes!("../sounds/weather_storm.mp3");
const WOLF_ATTACK: &[u8] = include_bytes!("../sounds/wolf_attack.mp3");
const ZOMBIE_GROWL: &[u8] = include_bytes!("../sounds/zombie_growl.mp3");
const ZOMBIE_PITCH_SPREAD: f32 = 0.18;

/// Which hostile kind's attack clip to play -- `None` for a kind that
/// never attacks (see `CreatureKind::is_hostile`).
fn attack_sound(kind: CreatureKind) -> Option<&'static [u8]> {
    match kind {
        CreatureKind::Wolf => Some(WOLF_ATTACK),
        CreatureKind::Goblin => Some(GOBLIN_ATTACK),
        CreatureKind::Stinger => Some(STINGER_ATTACK),
        CreatureKind::StoneGolem => Some(STONE_GOLEM_ATTACK),
        CreatureKind::Sunscorch => Some(SUNSCORCH_ATTACK),
        CreatureKind::Zombie => Some(ZOMBIE_GROWL),
        CreatureKind::Skeleton => Some(SKELETON),
        CreatureKind::DragonGreen | CreatureKind::DragonRed => Some(DRAGON_ATTACK),
        CreatureKind::Sheep | CreatureKind::Chicken | CreatureKind::Cow | CreatureKind::Fish => None,
    }
}

/// Idle vocalization clip for a kind that has one -- cow, sheep and zombie ship a
/// sound file for this (see `CreatureKind::has_ambient_call`).
fn ambient_sound(kind: CreatureKind) -> Option<&'static [u8]> {
    match kind {
        CreatureKind::Cow => Some(COW),
        CreatureKind::Sheep => Some(SHEEP),
        CreatureKind::Zombie => Some(ZOMBIE_GROWL),
        _ => None,
    }
}

fn flying_dragon_positions(entries: &[([f32; 3], u8, f32, u8, f32)], listener: Vec3) -> Vec<Vec3> {
    use crate::creature::AnimClip;
    entries.iter().filter_map(|&(pos, kind, _, clip, _)| {
        let pos = Vec3::from_array(pos);
        (CreatureKind::from_u8(kind & 0x0f).is_dragon()
            && matches!(AnimClip::from_u8(clip), AnimClip::Fly | AnimClip::AttackFly)
            && pos.is_finite() && pos.distance_squared(listener) <= 128.0 * 128.0)
            .then_some(pos)
    }).collect()
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

/// Volume for the looping background music theme -- deliberately quieter
/// than `AMBIENT_VOLUME` so it stays a faint undercurrent under weather/
/// creature ambience rather than a foreground track.
const MUSIC_VOLUME: f32 = 0.16;

/// How long (in seconds) a full silence-to-target fade takes for both the
/// music and ambient-track crossfades -- long enough to read as a smooth
/// ease rather than a cut, short enough not to lag noticeably behind a
/// day/night or weather transition.
const FADE_SECONDS: f32 = 4.0;

/// Moves `current` toward `target` at a constant rate such that a full
/// `0.0..=1.0` sweep takes `duration` seconds, clamping exactly to `target`
/// once within one step -- the shared ease-in/ease-out primitive behind the
/// music and ambient-track crossfades.
fn approach(current: f32, target: f32, dt: f32, duration: f32) -> f32 {
    if duration <= 0.0 {
        return target;
    }
    let max_delta = dt / duration;
    if (target - current).abs() <= max_delta {
        target
    } else if target > current {
        current + max_delta
    } else {
        current - max_delta
    }
}

/// Half the ear-to-ear separation used to derive left/right panning for a
/// creature sound -- see `spatial_positions`. Not a real physical head
/// width; just enough separation for `rodio`'s per-ear falloff model to
/// produce a clear stereo balance without being so wide it distorts the
/// distance falloff itself.
const EAR_HALF_SEPARATION: f32 = 0.1;

/// Reference distance (in blocks) within which a creature sound plays at
/// full volume; beyond it, volume falls off with the inverse square of how
/// far past the reference distance the source is -- see
/// `spatial_positions`. Tuned per sound category: an attack should still
/// read clearly across a typical aggro radius, while a single footstep is
/// a much smaller/closer-range detail sound.
const ATTACK_REFERENCE_DISTANCE: f32 = 7.0;
const DEATH_REFERENCE_DISTANCE: f32 = 7.0;
const AMBIENT_CALL_REFERENCE_DISTANCE: f32 = 5.0;
const STEP_REFERENCE_DISTANCE: f32 = 3.0;

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

/// Owns the audio output device, the current listener pose, and the one
/// active looping ambience sink. If no output device is available (e.g. a
/// headless CI box), every method silently no-ops rather than panicking --
/// sound is a nice-to-have, not something that should ever crash or block
/// the game.
pub struct AudioEngine {
    /// Reused spatial loops, one per audible flying dragon. Keeping the sinks
    /// alive avoids restarting or stacking the recording on every frame.
    flight_sinks: Vec<SpatialSink>,
    waterfall_sinks: Vec<SpatialSink>,
    /// Kept alive for as long as the engine exists -- dropping it stops
    /// all playback. Never read otherwise, hence the leading underscore.
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    rng: Rng,
    ambient_sink: Option<Sink>,
    /// Current fade multiplier (`0.0..=1.0`) applied on top of
    /// `AMBIENT_VOLUME` for `ambient_sink` -- ramps toward `1.0` after a
    /// track starts, via `approach` in `update_ambience`.
    ambient_fade: f32,
    /// The previous ambient sink, still playing while it fades out after a
    /// track change -- dropped once `ambient_out_fade` reaches zero.
    ambient_out_sink: Option<Sink>,
    ambient_out_fade: f32,
    ambient_track: AmbientTrack,
    /// The looping background-music sink -- created once (lazily, on first
    /// `update_ambience` call with an output device) and kept alive for the
    /// life of the engine; day/night is expressed purely as a volume fade
    /// (see `music_fade`) rather than stopping/restarting playback, so the
    /// loop never audibly restarts mid-phrase.
    music_sink: Option<Sink>,
    /// Current fade multiplier (`0.0..=1.0`) applied on top of
    /// `MUSIC_VOLUME` -- eased toward `1.0` by day and `0.0` by night, via
    /// `approach` in `update_ambience`.
    music_fade: f32,
    /// The local player's camera eye position -- the origin every creature
    /// sound's distance/panning is computed relative to. Updated once per
    /// frame via `update_listener`.
    listener_pos: Vec3,
    /// The local player's camera "right" direction (world-space, always
    /// horizontal -- see `Camera::right`), used to place the two virtual
    /// ears either side of `listener_pos` for panning.
    listener_right: Vec3,
}

impl AudioEngine {
    pub fn new() -> Self {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Self {
                flight_sinks: Vec::new(),
                waterfall_sinks: Vec::new(),
                _stream: Some(stream),
                handle: Some(handle),
                rng: Rng(0x9E3779B97F4A7C15),
                ambient_sink: None,
                ambient_fade: 0.0,
                ambient_out_sink: None,
                ambient_out_fade: 0.0,
                ambient_track: AmbientTrack::Silence,
                music_sink: None,
                music_fade: 0.0,
                listener_pos: Vec3::ZERO,
                listener_right: Vec3::X,
            },
            Err(err) => {
                log::warn!("No audio output device available, sounds disabled: {err}");
                Self {
                    flight_sinks: Vec::new(),
                    waterfall_sinks: Vec::new(),
                    _stream: None,
                    handle: None,
                    rng: Rng(1),
                    ambient_sink: None,
                    ambient_fade: 0.0,
                    ambient_out_sink: None,
                    ambient_out_fade: 0.0,
                    ambient_track: AmbientTrack::Silence,
                    music_sink: None,
                    music_fade: 0.0,
                    listener_pos: Vec3::ZERO,
                    listener_right: Vec3::X,
                }
            }
        }
    }

    /// Updates the listener pose every creature sound's distance falloff
    /// and left/right panning is computed relative to -- call once per
    /// frame (with the camera's eye position and its always-horizontal
    /// `right()` vector) before playing any creature sounds that frame.
    pub fn update_listener(&mut self, pos: Vec3, right: Vec3) {
        self.listener_pos = pos;
        self.listener_right = right;
    }

    /// Derives the emitter/left-ear/right-ear positions `rodio`'s
    /// `SpatialSink` needs to reproduce both distance falloff and
    /// left/right panning for a sound at `emitter_pos`, relative to the
    /// current listener.
    ///
    /// Positions are expressed relative to the listener and uniformly
    /// scaled by `1 / reference_distance` before being handed to
    /// `SpatialSink` -- `rodio`'s own falloff model is a flat
    /// `1 / distance^2` with no "stays at full volume up close" floor,
    /// which (fed real block distances directly) would make even a
    /// creature standing right next to the player sound quietly
    /// attenuated. Scaling by the reference distance turns that into a
    /// floor: distances at or inside `reference_distance` collapse to
    /// `<= 1.0` in the scaled space (full volume, since `rodio` caps each
    /// ear's modifier at `1.0`), and distances beyond it fall off with the
    /// inverse square of how far past it they are. Left/right panning is
    /// unaffected by the uniform scale -- it only depends on the *ratio*
    /// between the two ears' distances to the emitter, which a uniform
    /// scale preserves.
    fn spatial_positions(&self, emitter_pos: Vec3, reference_distance: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let scale = 1.0 / reference_distance.max(0.01);
        let emitter = ((emitter_pos - self.listener_pos) * scale).to_array();
        let ear_offset = self.listener_right * (EAR_HALF_SEPARATION * scale);
        ((emitter), (-ear_offset).to_array(), ear_offset.to_array())
    }

    /// Fire-and-forget centered (non-spatial) one-shot playback at an exact
    /// volume/pitch. A no-op if there's no output device, or (defensively)
    /// if decoding somehow fails -- a bad sound should never crash the game.
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

    /// Fire-and-forget spatialized one-shot playback -- distance falloff
    /// plus left/right panning relative to the listener, with the usual
    /// pitch/volume jitter on top. Falls back to a harmless no-op under the
    /// same conditions as `play`.
    fn play_spatial(
        &mut self,
        bytes: &'static [u8],
        base_volume: f32,
        pitch_spread: f32,
        volume_spread: f32,
        emitter_pos: Vec3,
        reference_distance: f32,
    ) {
        let Some(handle) = &self.handle else { return };
        let Ok(decoder) = Decoder::new(Cursor::new(bytes)) else { return };
        let (emitter, left_ear, right_ear) = self.spatial_positions(emitter_pos, reference_distance);
        let Ok(sink) = SpatialSink::try_new(handle, emitter, left_ear, right_ear) else { return };
        let pitch = self.rng.jitter(pitch_spread);
        let volume = base_volume * self.rng.jitter(volume_spread);
        sink.set_volume(volume.max(0.0));
        sink.set_speed(pitch.max(0.05));
        sink.append(decoder);
        sink.detach();
    }

    /// One player footstep -- alternates between the two human step clips
    /// (not just repeating one) on top of the usual pitch/volume jitter.
    /// Centered/non-spatial: it originates at the listener itself.
    pub fn play_player_step(&mut self) {
        let bytes = if self.rng.next_f32() < 0.5 { HUMAN_STEP } else { HUMAN_STEP_2 };
        self.play_varied(bytes, 0.45, 0.08, 0.18);
    }

    /// One creature footstep at `pos`, any kind -- a single generic clip
    /// (see `ANIMAL_STEP`), since the engine doesn't ship per-kind step
    /// sounds. Spatialized, falls off over a short range (see
    /// `STEP_REFERENCE_DISTANCE`) since it's a close-range detail sound.
    pub fn play_creature_step(&mut self, pos: Vec3) {
        self.play_spatial(ANIMAL_STEP, 0.3, 0.12, 0.22, pos, STEP_REFERENCE_DISTANCE);
    }

    /// A hostile creature's attack landing on a player, at `pos`. A no-op
    /// for a kind with no attack sound (sheep/chicken/cow never attack).
    /// Spatialized, carries further than a footstep (see
    /// `ATTACK_REFERENCE_DISTANCE`) so it still reads clearly across a
    /// typical aggro radius.
    pub fn play_creature_attack(&mut self, kind: CreatureKind, pos: Vec3) {
        if let Some(bytes) = attack_sound(kind) {
            let pitch_spread = if kind == CreatureKind::Zombie { ZOMBIE_PITCH_SPREAD } else { 0.05 };
            let reference = if kind.is_dragon() { 24.0 } else { ATTACK_REFERENCE_DISTANCE };
            self.play_spatial(bytes, 0.6, pitch_spread, 0.1, pos, reference);
        }
    }

    /// Continuous flight audio follows rendered state on both host and clients.
    /// Landing, death, removal, or leaving earshot drops the corresponding loop.
    pub fn update_creature_flight(&mut self, entries: &[([f32; 3], u8, f32, u8, f32)]) {
        let positions = flying_dragon_positions(entries, self.listener_pos);
        self.flight_sinks.truncate(positions.len());
        let Some(handle) = &self.handle else { return };
        while self.flight_sinks.len() < positions.len() {
            let Ok(decoder) = Decoder::new(Cursor::new(DRAGON_FLY)) else { break };
            let Ok(sink) = SpatialSink::try_new(handle, [0.0; 3], [-0.1, 0.0, 0.0], [0.1, 0.0, 0.0]) else { break };
            sink.set_volume(0.0);
            sink.append(decoder.repeat_infinite());
            self.flight_sinks.push(sink);
        }
        for (index, pos) in positions.into_iter().enumerate().take(self.flight_sinks.len()) {
            let (emitter, left, right) = self.spatial_positions(pos, 24.0);
            let sink = &self.flight_sinks[index];
            sink.set_emitter_position(emitter);
            sink.set_left_ear_position(left);
            sink.set_right_ear_position(right);
            sink.set_volume(0.45);
        }
    }

    /// Merge adjacent spill columns into one source, with at most two loops.
    /// Smooth distance attenuation reaches zero before a loop leaves earshot.
    pub fn update_waterfalls(&mut self, falls: &[crate::water::Waterfall]) {
        let mut positions: Vec<Vec3> = Vec::new();
        for fall in falls {
            let pos = fall.sound_position();
            if pos.distance_squared(self.listener_pos) < 48.0*48.0
                && positions.iter().all(|p|p.distance_squared(pos)>8.0*8.0) {
                positions.push(pos);
                if positions.len()==2 {break;}
            }
        }
        self.waterfall_sinks.truncate(positions.len());
        let Some(handle) = &self.handle else {return};
        while self.waterfall_sinks.len()<positions.len() {
            let Ok(decoder)=Decoder::new(Cursor::new(WATERFALL)) else {break};
            let Ok(sink)=SpatialSink::try_new(handle,[0.0;3],[-0.1,0.0,0.0],[0.1,0.0,0.0]) else {break};
            sink.set_volume(0.0);
            sink.append(decoder.repeat_infinite());
            self.waterfall_sinks.push(sink);
        }
        for (i,pos) in positions.into_iter().enumerate().take(self.waterfall_sinks.len()) {
            let (emitter,left,right)=self.spatial_positions(pos,12.0);
            let sink=&self.waterfall_sinks[i];
            sink.set_emitter_position(emitter);
            sink.set_left_ear_position(left);
            sink.set_right_ear_position(right);
            let fade=((48.0-pos.distance(self.listener_pos))/16.0).clamp(0.0,1.0);
            sink.set_volume(0.5*fade*fade);
        }
    }

    /// The player's own mining swing landing a hit -- the closest existing
    /// action to "attack" the player currently has (there's no player-vs-
    /// creature melee yet). Centered/non-spatial: it originates at the
    /// listener itself.
    pub fn play_player_attack(&mut self) {
        self.play_varied(PLAYER_ATTACK, 0.5, 0.06, 0.12);
    }

    /// Any creature's death at `pos` -- one generic clip regardless of
    /// kind. Spatialized, same carry as an attack.
    pub fn play_creature_death(&mut self, pos: Vec3) {
        self.play_spatial(CREATURE_DEATH, 0.55, 0.05, 0.1, pos, DEATH_REFERENCE_DISTANCE);
    }

    /// A cow, sheep or zombie's ambient vocalization at `pos`. A no-op for any other
    /// kind. Spatialized.
    pub fn play_creature_ambient(&mut self, kind: CreatureKind, pos: Vec3) {
        if let Some(bytes) = ambient_sound(kind) {
            let pitch_spread = if kind == CreatureKind::Zombie { ZOMBIE_PITCH_SPREAD } else { 0.08 };
            self.play_spatial(bytes, 0.4, pitch_spread, 0.15, pos, AMBIENT_CALL_REFERENCE_DISTANCE);
        }
    }

    /// A lightning strike (see `App::update_lightning`). Centered: no
    /// world position is tracked for a strike, and thunder reads as
    /// diffuse/overhead rather than from one point anyway.
    pub fn play_lightning(&mut self) {
        self.play_varied(LIGHTNING, 0.7, 0.03, 0.1);
    }

    /// A new bird flock spawning (see `App::update_birds`). Centered,
    /// deliberately -- kept as ambience rather than spatialized to the
    /// flock's spawn point, which starts far outside `ATTACK_REFERENCE_
    /// DISTANCE`-scale ranges and would just play near-silent.
    pub fn play_bird_flock(&mut self) {
        self.play_varied(BIRDS_FLOCK, 0.25, 0.03, 0.08);
    }

    /// Starts/swaps the single looping ambience sink to match the current
    /// weather + time of day -- see `ambient_track_for` -- crossfading the
    /// old track out and the new one in over `FADE_SECONDS` rather than
    /// cutting between them, and eases the background-music theme in by day
    /// and out by night (see `update_music`). Cheap to call every frame:
    /// once a fade settles at its target there's nothing left to do but a
    /// couple of float comparisons and `set_volume` calls. Centered, like
    /// all ambience.
    pub fn update_ambience(&mut self, weather: Weather, time_of_day: f32, dt: f32) {
        if self.handle.is_none() {
            return;
        }
        let desired = ambient_track_for(weather, time_of_day);
        if desired != self.ambient_track {
            if let Some(sink) = self.ambient_sink.take() {
                if let Some(old) = self.ambient_out_sink.replace(sink) {
                    old.stop();
                }
                self.ambient_out_fade = self.ambient_fade;
            }
            self.ambient_track = desired;
            self.ambient_fade = 0.0;
            if let Some(bytes) = desired.bytes() {
                let handle = self.handle.as_ref().expect("checked above");
                if let Ok(decoder) = Decoder::new(Cursor::new(bytes)) {
                    if let Ok(sink) = Sink::try_new(handle) {
                        sink.set_volume(0.0);
                        sink.append(decoder.repeat_infinite());
                        self.ambient_sink = Some(sink);
                    }
                }
            }
        }

        if let Some(sink) = &self.ambient_sink {
            self.ambient_fade = approach(self.ambient_fade, 1.0, dt, FADE_SECONDS);
            sink.set_volume(AMBIENT_VOLUME * self.ambient_fade);
        }
        if let Some(sink) = &self.ambient_out_sink {
            self.ambient_out_fade = approach(self.ambient_out_fade, 0.0, dt, FADE_SECONDS);
            sink.set_volume(AMBIENT_VOLUME * self.ambient_out_fade);
            if self.ambient_out_fade <= 0.0 {
                sink.stop();
                self.ambient_out_sink = None;
            }
        }

        self.update_music(time_of_day, dt);
    }

    /// Eases the looping background-music theme in by day and out by night
    /// -- a quiet undercurrent (see `MUSIC_VOLUME`) that never plays while
    /// `is_night` holds. The sink itself is created once and left looping
    /// for the engine's whole lifetime; day/night is expressed purely as a
    /// volume fade so the track never audibly restarts mid-phrase.
    fn update_music(&mut self, time_of_day: f32, dt: f32) {
        let Some(handle) = &self.handle else { return };
        if self.music_sink.is_none() {
            if let Ok(decoder) = Decoder::new(Cursor::new(BACKGROUND_MUSIC)) {
                if let Ok(sink) = Sink::try_new(handle) {
                    sink.set_volume(0.0);
                    sink.append(decoder.repeat_infinite());
                    self.music_sink = Some(sink);
                }
            }
        }
        let target = if is_night(time_of_day) { 0.0 } else { 1.0 };
        self.music_fade = approach(self.music_fade, target, dt, FADE_SECONDS);
        if let Some(sink) = &self.music_sink {
            sink.set_volume(MUSIC_VOLUME * self.music_fade);
        }
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
    fn new_creature_recordings_decode_and_are_assigned_to_the_correct_kinds() {
        assert_eq!(attack_sound(CreatureKind::Zombie), Some(ZOMBIE_GROWL));
        assert_eq!(ambient_sound(CreatureKind::Zombie), Some(ZOMBIE_GROWL));
        assert_eq!(attack_sound(CreatureKind::Skeleton), Some(SKELETON));
        for kind in [CreatureKind::DragonGreen, CreatureKind::DragonRed] {
            assert_eq!(attack_sound(kind), Some(DRAGON_ATTACK));
        }
        for bytes in [ZOMBIE_GROWL, SKELETON, DRAGON_ATTACK, DRAGON_FLY] {
            let decoder = Decoder::new(Cursor::new(bytes)).expect("bundled sound must decode");
            assert!(decoder.into_iter().any(|sample| sample != 0), "recording must contain audible samples");
        }
        let mut rng = Rng(42);
        let pitches: Vec<_> = (0..64).map(|_| rng.jitter(ZOMBIE_PITCH_SPREAD)).collect();
        assert!(pitches.iter().all(|&p| (0.82..=1.18).contains(&p)));
        assert!(pitches.windows(2).any(|p| (p[0] - p[1]).abs() > 0.05));
    }

    #[test]
    fn flight_audio_tracks_airborne_dragons_and_stops_for_grounded_or_removed_ones() {
        use crate::creature::AnimClip;
        let mut entries = vec![
            ([1.0, 10.0, 0.0], CreatureKind::DragonGreen.to_u8(), 0.0, AnimClip::Fly.to_u8(), 0.0),
            ([2.0, 10.0, 0.0], CreatureKind::DragonRed.to_u8(), 0.0, AnimClip::AttackFly.to_u8(), 0.0),
            ([3.0, 0.0, 0.0], CreatureKind::DragonRed.to_u8(), 0.0, AnimClip::AttackWalk.to_u8(), 0.0),
            ([4.0, 0.0, 0.0], CreatureKind::Zombie.to_u8(), 0.0, AnimClip::Walk.to_u8(), 0.0),
            ([500.0, 0.0, 0.0], CreatureKind::DragonGreen.to_u8(), 0.0, AnimClip::Fly.to_u8(), 0.0),
        ];
        assert_eq!(flying_dragon_positions(&entries, Vec3::ZERO).len(), 2);
        entries[0].3 = AnimClip::Walk.to_u8();
        entries[1].3 = AnimClip::Idle.to_u8();
        assert!(flying_dragon_positions(&entries, Vec3::ZERO).is_empty());
        assert!(flying_dragon_positions(&[], Vec3::ZERO).is_empty());
    }

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
    fn only_cow_sheep_and_zombie_have_an_ambient_sound() {
        assert!(ambient_sound(CreatureKind::Cow).is_some());
        assert!(ambient_sound(CreatureKind::Sheep).is_some());
        assert!(ambient_sound(CreatureKind::Zombie).is_some());
        for kind in [
            CreatureKind::Chicken,
            CreatureKind::StoneGolem,
            CreatureKind::Wolf,
            CreatureKind::Stinger,
            CreatureKind::Goblin,
            CreatureKind::Sunscorch,
        ] {
            assert!(ambient_sound(kind).is_none(), "this kind has no ambient call sound");
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

    fn silent_engine() -> AudioEngine {
        AudioEngine {
            flight_sinks: Vec::new(),
            waterfall_sinks: Vec::new(),
            _stream: None,
            handle: None,
            rng: Rng(42),
            ambient_sink: None,
            ambient_fade: 0.0,
            ambient_out_sink: None,
            ambient_out_fade: 0.0,
            ambient_track: AmbientTrack::Silence,
            music_sink: None,
            music_fade: 0.0,
            listener_pos: Vec3::ZERO,
            listener_right: Vec3::X,
        }
    }

    #[test]
    fn waterfall_clip_decodes_and_silent_output_handles_source_removal() {
        assert!(Decoder::new(Cursor::new(WATERFALL)).unwrap().next().is_some());
        let mut engine=silent_engine();
        let fall=crate::water::Waterfall {lip:Vec3::new(1.0,8.0,0.0),bottom:1.0,direction:Vec3::X};
        engine.update_waterfalls(&[fall]);
        engine.update_waterfalls(&[]);
        assert!(engine.waterfall_sinks.is_empty());
    }

    /// A degenerate/missing audio device (as in a headless test run) must
    /// never panic -- every public method should just silently no-op.
    #[test]
    fn every_playback_method_is_a_harmless_no_op_without_an_output_device() {
        let mut engine = silent_engine();
        engine.update_listener(Vec3::new(1.0, 2.0, 3.0), Vec3::X);
        engine.play_player_step();
        engine.play_creature_step(Vec3::new(5.0, 0.0, 0.0));
        engine.play_creature_attack(CreatureKind::Wolf, Vec3::new(2.0, 0.0, 0.0));
        engine.play_player_attack();
        engine.play_creature_death(Vec3::new(3.0, 0.0, 3.0));
        engine.play_creature_ambient(CreatureKind::Cow, Vec3::new(-4.0, 0.0, 1.0));
        engine.play_creature_ambient(CreatureKind::Zombie, Vec3::ZERO);
        engine.update_creature_flight(&[([5.0, 10.0, 0.0], CreatureKind::DragonRed.to_u8(), 0.0, crate::creature::AnimClip::Fly.to_u8(), 0.0)]);
        engine.update_creature_flight(&[]);
        assert!(engine.flight_sinks.is_empty());
        engine.play_lightning();
        engine.play_bird_flock();
        engine.update_ambience(Weather::Storm, 0.75, 1.0 / 60.0);
    }

    #[test]
    fn approach_reaches_target_exactly_without_overshoot() {
        let mut v = 0.0;
        for _ in 0..1000 {
            v = approach(v, 1.0, 1.0 / 60.0, FADE_SECONDS);
        }
        assert_eq!(v, 1.0);
        for _ in 0..1000 {
            v = approach(v, 0.0, 1.0 / 60.0, FADE_SECONDS);
        }
        assert_eq!(v, 0.0);
    }

    #[test]
    fn music_and_ambient_fades_stay_a_no_op_without_an_output_device() {
        let mut engine = silent_engine();
        for _ in 0..10 {
            engine.update_ambience(Weather::Sunny, 0.75, 1.0 / 60.0);
        }
        assert!(engine.music_sink.is_none());
        assert!(engine.ambient_sink.is_none());
    }

    #[test]
    fn rng_jitter_stays_within_the_requested_spread() {
        let mut rng = Rng(12345);
        for _ in 0..1000 {
            let j = rng.jitter(0.2);
            assert!((0.8..=1.2).contains(&j), "jitter {j} escaped its [0.8, 1.2] spread");
        }
    }

    #[test]
    fn a_source_dead_ahead_within_reference_distance_pans_dead_center() {
        let mut engine = silent_engine();
        engine.update_listener(Vec3::ZERO, Vec3::X);
        // Straight down the listener's forward axis (Z here, since "right"
        // is X) -- equidistant from both ears, so panning should be exactly
        // centered regardless of the (well within reference_distance) range.
        let (_, left_ear, right_ear) = engine.spatial_positions(Vec3::new(0.0, 0.0, 2.0), STEP_REFERENCE_DISTANCE);
        let left_dist = Vec3::from_array(left_ear).distance(Vec3::new(0.0, 0.0, 2.0) / STEP_REFERENCE_DISTANCE);
        let right_dist = Vec3::from_array(right_ear).distance(Vec3::new(0.0, 0.0, 2.0) / STEP_REFERENCE_DISTANCE);
        assert!(
            (left_dist - right_dist).abs() < 1e-5,
            "a dead-ahead source should be equidistant from both ears: left={left_dist} right={right_dist}"
        );
    }

    #[test]
    fn a_source_to_the_right_is_closer_to_the_right_ear_than_the_left() {
        let mut engine = silent_engine();
        engine.update_listener(Vec3::ZERO, Vec3::X);
        let emitter = Vec3::new(4.0, 0.0, 0.0); // straight along "right"
        let (scaled_emitter, left_ear, right_ear) = engine.spatial_positions(emitter, STEP_REFERENCE_DISTANCE);
        let scaled_emitter = Vec3::from_array(scaled_emitter);
        let left_dist = Vec3::from_array(left_ear).distance(scaled_emitter);
        let right_dist = Vec3::from_array(right_ear).distance(scaled_emitter);
        assert!(
            right_dist < left_dist,
            "a source to the listener's right should measure closer to the right ear: left={left_dist} right={right_dist}"
        );
    }

    #[test]
    fn moving_the_listener_changes_the_relative_emitter_position() {
        let mut engine = silent_engine();
        let emitter = Vec3::new(10.0, 0.0, 0.0);

        engine.update_listener(Vec3::ZERO, Vec3::X);
        let (near, ..) = engine.spatial_positions(emitter, ATTACK_REFERENCE_DISTANCE);

        engine.update_listener(Vec3::new(9.0, 0.0, 0.0), Vec3::X);
        let (far, ..) = engine.spatial_positions(emitter, ATTACK_REFERENCE_DISTANCE);

        let near_dist = Vec3::from_array(near).length();
        let far_dist = Vec3::from_array(far).length();
        assert!(
            far_dist < near_dist,
            "moving the listener toward the emitter should shrink the relative (scaled) distance: {near_dist} -> {far_dist}"
        );
    }

    #[test]
    fn a_source_at_the_reference_distance_scales_to_unit_distance() {
        let mut engine = silent_engine();
        engine.update_listener(Vec3::ZERO, Vec3::X);
        let (emitter, ..) = engine.spatial_positions(Vec3::new(0.0, 0.0, ATTACK_REFERENCE_DISTANCE), ATTACK_REFERENCE_DISTANCE);
        assert!(
            (Vec3::from_array(emitter).length() - 1.0).abs() < 1e-4,
            "a source exactly at the reference distance should scale to length 1.0, got {:?}",
            emitter
        );
    }
}
