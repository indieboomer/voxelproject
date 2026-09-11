use glam::Vec3;

#[path = "dragon.rs"]
mod dragon;
pub use dragon::DragonSave;
#[path = "fish.rs"]
mod fish;
pub use fish::FishSave;

use crate::model::{push_model, Models};
use crate::net::PlayerId;
use crate::voxel::mesher::MeshData;
use crate::voxel::world::SEA_LEVEL;
use crate::voxel::World;

/// Small self-contained xorshift RNG so creature AI doesn't need an
/// external `rand` crate dependency.
pub struct SimpleRng(u64);

impl SimpleRng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CreatureKind {
    Sheep,
    Chicken,
    /// Large, slow, hostile. See `is_hostile`/`Creatures::update`'s aggro
    /// pass -- every non-hostile kind only ever moves under Lua's `chase()`.
    StoneGolem,
    /// Fast, hostile, fragile compared to a golem -- attacks quickly on a
    /// short cooldown rather than hitting hard and slow. See `is_hostile`.
    Wolf,
    /// Hostile, fragile, precise -- stings for light damage on the shortest
    /// cooldown of any hostile kind rather than closing distance fast (no
    /// "run" clip, unlike Wolf). See `is_hostile`.
    Stinger,
    /// Neutral like Sheep/Chicken -- only ever moves under Lua's `chase()`,
    /// never attacks on its own.
    Cow,
    /// Hostile melee bruiser -- hits harder than Wolf and is tankier, but
    /// on a longer cooldown; chases at a real (if not Wolf-fast) run.
    /// Sits between Wolf and StoneGolem on every axis (speed, health,
    /// damage, cooldown). See `is_hostile`.
    Goblin,
    /// Hostile, tanky, relentless -- a slow-moving ghost that never speeds
    /// up even while aggro-chasing (like StoneGolem, "never runs" is true
    /// both mechanically and literally: its model has no run clip), but
    /// detects players from further away and hits close to as hard as a
    /// golem. Never gives up a chase once aggroed (see
    /// `chase_giveup_duration`), the same as StoneGolem. See `is_hostile`.
    Sunscorch,
    /// Walking-only hostile melee enemies, each with four visual variants.
    Zombie,
    Skeleton,
    DragonGreen,
    DragonRed,
    Fish,
}

impl CreatureKind {
    pub fn is_dragon(self) -> bool {
        matches!(self, Self::DragonGreen | Self::DragonRed)
    }

    /// Includes the rig's animated standing pose, verified against the player mesh.
    pub fn model_scale(self) -> f32 { if self.is_dragon() { 1.25 } else { 1.0 } }

    fn speed(self) -> f32 {
        match self {
            CreatureKind::Sheep => 1.4,
            CreatureKind::Chicken => 2.0,
            // Deliberately slower than either -- "moves slowly" is meant to
            // be an escapable, avoidable threat, not a fast ambush.
            CreatureKind::StoneGolem => 0.9,
            // Wander/walk pace; see `aggro_speed` for how fast it closes in
            // once it notices a player.
            CreatureKind::Wolf => 1.8,
            CreatureKind::Stinger => 2.2,
            CreatureKind::Cow => 1.3,
            CreatureKind::Goblin => 1.6,
            // Same pace whether wandering or aggro-chasing -- see
            // `aggro_speed` and this variant's own doc comment.
            CreatureKind::Sunscorch => 1.2,
            CreatureKind::Zombie => 1.2,
            CreatureKind::Skeleton => 1.5,
            CreatureKind::DragonGreen | CreatureKind::DragonRed => 2.4,
            CreatureKind::Fish => 1.1,
        }
    }

    pub fn max_health(self) -> f32 {
        match self {
            CreatureKind::Sheep => 12.0,
            CreatureKind::Chicken => 6.0,
            CreatureKind::StoneGolem => 40.0,
            CreatureKind::Wolf => 18.0,
            CreatureKind::Stinger => 14.0,
            CreatureKind::Cow => 20.0,
            CreatureKind::Goblin => 24.0,
            CreatureKind::Sunscorch => 28.0,
            CreatureKind::Zombie => 24.0,
            CreatureKind::Skeleton => 18.0,
            CreatureKind::DragonGreen | CreatureKind::DragonRed => 240.0,
            CreatureKind::Fish => 4.0,
        }
    }

    /// Whether this kind autonomously aggroes/chases/attacks the nearest
    /// player on its own -- entirely independent of any Lua rule, unlike
    /// every other kind which only ever moves under `chase()`. See
    /// `Creatures::update`'s aggro pass and `spawn_around`'s doc comment for
    /// how these kinds participate in the starter world scatter.
    fn is_hostile(self) -> bool {
        matches!(
            self,
            CreatureKind::StoneGolem
                | CreatureKind::Wolf
                | CreatureKind::Stinger
                | CreatureKind::Goblin
                | CreatureKind::Sunscorch
                | CreatureKind::Zombie
                | CreatureKind::Skeleton
                | CreatureKind::DragonGreen
                | CreatureKind::DragonRed
        )
    }

    /// Aggro detection radius -- only meaningful for a hostile kind.
    fn aggro_radius(self) -> f32 {
        match self {
            CreatureKind::StoneGolem => STONE_GOLEM_AGGRO_RADIUS,
            CreatureKind::Wolf => WOLF_AGGRO_RADIUS,
            CreatureKind::Stinger => STINGER_AGGRO_RADIUS,
            CreatureKind::Goblin => GOBLIN_AGGRO_RADIUS,
            CreatureKind::Sunscorch => SUNSCORCH_AGGRO_RADIUS,
            CreatureKind::Zombie | CreatureKind::Skeleton => 14.0,
            CreatureKind::DragonGreen | CreatureKind::DragonRed => 56.0,
            _ => 0.0,
        }
    }

    /// Movement speed while actively closing on an aggroed player. The
    /// golem deliberately reuses its own (slow) `speed()` here -- "moves
    /// slowly" is meant to stay true even while attacking -- while a wolf,
    /// a stinger, and (a step slower) a goblin speed up into a real chase,
    /// matching their "run" animation clip. Sunscorch reuses its own
    /// (slow) `speed()` too, same as the golem -- see its own doc comment.
    fn aggro_speed(self) -> f32 {
        match self {
            CreatureKind::Wolf => WOLF_RUN_SPEED,
            CreatureKind::Stinger => STINGER_AGGRO_SPEED,
            CreatureKind::Goblin => GOBLIN_RUN_SPEED,
            _ => self.speed(),
        }
    }

    /// How long this kind will keep chasing an aggroed player before giving
    /// up and reverting to wandering, even if the player is still within
    /// `aggro_radius` -- `None` means it never gives up on its own (a
    /// golem/stinger/sunscorch's aggro is purely radius-gated, re-evaluated
    /// fresh every tick). See `Creatures::update`'s `ChaseState` handling
    /// and `CHASE_GIVEUP_COOLDOWN_SECS` for what happens right after giving
    /// up.
    fn chase_giveup_duration(self) -> Option<f32> {
        match self {
            CreatureKind::Wolf => Some(WOLF_CHASE_GIVEUP_SECS),
            CreatureKind::Goblin => Some(GOBLIN_CHASE_GIVEUP_SECS),
            _ => None,
        }
    }

    fn attack_range(self) -> f32 {
        match self {
            CreatureKind::StoneGolem => STONE_GOLEM_ATTACK_RANGE,
            CreatureKind::Wolf => WOLF_ATTACK_RANGE,
            CreatureKind::Stinger => STINGER_ATTACK_RANGE,
            CreatureKind::Goblin => GOBLIN_ATTACK_RANGE,
            CreatureKind::Sunscorch => SUNSCORCH_ATTACK_RANGE,
            CreatureKind::Zombie | CreatureKind::Skeleton => 1.8,
            CreatureKind::DragonGreen | CreatureKind::DragonRed => 14.0,
            _ => 0.0,
        }
    }

    fn attack_damage(self) -> f32 {
        match self {
            CreatureKind::StoneGolem => STONE_GOLEM_ATTACK_DAMAGE,
            CreatureKind::Wolf => WOLF_ATTACK_DAMAGE,
            CreatureKind::Stinger => STINGER_ATTACK_DAMAGE,
            CreatureKind::Goblin => GOBLIN_ATTACK_DAMAGE,
            CreatureKind::Sunscorch => SUNSCORCH_ATTACK_DAMAGE,
            CreatureKind::Zombie => 4.0,
            CreatureKind::Skeleton => 3.0,
            CreatureKind::DragonGreen | CreatureKind::DragonRed => 8.0,
            _ => 0.0,
        }
    }

    fn attack_cooldown(self) -> f32 {
        match self {
            CreatureKind::StoneGolem => STONE_GOLEM_ATTACK_COOLDOWN,
            CreatureKind::Wolf => WOLF_ATTACK_COOLDOWN,
            CreatureKind::Stinger => STINGER_ATTACK_COOLDOWN,
            CreatureKind::Goblin => GOBLIN_ATTACK_COOLDOWN,
            CreatureKind::Sunscorch => SUNSCORCH_ATTACK_COOLDOWN,
            CreatureKind::Zombie => 2.0,
            CreatureKind::Skeleton => 1.5,
            CreatureKind::DragonGreen | CreatureKind::DragonRed => 3.0,
            _ => f32::MAX,
        }
    }

    /// Whether this kind's model has a dedicated "run" animation clip to
    /// switch to while moving at an elevated pace (aggro-chasing, or
    /// Lua-`chase()`d with `Wander::hunting` set) -- every other kind falls
    /// back to its "walk" clip played at normal speed instead. Reflects
    /// each model's actual exported clips (see `model.rs`), not a fixed
    /// design choice -- e.g. Cow's reimported model dropped its run clip,
    /// while Stinger's gained one.
    fn has_run_clip(self) -> bool {
        matches!(
            self,
            CreatureKind::Wolf | CreatureKind::Stinger | CreatureKind::Goblin
        )
    }

    /// Whether this kind periodically plays an idle vocalization while
    /// alive and wandering -- only kinds with a dedicated ambient
    /// sound file (see `audio.rs`'s `ambient_sound`). Every other kind's
    /// `AmbientCall` timer is left at `f32::INFINITY` at spawn and never
    /// fires; see `Creatures::spawn_with_rng`/`update`.
    fn has_ambient_call(self) -> bool {
        matches!(self, CreatureKind::Cow | CreatureKind::Sheep | CreatureKind::Zombie)
    }

    pub fn to_u8(self) -> u8 {
        match self {
            CreatureKind::Sheep => 0,
            CreatureKind::Chicken => 1,
            CreatureKind::StoneGolem => 2,
            CreatureKind::Wolf => 3,
            CreatureKind::Stinger => 4,
            CreatureKind::Cow => 5,
            CreatureKind::Goblin => 6,
            CreatureKind::Sunscorch => 7,
            CreatureKind::Zombie => 8,
            CreatureKind::Skeleton => 9,
            CreatureKind::DragonGreen => 10,
            CreatureKind::DragonRed => 11,
            CreatureKind::Fish => 12,
        }
    }

    pub fn from_u8(v: u8) -> CreatureKind {
        match v {
            1 => CreatureKind::Chicken,
            2 => CreatureKind::StoneGolem,
            3 => CreatureKind::Wolf,
            4 => CreatureKind::Stinger,
            5 => CreatureKind::Cow,
            7 => CreatureKind::Sunscorch,
            8 => CreatureKind::Zombie,
            9 => CreatureKind::Skeleton,
            10 => CreatureKind::DragonGreen,
            11 => CreatureKind::DragonRed,
            12 => CreatureKind::Fish,
            6 => CreatureKind::Goblin,
            _ => CreatureKind::Sheep,
        }
    }
}

/// Relative weight of each kind in a fresh world's starter scatter (see
/// `Creatures::spawn_around`) -- not a probability, just a share of the
/// total. Neutral grazers dominate (72 of 110 shares); ordinary hostiles
/// have 5–8 shares each, while stone_golem and sunscorch have only 2 each.
const STARTER_KIND_WEIGHTS: &[(CreatureKind, u32)] = &[
    (CreatureKind::Sheep, 26),
    (CreatureKind::Chicken, 26),
    (CreatureKind::Cow, 20),
    (CreatureKind::Wolf, 8),
    (CreatureKind::Stinger, 8),
    (CreatureKind::Goblin, 8),
    (CreatureKind::StoneGolem, 2),
    (CreatureKind::Sunscorch, 2),
    (CreatureKind::Zombie, 5),
    (CreatureKind::Skeleton, 5),
];

/// Draws one creature kind for the starter world scatter, weighted per
/// `STARTER_KIND_WEIGHTS`. The final entry is the fallback for any rounding
/// slack in `next_f32`'s range, so this always returns *some* kind rather
/// than needing an `Option`.
fn pick_starter_kind(rng: &mut SimpleRng) -> CreatureKind {
    let total: u32 = STARTER_KIND_WEIGHTS.iter().map(|&(_, w)| w).sum();
    let mut roll = (rng.next_f32() * total as f32) as u32;
    for &(kind, weight) in STARTER_KIND_WEIGHTS {
        if roll < weight {
            return kind;
        }
        roll -= weight;
    }
    STARTER_KIND_WEIGHTS.last().unwrap().0
}

/// Which animation clip a creature is currently posed with. Not every model
/// has every clip (see `AnimatedModel`'s doc comment) -- `Creatures::update`
/// only ever picks `Run`/`Attack` for a kind whose model actually has that
/// clip (`CreatureKind::has_run_clip`/`is_hostile`), so `model_name` always
/// resolves to something the model defines in practice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnimClip {
    Idle,
    Walk,
    Run,
    Attack,
    Fly,
    AttackWalk,
    AttackFly,
}

impl AnimClip {
    fn model_name(self) -> &'static str {
        match self {
            AnimClip::Idle => "idle",
            AnimClip::Walk => "walk",
            AnimClip::Run => "run",
            AnimClip::Attack => "attack",
            AnimClip::Fly => "fly",
            AnimClip::AttackWalk => "attack_walk",
            AnimClip::AttackFly => "attack_fly",
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            AnimClip::Idle => 0,
            AnimClip::Walk => 1,
            AnimClip::Run => 2,
            AnimClip::Attack => 3,
            AnimClip::Fly => 4,
            AnimClip::AttackWalk => 5,
            AnimClip::AttackFly => 6,
        }
    }

    pub fn from_u8(v: u8) -> AnimClip {
        match v {
            1 => AnimClip::Walk,
            2 => AnimClip::Run,
            3 => AnimClip::Attack,
            4 => AnimClip::Fly,
            5 => AnimClip::AttackWalk,
            6 => AnimClip::AttackFly,
            _ => AnimClip::Idle,
        }
    }
}

/// Steps `current` toward `target` (both radians) by at most `max_delta`,
/// turning whichever way is shorter. Keeps the result normalized to
/// `(-PI, PI]` so it doesn't grow unbounded over a long play session.
fn turn_toward(current: f32, target: f32, max_delta: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let diff = (target - current + PI).rem_euclid(TAU) - PI;
    let step = diff.clamp(-max_delta, max_delta);
    (current + step + PI).rem_euclid(TAU) - PI
}

struct Pos(Vec3);
struct Wander {
    target: (f32, f32),
    timer: f32,
    /// Set by a Lua module's `chase()` call and refreshed every tick it's
    /// still called; left alone it naturally clears itself once `timer`
    /// expires, so a disabled module's creatures drift back to wandering
    /// within a few seconds without any explicit "revert" bookkeeping.
    hunting: bool,
}
struct Kind(CreatureKind);
struct Health(f32);
/// Seconds until this creature may attack again -- only ever decremented/
/// read for a hostile kind (see `CreatureKind::is_hostile`), but attached to
/// every creature uniformly (unused, harmless) rather than giving hecs
/// entities two different component shapes depending on kind.
struct AttackCooldown(f32);
/// Seconds remaining to show the "attack" animation clip, set to
/// `ATTACK_ANIM_DURATION` whenever a hostile creature's attack lands (see
/// `Creatures::update`) -- separate from `AttackCooldown` since the two
/// durations differ per kind and the clip should finish well before the next
/// hit is even possible.
struct AttackAnimTimer(f32);
/// Stable id exposed to Lua scripts, since a `hecs::Entity` isn't a
/// convenient thing to hand across the API boundary.
struct CreatureId(u32);
/// Current heading in radians (same convention as `Camera::forward`: 0
/// faces +X, increasing turns toward +Z). Turned toward the movement
/// direction each tick the creature is actually moving; held steady while
/// idle, so a stopped creature doesn't visibly snap back to some default.
struct Facing(f32);
/// Which animation clip is currently posed, and how long it's been playing.
/// `time` resets to 0 whenever `clip` changes (see `Creatures::update`) so a
/// freshly-switched-to clip always starts from its first frame instead of
/// jumping into the middle of it.
struct AnimState {
    clip: AnimClip,
    time: f32,
}
/// Only meaningful for a kind with a `chase_giveup_duration` (currently
/// Wolf/Goblin) -- attached to every creature uniformly regardless (same
/// pattern as `AttackCooldown`), harmlessly staying at zero for every other
/// kind.
struct ChaseState {
    /// Seconds this creature has been continuously aggro-chasing a player.
    /// Reset to 0 whenever aggro lapses (player leaves aggro_radius) or it
    /// hits `chase_giveup_duration` and gives up.
    chasing_for: f32,
    /// Seconds remaining before this creature will consider aggroing again
    /// -- set to `CHASE_GIVEUP_COOLDOWN_SECS` the instant it gives up, so
    /// it doesn't just re-aggro the very next tick.
    giveup_cooldown: f32,
}
/// Horizontal distance walked/run since the last footstep sound, in
/// blocks -- wraps back down by `CREATURE_STEP_LENGTH` (not reset to 0)
/// each time it crosses that threshold, so a creature moving faster than
/// one step's worth per tick doesn't lose the leftover distance. See
/// `Creatures::update`'s `pending_audio.steps`.
struct Steps(f32);
/// Seconds remaining until this creature's next idle vocalization (only
/// meaningful for `CreatureKind::has_ambient_call`; left at
/// `f32::INFINITY` for every other kind so it harmlessly never fires). See
/// `Creatures::update`'s `pending_audio.ambient_calls`.
struct AmbientCall(f32);

const HUNT_SPEED_MULTIPLIER: f32 = 1.6;
/// How fast a creature visually turns to face its movement direction --
/// fast enough that picking a new wander target doesn't look like an
/// instant snap, but still lively. ~220 degrees/sec.
const TURN_RATE: f32 = 3.84;
/// How long a Lua-set chase target stays valid before the creature reverts
/// to normal wandering if it isn't refreshed again.
const HUNT_TARGET_TTL: f32 = 0.3;
/// How long the "attack" animation clip plays for after a hostile creature's
/// attack lands, regardless of kind -- shorter than either kind's attack
/// cooldown, so it always finishes (and the creature is back to idle/walk/
/// run) well before the next hit is even possible.
const ATTACK_ANIM_DURATION: f32 = 0.5;

/// How close a player has to be before a stone golem notices and starts
/// closing in, overriding whatever it was doing (wandering, or even a
/// Lua-set chase target -- see `Creatures::update`).
const STONE_GOLEM_AGGRO_RADIUS: f32 = 12.0;
/// Melee reach -- generous relative to the player/golem hitboxes so "large"
/// actually reads as dangerous up close rather than needing to overlap.
const STONE_GOLEM_ATTACK_RANGE: f32 = 2.2;
const STONE_GOLEM_ATTACK_DAMAGE: f32 = 4.0;
/// Seconds between hits once in range -- slow like its movement, so a
/// player has a real window to retreat or fight back between hits.
const STONE_GOLEM_ATTACK_COOLDOWN: f32 = 1.5;

/// How close a player has to be before a wolf notices and starts chasing --
/// see `STONE_GOLEM_AGGRO_RADIUS`.
const WOLF_AGGRO_RADIUS: f32 = 10.0;
/// Chase speed once aggroed -- well under the player's sprint speed (7.5,
/// see `player.rs::SPRINT_SPEED`) so a wolf stays a real but escapable
/// threat: a player who notices in time and sprints away can outrun it.
const WOLF_RUN_SPEED: f32 = 5.0;
/// Bite reach -- tighter than the golem's, matching a wolf's smaller frame.
const WOLF_ATTACK_RANGE: f32 = 1.6;
/// Weaker per hit than the golem but on a much shorter cooldown (see
/// `WOLF_ATTACK_COOLDOWN`) -- a fast, nagging threat rather than a heavy one.
const WOLF_ATTACK_DAMAGE: f32 = 3.0;
const WOLF_ATTACK_COOLDOWN: f32 = 0.9;
/// How long a wolf keeps chasing before giving up -- see
/// `CreatureKind::chase_giveup_duration`.
const WOLF_CHASE_GIVEUP_SECS: f32 = 6.0;

/// How close a player has to be before a stinger notices and starts closing
/// in -- see `STONE_GOLEM_AGGRO_RADIUS`.
const STINGER_AGGRO_RADIUS: f32 = 9.0;
/// Chase speed once aggroed -- quicker than its wander pace, and (like a
/// wolf) shown with its own "run" clip; its threat is a fast attack rhythm
/// more than raw chase speed, but it isn't slow either.
const STINGER_AGGRO_SPEED: f32 = 5.0;
/// Sting reach -- tight, matching a small, precise attacker.
const STINGER_ATTACK_RANGE: f32 = 1.4;
/// Weakest hit of any hostile kind, but on the shortest cooldown (see
/// `STINGER_ATTACK_COOLDOWN`) -- many light, fast stings rather than a
/// wolf's moderate bite or a golem's heavy blow.
const STINGER_ATTACK_DAMAGE: f32 = 2.5;
const STINGER_ATTACK_COOLDOWN: f32 = 0.7;

/// How close a player has to be before a goblin notices and starts closing
/// in -- see `STONE_GOLEM_AGGRO_RADIUS`.
const GOBLIN_AGGRO_RADIUS: f32 = 11.0;
/// Chase speed once aggroed -- a real run (it has the clip), but a step
/// slower than a wolf's, matching its bulkier, weapon-wielding build.
const GOBLIN_RUN_SPEED: f32 = 4.2;
/// Club reach -- longer than a wolf's bite, shorter than a golem's.
const GOBLIN_ATTACK_RANGE: f32 = 1.9;
/// Sits between Wolf and StoneGolem on both damage and cooldown -- a
/// mid-weight melee threat rather than either extreme.
const GOBLIN_ATTACK_DAMAGE: f32 = 3.5;
const GOBLIN_ATTACK_COOLDOWN: f32 = 1.1;
/// How long a goblin keeps chasing before giving up -- see
/// `CreatureKind::chase_giveup_duration`.
const GOBLIN_CHASE_GIVEUP_SECS: f32 = 7.0;

/// How close a player has to be before a sunscorch notices and starts
/// closing in -- larger than any other hostile kind's, since it never
/// speeds up once it has (see `CreatureKind::aggro_speed`) and relies on
/// noticing early rather than closing distance fast.
const SUNSCORCH_AGGRO_RADIUS: f32 = 13.0;
/// Reach -- between a wolf's bite and a golem's much longer one.
const SUNSCORCH_ATTACK_RANGE: f32 = 1.7;
/// Close to a golem's heavy blow, on a shorter cooldown -- a real threat
/// once it actually catches up, matching "never runs, but never stops."
const SUNSCORCH_ATTACK_DAMAGE: f32 = 3.8;
const SUNSCORCH_ATTACK_COOLDOWN: f32 = 1.3;

/// How long a creature that just gave up a chase (see
/// `CreatureKind::chase_giveup_duration`) waits before it will consider
/// aggroing again -- without this, a creature that gives up while the
/// player is still standing right next to it would just re-aggro the very
/// next tick, making the "give up" invisible in practice.
const CHASE_GIVEUP_COOLDOWN_SECS: f32 = 4.0;

/// How far (in blocks) a creature walks/runs between footstep sounds --
/// see `Steps`/`Creatures::update`. One value for every kind: the sound
/// itself (`animal_step.mp3`) is generic, so a per-kind cadence wouldn't
/// read as meaningfully different without kind-specific clips to match.
const CREATURE_STEP_LENGTH: f32 = 1.6;
/// Random range (seconds) between ambient vocalizations, including zombie growls -- see
/// `CreatureKind::has_ambient_call`/`AmbientCall`.
const AMBIENT_CALL_INTERVAL_MIN: f32 = 8.0;
const AMBIENT_CALL_INTERVAL_MAX: f32 = 22.0;

/// Reported when `damage`/`destroy` kills a creature, so the caller can
/// fire the `on_death` event to every rule module (not just the one that
/// caused it) and so the corpse's last position is still available after
/// the entity itself has already been despawned.
#[derive(Clone, Copy)]
pub struct DeathEvent {
    pub kind: CreatureKind,
    pub pos: Vec3,
}

/// Creature-triggered sound events accumulated since the last
/// `Creatures::take_audio_events` call. Kept as a separate side channel
/// rather than folded into `update`'s own `Vec<(PlayerId, f32)>` return
/// (used for player damage application) so adding audio support didn't
/// need to touch that already-widely-tested signature, and rather than
/// threading through `scripting.rs`'s `TickOutcome` since `deaths` in
/// particular is only actually known at `damage`/`destroy` commit time,
/// deep inside Lua dispatch.
/// Kind plus the world position it happened at, so the caller can spatialize
/// the sound (distance falloff + left/right panning relative to the
/// listener) -- see `audio.rs`'s `AudioEngine::play_creature_attack` and
/// friends, all of which take a position for exactly this reason.
pub type CreatureAudioEvent = (CreatureKind, Vec3);

#[derive(Default)]
pub struct CreatureAudioEvents {
    /// One entry per creature that died this call (`damage` reaching zero
    /// health, or `destroy`) -- see `audio.rs`'s generic death sound.
    pub deaths: Vec<CreatureAudioEvent>,
    /// One entry per hostile creature's attack that landed on a player.
    pub attacks: Vec<CreatureAudioEvent>,
    /// One entry per creature that completed a `CREATURE_STEP_LENGTH`
    /// stride while walking/running.
    pub steps: Vec<CreatureAudioEvent>,
    /// One entry per cow, sheep or zombie ambient vocalization that fired.
    pub ambient_calls: Vec<CreatureAudioEvent>,
}

pub type SavedCreature = (u32, u8, [f32; 3], f32, f32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BehaviorMode { Auto, Chase, Attack, Ignore }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BehaviorTarget { Creature(u32), Player(u32) }
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct CreatureBehavior {
    pub aggressive: bool,
    pub mode: BehaviorMode,
    pub target: Option<BehaviorTarget>,
}
impl CreatureBehavior {
    fn natural(kind: CreatureKind) -> Self {
        Self { aggressive: kind.is_hostile(), mode: BehaviorMode::Auto, target: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackPolicy {
    ProtectPlayer(u32, u8),
    SuppressCreature(u32),
}

pub struct Creatures {
    pub player_kills: Vec<DeathEvent>,
    pub(crate) wildlife: std::collections::BTreeMap<u32, Option<(i32, i32)>>,
    population_timer: f32,
    population_sequence: u64,
    start_protection: std::collections::BTreeMap<PlayerId,f32>,
    fish_regions: std::collections::BTreeSet<(i32, i32)>,
    fish_scan_timer: f32,
    fish_scan_cursor: usize,
    dragon_regions: std::collections::BTreeSet<(i32, i32)>,
    ecs: hecs::World,
    next_id: u32,
    pending_audio: CreatureAudioEvents,
    pub behaviors: std::collections::BTreeMap<u32, CreatureBehavior>,
    pub combat_deaths: Vec<DeathEvent>,
    pub attack_policies: std::collections::BTreeMap<(u64, u64), Vec<AttackPolicy>>,
}

#[path = "wildlife.rs"]
mod wildlife;

/// A callback's private creature view and ordered commands. No live ECS
/// mutation occurs until commit, so discarded drafts also preserve AI,
/// animation, entity IDs, and the spawn sequence without cloning the ECS.
pub(crate) struct CreatureDraft {
    dragon_homes: Vec<Vec3>,
    pub snapshot: Vec<(u32, u8, [f32; 3], f32, f32)>,
    next_id: u32,
    commands: Vec<CreatureCommand>,
    pub behaviors: std::collections::BTreeMap<u32, CreatureBehavior>,
}

enum CreatureCommand {
    Spawn(u32, CreatureKind, Vec3, u64),
    Chase(u32, Vec3),
    Behavior(u32, CreatureBehavior),
    Damage(u32, f32),
    Destroy(u32),
}

impl CreatureDraft {
    pub fn spawn_in_world(&mut self, world: &World, kind: CreatureKind, pos: Vec3, seed: u64) -> Option<u32> {
        if kind == CreatureKind::Fish && !fish::spawn_clear(world, pos) { return None; }
        self.spawn(kind, pos, seed)
    }
    pub fn new(creatures: &Creatures) -> Self {
        let mut snapshot = creatures.snapshot_with_ids();
        snapshot.sort_by_key(|entry| entry.0);
        let dragon_homes = creatures.ecs.query::<&dragon::Dragon>().iter().map(|(_, d)| d.home).collect();
        Self { snapshot, next_id: creatures.next_id, commands: Vec::new(), behaviors: creatures.behaviors.clone(), dragon_homes }
    }

    pub fn spawn(&mut self, kind: CreatureKind, pos: Vec3, seed: u64) -> Option<u32> {
        if kind.is_dragon() {
            if !pos.is_finite() || pos.abs().max_element() > 1_000_000.0 { return None; }
            // Includes earlier spawns in this callback: a generated rule cannot create a pack.
            if self.dragon_homes.iter().any(|&home| dragon::horizontal_distance(home, pos) < dragon::MIN_HOME_SPACING) {
                return None;
            }
        }
        if self.snapshot.len() >= crate::world_api_gen::SCRIPT_CREATURES_MAX {
            return None;
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1)?;
        if kind.is_dragon() { self.dragon_homes.push(pos); }
        self.snapshot.push((id, kind.to_u8(), pos.to_array(), kind.max_health(), kind.max_health()));
        self.behaviors.insert(id, CreatureBehavior::natural(kind));
        self.commands.push(CreatureCommand::Spawn(id, kind, pos, seed));
        Some(id)
    }

    pub fn behavior(&self, id: u32) -> Option<CreatureBehavior> {
        self.snapshot.iter().find(|c| c.0 == id)?;
        self.behaviors.get(&id).copied()
    }
    pub fn set_behavior(&mut self, id: u32, state: CreatureBehavior) -> bool {
        if self.behavior(id).is_none() { return false; }
        self.behaviors.insert(id, state);
        self.commands.push(CreatureCommand::Behavior(id, state));
        true
    }
    pub fn chase(&mut self, id: u32, target: Vec3) {
        if let Some(mut state) = self.behavior(id) {
            state.mode = BehaviorMode::Auto;
            state.target = None;
            self.set_behavior(id, state);
            self.commands.push(CreatureCommand::Chase(id, target));
        }
    }

    pub fn damage(&mut self, id: u32, amount: f32) -> Option<DeathEvent> {
        let index = self.snapshot.iter().position(|entry| entry.0 == id)?;
        self.snapshot[index].3 -= amount;
        self.commands.push(CreatureCommand::Damage(id, amount));
        if self.snapshot[index].3 <= 0.0 {
            let (_, kind, pos, ..) = self.snapshot.remove(index);
            Some(DeathEvent { kind: CreatureKind::from_u8(kind), pos: Vec3::from_array(pos) })
        } else {
            None
        }
    }

    pub fn destroy(&mut self, id: u32) -> Option<DeathEvent> {
        let index = self.snapshot.iter().position(|entry| entry.0 == id)?;
        let (_, kind, pos, ..) = self.snapshot.remove(index);
        self.commands.push(CreatureCommand::Destroy(id));
        Some(DeathEvent { kind: CreatureKind::from_u8(kind), pos: Vec3::from_array(pos) })
    }

    pub fn commit(self, creatures: &mut Creatures) {
        for command in self.commands {
            match command {
                CreatureCommand::Spawn(id, kind, pos, seed) => {
                    let actual = creatures.spawn_one(kind, pos, seed);
                    debug_assert_eq!(actual, id, "callbacks commit before simulation advances");
                }
                CreatureCommand::Behavior(id, state) => {
                    let previous = creatures.behaviors.insert(id, state);
                    let changed = previous.map_or(true, |old| old.mode != state.mode || old.aggressive != state.aggressive);
                    if matches!(state.mode, BehaviorMode::Ignore | BehaviorMode::Auto) {
                        for (_, (cid, wander, animation)) in creatures.ecs.query_mut::<(&CreatureId, &mut Wander, &mut AttackAnimTimer)>() {
                            if cid.0 == id && (changed || wander.hunting) { wander.hunting = false; wander.timer = 0.0; animation.0 = 0.0; }
                        }
                    }
                }
                CreatureCommand::Chase(id, target) => { creatures.set_chase_target(id, target); }
                CreatureCommand::Damage(id, amount) => { creatures.damage(id, amount); }
                CreatureCommand::Destroy(id) => { creatures.destroy(id); }
            }
        }
    }
}

impl Creatures {
    pub fn new() -> Self {
        Self {
            start_protection: Default::default(),
            wildlife: Default::default(),
            population_timer: 0.0,
            population_sequence: 0,
            dragon_regions: Default::default(),
            fish_regions: Default::default(),
            fish_scan_timer: 0.0,
            fish_scan_cursor: 0,
            ecs: hecs::World::new(),
            next_id: 1,
            pending_audio: CreatureAudioEvents::default(),
            behaviors: Default::default(),
            combat_deaths: Vec::new(),
            player_kills: Vec::new(),
            attack_policies: Default::default(),
        }
    }

    /// Populates a brand-new world's starter creatures by drawing each
    /// spawn's kind from `STARTER_KIND_WEIGHTS` -- every kind can appear,
    /// including hostile ones, but weighted heavily toward the neutral
    /// grazers (sheep/chicken/cow together are ~65% of the table), with
    /// wolf/stinger/goblin/zombie/skeleton uncommon and stone_golem/sunscorch
    /// rare (~2% each). Hostiles start 48-72 blocks away; neutral animals
    /// remain within 24 blocks so the starting area still feels alive.
    pub fn spawn_around(&mut self, world: &World, center: Vec3, count: usize, seed: u32) {
        let mut rng = SimpleRng::new(seed as u64 ^ 0xC0FFEE);
        for i in 0..count {
            let kind = pick_starter_kind(&mut rng);
            let spot = if kind.is_hostile() {
                (0..12).find_map(|_|find_land_spot(world,&mut rng,center.x,center.z,72.0)
                    .filter(|&(x,z)|(x-center.x).powi(2)+(z-center.z).powi(2)>=48.0*48.0))
            } else {find_land_spot(world,&mut rng,center.x,center.z,24.0)};
            if let Some((x, z)) = spot {
                let y = world.terrain_height(x.floor() as i32, z.floor() as i32) as f32 + 1.0;
                let id = self.spawn_with_rng(
                    kind,
                    Vec3::new(x, y, z),
                    (seed as u64).wrapping_add(i as u64 * 7919) ^ 0xA5A5A5,
                );
                self.wildlife.insert(id, None);
            }
        }
    }

    fn spawn_with_rng(&mut self, kind: CreatureKind, pos: Vec3, rng_seed: u64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let mut rng = SimpleRng::new(rng_seed);
        // Randomized per-creature so a whole herd doesn't call out in sync;
        // a kind with no ambient sound gets `INFINITY` and never fires (see
        // `AmbientCall`'s doc comment).
        let ambient_call = AmbientCall(if kind.has_ambient_call() {
            AMBIENT_CALL_INTERVAL_MIN + rng.next_f32() * (AMBIENT_CALL_INTERVAL_MAX - AMBIENT_CALL_INTERVAL_MIN)
        } else {
            f32::INFINITY
        });
        self.behaviors.insert(id, CreatureBehavior::natural(kind));
        let entity = self.ecs.spawn((
            Pos(pos),
            Wander {
                target: (pos.x, pos.z),
                timer: 0.0,
                hunting: false,
            },
            Kind(kind),
            Health(kind.max_health()),
            AttackCooldown(0.0),
            AttackAnimTimer(0.0),
            CreatureId(id),
            Facing(0.0),
            AnimState {
                clip: AnimClip::Idle,
                time: 0.0,
            },
            ChaseState {
                chasing_for: 0.0,
                giveup_cooldown: 0.0,
            },
            Steps(0.0),
            ambient_call,
            rng,
        ));
        if kind.is_dragon() {
            self.ecs.insert_one(entity, dragon::Dragon::new(pos, rng_seed)).unwrap();
        }
        if kind == CreatureKind::Fish {
            self.ecs.insert_one(entity, fish::Fish::new(pos, &mut SimpleRng::new(rng_seed))).unwrap();
        }
        id
    }

    /// Called by the Lua `spawn_creature` API function.
    pub fn spawn_one(&mut self, kind: CreatureKind, pos: Vec3, rng_seed: u64) -> u32 {
        self.spawn_with_rng(kind, pos, rng_seed)
    }

    /// Restore world creatures, preserving rule-visible IDs and health.
    /// Transient animation and chase state start idle after loading.
    pub fn restore_saved(&mut self, entries: &[SavedCreature], seed: u64) {
        let mut seen = std::collections::HashSet::new();
        let mut next_id = self.next_id;
        for &(id, kind, pos, health, _) in entries {
            if id == u32::MAX
                || kind > 12
                || !seen.insert(id)
                || !Vec3::from_array(pos).is_finite()
                || !health.is_finite()
                || health <= 0.0
            {
                continue;
            }
            self.next_id = id;
            let kind = CreatureKind::from_u8(kind);
            self.spawn_one(kind, Vec3::from_array(pos), seed ^ id as u64);
            self.damage(id, (kind.max_health() - health).max(0.0));
            next_id = next_id.max(id + 1);
        }
        self.next_id = next_id;
    }
    /// Only the host runs creature AI; joined clients just render whatever
    /// positions the host's snapshot reports. `player_targets` (id +
    /// position) drives hostile-kind aggro (`CreatureKind::is_hostile`) --
    /// the only creature-vs-player interaction that isn't Lua-driven;
    /// everything else about this method's per-tick wander/chase movement is
    /// unchanged for sheep and chicken. Returns `(player_id, damage)` for
    /// every attack that landed this tick, for the caller to apply via the
    /// same `PlayerEffect::Health` path `api.damage_player` uses.
    /// Host-owned entry grace, independent of Lua attack policies. New guests
    /// receive the full interval even when the host has already used theirs.
    pub fn update_start_protection(&mut self,dt:f32,players:&[(PlayerId,Vec3)]) {
        self.start_protection.retain(|id,_|players.iter().any(|p|p.0==*id));
        let elapsed=if dt.is_finite(){dt.max(0.0)}else{0.0};
        for remaining in self.start_protection.values_mut() {*remaining=(*remaining-elapsed).max(0.0);}
        for &(id,_) in players {self.start_protection.entry(id).or_insert(60.0);}
    }

    pub fn update(
        &mut self,
        world: &World,
        dt: f32,
        player_targets: &[(PlayerId, Vec3)],
    ) -> Vec<(PlayerId, f32)> {
        // Filter natural aggro and explicit Lua player targets together.
        let eligible_players:Vec<_>=player_targets.iter().copied()
            .filter(|(id,_)|!self.start_protection.get(id).is_some_and(|t|*t>0.0)).collect();
        let player_targets=eligible_players.as_slice();
        let mut attacks = Vec::new();
        let mut stranded_fish = Vec::new();
        // Collected locally and merged into `self.pending_audio` after the
        // loop, rather than pushed to it directly, since the loop already
        // holds `self.ecs` borrowed via `query_mut`.
        let mut step_events = Vec::new();
        let mut creature_hits = Vec::new();
        let targets = self.snapshot_with_ids();
        let mut attack_sound_events = Vec::new();
        let mut ambient_events = Vec::new();

        for (_, (pos, wander, kind, rng, facing, cooldown, atk_anim, anim, chase, steps, ambient_call, cid, dragon, fish)) in
            self.ecs.query_mut::<(
                &mut Pos,
                &mut Wander,
                &Kind,
                &mut SimpleRng,
                &mut Facing,
                &mut AttackCooldown,
                &mut AttackAnimTimer,
                &mut AnimState,
                &mut ChaseState,
                &mut Steps,
                &mut AmbientCall,
                &CreatureId,
                Option<&mut dragon::Dragon>,
                Option<&mut fish::Fish>,
            )>()
        {
            if let Some(fish) = fish {
                let state = self.behaviors.get(&cid.0).unwrap();
                let target = state.target.and_then(|target| match target {
                    BehaviorTarget::Creature(id) => targets.iter().find(|c| c.0 == id).map(|c| Vec3::from_array(c.2)),
                    BehaviorTarget::Player(id) => player_targets.iter().find(|p| p.0 == id).map(|p| p.1),
                });
                if !fish::update(world, dt, fish, pos, facing, anim, wander, rng, target) {
                    stranded_fish.push(cid.0);
                }
                continue;
            }
            cooldown.0 = (cooldown.0 - dt).max(0.0);
            atk_anim.0 = (atk_anim.0 - dt).max(0.0);
            chase.giveup_cooldown = (chase.giveup_cooldown - dt).max(0.0);

            // `AmbientCall.0` is `f32::INFINITY` for a kind with no ambient
            // sound (see `spawn_with_rng`), so subtracting dt never brings
            // it to/below zero and this never fires for those kinds.
            ambient_call.0 -= dt;
            if ambient_call.0 <= 0.0 {
                ambient_events.push((kind.0, pos.0));
                ambient_call.0 = AMBIENT_CALL_INTERVAL_MIN
                    + rng.next_f32() * (AMBIENT_CALL_INTERVAL_MAX - AMBIENT_CALL_INTERVAL_MIN);
            }

            let state = self.behaviors.get_mut(&cid.0).unwrap();
            let detection_range = if kind.0.is_hostile() { kind.0.aggro_radius() } else { 16.0 };
            let attack_range = if kind.0.is_hostile() { kind.0.attack_range() } else { 1.5 };
            let explicit = state.target.and_then(|target| match target {
                BehaviorTarget::Creature(id) => targets.iter().find(|c| c.0 == id && id != cid.0)
                    .map(|c| (target, Vec3::from_array(c.2))),
                BehaviorTarget::Player(id) => player_targets.iter().find(|p| p.0 == id).map(|p| (target, p.1)),
            });
            let entry_protected=matches!(state.target,Some(BehaviorTarget::Player(id)) if self.start_protection.get(&id).is_some_and(|t|*t>0.0));
            if state.target.is_some() && explicit.is_none() && !entry_protected { state.target = None; }
            let mut aggro = if state.mode == BehaviorMode::Auto && state.aggressive
                && !wander.hunting && chase.giveup_cooldown <= 0.0 {
                player_targets
                    .iter()
                    .map(|&(id, p)| (BehaviorTarget::Player(id), p, pos.0.distance(p)))
                    .filter(|&(_, _, dist)| dist <= detection_range)
                    .min_by(|a, b| a.2.total_cmp(&b.2))
            } else {
                None
            };

            // A kind with a `chase_giveup_duration` tracks how long it's
            // been continuously chasing and forces itself back to
            // wandering once it's had enough, even if the player never
            // left aggro_radius -- see `ChaseState`/`CHASE_GIVEUP_COOLDOWN_SECS`.
            if let Some(giveup_secs) = kind.0.chase_giveup_duration() {
                if aggro.is_some() {
                    chase.chasing_for += dt;
                    if chase.chasing_for >= giveup_secs {
                        aggro = None;
                        chase.chasing_for = 0.0;
                        chase.giveup_cooldown = CHASE_GIVEUP_COOLDOWN_SECS;
                    }
                } else {
                    chase.chasing_for = 0.0;
                }
            }

            if matches!(state.mode, BehaviorMode::Attack | BehaviorMode::Chase) {
                aggro = explicit.map(|(target, p)| (target, p, pos.0.distance(p)));
            }
            if state.mode == BehaviorMode::Ignore && wander.hunting {
                wander.hunting = false;
                wander.timer = 0.0;
            }
            if let Some(dragon) = dragon {
                let protected = self.attack_policies.values().flatten().any(|policy| match *policy {
                    AttackPolicy::SuppressCreature(id) => id == cid.0,
                    AttackPolicy::ProtectPlayer(id, species) => aggro.is_some_and(|(target, _, _)| target == BehaviorTarget::Player(id)) && kind.0.to_u8() == species,
                });
                let result = dragon::update(world, dt, dragon, pos, facing, anim, cooldown,
                    atk_anim, wander, kind.0, aggro.map(|(id, p, _)| (id, p)),
                    !protected && state.mode != BehaviorMode::Chase);
                if let Some(target) = result.attack {
                    match target {
                        BehaviorTarget::Player(id) => attacks.push((id, kind.0.attack_damage())),
                        BehaviorTarget::Creature(id) => creature_hits.push((id, kind.0.attack_damage())),
                    }
                    attack_sound_events.push((kind.0, pos.0));
                }
                steps.0 += result.walked;
                if steps.0 >= 4.0 {
                    steps.0 %= 4.0;
                    step_events.push((kind.0, pos.0));
                }
                continue;
            }
            let mut moving = false;
            // Whether this tick's movement should read as the elevated
            // "run" pace -- either built-in aggro-chasing, or Lua-`chase()`d
            // fast (`Wander::hunting`) -- as opposed to ordinary wandering.
            let mut fast = false;
            // Horizontal distance actually covered this tick, for the
            // footstep accumulator below -- 0 unless one of the movement
            // branches below sets it.
            let mut moved = 0.0f32;

            if let Some((player_id, player_pos, dist)) = aggro {
                // Beeline for the player every tick this close, overriding
                // whatever wander/Lua-chase target it had.
                let to_target = Vec3::new(player_pos.x - pos.0.x, 0.0, player_pos.z - pos.0.z);
                let horiz_dist = to_target.length();
                if horiz_dist > 0.05 {
                    let dir = to_target / horiz_dist;
                    let step = (kind.0.aggro_speed() * dt).min(horiz_dist);
                    pos.0.x += dir.x * step;
                    pos.0.z += dir.z * step;
                    facing.0 = turn_toward(facing.0, dir.z.atan2(dir.x), TURN_RATE * dt);
                    moving = true;
                    fast = true;
                    moved = step;
                }
                let protected = self.attack_policies.values().flatten().any(|policy| match *policy {
                    AttackPolicy::SuppressCreature(id) => id == cid.0,
                    AttackPolicy::ProtectPlayer(id, species) => player_id == BehaviorTarget::Player(id) && kind.0.to_u8() == species,
                });
                if !protected && state.mode != BehaviorMode::Chase && dist <= attack_range && cooldown.0 <= 0.0 {
                    let damage = kind.0.attack_damage().max(2.0);
                    match player_id {
                        BehaviorTarget::Player(id) => attacks.push((id, damage)),
                        BehaviorTarget::Creature(id) => creature_hits.push((id, damage)),
                    }
                    attack_sound_events.push((kind.0, pos.0));
                    cooldown.0 = if kind.0.is_hostile() { kind.0.attack_cooldown() } else { 2.0 };
                    atk_anim.0 = ATTACK_ANIM_DURATION.min(kind.0.attack_cooldown());
                }
                // Force an immediate retarget (see the `timer <= 0.0` check
                // below) the moment aggro lapses, instead of resuming a
                // stale wander target from wherever it was before -- keeps
                // the transition back to wandering from looking like a
                // sudden beeline to some far-off point.
                wander.target = (pos.0.x, pos.0.z);
                wander.timer = 0.0;
            } else {
                wander.timer -= dt;
                if wander.timer <= 0.0 {
                    if let Some((x, z)) = find_land_spot(world, rng, pos.0.x, pos.0.z, 6.0) {
                        wander.target = (x, z);
                    }
                    wander.timer = 2.5 + rng.next_f32() * 3.5;
                    wander.hunting = false;
                }

                let to_target =
                    Vec3::new(wander.target.0 - pos.0.x, 0.0, wander.target.1 - pos.0.z);
                let dist = to_target.length();
                if dist > 0.15 {
                    let dir = to_target / dist;
                    let speed = kind.0.speed()
                        * if wander.hunting && !matches!(kind.0, CreatureKind::Zombie | CreatureKind::Skeleton) {
                            HUNT_SPEED_MULTIPLIER
                        } else {
                            1.0
                        };
                    let step = (speed * dt).min(dist);
                    pos.0.x += dir.x * step;
                    pos.0.z += dir.z * step;
                    facing.0 = turn_toward(facing.0, dir.z.atan2(dir.x), TURN_RATE * dt);
                    moving = true;
                    fast = wander.hunting;
                    moved = step;
                }
            }

            if moved > 0.0 {
                steps.0 += moved;
                if steps.0 >= CREATURE_STEP_LENGTH {
                    // `%=`, not reset to 0, so a creature covering more
                    // than one step's worth of distance in a single tick
                    // (fast movement at a low framerate) doesn't lose the
                    // leftover distance toward its next footstep.
                    steps.0 %= CREATURE_STEP_LENGTH;
                    step_events.push((kind.0, pos.0));
                }
            }

            let ground = world.terrain_height(pos.0.x.floor() as i32, pos.0.z.floor() as i32);
            pos.0.y = ground as f32 + 1.0;

            let clip = if atk_anim.0 > 0.0 {
                AnimClip::Attack
            } else if !moving {
                AnimClip::Idle
            } else if fast && kind.0.has_run_clip() {
                AnimClip::Run
            } else {
                AnimClip::Walk
            };
            if clip != anim.clip {
                anim.clip = clip;
                anim.time = 0.0;
            } else {
                anim.time += dt;
            }
        }

        for (id, amount) in creature_hits {
            if let Some(death) = self.damage(id, amount) { self.combat_deaths.push(death); }
        }
        self.pending_audio.attacks.extend(attack_sound_events);
        for id in stranded_fish { self.destroy(id); }
        self.pending_audio.steps.extend(step_events);
        self.pending_audio.ambient_calls.extend(ambient_events);

        attacks
    }

    /// Drains and returns every creature-triggered sound event queued
    /// since the last call -- see `CreatureAudioEvents`. Call once per
    /// frame, after both `update` and any Lua tick (whose
    /// `api.damage`/`api.destroy` calls are what actually produce
    /// `deaths`, via `CreatureDraft::commit`) have run.
    pub fn take_audio_events(&mut self) -> CreatureAudioEvents {
        std::mem::take(&mut self.pending_audio)
    }

    #[cfg(test)]
    pub fn build_mesh(&self, models: &Models) -> MeshData {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (_, (id, pos, kind, facing, anim)) in
            self.ecs.query::<(&CreatureId, &Pos, &Kind, &Facing, &AnimState)>().iter()
        {
            push_model(
                &mut vertices,
                &mut indices,
                models.for_variant(kind.0, (id.0 % 4) as u8),
                kind.0,
                anim.clip.model_name(),
                anim.time,
                pos.0,
                facing.0,
            );
        }
        MeshData { vertices, indices }
    }

    /// Weapon reach is measured to the body, not the feet origin of a giant model.
    pub fn weapon_target(&self, world: &World, eye: Vec3, dir: Vec3, reach: f32) -> Option<u32> {
        let mut best = None;
        let mut distance = reach;
        for (_, (id, kind, pos, facing)) in self.ecs.query::<(&CreatureId, &Kind, &Pos, &Facing)>().iter() {
            let hit = if kind.0.is_dragon() {
                dragon::body_hit(eye - pos.0, dir, facing.0, reach)
            } else {
                let center = pos.0 + Vec3::Y * if kind.0 == CreatureKind::Fish { 0.0 } else { 0.65 };
                let t = (center - eye).dot(dir);
                (t >= 0.0 && (eye + dir * t).distance(center) < 0.8).then_some(t)
            };
            if let Some(t) = hit {
                if t < distance && crate::raycast::raycast(world, eye, dir, t).is_none() {
                    best = Some(id.0);
                    distance = t;
                }
            }
        }
        best
    }

    /// Positions + kinds + facing + anim clip/time for broadcasting to
    /// clients over the network, so a creature turns and animates the same
    /// way on every screen. The kind byte's low nibble identifies the
    /// species and its high nibble selects the visual variant (0–3).
    pub fn snapshot(&self) -> Vec<([f32; 3], u8, f32, u8, f32)> {
        self.ecs
            .query::<(&CreatureId, &Pos, &Kind, &Facing, &AnimState)>()
            .iter()
            .map(|(_, (id, pos, kind, facing, anim))| {
                (
                    pos.0.to_array(),
                    // Low nibble is species; high nibble is visual variant.
                    // Stable IDs preserve appearance across save/reload.
                    kind.0.to_u8() | if matches!(kind.0, CreatureKind::Zombie | CreatureKind::Skeleton) {
                        ((id.0 % 4) as u8) << 4
                    } else { 0 },
                    facing.0,
                    anim.clip.to_u8(),
                    anim.time,
                )
            })
            .collect()
    }

    /// Positions + kinds + stable ids + health, for exposing to Lua modules.
    pub fn snapshot_with_ids(&self) -> Vec<(u32, u8, [f32; 3], f32, f32)> {
        self.ecs
            .query::<(&CreatureId, &Kind, &Pos, &Health)>()
            .iter()
            .map(|(_, (id, kind, pos, health))| {
                (
                    id.0,
                    kind.0.to_u8(),
                    pos.0.to_array(),
                    health.0,
                    kind.0.max_health(),
                )
            })
            .collect()
    }

    /// Called by the Lua `chase()` API function. Returns false if no
    /// creature has this id (e.g. it despawned).
    pub fn set_chase_target(&mut self, id: u32, target: Vec3) -> bool {
        for (_, (cid, wander)) in self.ecs.query_mut::<(&CreatureId, &mut Wander)>() {
            if cid.0 == id {
                wander.target = (target.x, target.z);
                wander.timer = HUNT_TARGET_TTL;
                wander.hunting = true;
                return true;
            }
        }
        false
    }

    /// Called by the Lua `damage` API function. Returns a `DeathEvent` if
    /// this brought the creature's health to zero (and despawns it);
    /// returns `None` if the creature survived or wasn't found.
    pub fn damage(&mut self, id: u32, amount: f32) -> Option<DeathEvent> {
        let mut target = None;
        for (entity, (cid, health, kind, pos)) in self
            .ecs
            .query_mut::<(&CreatureId, &mut Health, &Kind, &Pos)>()
        {
            if cid.0 == id {
                health.0 -= amount;
                if health.0 <= 0.0 {
                    target = Some((
                        entity,
                        DeathEvent {
                            kind: kind.0,
                            pos: pos.0,
                        },
                    ));
                }
                break;
            }
        }
        if let Some((entity, event)) = target {
            let _ = self.ecs.despawn(entity);
            self.behaviors.remove(&id);
            self.pending_audio.deaths.push((event.kind, event.pos));
            return Some(event);
        }
        None
    }

    /// Called by the Lua `destroy` API function: removes the creature
    /// immediately regardless of remaining health.
    pub fn destroy(&mut self, id: u32) -> Option<DeathEvent> {
        let mut target = None;
        for (entity, (cid, kind, pos)) in self.ecs.query_mut::<(&CreatureId, &Kind, &Pos)>() {
            if cid.0 == id {
                target = Some((
                    entity,
                    DeathEvent {
                        kind: kind.0,
                        pos: pos.0,
                    },
                ));
                break;
            }
        }
        if let Some((entity, event)) = target {
            let _ = self.ecs.despawn(entity);
            self.behaviors.remove(&id);
            self.pending_audio.deaths.push((event.kind, event.pos));
            return Some(event);
        }
        None
    }

    /// Passive engine effect for the RedStone block (not Lua-driven): heals
    /// every creature within `radius` of `center` by `amount`, capped at
    /// each creature's max health.
    pub fn heal_near(&mut self, center: Vec3, radius: f32, amount: f32) {
        let radius_sq = radius * radius;
        for (_, (pos, kind, health)) in self.ecs.query_mut::<(&Pos, &Kind, &mut Health)>() {
            if pos.0.distance_squared(center) <= radius_sq {
                health.0 = (health.0 + amount).min(kind.0.max_health());
            }
        }
    }

    /// Whether any creature is currently in Lua-driven chase mode. Used by
    /// tests to verify the Lua API actually changes creature behavior; kept
    /// public since it's a reasonable hook for a future HUD indicator too.
    #[allow(dead_code)]
    pub fn any_hunting(&self) -> bool {
        self.ecs.query::<&Wander>().iter().any(|(_, w)| w.hunting)
    }

    /// Current heading (radians, `Camera::forward` convention) of the
    /// creature with this id, if it still exists. Exposed mainly for
    /// tests; also a reasonable hook for a future debug HUD.
    #[allow(dead_code)]
    pub fn facing_of(&self, id: u32) -> Option<f32> {
        self.ecs
            .query::<(&CreatureId, &Facing)>()
            .iter()
            .find(|(_, (cid, _))| cid.0 == id)
            .map(|(_, (_, facing))| facing.0)
    }

    /// Current animation clip of the creature with this id, if it still
    /// exists. Exposed mainly for tests.
    #[allow(dead_code)]
    pub fn anim_clip_of(&self, id: u32) -> Option<AnimClip> {
        self.ecs
            .query::<(&CreatureId, &AnimState)>()
            .iter()
            .find(|(_, (cid, _))| cid.0 == id)
            .map(|(_, (_, anim))| anim.clip)
    }
}

/// Builds a creature mesh straight from a network snapshot, for clients that
/// don't run creature AI locally.
pub fn mesh_for_snapshot(entries: &[([f32; 3], u8, f32, u8, f32)], models: &Models) -> MeshData {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for &(pos, kind, facing, clip, time) in entries {
        let variant = kind >> 4;
        let kind = CreatureKind::from_u8(kind & 0x0f);
        push_model(
            &mut vertices,
            &mut indices,
            models.for_variant(kind, variant),
            kind,
            AnimClip::from_u8(clip).model_name(),
            time,
            Vec3::from_array(pos),
            facing,
        );
    }
    MeshData { vertices, indices }
}

fn find_land_spot(
    world: &World,
    rng: &mut SimpleRng,
    cx: f32,
    cz: f32,
    radius: f32,
) -> Option<(f32, f32)> {
    for _ in 0..6 {
        let angle = rng.next_f32() * std::f32::consts::TAU;
        let dist = rng.next_f32() * radius;
        let x = cx + angle.cos() * dist;
        let z = cz + angle.sin() * dist;
        if world.terrain_height(x.floor() as i32, z.floor() as i32) > SEA_LEVEL {
            return Some((x, z));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn starter_hostiles_keep_outside_the_player_start_area() {
        let mut world=World::new(42);
        world.generation.shape=crate::worldgen::Shape::Flat;
        let center=Vec3::new(-30.0,26.0,17.0);
        for seed in [7,42,2026] {
            let mut creatures=Creatures::new();
            creatures.spawn_around(&world,center,100,seed);
            let snapshot=creatures.snapshot_with_ids();
            assert!(snapshot.iter().any(|c|CreatureKind::from_u8(c.1).is_hostile()));
            for (_,kind,pos,_,_) in snapshot {
                let d=Vec3::new(pos[0]-center.x,0.0,pos[2]-center.z).length();
                if CreatureKind::from_u8(kind).is_hostile() {assert!(d>=48.0 && d<=72.01);}
                else {assert!(d<=24.01);}
            }
        }
    }

    #[test]
    fn entry_grace_blocks_all_creature_player_attacks_including_explicit_targets() {
        let world=World::new(42);
        let pos=Vec3::new(0.0,30.0,0.0);
        for kind in [CreatureKind::Wolf,CreatureKind::Zombie,CreatureKind::Skeleton,
            CreatureKind::StoneGolem,CreatureKind::DragonGreen,CreatureKind::DragonRed,CreatureKind::Sheep] {
            let mut creatures=Creatures::new();
            let id=creatures.spawn_one(kind,pos,1);
            creatures.behaviors.get_mut(&id).unwrap().mode=BehaviorMode::Attack;
            creatures.behaviors.get_mut(&id).unwrap().target=Some(BehaviorTarget::Player(7));
            let players=[(0,pos+Vec3::X),(7,pos+Vec3::Z)];
            creatures.update_start_protection(0.0,&players);
            assert!(creatures.update(&world,0.01,&players).is_empty());
            assert_ne!(creatures.anim_clip_of(id),Some(AnimClip::Attack));
            assert_eq!(creatures.behaviors[&id].target,Some(BehaviorTarget::Player(7)));
        }
    }

    #[test]
    fn entry_grace_expires_without_resetting_and_late_guests_get_their_own_interval() {
        let world=World::new(42);let pos=Vec3::new(0.0,30.0,0.0);
        let mut creatures=Creatures::new();
        let host=[(0,pos+Vec3::X)];
        creatures.update_start_protection(0.0,&host);
        creatures.update_start_protection(59.0,&host);
        assert_eq!(creatures.start_protection[&0],1.0);
        let both=[host[0],(7,pos+Vec3::Z)];
        creatures.update_start_protection(1.0,&both);
        assert_eq!(creatures.start_protection[&0],0.0);
        assert_eq!(creatures.start_protection[&7],60.0);
        creatures.spawn_one(CreatureKind::Zombie,pos,1);
        assert_eq!(creatures.update(&world,0.01,&both),vec![(0,CreatureKind::Zombie.attack_damage())]);
        creatures.update_start_protection(1.0,&both);
        assert_eq!(creatures.start_protection[&0],0.0);
        assert_eq!(creatures.start_protection[&7],59.0);
        creatures.update_start_protection(1.0,&host);
        creatures.update_start_protection(1.0,&both);
        assert_eq!(creatures.start_protection[&7],60.0);
    }

    #[test]
    fn zombies_queue_repeated_ambient_growls_at_random_intervals() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        creatures.spawn_one(CreatureKind::Zombie, Vec3::new(0.0, 30.0, 0.0), 9);
        let mut calls = Vec::new();
        for tick in 0..900 {
            creatures.update(&world, 0.1, &[]);
            if creatures.take_audio_events().ambient_calls.iter().any(|(kind, _)| *kind == CreatureKind::Zombie) {
                calls.push(tick);
            }
        }
        assert!(calls.len() >= 3);
        let intervals: Vec<_> = calls.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(intervals.iter().all(|&ticks| (80..=221).contains(&ticks)));
        assert!(intervals.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn undead_walk_during_natural_and_scripted_chases_and_attack_in_melee() {
        let world = World::new(1);
        let spawn = Vec3::new(0.0, world.terrain_height(0, 0) as f32 + 1.0, 0.0);
        for kind in [CreatureKind::Zombie, CreatureKind::Skeleton] {
            for scripted in [false, true] {
                let mut creatures = Creatures::new();
                let id = creatures.spawn_one(kind, spawn, 1);
                let target = spawn + Vec3::X * 6.0;
                if scripted { creatures.set_chase_target(id, target); }
                creatures.update(&world, 0.1, &[(5, target)]);
                let after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
                assert!((after.x - spawn.x - kind.speed() * 0.1).abs() < 0.0001);
                assert_eq!(creatures.anim_clip_of(id), Some(AnimClip::Walk));
            }
            let mut creatures = Creatures::new();
            let id = creatures.spawn_one(kind, spawn, 1);
            let target = spawn + Vec3::X;
            assert_eq!(creatures.update(&world, 0.01, &[(5, target)]), vec![(5, kind.attack_damage())]);
            assert_eq!(creatures.anim_clip_of(id), Some(AnimClip::Attack));
            assert!(creatures.update(&world, 0.01, &[(5, target)]).is_empty());
        }
    }

    #[test]
    fn undead_variants_survive_save_restore_and_render_identically_on_clients() {
        let models = Models::load();
        let mut creatures = Creatures::new();
        for kind in [CreatureKind::Zombie, CreatureKind::Skeleton] {
            for i in 0..4 {
                creatures.spawn_one(kind, Vec3::new(i as f32 * 3.0, 5.0, 0.0), i);
            }
        }
        let saved = creatures.snapshot_with_ids();
        let mut restored = Creatures::new();
        restored.restore_saved(&saved, 99);
        let mut original = creatures.snapshot();
        let mut loaded = restored.snapshot();
        original.sort_by_key(|entry| entry.1);
        loaded.sort_by_key(|entry| entry.1);
        assert_eq!(original, loaded);
        assert_eq!(original.len(), 8);
        let host = restored.build_mesh(&models);
        let client = mesh_for_snapshot(&restored.snapshot(), &models);
        assert_eq!(host.indices, client.indices);
        assert_eq!(bytemuck::cast_slice::<_, u8>(&host.vertices), bytemuck::cast_slice::<_, u8>(&client.vertices));
        for layer in 17..=24 {
            assert!(client.vertices.iter().any(|v| v.tex_layer == layer as f32));
        }
    }

    #[test]
    fn turn_toward_steps_by_at_most_max_delta() {
        let result = turn_toward(0.0, FRAC_PI_2, 0.1);
        assert!(
            (result - 0.1).abs() < 1e-5,
            "expected a single 0.1 rad step toward the target, got {result}"
        );
    }

    #[test]
    fn turn_toward_reaches_the_target_without_overshooting_when_close_enough() {
        let result = turn_toward(0.0, 0.05, 0.1);
        assert!(
            (result - 0.05).abs() < 1e-5,
            "a target closer than max_delta should be reached exactly, got {result}"
        );
    }

    #[test]
    fn turn_toward_takes_the_shorter_way_across_the_wrap_boundary() {
        // From just under +PI toward just under -PI: the short way is
        // forward (increasing, wrapping past PI), not backward through 0.
        let result = turn_toward(3.0, -3.0, 0.05);
        assert!(
            (result - 3.05).abs() < 1e-4,
            "expected the turn to continue increasing (wrapping), got {result}"
        );
    }

    #[test]
    fn turn_toward_normalizes_the_result_after_wrapping_past_pi() {
        let result = turn_toward(3.1, -3.1, 1.0);
        assert!(
            (-PI..=PI).contains(&result),
            "result should stay normalized to (-PI, PI], got {result}"
        );
        // Should have landed close to the target, on the correct (negative)
        // side of the wrap rather than back near +3.1.
        assert!(
            (result - (-3.0998)).abs() < 0.01,
            "expected the wrapped result near the target, got {result}"
        );
    }

    #[test]
    fn a_creature_turns_to_face_its_chase_target_over_several_ticks() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Sheep, spawn, 1);

        // Default facing is 0.0 (+X); send it due +Z instead (a 90 degree
        // turn) so any turning that happens is unambiguous.
        let target = spawn + Vec3::new(0.0, 0.0, 10.0);
        for _ in 0..40 {
            creatures.set_chase_target(id, target);
            creatures.update(&world, 1.0 / 60.0, &[]);
        }

        let facing = creatures.facing_of(id).expect("creature should still exist");
        assert!(
            (facing - FRAC_PI_2).abs() < 0.05,
            "expected the creature to have turned to face +Z (facing ~= {FRAC_PI_2}), got {facing}"
        );
    }

    #[test]
    fn a_stationary_creature_keeps_its_last_facing() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Sheep, spawn, 1);

        // Turn it to face +Z first.
        let target = spawn + Vec3::new(0.0, 0.0, 10.0);
        for _ in 0..40 {
            creatures.set_chase_target(id, target);
            creatures.update(&world, 1.0 / 60.0, &[]);
        }
        let facing_after_turn = creatures.facing_of(id).unwrap();

        // Now let the chase target lapse (stop refreshing it) right at its
        // current position -- with nothing to walk toward, facing must not
        // drift or reset.
        let pos = creatures.snapshot_with_ids()[0].2;
        creatures.set_chase_target(id, Vec3::from_array(pos));
        for _ in 0..10 {
            creatures.update(&world, 1.0 / 60.0, &[]);
        }

        let facing_after_idle = creatures.facing_of(id).unwrap();
        assert!(
            (facing_after_idle - facing_after_turn).abs() < 1e-4,
            "facing shouldn't change while not moving: {facing_after_turn} -> {facing_after_idle}"
        );
    }

    #[test]
    fn creature_kind_u8_round_trips_for_every_kind() {
        for v in 0..=12u8 {
            assert_eq!(CreatureKind::from_u8(v).to_u8(), v);
        }
        assert_eq!(CreatureKind::StoneGolem.to_u8(), 2);
        assert_eq!(CreatureKind::Wolf.to_u8(), 3);
        assert_eq!(CreatureKind::Stinger.to_u8(), 4);
        assert_eq!(CreatureKind::Cow.to_u8(), 5);
        assert_eq!(CreatureKind::Goblin.to_u8(), 6);
    }

    #[test]
    fn a_stone_golem_closes_in_on_a_player_within_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        // Within aggro range but well outside melee range, so this window
        // only exercises the "close the distance" part of the behavior.
        let player_pos = golem_pos + Vec3::new(6.0, 0.0, 0.0);
        let initial_dist = golem_pos.distance(player_pos);

        for _ in 0..60 {
            creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
        }

        let golem_pos_after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
        let dist_after = golem_pos_after.distance(player_pos);
        assert!(
            dist_after < initial_dist,
            "expected the golem to have moved closer to the player: {initial_dist} -> {dist_after}"
        );
    }

    #[test]
    fn a_stone_golem_ignores_a_player_outside_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = golem_pos + Vec3::new(STONE_GOLEM_AGGRO_RADIUS + 5.0, 0.0, 0.0);

        for _ in 0..120 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "a golem should never attack a player outside its aggro radius"
            );
        }
    }

    #[test]
    fn sheep_chicken_and_cow_never_attack_a_nearby_player() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Sheep, spawn, 1);
        creatures.spawn_one(CreatureKind::Chicken, spawn, 2);
        creatures.spawn_one(CreatureKind::Cow, spawn, 3);
        let player_id: PlayerId = 7;
        // Standing right on top of all three -- if any could attack, this is
        // as favorable a setup for it as possible.
        let player_pos = spawn;

        for _ in 0..300 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "sheep, chicken, and cow must never attack a player, regardless of proximity"
            );
        }
    }

    #[test]
    fn a_wolf_closes_in_on_a_player_within_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let wolf_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Wolf, wolf_pos, 1);
        let player_id: PlayerId = 5;
        // Within aggro range but well outside melee range, so this window
        // only exercises the "close the distance" part of the behavior.
        let player_pos = wolf_pos + Vec3::new(6.0, 0.0, 0.0);
        let initial_dist = wolf_pos.distance(player_pos);

        for _ in 0..60 {
            creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
        }

        let wolf_pos_after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
        let dist_after = wolf_pos_after.distance(player_pos);
        assert!(
            dist_after < initial_dist,
            "expected the wolf to have moved closer to the player: {initial_dist} -> {dist_after}"
        );
    }

    #[test]
    fn a_wolf_ignores_a_player_outside_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let wolf_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Wolf, wolf_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = wolf_pos + Vec3::new(WOLF_AGGRO_RADIUS + 5.0, 0.0, 0.0);

        for _ in 0..120 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "a wolf should never attack a player outside its aggro radius"
            );
        }
    }

    #[test]
    fn a_wolf_already_in_melee_range_attacks_on_a_cooldown_not_every_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let wolf_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Wolf, wolf_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = wolf_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(wolf_pos.distance(player_pos) <= WOLF_ATTACK_RANGE);

        let dt = 1.0 / 60.0;
        let duration = WOLF_ATTACK_COOLDOWN * 3.0 + 0.5;
        let ticks = (duration / dt) as usize;
        let mut hit_times = Vec::new();
        let mut t = 0.0f32;
        for _ in 0..ticks {
            let attacks = creatures.update(&world, dt, &[(player_id, player_pos)]);
            if !attacks.is_empty() {
                assert_eq!(
                    attacks,
                    vec![(player_id, WOLF_ATTACK_DAMAGE)],
                    "at most one hit per tick, for the right amount"
                );
                hit_times.push(t);
            }
            t += dt;
        }

        assert!(
            hit_times.len() >= 3,
            "expected several hits over {duration}s at a {WOLF_ATTACK_COOLDOWN}s cooldown, got {}: {hit_times:?}",
            hit_times.len()
        );
        for pair in hit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= WOLF_ATTACK_COOLDOWN - dt * 2.0,
                "hits should be spaced at least the cooldown apart, got a {gap}s gap: {hit_times:?}"
            );
        }
    }

    #[test]
    fn a_stone_golem_already_in_melee_range_attacks_on_a_cooldown_not_every_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = golem_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(golem_pos.distance(player_pos) <= STONE_GOLEM_ATTACK_RANGE);

        let dt = 1.0 / 60.0;
        let duration = STONE_GOLEM_ATTACK_COOLDOWN * 3.0 + 0.5;
        let ticks = (duration / dt) as usize;
        let mut hit_times = Vec::new();
        let mut t = 0.0f32;
        for _ in 0..ticks {
            let attacks = creatures.update(&world, dt, &[(player_id, player_pos)]);
            if !attacks.is_empty() {
                assert_eq!(
                    attacks,
                    vec![(player_id, STONE_GOLEM_ATTACK_DAMAGE)],
                    "at most one hit per tick, for the right amount"
                );
                hit_times.push(t);
            }
            t += dt;
        }

        assert!(
            hit_times.len() >= 3,
            "expected several hits over {duration}s at a {STONE_GOLEM_ATTACK_COOLDOWN}s cooldown, got {}: {hit_times:?}",
            hit_times.len()
        );
        for pair in hit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= STONE_GOLEM_ATTACK_COOLDOWN - dt * 2.0,
                "hits should be spaced at least the cooldown apart, got a {gap}s gap: {hit_times:?}"
            );
        }
    }

    #[test]
    fn a_wolf_chasing_a_player_plays_the_run_clip_not_walk() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let wolf_pos = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Wolf, wolf_pos, 1);
        // Within aggro range but well outside melee range, so this stays in
        // the "closing the distance" phase rather than the attack clip.
        let player_pos = wolf_pos + Vec3::new(6.0, 0.0, 0.0);

        creatures.update(&world, 1.0 / 60.0, &[(1, player_pos)]);

        assert_eq!(
            creatures.anim_clip_of(id),
            Some(AnimClip::Run),
            "an aggroed wolf closing on a player should play its run clip"
        );
    }

    #[test]
    fn a_sheep_wandering_never_plays_the_run_clip() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Sheep, spawn, 1);

        let mut saw_walk = false;
        for _ in 0..300 {
            creatures.update(&world, 1.0 / 60.0, &[]);
            match creatures.anim_clip_of(id) {
                Some(AnimClip::Run) | Some(AnimClip::Attack) => {
                    panic!("sheep has no run/attack clip and must never play one")
                }
                Some(AnimClip::Walk) => saw_walk = true,
                _ => {}
            }
        }
        assert!(saw_walk, "expected the wandering sheep to walk at some point");
    }

    #[test]
    fn an_attack_landing_switches_to_the_attack_clip_which_later_lapses() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_pos = golem_pos + Vec3::new(1.0, 0.0, 0.0);

        let dt = 1.0 / 60.0;
        let mut attacked = false;
        for _ in 0..600 {
            let attacks = creatures.update(&world, dt, &[(1, player_pos)]);
            if !attacks.is_empty() {
                attacked = true;
                assert_eq!(
                    creatures.anim_clip_of(id),
                    Some(AnimClip::Attack),
                    "the tick an attack lands should immediately show the attack clip"
                );
                break;
            }
        }
        assert!(attacked, "expected the golem to land at least one attack");

        // ATTACK_ANIM_DURATION is well under STONE_GOLEM_ATTACK_COOLDOWN, so
        // ticking well past it (without the golem attacking again, since
        // it's on cooldown) must let the clip lapse back to idle/walk.
        for _ in 0..((ATTACK_ANIM_DURATION + 0.2) / dt) as usize {
            creatures.update(&world, dt, &[(1, player_pos)]);
        }
        assert_ne!(
            creatures.anim_clip_of(id),
            Some(AnimClip::Attack),
            "the attack clip should lapse once ATTACK_ANIM_DURATION has passed"
        );
    }

    #[test]
    fn a_stinger_closes_in_on_a_player_within_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let stinger_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Stinger, stinger_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = stinger_pos + Vec3::new(6.0, 0.0, 0.0);
        let initial_dist = stinger_pos.distance(player_pos);

        for _ in 0..60 {
            creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
        }

        let stinger_pos_after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
        let dist_after = stinger_pos_after.distance(player_pos);
        assert!(
            dist_after < initial_dist,
            "expected the stinger to have moved closer to the player: {initial_dist} -> {dist_after}"
        );
    }

    #[test]
    fn a_stinger_ignores_a_player_outside_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let stinger_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Stinger, stinger_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = stinger_pos + Vec3::new(STINGER_AGGRO_RADIUS + 5.0, 0.0, 0.0);

        for _ in 0..120 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "a stinger should never attack a player outside its aggro radius"
            );
        }
    }

    #[test]
    fn a_stinger_already_in_melee_range_attacks_on_a_cooldown_not_every_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let stinger_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Stinger, stinger_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = stinger_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(stinger_pos.distance(player_pos) <= STINGER_ATTACK_RANGE);

        let dt = 1.0 / 60.0;
        let duration = STINGER_ATTACK_COOLDOWN * 3.0 + 0.5;
        let ticks = (duration / dt) as usize;
        let mut hit_times = Vec::new();
        let mut t = 0.0f32;
        for _ in 0..ticks {
            let attacks = creatures.update(&world, dt, &[(player_id, player_pos)]);
            if !attacks.is_empty() {
                assert_eq!(
                    attacks,
                    vec![(player_id, STINGER_ATTACK_DAMAGE)],
                    "at most one hit per tick, for the right amount"
                );
                hit_times.push(t);
            }
            t += dt;
        }

        assert!(
            hit_times.len() >= 3,
            "expected several hits over {duration}s at a {STINGER_ATTACK_COOLDOWN}s cooldown, got {}: {hit_times:?}",
            hit_times.len()
        );
        for pair in hit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= STINGER_ATTACK_COOLDOWN - dt * 2.0,
                "hits should be spaced at least the cooldown apart, got a {gap}s gap: {hit_times:?}"
            );
        }
    }

    #[test]
    fn a_stinger_chasing_a_player_plays_its_run_clip() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let stinger_pos = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Stinger, stinger_pos, 1);
        let player_pos = stinger_pos + Vec3::new(6.0, 0.0, 0.0);

        creatures.update(&world, 1.0 / 60.0, &[(1, player_pos)]);

        assert_eq!(
            creatures.anim_clip_of(id),
            Some(AnimClip::Run),
            "a stinger's model has a run clip, so an aggro-closing stinger should use it"
        );
    }

    #[test]
    fn a_cow_lua_chased_fast_plays_walk_not_run_since_its_model_has_no_run_clip() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Cow, spawn, 1);
        let target = spawn + Vec3::new(10.0, 0.0, 0.0);

        creatures.set_chase_target(id, target);
        creatures.update(&world, 1.0 / 60.0, &[]);

        assert_eq!(
            creatures.anim_clip_of(id),
            Some(AnimClip::Walk),
            "a cow's model has no run clip -- even a fast Lua chase() should stay on walk"
        );
    }

    #[test]
    fn a_goblin_closes_in_on_a_player_within_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let goblin_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Goblin, goblin_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = goblin_pos + Vec3::new(6.0, 0.0, 0.0);
        let initial_dist = goblin_pos.distance(player_pos);

        for _ in 0..60 {
            creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
        }

        let goblin_pos_after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
        let dist_after = goblin_pos_after.distance(player_pos);
        assert!(
            dist_after < initial_dist,
            "expected the goblin to have moved closer to the player: {initial_dist} -> {dist_after}"
        );
    }

    #[test]
    fn a_goblin_ignores_a_player_outside_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let goblin_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Goblin, goblin_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = goblin_pos + Vec3::new(GOBLIN_AGGRO_RADIUS + 5.0, 0.0, 0.0);

        for _ in 0..120 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "a goblin should never attack a player outside its aggro radius"
            );
        }
    }

    #[test]
    fn a_goblin_already_in_melee_range_attacks_on_a_cooldown_not_every_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let goblin_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Goblin, goblin_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = goblin_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(goblin_pos.distance(player_pos) <= GOBLIN_ATTACK_RANGE);

        let dt = 1.0 / 60.0;
        let duration = GOBLIN_ATTACK_COOLDOWN * 3.0 + 0.5;
        let ticks = (duration / dt) as usize;
        let mut hit_times = Vec::new();
        let mut t = 0.0f32;
        for _ in 0..ticks {
            let attacks = creatures.update(&world, dt, &[(player_id, player_pos)]);
            if !attacks.is_empty() {
                assert_eq!(
                    attacks,
                    vec![(player_id, GOBLIN_ATTACK_DAMAGE)],
                    "at most one hit per tick, for the right amount"
                );
                hit_times.push(t);
            }
            t += dt;
        }

        assert!(
            hit_times.len() >= 3,
            "expected several hits over {duration}s at a {GOBLIN_ATTACK_COOLDOWN}s cooldown, got {}: {hit_times:?}",
            hit_times.len()
        );
        for pair in hit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= GOBLIN_ATTACK_COOLDOWN - dt * 2.0,
                "hits should be spaced at least the cooldown apart, got a {gap}s gap: {hit_times:?}"
            );
        }
    }

    #[test]
    fn a_goblin_chasing_a_player_plays_the_run_clip_not_walk() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let goblin_pos = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Goblin, goblin_pos, 1);
        let player_pos = goblin_pos + Vec3::new(6.0, 0.0, 0.0);

        creatures.update(&world, 1.0 / 60.0, &[(1, player_pos)]);

        assert_eq!(
            creatures.anim_clip_of(id),
            Some(AnimClip::Run),
            "a goblin's model has a run clip, so an aggroed goblin closing on a player should use it"
        );
    }

    #[test]
    fn a_wolf_gives_up_chasing_after_a_while_even_if_the_player_stays_in_range() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let wolf_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Wolf, wolf_pos, 1);
        let player_id: PlayerId = 5;
        // Right in melee range the whole test, and never moves -- isolates
        // the give-up timer from "did it just fail to close the distance".
        let player_pos = wolf_pos + Vec3::new(1.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        let run_and_check_for_attacks = |creatures: &mut Creatures, seconds: f32| -> bool {
            let mut attacked = false;
            for _ in 0..((seconds / dt) as usize) {
                if !creatures.update(&world, dt, &[(player_id, player_pos)]).is_empty() {
                    attacked = true;
                }
            }
            attacked
        };

        assert!(
            run_and_check_for_attacks(&mut creatures, 2.0),
            "expected the wolf to have attacked at least once early on"
        );
        // Run well past the give-up point without checking anything --
        // gives the *exact* crossing tick (which can still land one last
        // legitimate attack right up until the moment it actually gives
        // up) a full second of margin on either side, so it can't leak
        // into either measured window.
        run_and_check_for_attacks(&mut creatures, WOLF_CHASE_GIVEUP_SECS - 2.0 + 1.0);
        assert!(
            !run_and_check_for_attacks(&mut creatures, 2.0),
            "expected the wolf to give up (and stop attacking) after {WOLF_CHASE_GIVEUP_SECS}s of \
             continuous chasing, even with the player still standing in melee range"
        );
    }

    #[test]
    fn a_goblin_gives_up_chasing_after_a_while_even_if_the_player_stays_in_range() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let goblin_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Goblin, goblin_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = goblin_pos + Vec3::new(1.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        let run_and_check_for_attacks = |creatures: &mut Creatures, seconds: f32| -> bool {
            let mut attacked = false;
            for _ in 0..((seconds / dt) as usize) {
                if !creatures.update(&world, dt, &[(player_id, player_pos)]).is_empty() {
                    attacked = true;
                }
            }
            attacked
        };

        assert!(
            run_and_check_for_attacks(&mut creatures, 2.0),
            "expected the goblin to have attacked at least once early on"
        );
        run_and_check_for_attacks(&mut creatures, GOBLIN_CHASE_GIVEUP_SECS - 2.0 + 1.0);
        assert!(
            !run_and_check_for_attacks(&mut creatures, 2.0),
            "expected the goblin to give up (and stop attacking) after {GOBLIN_CHASE_GIVEUP_SECS}s of \
             continuous chasing, even with the player still standing in melee range"
        );
    }

    #[test]
    fn a_stone_golem_never_gives_up_chasing_a_player_who_stays_in_range() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let golem_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::StoneGolem, golem_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = golem_pos + Vec3::new(1.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;

        // Well past both a wolf's and a goblin's give-up duration -- a
        // golem has no `chase_giveup_duration`, so it should keep landing
        // hits on its normal cooldown the entire time, never falling
        // silent the way a wolf/goblin would.
        let ticks = ((WOLF_CHASE_GIVEUP_SECS.max(GOBLIN_CHASE_GIVEUP_SECS) + 3.0) / dt) as usize;
        let mut hit_count = 0;
        for _ in 0..ticks {
            if !creatures.update(&world, dt, &[(player_id, player_pos)]).is_empty() {
                hit_count += 1;
            }
        }
        let expected_min_hits =
            ((WOLF_CHASE_GIVEUP_SECS.max(GOBLIN_CHASE_GIVEUP_SECS) + 3.0) / STONE_GOLEM_ATTACK_COOLDOWN) as i32 - 2;
        assert!(
            hit_count as i32 >= expected_min_hits,
            "expected a golem to keep attacking on its normal cooldown throughout, with no give-up \
             pause; got {hit_count} hits, expected at least {expected_min_hits}"
        );
    }

    #[test]
    fn a_sunscorch_closes_in_on_a_player_within_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let sunscorch_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Sunscorch, sunscorch_pos, 1);
        let player_id: PlayerId = 5;
        // Within aggro range but well outside melee range, so this window
        // only exercises the "close the distance" part of the behavior.
        let player_pos = sunscorch_pos + Vec3::new(6.0, 0.0, 0.0);
        let initial_dist = sunscorch_pos.distance(player_pos);

        for _ in 0..60 {
            creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
        }

        let sunscorch_pos_after = Vec3::from_array(creatures.snapshot_with_ids()[0].2);
        let dist_after = sunscorch_pos_after.distance(player_pos);
        assert!(
            dist_after < initial_dist,
            "expected the sunscorch to have moved closer to the player: {initial_dist} -> {dist_after}"
        );
    }

    #[test]
    fn a_sunscorch_ignores_a_player_outside_its_aggro_radius() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let sunscorch_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Sunscorch, sunscorch_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = sunscorch_pos + Vec3::new(SUNSCORCH_AGGRO_RADIUS + 5.0, 0.0, 0.0);

        for _ in 0..120 {
            let attacks = creatures.update(&world, 1.0 / 60.0, &[(player_id, player_pos)]);
            assert!(
                attacks.is_empty(),
                "a sunscorch should never attack a player outside its aggro radius"
            );
        }
    }

    #[test]
    fn a_sunscorch_already_in_melee_range_attacks_on_a_cooldown_not_every_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let sunscorch_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Sunscorch, sunscorch_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = sunscorch_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(sunscorch_pos.distance(player_pos) <= SUNSCORCH_ATTACK_RANGE);

        let dt = 1.0 / 60.0;
        let duration = SUNSCORCH_ATTACK_COOLDOWN * 3.0 + 0.5;
        let ticks = (duration / dt) as usize;
        let mut hit_times = Vec::new();
        let mut t = 0.0f32;
        for _ in 0..ticks {
            let attacks = creatures.update(&world, dt, &[(player_id, player_pos)]);
            if !attacks.is_empty() {
                assert_eq!(
                    attacks,
                    vec![(player_id, SUNSCORCH_ATTACK_DAMAGE)],
                    "at most one hit per tick, for the right amount"
                );
                hit_times.push(t);
            }
            t += dt;
        }

        assert!(
            hit_times.len() >= 3,
            "expected several hits over {duration}s at a {SUNSCORCH_ATTACK_COOLDOWN}s cooldown, got {}: {hit_times:?}",
            hit_times.len()
        );
        for pair in hit_times.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap >= SUNSCORCH_ATTACK_COOLDOWN - dt * 2.0,
                "hits should be spaced at least the cooldown apart, got a {gap}s gap: {hit_times:?}"
            );
        }
    }

    #[test]
    fn a_sunscorch_never_gives_up_chasing_a_player_who_stays_in_range() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let sunscorch_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Sunscorch, sunscorch_pos, 1);
        let player_id: PlayerId = 5;
        let player_pos = sunscorch_pos + Vec3::new(1.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;

        // Well past both a wolf's and a goblin's give-up duration -- like a
        // golem, a sunscorch has no `chase_giveup_duration`, so it should
        // keep landing hits on its normal cooldown the entire time, never
        // falling silent the way a wolf/goblin would.
        let ticks = ((WOLF_CHASE_GIVEUP_SECS.max(GOBLIN_CHASE_GIVEUP_SECS) + 3.0) / dt) as usize;
        let mut hit_count = 0;
        for _ in 0..ticks {
            if !creatures.update(&world, dt, &[(player_id, player_pos)]).is_empty() {
                hit_count += 1;
            }
        }
        let expected_min_hits =
            ((WOLF_CHASE_GIVEUP_SECS.max(GOBLIN_CHASE_GIVEUP_SECS) + 3.0) / SUNSCORCH_ATTACK_COOLDOWN) as i32 - 2;
        assert!(
            hit_count as i32 >= expected_min_hits,
            "expected a sunscorch to keep attacking on its normal cooldown throughout, with no \
             give-up pause; got {hit_count} hits, expected at least {expected_min_hits}"
        );
    }

    #[test]
    fn a_sunscorch_never_speeds_up_while_chasing_since_it_never_runs() {
        let kind = CreatureKind::Sunscorch;
        assert_eq!(
            kind.aggro_speed(),
            kind.speed(),
            "a sunscorch's aggro speed must equal its wander speed -- it never runs, mechanically \
             or in its animation, unlike a wolf/stinger/goblin which speed up while chasing"
        );
    }

    #[test]
    fn pick_starter_kind_can_draw_every_kind_and_roughly_matches_its_weights() {
        let mut rng = SimpleRng::new(0xF00D);
        let mut counts: std::collections::HashMap<u8, u32> = std::collections::HashMap::new();
        const DRAWS: u32 = 200_000;
        for _ in 0..DRAWS {
            *counts.entry(pick_starter_kind(&mut rng).to_u8()).or_insert(0) += 1;
        }

        let total_weight: u32 = STARTER_KIND_WEIGHTS.iter().map(|&(_, w)| w).sum();
        for &(kind, weight) in STARTER_KIND_WEIGHTS {
            let observed = *counts.get(&kind.to_u8()).unwrap_or(&0);
            let expected = DRAWS as f32 * weight as f32 / total_weight as f32;
            let tolerance = (expected * 0.15).max(50.0);
            assert!(
                (observed as f32 - expected).abs() < tolerance,
                "kind {:?}: expected roughly {expected:.0} draws out of {DRAWS} (weight {weight}/{total_weight}), got {observed}",
                kind.to_u8(),
            );
        }
    }

    #[test]
    fn starter_scatter_favors_neutral_kinds_over_common_hostiles_over_rare_hostiles() {
        let world = World::new(7);
        let mut creatures = Creatures::new();
        creatures.spawn_around(&world, Vec3::new(0.0, 0.0, 0.0), 3000, 7);

        let mut counts: std::collections::HashMap<u8, u32> = std::collections::HashMap::new();
        for (_, kind, ..) in creatures.snapshot_with_ids() {
            *counts.entry(kind).or_insert(0) += 1;
        }
        let neutral: u32 = [CreatureKind::Sheep, CreatureKind::Chicken, CreatureKind::Cow]
            .iter()
            .map(|k| *counts.get(&k.to_u8()).unwrap_or(&0))
            .sum();
        let common_hostile: u32 = [CreatureKind::Wolf, CreatureKind::Stinger, CreatureKind::Goblin]
            .iter()
            .map(|k| *counts.get(&k.to_u8()).unwrap_or(&0))
            .sum();
        let rare_hostile: u32 = [CreatureKind::StoneGolem, CreatureKind::Sunscorch]
            .iter()
            .map(|k| *counts.get(&k.to_u8()).unwrap_or(&0))
            .sum();

        assert!(
            neutral > common_hostile,
            "expected neutral kinds ({neutral}) to noticeably outnumber the common hostile kinds ({common_hostile})"
        );
        assert!(
            common_hostile > rare_hostile,
            "expected the common hostile kinds ({common_hostile}) to outnumber the rare ones ({rare_hostile})"
        );
        assert!(rare_hostile > 0, "expected at least one rare hostile kind to appear over 3000 spawns");
    }

    #[test]
    fn a_walking_creature_eventually_queues_a_footstep_audio_event() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let id = creatures.spawn_one(CreatureKind::Sheep, Vec3::new(0.0, spawn_y, 0.0), 1);
        // Force a distant wander/chase target so it walks in a straight
        // line for long enough to cross CREATURE_STEP_LENGTH.
        creatures.set_chase_target(id, Vec3::new(50.0, spawn_y, 0.0));

        let mut saw_step = false;
        for _ in 0..600 {
            creatures.update(&world, 1.0 / 60.0, &[]);
            let events = creatures.take_audio_events();
            if events.steps.iter().any(|&(kind, _)| kind == CreatureKind::Sheep) {
                saw_step = true;
                break;
            }
        }
        assert!(saw_step, "expected a moving creature to eventually queue a footstep audio event");
    }

    #[test]
    fn a_single_tick_never_covers_a_whole_step_length_so_never_queues_a_footstep_yet() {
        // No kind's per-tick movement (speed * one 1/60s frame) comes
        // anywhere close to CREATURE_STEP_LENGTH (1.6 blocks) -- the
        // fastest kind, Stinger at up to HUNT_SPEED_MULTIPLIER * 2.2 ≈
        // 3.5 blocks/sec, covers well under 0.1 blocks in a single 1/60s
        // tick. So the very first tick after spawning (Steps starts at 0)
        // should never itself queue a footstep, regardless of kind.
        let world = World::new(1);
        for &kind in &[
            CreatureKind::Sheep,
            CreatureKind::Chicken,
            CreatureKind::StoneGolem,
            CreatureKind::Wolf,
            CreatureKind::Stinger,
            CreatureKind::Cow,
            CreatureKind::Goblin,
            CreatureKind::Sunscorch,
        ] {
            let mut creatures = Creatures::new();
            let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
            creatures.spawn_one(kind, Vec3::new(0.0, spawn_y, 0.0), 1);

            creatures.update(&world, 1.0 / 60.0, &[]);
            let events = creatures.take_audio_events();
            assert!(
                events.steps.is_empty(),
                "kind {:?} queued a footstep on its very first tick, which should be impossible",
                kind.to_u8()
            );
        }
    }

    #[test]
    fn a_landed_hostile_attack_queues_an_attack_audio_event_of_its_own_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let wolf_pos = Vec3::new(0.0, spawn_y, 0.0);
        creatures.spawn_one(CreatureKind::Wolf, wolf_pos, 1);
        let player_pos = wolf_pos + Vec3::new(1.0, 0.0, 0.0);
        assert!(wolf_pos.distance(player_pos) <= WOLF_ATTACK_RANGE);

        creatures.update(&world, 1.0 / 60.0, &[(5, player_pos)]);
        let events = creatures.take_audio_events();

        assert_eq!(events.attacks.len(), 1, "expected exactly one attack-sound event: {:?}", events.attacks);
        let (kind, pos) = events.attacks[0];
        assert_eq!(kind, CreatureKind::Wolf, "a landed wolf attack should queue a Wolf attack-sound event");
        assert!(
            pos.distance(wolf_pos) < 0.5,
            "the attack sound's position should be roughly where the wolf actually is: {pos:?} vs {wolf_pos:?}"
        );
    }

    #[test]
    fn damage_that_kills_a_creature_queues_a_death_audio_event() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let spawn = Vec3::new(0.0, spawn_y, 0.0);
        let id = creatures.spawn_one(CreatureKind::Sheep, spawn, 1);

        let event = creatures.damage(id, 1000.0);
        assert!(event.is_some(), "expected lethal damage to actually kill the sheep");

        let events = creatures.take_audio_events();
        assert_eq!(events.deaths.len(), 1);
        assert_eq!(events.deaths[0].0, CreatureKind::Sheep);
        assert_eq!(events.deaths[0].1, spawn, "the death sound's position should be where the sheep died");
    }

    #[test]
    fn nonlethal_damage_queues_no_death_audio_event() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let id = creatures.spawn_one(CreatureKind::StoneGolem, Vec3::new(0.0, spawn_y, 0.0), 1);

        creatures.damage(id, 1.0);
        let events = creatures.take_audio_events();
        assert!(events.deaths.is_empty(), "surviving damage shouldn't queue a death sound");
    }

    #[test]
    fn destroy_queues_a_death_audio_event_too() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let id = creatures.spawn_one(CreatureKind::Goblin, Vec3::new(0.0, spawn_y, 0.0), 1);

        creatures.destroy(id);
        let events = creatures.take_audio_events();
        assert_eq!(events.deaths.len(), 1);
        assert_eq!(events.deaths[0].0, CreatureKind::Goblin);
    }

    #[test]
    fn take_audio_events_drains_so_the_same_event_is_never_reported_twice() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        let id = creatures.spawn_one(CreatureKind::Sheep, Vec3::new(0.0, spawn_y, 0.0), 1);
        creatures.damage(id, 1000.0);

        let first = creatures.take_audio_events();
        assert_eq!(first.deaths.len(), 1);
        assert_eq!(first.deaths[0].0, CreatureKind::Sheep);
        let second = creatures.take_audio_events();
        assert!(second.deaths.is_empty(), "a drained event must not reappear on the next take_audio_events call");
    }

    #[test]
    fn a_cow_eventually_queues_an_ambient_call_but_a_goblin_never_does() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let spawn_y = world.terrain_height(0, 0) as f32 + 1.0;
        creatures.spawn_one(CreatureKind::Cow, Vec3::new(0.0, spawn_y, 0.0), 1);
        creatures.spawn_one(CreatureKind::Goblin, Vec3::new(10.0, spawn_y, 0.0), 2);

        let mut saw_cow_call = false;
        // AMBIENT_CALL_INTERVAL_MAX is 22s; run comfortably past it.
        for _ in 0..(30 * 60) {
            creatures.update(&world, 1.0 / 60.0, &[]);
            let events = creatures.take_audio_events();
            assert!(
                !events.ambient_calls.iter().any(|&(kind, _)| kind == CreatureKind::Goblin),
                "a goblin has no ambient sound and should never queue an ambient_call event"
            );
            if events.ambient_calls.iter().any(|&(kind, _)| kind == CreatureKind::Cow) {
                saw_cow_call = true;
            }
        }
        assert!(saw_cow_call, "expected a cow to eventually queue an ambient_call event within 30s");
    }
}
