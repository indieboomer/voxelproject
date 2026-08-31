use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glam::Vec3;
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table};
use serde::{Deserialize, Serialize};

use crate::creature::{CreatureKind, Creatures, DeathEvent};
use crate::daynight::is_night;
use crate::net::{PlayerId, HOST_PLAYER_ID};
/// Re-exported clamp constants rather than duplicated, so the range Lua
/// sees for `api.set_player_speed`/`api.set_player_jump` can never drift
/// from the one `Player::update` is actually bound by. Health clamping
/// happens on `App`'s side, in `Player::damage`/`Player::heal` themselves,
/// when a `PlayerEffect::Health` is applied.
use crate::player::{MAX_ATTRIBUTE_MULTIPLIER, MIN_ATTRIBUTE_MULTIPLIER};
use crate::voxel::block::{BlockType, COLLECTIBLE_BLOCKS};
use crate::voxel::World;
use crate::weather::{Weather, WeatherState};

/// How often (in seconds) modules get ticked. Real "tick" cadence rather
/// than every render frame, matching the README's tick/event model.
pub const TICK_INTERVAL: f32 = 0.1;

const MAX_SCRIPT_TIME: Duration = Duration::from_millis(5);
const MEMORY_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const HOOK_INSTRUCTION_INTERVAL: u32 = 1000;
/// Resource budgets, enforced per module per call (tick or death event) --
/// a module can't flood the world with edits or creatures in one shot.
const MAX_BLOCK_EDITS_PER_CALL: u32 = 32;
const MAX_SPAWNS_PER_CALL: u32 = 4;
const MAX_FIND_RADIUS: f32 = 10.0;
const MAX_FIND_RESULTS: usize = 64;
/// Higher one-shot budgets for an instant spell's `on_cast` call -- it runs
/// exactly once per Run click rather than ~10/sec, so it can afford to do
/// more work in that one call than a rule's per-tick budget allows.
const MAX_BLOCK_EDITS_PER_CAST: u32 = 300;
const MAX_SPAWNS_PER_CAST: u32 = 30;
/// Caps the `amount` argument to `api.give_item` itself (not a call-count
/// budget like the ones above) -- see world_api/schema.yaml's
/// `item_grant_max`.
const MAX_ITEM_GRANT_AMOUNT: u32 = 500;

/// One player's state as exposed to the World API -- position plus enough
/// movement/environment context for rules to tell walking from sprinting,
/// grounded from airborne, dry from submerged. For remote players the host
/// only ever sees network snapshots, so `velocity`/`on_ground`/`in_water`
/// are approximated from those rather than read from real physics state;
/// see `App::host_player_positions`.
#[derive(Clone, Copy)]
pub struct PlayerSnapshot {
    pub id: PlayerId,
    pub pos: Vec3,
    pub carrying_crystal: bool,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub sprinting: bool,
    pub in_water: bool,
    pub health: f32,
    pub poisoned: bool,
    pub speed_multiplier: f32,
    pub jump_multiplier: f32,
    /// 0-100, drains while `in_water`, regenerates otherwise -- see
    /// `player::OXYGEN_DRAIN_PER_SEC`/`_REGEN_PER_SEC`. Read-only from Lua;
    /// there's no `api.set_player_oxygen`, since submersion alone drives it.
    pub oxygen: f32,
}

/// A player-caused block break (mining, not a rule's own `replace_block`),
/// collected by `App` so `ScriptHost::run_tick` can fire `on_block_break` to
/// every module -- not just whichever one, if any, caused it.
pub struct BlockBreakEvent {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block: BlockType,
    pub player_id: PlayerId,
}

/// One player-targeted state change requested by a World API action;
/// collected during a Lua call the same way `block_edits`/`broadcasts` are,
/// and applied by the caller (`App`) afterward -- directly for the host's
/// own `Player`, or replicated to that client for a remote one (see
/// world_api/schema.yaml's `replication.item_grants`/`player_attributes`).
/// One enum instead of a separate `Vec` per kind keeps `TickInput` from
/// growing a new field for every future player-targeted action.
#[derive(Debug, PartialEq)]
pub enum PlayerEffect {
    /// `api.give_item` -- always succeeds once accepted (there's no
    /// "insufficient inventory" failure mode for adding), so nothing else
    /// needs to observe the outcome.
    GiveItem {
        player_id: PlayerId,
        block: BlockType,
        amount: u32,
    },
    /// `api.take_item` -- only ever queued for `HOST_PLAYER_ID` (see
    /// `take_item`'s doc); the boolean Lua already got back came from a
    /// synchronous `host_resources` check at call time, so the apply side
    /// just has to match that check, not repeat it.
    TakeItem {
        player_id: PlayerId,
        block: BlockType,
        amount: u32,
    },
    /// `api.damage_player` (negative `delta`) / `api.heal_player` (positive).
    Health { player_id: PlayerId, delta: f32 },
    /// `api.set_poisoned`.
    Poisoned { player_id: PlayerId, poisoned: bool },
    /// `api.set_player_speed`.
    SpeedMultiplier { player_id: PlayerId, multiplier: f32 },
    /// `api.set_player_jump`.
    JumpMultiplier { player_id: PlayerId, multiplier: f32 },
}

/// What a module needs from the live game each tick, plus the outputs it
/// can produce. Only what concrete rules actually need is exposed to Lua --
/// extend this as new rules require more of the World API.
pub struct TickInput<'a> {
    pub creatures: &'a mut Creatures,
    pub world: &'a World,
    pub players: &'a [PlayerSnapshot],
    /// Mutable so `api.set_time_of_day`/`set_time_dawn`/`set_time_night` can
    /// steer the real clock directly, the same way `weather` already does
    /// for `set_weather`/`start_rain`/`stop_rain`.
    pub time_of_day: &'a mut f32,
    pub weather: &'a mut WeatherState,
    /// Block changes a module asked for; the caller (`App`) applies these
    /// through the normal replicated edit path after the tick, since a Lua
    /// closure can't safely hold a `&mut World` mid-mesh-rebuild.
    pub block_edits: &'a mut Vec<(i32, i32, i32, BlockType)>,
    /// Creature deaths caused this call (via `damage`/`destroy`), collected
    /// so the caller can fire `on_death` to every module afterward -- not
    /// just the one that caused it.
    pub death_events: &'a mut Vec<DeathEvent>,
    /// Messages a module asked to broadcast via `api.broadcast`; the caller
    /// (`App`) relays each through the same host-to-all notification path
    /// (toast + chat log + network `Notify`) every other rule-triggered
    /// message already uses.
    pub broadcasts: &'a mut Vec<String>,
    /// Player-targeted state changes requested this call; see `PlayerEffect`.
    pub player_effects: &'a mut Vec<PlayerEffect>,
    /// A snapshot of the host's own `Player::resources_snapshot()`, taken
    /// once before the Lua call -- lets `get_resource_count`/`take_item`
    /// answer synchronously for `HOST_PLAYER_ID` without needing a `&mut
    /// Player` inside the Lua scope. See their host-only caveat in
    /// world_api/schema.yaml.
    pub host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ModuleSaveEntry {
    pub name: String,
    pub prompt: String,
    pub source: String,
    pub enabled: bool,
}

/// One sandboxed rule: its own Lua VM (so one module's globals/state can
/// never leak into another's, and a crashed module can't corrupt the rest),
/// with instruction/time/memory budgets enforced at the VM level.
pub struct Module {
    pub name: String,
    pub prompt: String,
    pub source: String,
    pub enabled: bool,
    pub error: Option<String>,
    /// True for an instant spell (defines `on_cast`), false for a
    /// continuous rule (defines `on_tick`) -- see `Module::load`. Derived
    /// fresh from `source` every load, never persisted; see
    /// world_api/schema.yaml's `persistence.instant_vs_rule`.
    pub is_instant: bool,
    lua: Lua,
    call_start: Rc<Cell<Option<Instant>>>,
    spawn_seed: Cell<u64>,
}

impl Module {
    pub fn load(name: String, prompt: String, source: String) -> Result<Module, String> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::new(),
        )
        .map_err(|e| e.to_string())?;
        lua.set_memory_limit(MEMORY_LIMIT_BYTES)
            .map_err(|e| e.to_string())?;

        let call_start: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let hook_start = call_start.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTION_INTERVAL),
            move |_lua, _debug| {
                if let Some(start) = hook_start.get() {
                    if start.elapsed() > MAX_SCRIPT_TIME {
                        return Err(mlua::Error::RuntimeError(
                            "script exceeded its time budget".into(),
                        ));
                    }
                }
                Ok(())
            },
        );

        lua.load(&source)
            .exec()
            .map_err(|e| format!("load error: {e}"))?;

        let has_on_tick = lua
            .globals()
            .get::<_, Function>("on_tick")
            .map(|_| true)
            .unwrap_or(false);
        let has_on_cast = lua
            .globals()
            .get::<_, Function>("on_cast")
            .map(|_| true)
            .unwrap_or(false);
        let is_instant = match (has_on_tick, has_on_cast) {
            (true, false) => false,
            (false, true) => true,
            (true, true) => {
                return Err(
                    "module defines both `on_tick(api)` and `on_cast(api, event)` -- it must be \
                     exactly one: a continuous rule (on_tick) or an instant spell (on_cast), \
                     never both"
                        .to_string(),
                )
            }
            (false, false) => {
                return Err(
                    "module does not define a global `on_tick(api)` function (for a continuous \
                     rule) or `on_cast(api, event)` function (for an instant spell)"
                        .to_string(),
                )
            }
        };

        let seed_hash = name.bytes().fold(0x1234_5678_9abc_def0u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });

        Ok(Module {
            name,
            prompt,
            source,
            enabled: false,
            error: None,
            is_instant,
            lua,
            call_start,
            spawn_seed: Cell::new(seed_hash),
        })
    }

    fn disable(&mut self, err: impl std::fmt::Display) {
        log::warn!("Module '{}' disabled: {}", self.name, err);
        self.error = Some(err.to_string());
        self.enabled = false;
    }

    pub fn run_tick(&mut self, input: &mut TickInput) {
        if !self.enabled {
            return;
        }
        self.call_start.set(Some(Instant::now()));
        let result = call_on_tick(&self.lua, input, &self.spawn_seed);
        self.call_start.set(None);
        if let Err(e) = result {
            self.disable(e);
        }
    }

    /// Fires `on_death` for one death event, if the module defines it. Runs
    /// with the same World API as `on_tick`, so a rule can react to a death
    /// by e.g. placing a block or spawning a creature.
    pub fn run_death(&mut self, input: &mut TickInput, event: &DeathEvent) {
        if !self.enabled {
            return;
        }
        self.call_start.set(Some(Instant::now()));
        let result = call_on_death(&self.lua, input, event, &self.spawn_seed);
        self.call_start.set(None);
        if let Err(e) = result {
            self.disable(e);
        }
    }

    /// Fires `on_block_break` for one player-caused break, if the module
    /// defines it -- same World API and optionality as `on_death`.
    pub fn run_block_break(&mut self, input: &mut TickInput, event: &BlockBreakEvent) {
        if !self.enabled {
            return;
        }
        self.call_start.set(Some(Instant::now()));
        let result = call_on_block_break(&self.lua, input, event, &self.spawn_seed);
        self.call_start.set(None);
        if let Err(e) = result {
            self.disable(e);
        }
    }

    /// Runs an instant spell's `on_cast` exactly once -- the analog of
    /// `run_tick` for `is_instant` modules, triggered by the Rules panel's
    /// Run button rather than the tick loop. Unlike `run_tick`/`run_death`/
    /// `run_block_break`, there's no `enabled` flag to gate on (an instant
    /// spell has no ongoing on/off state) and a failure doesn't disable
    /// anything -- it's just reported back for this one Run click, and the
    /// spell is ready to try again next time.
    pub fn run_cast(&mut self, input: &mut TickInput, caster_id: PlayerId) -> Result<(), String> {
        self.call_start.set(Some(Instant::now()));
        let result = call_on_cast(&self.lua, input, caster_id, &self.spawn_seed);
        self.call_start.set(None);
        match result {
            Ok(()) => {
                self.error = None;
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                log::warn!("Spell '{}' failed: {}", self.name, msg);
                self.error = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// The World API version this module's source is tagged with (see
    /// `world_api_validate::extract_api_version`), if any -- shown in the
    /// Rules panel. `None` for a rule saved before the tagging convention
    /// existed, or an ad-hoc one that never went through the generation
    /// pipeline; that's a normal, fully-supported state, not an error.
    pub fn api_version(&self) -> Option<&str> {
        crate::world_api_validate::extract_api_version(&self.source)
    }

    pub fn to_save_entry(&self) -> ModuleSaveEntry {
        ModuleSaveEntry {
            name: self.name.clone(),
            prompt: self.prompt.clone(),
            source: self.source.clone(),
            enabled: self.enabled,
        }
    }
}

fn crash_message(module: &Module) -> String {
    format!(
        "Rule '{}' crashed and was disabled: {}",
        module.name,
        module.error.as_deref().unwrap_or("unknown error")
    )
}

/// Everything one `ScriptHost::run_tick`/`run_cast` call produced, for the
/// caller (`App`) to apply/relay. Replaces what used to be a growing
/// `(Vec, Vec, Vec)` tuple return -- `item_grants` was the field that made
/// a 4th positional element one too many to keep reading at call sites.
#[derive(Default)]
pub struct TickOutcome {
    /// Block changes requested; apply through the normal replicated edit
    /// path (a Lua closure can't safely hold a `&mut World` mid-mesh-rebuild).
    pub block_edits: Vec<(i32, i32, i32, BlockType)>,
    /// One message per module that crashed (and, for a rule, got
    /// auto-disabled) this call -- a rule/spell can pass load-time
    /// validation and still hit a runtime error the first time it actually
    /// executes (e.g. calling an undefined helper), so this is surfaced
    /// rather than letting it go silently dark.
    pub crashes: Vec<String>,
    /// Messages passed to `api.broadcast`; relay to all players.
    pub broadcasts: Vec<String>,
    /// Player-targeted state changes requested; see `PlayerEffect`.
    pub player_effects: Vec<PlayerEffect>,
}

/// Maps a Lua-facing kind string to the internal `u8` kind code used by
/// `find_creatures`/`nearest_creature`. `None` means "any kind" (both the
/// filter argument itself, when it's "any", and an unrecognized string fall
/// through to matching everything rather than erroring).
fn creature_kind_filter(kind: &str) -> Option<u8> {
    match kind.to_ascii_lowercase().as_str() {
        "sheep" => Some(0),
        "chicken" => Some(1),
        "stone_golem" => Some(2),
        _ => None,
    }
}

/// The Lua-facing kind string for the internal `u8` kind code -- the
/// inverse of `creature_kind_filter`, used everywhere a creature's kind is
/// reported back into a Lua table (`creatures()`/`find_creatures()`/
/// `nearest_creature()`/`on_death`'s event).
fn creature_kind_name(kind_u8: u8) -> &'static str {
    match kind_u8 {
        1 => "chicken",
        2 => "stone_golem",
        _ => "sheep",
    }
}

/// Parses a Lua-facing kind string for `spawn_creature`/
/// `spawn_creature_near_player` -- anything unrecognized (including a
/// typo) silently becomes `Sheep`, matching those methods' documented
/// "not an error" behavior in world_api/schema.yaml.
fn parse_creature_kind(kind: &str) -> CreatureKind {
    if kind.eq_ignore_ascii_case("chicken") {
        CreatureKind::Chicken
    } else if kind.eq_ignore_ascii_case("stone_golem") {
        CreatureKind::StoneGolem
    } else {
        CreatureKind::Sheep
    }
}

/// Horizontal-only speed (excludes fall/jump velocity), which is what
/// "speed" intuitively means for a rule checking how fast a player is
/// moving across the ground.
fn horizontal_speed(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
}

/// Every `PlayerSnapshot` field shared by `players()` and `nearest_player()`
/// -- factored out so the two don't drift out of sync (`nearest_player`
/// just adds its own `distance` on top).
fn set_player_fields<'lua>(e: &Table<'lua>, p: &PlayerSnapshot) -> mlua::Result<()> {
    e.set("id", p.id)?;
    e.set("x", p.pos.x)?;
    e.set("y", p.pos.y)?;
    e.set("z", p.pos.z)?;
    e.set("carrying_crystal", p.carrying_crystal)?;
    e.set("speed", horizontal_speed(p.velocity))?;
    e.set("vertical_speed", p.velocity.y)?;
    e.set("on_ground", p.on_ground)?;
    e.set("running", p.sprinting)?;
    e.set("in_water", p.in_water)?;
    e.set("health", p.health)?;
    e.set("poisoned", p.poisoned)?;
    e.set("speed_multiplier", p.speed_multiplier)?;
    e.set("jump_multiplier", p.jump_multiplier)?;
    e.set("oxygen", p.oxygen)?;
    Ok(())
}

/// Deterministic pseudo-random point offset within `radius` of the origin,
/// derived from a spawn-seed value the same way the rest of this file
/// derives creature spawn ids -- a cheap xorshift mix, not cryptographic,
/// just enough to scatter `spawn_creature_near_player` spawns believably.
/// Uniform over the disk area (not radius-linear-biased toward the center).
fn random_offset_in_disk(seed: u64, radius: f32) -> (f32, f32) {
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    let angle = (x >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;

    let mut y = x ^ 0xD1B5_4A32_D192_ED03;
    y ^= y << 13;
    y ^= y >> 7;
    y ^= y << 17;
    let frac = (y >> 40) as f32 / (1u64 << 24) as f32;
    let r = radius * frac.sqrt();

    (angle.cos() * r, angle.sin() * r)
}

/// Builds the `api` table's functions, shared between `on_tick` and
/// `on_death` calls so both expose an identical World API.
#[allow(clippy::too_many_arguments)]
fn populate_api<'lua, 'scope>(
    _lua: &'lua Lua,
    scope: &mlua::Scope<'lua, 'scope>,
    api: &Table<'lua>,
    time_of_day: f32,
    night: bool,
    weather_name: &'static str,
    players_data: &'scope [PlayerSnapshot],
    creature_list: &'scope [(u32, u8, [f32; 3], f32, f32)],
    creatures_cell: &'scope RefCell<&mut Creatures>,
    world: &'scope World,
    block_edits_cell: &'scope RefCell<&mut Vec<(i32, i32, i32, BlockType)>>,
    death_events_cell: &'scope RefCell<&mut Vec<DeathEvent>>,
    broadcasts_cell: &'scope RefCell<&mut Vec<String>>,
    player_effects_cell: &'scope RefCell<&mut Vec<PlayerEffect>>,
    host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
    weather_cell: &'scope RefCell<&mut WeatherState>,
    time_cell: &'scope RefCell<&mut f32>,
    block_budget: &'scope Cell<u32>,
    spawn_budget: &'scope Cell<u32>,
    spawn_seed: &'scope Cell<u64>,
) -> mlua::Result<()> {
    api.set("time_of_day", time_of_day)?;
    api.set("is_night", night)?;
    api.set("weather", weather_name)?;

    api.set(
        "players",
        scope.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            for (i, p) in players_data.iter().enumerate() {
                let e = lua.create_table()?;
                set_player_fields(&e, p)?;
                t.set(i + 1, e)?;
            }
            Ok(t)
        })?,
    )?;

    api.set(
        "creatures",
        scope.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            for (i, (id, kind, pos, health, max_health)) in creature_list.iter().enumerate() {
                let c = lua.create_table()?;
                c.set("id", *id)?;
                c.set("kind", creature_kind_name(*kind))?;
                c.set("x", pos[0])?;
                c.set("y", pos[1])?;
                c.set("z", pos[2])?;
                c.set("health", *health)?;
                c.set("max_health", *max_health)?;
                t.set(i + 1, c)?;
            }
            Ok(t)
        })?,
    )?;

    api.set(
        "find_creatures",
        scope.create_function(
            move |lua, (kind, cx, cy, cz, radius): (String, f32, f32, f32, f32)| {
                let t = lua.create_table()?;
                let kind_filter = creature_kind_filter(&kind);
                let radius = radius.clamp(0.0, MAX_FIND_RADIUS);
                let radius_sq = radius * radius;
                let center = Vec3::new(cx, cy, cz);
                let mut count = 0usize;
                for (id, k, pos, health, max_health) in creature_list.iter() {
                    if count >= MAX_FIND_RESULTS {
                        break;
                    }
                    if let Some(want) = kind_filter {
                        if *k != want {
                            continue;
                        }
                    }
                    let p = Vec3::from_array(*pos);
                    if p.distance_squared(center) > radius_sq {
                        continue;
                    }
                    let e = lua.create_table()?;
                    e.set("id", *id)?;
                    e.set("kind", creature_kind_name(*k))?;
                    e.set("x", pos[0])?;
                    e.set("y", pos[1])?;
                    e.set("z", pos[2])?;
                    e.set("health", *health)?;
                    e.set("max_health", *max_health)?;
                    count += 1;
                    t.set(count, e)?;
                }
                Ok(t)
            },
        )?,
    )?;

    api.set(
        "nearest_creature",
        scope.create_function(
            move |lua, (kind, x, y, z): (String, f32, f32, f32)| {
                let kind_filter = creature_kind_filter(&kind);
                let center = Vec3::new(x, y, z);
                let nearest = creature_list
                    .iter()
                    .filter(|(_, k, ..)| kind_filter.map_or(true, |want| *k == want))
                    .map(|(id, k, pos, health, max_health)| {
                        let p = Vec3::from_array(*pos);
                        (id, k, pos, health, max_health, p.distance(center))
                    })
                    .min_by(|a, b| a.5.total_cmp(&b.5));
                let Some((id, k, pos, health, max_health, dist)) = nearest else {
                    return Ok(None);
                };
                let e = lua.create_table()?;
                e.set("id", *id)?;
                e.set("kind", creature_kind_name(*k))?;
                e.set("x", pos[0])?;
                e.set("y", pos[1])?;
                e.set("z", pos[2])?;
                e.set("health", *health)?;
                e.set("max_health", *max_health)?;
                e.set("distance", dist)?;
                Ok(Some(e))
            },
        )?,
    )?;

    api.set(
        "nearest_player",
        scope.create_function(move |lua, (x, y, z): (f32, f32, f32)| {
            let center = Vec3::new(x, y, z);
            let nearest = players_data
                .iter()
                .map(|p| (p, p.pos.distance(center)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((p, dist)) = nearest else {
                return Ok(None);
            };
            let e = lua.create_table()?;
            set_player_fields(&e, p)?;
            e.set("distance", dist)?;
            Ok(Some(e))
        })?,
    )?;

    api.set(
        "terrain_height",
        scope.create_function(move |_, (x, z): (i32, i32)| Ok(world.terrain_height(x, z)))?,
    )?;

    api.set(
        "distance",
        scope.create_function(
            |_, (x1, y1, z1, x2, y2, z2): (f32, f32, f32, f32, f32, f32)| {
                Ok(Vec3::new(x1, y1, z1).distance(Vec3::new(x2, y2, z2)))
            },
        )?,
    )?;

    api.set(
        "chase",
        scope.create_function(move |_, (id, x, y, z): (u32, f32, f32, f32)| {
            creatures_cell
                .borrow_mut()
                .set_chase_target(id, Vec3::new(x, y, z));
            Ok(())
        })?,
    )?;

    api.set(
        "damage",
        scope.create_function(move |_, (id, amount): (u32, f32)| {
            if let Some(event) = creatures_cell.borrow_mut().damage(id, amount) {
                death_events_cell.borrow_mut().push(event);
            }
            Ok(())
        })?,
    )?;

    api.set(
        "destroy",
        scope.create_function(move |_, id: u32| {
            if let Some(event) = creatures_cell.borrow_mut().destroy(id) {
                death_events_cell.borrow_mut().push(event);
            }
            Ok(())
        })?,
    )?;

    api.set(
        "spawn_creature",
        scope.create_function(move |_, (kind, x, y, z): (String, f32, f32, f32)| {
            if spawn_budget.get() == 0 {
                return Ok(None);
            }
            spawn_budget.set(spawn_budget.get() - 1);
            let kind = parse_creature_kind(&kind);
            let seed = spawn_seed.get();
            spawn_seed.set(seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
            let id = creatures_cell
                .borrow_mut()
                .spawn_one(kind, Vec3::new(x, y, z), seed);
            Ok(Some(id))
        })?,
    )?;

    api.set(
        "spawn_creature_near_player",
        scope.create_function(
            move |_, (player_id, kind, radius): (u32, String, f32)| {
                if spawn_budget.get() == 0 {
                    return Ok(None);
                }
                let Some(target) = players_data.iter().find(|p| p.id == player_id) else {
                    return Ok(None);
                };
                let radius = radius.clamp(1.0, MAX_FIND_RADIUS);
                spawn_budget.set(spawn_budget.get() - 1);
                let kind = parse_creature_kind(&kind);
                let seed = spawn_seed.get();
                spawn_seed.set(seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
                let (dx, dz) = random_offset_in_disk(seed, radius);
                let sx = target.pos.x + dx;
                let sz = target.pos.z + dz;
                let sy = world.terrain_height(sx.floor() as i32, sz.floor() as i32) as f32 + 1.0;
                let id = creatures_cell
                    .borrow_mut()
                    .spawn_one(kind, Vec3::new(sx, sy, sz), seed);
                Ok(Some(id))
            },
        )?,
    )?;

    api.set(
        "get_block",
        scope.create_function(move |_, (x, y, z): (i32, i32, i32)| {
            Ok(world.get_block(x, y, z).id().to_string())
        })?,
    )?;

    api.set(
        "find_blocks",
        scope.create_function(
            move |lua, (kind, cx, cy, cz, radius): (String, i32, i32, i32, f32)| {
                let t = lua.create_table()?;
                // "wood" is a category alias matching any tree species
                // (oak/spruce/birch/cherry), since rules like rain_mud.lua
                // search for "a tree" generically rather than one species.
                let is_match: Box<dyn Fn(BlockType) -> bool> =
                    if kind.eq_ignore_ascii_case("wood") {
                        Box::new(BlockType::is_wood)
                    } else {
                        let Some(block) = BlockType::from_name(&kind) else {
                            return Ok(t);
                        };
                        Box::new(move |b| b == block)
                    };
                let radius = radius.clamp(0.0, MAX_FIND_RADIUS);
                let r = radius.ceil() as i32;
                let radius_sq = radius * radius;
                let mut count = 0usize;
                'search: for dx in -r..=r {
                    for dy in -r..=r {
                        for dz in -r..=r {
                            if count >= MAX_FIND_RESULTS {
                                break 'search;
                            }
                            if (dx * dx + dy * dy + dz * dz) as f32 > radius_sq {
                                continue;
                            }
                            let (x, y, z) = (cx + dx, cy + dy, cz + dz);
                            if is_match(world.get_block(x, y, z)) {
                                let e = lua.create_table()?;
                                e.set("x", x)?;
                                e.set("y", y)?;
                                e.set("z", z)?;
                                count += 1;
                                t.set(count, e)?;
                            }
                        }
                    }
                }
                Ok(t)
            },
        )?,
    )?;

    api.set(
        "replace_block",
        scope.create_function(move |_, (x, y, z, kind): (i32, i32, i32, String)| {
            let Some(block) = BlockType::from_name(&kind) else {
                return Ok(false);
            };
            if block_budget.get() == 0 {
                return Ok(false);
            }
            block_budget.set(block_budget.get() - 1);
            block_edits_cell.borrow_mut().push((x, y, z, block));
            Ok(true)
        })?,
    )?;

    api.set(
        "set_weather",
        scope.create_function(move |_, name: String| {
            if let Some(w) = Weather::from_name(&name) {
                weather_cell.borrow_mut().set(w);
            }
            Ok(())
        })?,
    )?;

    api.set(
        "start_rain",
        scope.create_function(move |_, ()| {
            weather_cell.borrow_mut().set(Weather::Rain);
            Ok(())
        })?,
    )?;

    api.set(
        "stop_rain",
        scope.create_function(move |_, ()| {
            weather_cell.borrow_mut().set(Weather::Sunny);
            Ok(())
        })?,
    )?;

    api.set(
        "set_time_of_day",
        scope.create_function(move |_, t: f32| {
            **time_cell.borrow_mut() = t.rem_euclid(1.0);
            Ok(())
        })?,
    )?;

    api.set(
        "set_time_dawn",
        scope.create_function(move |_, ()| {
            **time_cell.borrow_mut() = 0.0;
            Ok(())
        })?,
    )?;

    api.set(
        "set_time_night",
        scope.create_function(move |_, ()| {
            **time_cell.borrow_mut() = 0.5;
            Ok(())
        })?,
    )?;

    api.set(
        "broadcast",
        scope.create_function(move |_, msg: String| {
            log::info!("[rule] {msg}");
            broadcasts_cell.borrow_mut().push(msg);
            Ok(())
        })?,
    )?;

    api.set(
        "give_item",
        scope.create_function(
            move |_, (player_id, kind, amount): (PlayerId, String, u32)| {
                if !players_data.iter().any(|p| p.id == player_id) {
                    return Ok(false);
                }
                let Some(block) = BlockType::from_name(&kind) else {
                    return Ok(false);
                };
                if !COLLECTIBLE_BLOCKS.contains(&block) {
                    return Ok(false);
                }
                let amount = amount.min(MAX_ITEM_GRANT_AMOUNT);
                if amount == 0 {
                    return Ok(false);
                }
                player_effects_cell
                    .borrow_mut()
                    .push(PlayerEffect::GiveItem { player_id, block, amount });
                Ok(true)
            },
        )?,
    )?;

    api.set(
        "take_item",
        scope.create_function(
            move |_, (player_id, kind, amount): (PlayerId, String, u32)| {
                // Only the host's own inventory is tracked host-side (see
                // this function's doc in schema.yaml) -- host_resources is
                // that snapshot, so this can answer synchronously and
                // correctly for HOST_PLAYER_ID, and honestly say "no" for
                // anyone else rather than pretend to know.
                if player_id != HOST_PLAYER_ID {
                    return Ok(false);
                }
                let Some(block) = BlockType::from_name(&kind) else {
                    return Ok(false);
                };
                let Some(i) = COLLECTIBLE_BLOCKS.iter().position(|&b| b == block) else {
                    return Ok(false);
                };
                if host_resources[i] < amount {
                    return Ok(false);
                }
                player_effects_cell
                    .borrow_mut()
                    .push(PlayerEffect::TakeItem { player_id, block, amount });
                Ok(true)
            },
        )?,
    )?;

    api.set(
        "get_resource_count",
        scope.create_function(move |_, (player_id, kind): (PlayerId, String)| {
            if player_id != HOST_PLAYER_ID {
                return Ok(None);
            }
            let Some(block) = BlockType::from_name(&kind) else {
                return Ok(None);
            };
            let Some(i) = COLLECTIBLE_BLOCKS.iter().position(|&b| b == block) else {
                return Ok(None);
            };
            Ok(Some(host_resources[i]))
        })?,
    )?;

    api.set(
        "damage_player",
        scope.create_function(move |_, (player_id, amount): (PlayerId, f32)| {
            if !players_data.iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::Health { player_id, delta: -amount });
            Ok(true)
        })?,
    )?;

    api.set(
        "heal_player",
        scope.create_function(move |_, (player_id, amount): (PlayerId, f32)| {
            if !players_data.iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::Health { player_id, delta: amount });
            Ok(true)
        })?,
    )?;

    api.set(
        "set_poisoned",
        scope.create_function(move |_, (player_id, poisoned): (PlayerId, bool)| {
            if !players_data.iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::Poisoned { player_id, poisoned });
            Ok(true)
        })?,
    )?;

    api.set(
        "set_player_speed",
        scope.create_function(move |_, (player_id, multiplier): (PlayerId, f32)| {
            if !players_data.iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            let multiplier = multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::SpeedMultiplier { player_id, multiplier });
            Ok(true)
        })?,
    )?;

    api.set(
        "set_player_jump",
        scope.create_function(move |_, (player_id, multiplier): (PlayerId, f32)| {
            if !players_data.iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            let multiplier = multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::JumpMultiplier { player_id, multiplier });
            Ok(true)
        })?,
    )?;

    Ok(())
}

fn call_on_tick(lua: &Lua, input: &mut TickInput, spawn_seed: &Cell<u64>) -> mlua::Result<()> {
    let on_tick: Option<Function> = lua.globals().get("on_tick").ok();
    let Some(on_tick) = on_tick else {
        return Ok(());
    };

    let time_of_day = *input.time_of_day;
    let night = is_night(time_of_day);
    let weather_name = input.weather.current.name();
    let players_data = input.players;
    let creature_list = input.creatures.snapshot_with_ids();
    let creatures_cell = RefCell::new(&mut *input.creatures);
    let block_edits_cell = RefCell::new(&mut *input.block_edits);
    let death_events_cell = RefCell::new(&mut *input.death_events);
    let broadcasts_cell = RefCell::new(&mut *input.broadcasts);
    let player_effects_cell = RefCell::new(&mut *input.player_effects);
    let host_resources = input.host_resources;
    let weather_cell = RefCell::new(&mut *input.weather);
    let time_cell = RefCell::new(&mut *input.time_of_day);
    let block_budget = Cell::new(MAX_BLOCK_EDITS_PER_CALL);
    let spawn_budget = Cell::new(MAX_SPAWNS_PER_CALL);
    let world = input.world;

    lua.scope(|scope| {
        let api = lua.create_table()?;
        populate_api(
            lua,
            scope,
            &api,
            time_of_day,
            night,
            weather_name,
            players_data,
            &creature_list,
            &creatures_cell,
            world,
            &block_edits_cell,
            &death_events_cell,
            &broadcasts_cell,
            &player_effects_cell,
            host_resources,
            &weather_cell,
            &time_cell,
            &block_budget,
            &spawn_budget,
            spawn_seed,
        )?;
        on_tick.call::<_, ()>(api)
    })
}

fn call_on_death(
    lua: &Lua,
    input: &mut TickInput,
    event: &DeathEvent,
    spawn_seed: &Cell<u64>,
) -> mlua::Result<()> {
    let on_death: Option<Function> = lua.globals().get("on_death").ok();
    let Some(on_death) = on_death else {
        return Ok(());
    };

    let time_of_day = *input.time_of_day;
    let night = is_night(time_of_day);
    let weather_name = input.weather.current.name();
    let players_data = input.players;
    let creature_list = input.creatures.snapshot_with_ids();
    let creatures_cell = RefCell::new(&mut *input.creatures);
    let block_edits_cell = RefCell::new(&mut *input.block_edits);
    let mut scratch_deaths: Vec<DeathEvent> = Vec::new();
    let death_events_cell = RefCell::new(&mut scratch_deaths);
    let broadcasts_cell = RefCell::new(&mut *input.broadcasts);
    let player_effects_cell = RefCell::new(&mut *input.player_effects);
    let host_resources = input.host_resources;
    let weather_cell = RefCell::new(&mut *input.weather);
    let time_cell = RefCell::new(&mut *input.time_of_day);
    let block_budget = Cell::new(MAX_BLOCK_EDITS_PER_CALL);
    let spawn_budget = Cell::new(MAX_SPAWNS_PER_CALL);
    let world = input.world;
    let kind_name = creature_kind_name(event.kind.to_u8());
    let pos = event.pos;

    lua.scope(|scope| {
        let api = lua.create_table()?;
        populate_api(
            lua,
            scope,
            &api,
            time_of_day,
            night,
            weather_name,
            players_data,
            &creature_list,
            &creatures_cell,
            world,
            &block_edits_cell,
            &death_events_cell,
            &broadcasts_cell,
            &player_effects_cell,
            host_resources,
            &weather_cell,
            &time_cell,
            &block_budget,
            &spawn_budget,
            spawn_seed,
        )?;
        let event_table = lua.create_table()?;
        event_table.set("kind", kind_name)?;
        event_table.set("x", pos.x)?;
        event_table.set("y", pos.y)?;
        event_table.set("z", pos.z)?;
        on_death.call::<_, ()>((api, event_table))
    })
}

fn call_on_block_break(
    lua: &Lua,
    input: &mut TickInput,
    event: &BlockBreakEvent,
    spawn_seed: &Cell<u64>,
) -> mlua::Result<()> {
    let on_block_break: Option<Function> = lua.globals().get("on_block_break").ok();
    let Some(on_block_break) = on_block_break else {
        return Ok(());
    };

    let time_of_day = *input.time_of_day;
    let night = is_night(time_of_day);
    let weather_name = input.weather.current.name();
    let players_data = input.players;
    let creature_list = input.creatures.snapshot_with_ids();
    let creatures_cell = RefCell::new(&mut *input.creatures);
    let block_edits_cell = RefCell::new(&mut *input.block_edits);
    let death_events_cell = RefCell::new(&mut *input.death_events);
    let broadcasts_cell = RefCell::new(&mut *input.broadcasts);
    let player_effects_cell = RefCell::new(&mut *input.player_effects);
    let host_resources = input.host_resources;
    let weather_cell = RefCell::new(&mut *input.weather);
    let time_cell = RefCell::new(&mut *input.time_of_day);
    let block_budget = Cell::new(MAX_BLOCK_EDITS_PER_CALL);
    let spawn_budget = Cell::new(MAX_SPAWNS_PER_CALL);
    let world = input.world;
    let kind_name = event.block.id();
    let (bx, by, bz) = (event.x, event.y, event.z);
    let player_id = event.player_id;

    lua.scope(|scope| {
        let api = lua.create_table()?;
        populate_api(
            lua,
            scope,
            &api,
            time_of_day,
            night,
            weather_name,
            players_data,
            &creature_list,
            &creatures_cell,
            world,
            &block_edits_cell,
            &death_events_cell,
            &broadcasts_cell,
            &player_effects_cell,
            host_resources,
            &weather_cell,
            &time_cell,
            &block_budget,
            &spawn_budget,
            spawn_seed,
        )?;
        let event_table = lua.create_table()?;
        event_table.set("kind", kind_name)?;
        event_table.set("x", bx)?;
        event_table.set("y", by)?;
        event_table.set("z", bz)?;
        event_table.set("player_id", player_id)?;
        on_block_break.call::<_, ()>((api, event_table))
    })
}

fn call_on_cast(
    lua: &Lua,
    input: &mut TickInput,
    caster_id: PlayerId,
    spawn_seed: &Cell<u64>,
) -> mlua::Result<()> {
    let on_cast: Option<Function> = lua.globals().get("on_cast").ok();
    let Some(on_cast) = on_cast else {
        return Ok(());
    };

    let time_of_day = *input.time_of_day;
    let night = is_night(time_of_day);
    let weather_name = input.weather.current.name();
    let players_data = input.players;
    let creature_list = input.creatures.snapshot_with_ids();
    let creatures_cell = RefCell::new(&mut *input.creatures);
    let block_edits_cell = RefCell::new(&mut *input.block_edits);
    let death_events_cell = RefCell::new(&mut *input.death_events);
    let broadcasts_cell = RefCell::new(&mut *input.broadcasts);
    let player_effects_cell = RefCell::new(&mut *input.player_effects);
    let host_resources = input.host_resources;
    let weather_cell = RefCell::new(&mut *input.weather);
    let time_cell = RefCell::new(&mut *input.time_of_day);
    // An on_cast call runs exactly once per Run click rather than ~10/sec,
    // so it gets a higher one-shot budget than a rule's per-tick call --
    // see MAX_BLOCK_EDITS_PER_CAST/MAX_SPAWNS_PER_CAST.
    let block_budget = Cell::new(MAX_BLOCK_EDITS_PER_CAST);
    let spawn_budget = Cell::new(MAX_SPAWNS_PER_CAST);
    let world = input.world;

    lua.scope(|scope| {
        let api = lua.create_table()?;
        populate_api(
            lua,
            scope,
            &api,
            time_of_day,
            night,
            weather_name,
            players_data,
            &creature_list,
            &creatures_cell,
            world,
            &block_edits_cell,
            &death_events_cell,
            &broadcasts_cell,
            &player_effects_cell,
            host_resources,
            &weather_cell,
            &time_cell,
            &block_budget,
            &spawn_budget,
            spawn_seed,
        )?;
        let event_table = lua.create_table()?;
        event_table.set("player_id", caster_id)?;
        on_cast.call::<_, ()>((api, event_table))
    })
}

/// Owns the set of rule modules the host has loaded. Only meaningful on the
/// host -- joined clients never touch Lua at all; module-driven changes
/// reach them exactly the same way any other host-side state change does
/// (block edits replicate reliably, creature positions ride the existing
/// snapshot broadcast).
pub struct ScriptHost {
    pub modules: Vec<Module>,
}

impl ScriptHost {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Restores modules exactly as they were saved (source included, so a
    /// world's rules keep working even if the on-disk `modules/` template
    /// they started from is later edited or removed).
    pub fn load_from_save(entries: &[ModuleSaveEntry]) -> Self {
        let mut host = Self::new();
        for e in entries {
            match Module::load(e.name.clone(), e.prompt.clone(), e.source.clone()) {
                Ok(mut m) => {
                    m.enabled = e.enabled;
                    host.modules.push(m);
                }
                Err(err) => log::error!("Failed to reload module '{}': {err}", e.name),
            }
        }
        host
    }

    /// Scans a directory of `.lua` files as the starter rule library for a
    /// brand new world. Loaded disabled -- the host has to review and
    /// activate them, per the runtime rule pipeline.
    pub fn scan_dir(dir: &str) -> Self {
        let mut host = Self::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return host;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let prompt = format!("(manually written) {name}");
            match Module::load(name.to_string(), prompt, source) {
                Ok(m) => {
                    log::info!(
                        "Loaded rule module '{name}' (disabled; activate it from the Rules panel)"
                    );
                    host.modules.push(m);
                }
                Err(err) => log::error!("Failed to load module '{name}': {err}"),
            }
        }
        host
    }

    /// Adds a module that already passed `Module::load` (e.g. one just
    /// generated by the LLM), disabled by default like any other new rule.
    /// Returns its index for later activation.
    pub fn add_generated(&mut self, module: Module) -> usize {
        self.modules.push(module);
        self.modules.len() - 1
    }

    /// Returns (name, new enabled state) so the caller can build a
    /// notification message, or `None` if the index no longer exists.
    pub fn toggle_at(&mut self, index: usize) -> Option<(String, bool)> {
        let m = self.modules.get_mut(index)?;
        m.enabled = !m.enabled;
        m.error = None;
        log::info!(
            "Module '{}' {}",
            m.name,
            if m.enabled { "enabled" } else { "disabled" }
        );
        Some((m.name.clone(), m.enabled))
    }

    /// Returns the removed module's name, if any, for a notification.
    pub fn remove(&mut self, index: usize) -> Option<String> {
        if index >= self.modules.len() {
            return None;
        }
        let m = self.modules.remove(index);
        log::info!("Module '{}' deleted", m.name);
        Some(m.name)
    }

    /// Ticks every enabled module, fires `on_block_break` for each pending
    /// player-caused break, then fires `on_death` for every death any of the
    /// above caused (via `fire_deaths`). `time_of_day` is mutated in place
    /// if a rule calls `set_time_of_day`/`set_time_dawn`/`set_time_night`,
    /// the same way `weather` already is for the weather setters. See
    /// `TickOutcome` for what's returned.
    pub fn run_tick(
        &mut self,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: &mut f32,
        weather: &mut WeatherState,
        block_breaks: &[BlockBreakEvent],
        host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
    ) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let mut death_events = Vec::new();

        for module in &mut self.modules {
            let was_enabled = module.enabled;
            let mut input = TickInput {
                creatures: &mut *creatures,
                world,
                players,
                time_of_day: &mut *time_of_day,
                weather: &mut *weather,
                block_edits: &mut outcome.block_edits,
                death_events: &mut death_events,
                broadcasts: &mut outcome.broadcasts,
                player_effects: &mut outcome.player_effects,
                host_resources,
            };
            module.run_tick(&mut input);
            if was_enabled && !module.enabled {
                outcome.crashes.push(crash_message(module));
            }
        }

        for event in block_breaks {
            for module in &mut self.modules {
                let was_enabled = module.enabled;
                let mut input = TickInput {
                    creatures: &mut *creatures,
                    world,
                    players,
                    time_of_day: &mut *time_of_day,
                    weather: &mut *weather,
                    block_edits: &mut outcome.block_edits,
                    death_events: &mut death_events,
                    broadcasts: &mut outcome.broadcasts,
                    player_effects: &mut outcome.player_effects,
                    host_resources,
                };
                module.run_block_break(&mut input, event);
                if was_enabled && !module.enabled {
                    outcome.crashes.push(crash_message(module));
                }
            }
        }

        self.fire_deaths(
            &death_events,
            world,
            creatures,
            players,
            time_of_day,
            weather,
            host_resources,
            &mut outcome,
        );

        outcome
    }

    /// Executes one instant spell's `on_cast` exactly once -- the Rules
    /// panel Run button's entry point. Unlike `run_tick`, this acts on a
    /// single named module rather than the whole set, and isn't gated by
    /// any enabled flag (an instant spell has no ongoing on/off state). A
    /// death caused by the cast still chains into every module's on_death,
    /// same as `run_tick`. `index` naming anything other than an
    /// `is_instant` module (a bad index, or a regular rule) is a no-op
    /// returning an empty outcome.
    pub fn run_cast(
        &mut self,
        index: usize,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: &mut f32,
        weather: &mut WeatherState,
        caster_id: PlayerId,
        host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
    ) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let Some(module) = self.modules.get_mut(index) else {
            return outcome;
        };
        if !module.is_instant {
            return outcome;
        }

        let mut death_events = Vec::new();
        let mut input = TickInput {
            creatures: &mut *creatures,
            world,
            players,
            time_of_day: &mut *time_of_day,
            weather: &mut *weather,
            block_edits: &mut outcome.block_edits,
            death_events: &mut death_events,
            broadcasts: &mut outcome.broadcasts,
            player_effects: &mut outcome.player_effects,
            host_resources,
        };
        if let Err(err) = module.run_cast(&mut input, caster_id) {
            outcome
                .crashes
                .push(format!("Spell '{}' failed: {}", module.name, err));
        }

        self.fire_deaths(
            &death_events,
            world,
            creatures,
            players,
            time_of_day,
            weather,
            host_resources,
            &mut outcome,
        );

        outcome
    }

    /// Fires `on_death` to every module for each event in `events`, folding
    /// each module's own block edits/broadcasts/item grants and any crash
    /// into `outcome`. Does NOT chain further -- a death an on_death handler
    /// itself causes does not fire on_death again (see the World API doc's
    /// on_death `desc`). Shared between `run_tick` (deaths from ticks and
    /// block breaks) and `run_cast` (deaths from one instant spell).
    #[allow(clippy::too_many_arguments)]
    fn fire_deaths(
        &mut self,
        events: &[DeathEvent],
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: &mut f32,
        weather: &mut WeatherState,
        host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
        outcome: &mut TickOutcome,
    ) {
        for event in events {
            for module in &mut self.modules {
                let was_enabled = module.enabled;
                let mut chained = Vec::new();
                let mut input = TickInput {
                    creatures: &mut *creatures,
                    world,
                    players,
                    time_of_day: &mut *time_of_day,
                    weather: &mut *weather,
                    block_edits: &mut outcome.block_edits,
                    death_events: &mut chained,
                    broadcasts: &mut outcome.broadcasts,
                    player_effects: &mut outcome.player_effects,
                    host_resources,
                };
                module.run_death(&mut input, event);
                if was_enabled && !module.enabled {
                    outcome.crashes.push(crash_message(module));
                }
            }
        }
    }

    pub fn save_entries(&self) -> Vec<ModuleSaveEntry> {
        self.modules.iter().map(Module::to_save_entry).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{MAX_HEALTH, MAX_OXYGEN};
    use crate::voxel::World;

    fn make_creatures(seed: u32) -> Creatures {
        let world = World::new(seed);
        let mut creatures = Creatures::new();
        creatures.spawn_around(&world, Vec3::new(0.0, 0.0, 0.0), 6, seed);
        creatures
    }

    /// Empty resource counts for tests that don't care about inventory --
    /// most of them, since `get_resource_count`/`take_item` have their own
    /// dedicated tests.
    const NO_RESOURCES: [u32; COLLECTIBLE_BLOCKS.len()] = [0; COLLECTIBLE_BLOCKS.len()];

    /// A player snapshot with the new movement/environment/status fields
    /// defaulted to "standing still on dry ground, full health, unpoisoned,
    /// normal attributes" -- most tests only care about
    /// id/pos/carrying_crystal, matching the old 3-tuple's shape.
    fn snapshot(id: PlayerId, pos: Vec3, carrying_crystal: bool) -> PlayerSnapshot {
        PlayerSnapshot {
            id,
            pos,
            carrying_crystal,
            velocity: Vec3::ZERO,
            on_ground: true,
            sprinting: false,
            in_water: false,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        }
    }

    fn run_one_tick(
        module: &mut Module,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: f32,
        weather: &mut WeatherState,
    ) -> Vec<(i32, i32, i32, BlockType)> {
        let mut block_edits = Vec::new();
        let mut death_events = Vec::new();
        let mut broadcasts = Vec::new();
        let mut player_effects = Vec::new();
        let mut tod = time_of_day;
        let mut input = TickInput {
            creatures,
            world,
            players,
            time_of_day: &mut tod,
            weather,
            block_edits: &mut block_edits,
            death_events: &mut death_events,
            broadcasts: &mut broadcasts,
            player_effects: &mut player_effects,
            host_resources: NO_RESOURCES,
        };
        module.run_tick(&mut input);
        block_edits
    }

    /// Like `run_one_tick` but also returns the possibly-changed
    /// `time_of_day`, for tests of `set_time_of_day`/`set_time_dawn`/
    /// `set_time_night`.
    fn run_one_tick_with_time(
        module: &mut Module,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: f32,
        weather: &mut WeatherState,
    ) -> (Vec<(i32, i32, i32, BlockType)>, f32) {
        let mut block_edits = Vec::new();
        let mut death_events = Vec::new();
        let mut broadcasts = Vec::new();
        let mut player_effects = Vec::new();
        let mut tod = time_of_day;
        let mut input = TickInput {
            creatures,
            world,
            players,
            time_of_day: &mut tod,
            weather,
            block_edits: &mut block_edits,
            death_events: &mut death_events,
            broadcasts: &mut broadcasts,
            player_effects: &mut player_effects,
            host_resources: NO_RESOURCES,
        };
        module.run_tick(&mut input);
        (block_edits, tod)
    }

    #[test]
    fn night_hunt_module_chases_nearby_sheep_at_night_when_crystal_carried() {
        let world = World::new(42);
        let mut creatures = make_creatures(42);
        let sheep = creatures
            .snapshot_with_ids()
            .into_iter()
            .find(|(_, kind, _, _, _)| *kind == 0)
            .expect("at least one sheep should have spawned");
        let players = vec![snapshot(0u32, Vec3::from_array(sheep.2), true)];
        let mut weather = WeatherState::new(1);

        let source = std::fs::read_to_string("modules/night_hunt.lua").unwrap();
        let mut module = Module::load("night_hunt".into(), "test".into(), source).unwrap();
        module.enabled = true;

        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.75,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            creatures.any_hunting(),
            "expected a sheep to start chasing the crystal-carrying player"
        );
    }

    #[test]
    fn night_hunt_module_ignores_players_without_a_crystal() {
        let world = World::new(42);
        let mut creatures = make_creatures(42);
        let sheep = creatures
            .snapshot_with_ids()
            .into_iter()
            .find(|(_, kind, _, _, _)| *kind == 0)
            .expect("at least one sheep should have spawned");
        let players = vec![snapshot(0u32, Vec3::from_array(sheep.2), false)];
        let mut weather = WeatherState::new(1);

        let source = std::fs::read_to_string("modules/night_hunt.lua").unwrap();
        let mut module = Module::load("night_hunt".into(), "test".into(), source).unwrap();
        module.enabled = true;

        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.75,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            !creatures.any_hunting(),
            "sheep should not chase a player without a crystal"
        );
    }

    #[test]
    fn rain_mud_module_places_mud_near_a_tree_during_rain() {
        let mut world = World::new(7);
        // Load a wide area so the rule has real Wood blocks to search
        // around -- trees are rare, so one chunk isn't reliably enough.
        for cx in -3..=3 {
            for cz in -3..=3 {
                world.ensure_chunk_loaded(cx, cz);
            }
        }
        // Find a real tree trunk to stand the test player next to, so the
        // rule (which searches near players) has something to act on.
        let mut tree_pos = None;
        'search: for cx in -48..48 {
            for cz in -48..48 {
                for cy in 0..48 {
                    if world.get_block(cx, cy, cz).is_wood() {
                        tree_pos = Some(Vec3::new(cx as f32, cy as f32, cz as f32));
                        break 'search;
                    }
                }
            }
        }
        let tree_pos = tree_pos.expect("test world should contain at least one tree");

        let mut creatures = Creatures::new();
        let players = vec![snapshot(0, tree_pos, false)];
        let mut weather = WeatherState::new(1);
        weather.set(Weather::Rain);

        let source = std::fs::read_to_string("modules/rain_mud.lua").unwrap();
        let mut module = Module::load("rain_mud".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.3,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::Mud),
            "expected at least one mud edit: {edits:?}"
        );
    }

    #[test]
    fn redstone_module_spawns_a_healing_stone_after_three_deaths_in_one_spot() {
        // No player-vs-creature combat exists yet, so this drives deaths the
        // same way a future combat system would: a module calling
        // `api.destroy` on a creature. What's under test is the *reaction*
        // -- on_death firing to every module and the third clustered death
        // triggering a redstone spawn -- not how the death was caused.
        let world = World::new(3);
        // Spawned deliberately close together (not via spawn_around's wide
        // scatter) so all three deaths fall within the rule's cluster
        // radius.
        let mut creatures = Creatures::new();
        let ids: Vec<u32> = vec![
            creatures.spawn_one(CreatureKind::Sheep, Vec3::new(10.0, 5.0, 10.0), 1),
            creatures.spawn_one(CreatureKind::Sheep, Vec3::new(10.5, 5.0, 10.5), 2),
            creatures.spawn_one(CreatureKind::Sheep, Vec3::new(11.0, 5.0, 11.0), 3),
        ];
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let killer_source = r#"
            function on_tick(api)
                local cs = api.creatures()
                for _, c in ipairs(cs) do
                    api.destroy(c.id)
                    return
                end
            end
        "#;
        let mut killer =
            Module::load("killer".into(), "test".into(), killer_source.to_string()).unwrap();
        killer.enabled = true;

        let redstone_source = std::fs::read_to_string("modules/redstone_healing.lua").unwrap();
        let mut redstone_module =
            Module::load("redstone_healing".into(), "test".into(), redstone_source).unwrap();
        redstone_module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(killer);
        host.modules.push(redstone_module);

        let mut all_edits = Vec::new();
        let mut time_of_day = 0.5;
        for _ in 0..ids.len() {
            let outcome = host.run_tick(
                &world,
                &mut creatures,
                &players,
                &mut time_of_day,
                &mut weather,
                &[],
                NO_RESOURCES,
            );
            all_edits.extend(outcome.block_edits);
        }

        assert!(
            all_edits
                .iter()
                .any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected a redstone block after three deaths: {all_edits:?}"
        );
    }

    #[test]
    fn run_tick_reports_a_message_for_a_module_that_crashes_at_runtime() {
        // A rule can pass `Module::load`'s validation (valid syntax, has
        // `on_tick`) and still blow up the first time it actually runs --
        // e.g. an LLM-generated rule calling a helper function it forgot to
        // define. `ScriptHost::run_tick` should report that, not just
        // silently flip the module to disabled.
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = "function on_tick(api) this_helper_was_never_defined() end".to_string();
        let mut module = Module::load("broken_helper".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[],
            NO_RESOURCES,
        );
        let crashes = outcome.crashes;

        assert!(
            !host.modules[0].enabled,
            "a module that errors at runtime should be auto-disabled"
        );
        assert_eq!(crashes.len(), 1, "expected exactly one crash report: {crashes:?}");
        assert!(
            crashes[0].contains("broken_helper"),
            "crash message should name the module: {}",
            crashes[0]
        );
    }

    #[test]
    fn runaway_script_is_auto_disabled_by_the_time_budget() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let source = "function on_tick(api) while true do end end".to_string();
        let mut module = Module::load("runaway".into(), "test".into(), source).unwrap();
        module.enabled = true;

        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.75,
            &mut weather,
        );

        assert!(
            !module.enabled,
            "a script stuck in an infinite loop must be auto-disabled"
        );
        assert!(module.error.is_some());
    }

    #[test]
    fn sandbox_blocks_os_io_and_require() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let source = r#"
            function on_tick(api)
                if os ~= nil then error("os should not be available") end
                if io ~= nil then error("io should not be available") end
                if require ~= nil then error("require should not be available") end
                if debug ~= nil then error("debug should not be available") end
            end
        "#
        .to_string();
        let mut module = Module::load("sandbox_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.75,
            &mut weather,
        );

        assert!(
            module.error.is_none(),
            "sandbox check failed: {:?}",
            module.error
        );
    }

    #[test]
    fn a_lua_syntax_error_is_reported_at_load_time() {
        let result = Module::load(
            "broken".into(),
            "test".into(),
            "function on_tick(".to_string(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn block_edit_budget_caps_replace_block_calls_per_tick() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let source = r#"
            function on_tick(api)
                for i = 1, 200 do
                    api.replace_block(i, 10, 0, "mud")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("greedy".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.75,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert_eq!(
            edits.len(),
            MAX_BLOCK_EDITS_PER_CALL as usize,
            "budget should cap edits at {MAX_BLOCK_EDITS_PER_CALL}"
        );
    }

    #[test]
    fn distance_and_terrain_height_report_correct_values() {
        let world = World::new(9);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let expected_height = world.terrain_height(5, 5);

        let source = format!(
            r#"
            function on_tick(api)
                local d = api.distance(0, 0, 0, 3, 4, 0)
                local h = api.terrain_height(5, 5)
                if math.abs(d - 5.0) < 0.001 and h == {expected_height} then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module = Module::load("geometry_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected distance/terrain_height to both check out: {edits:?}"
        );
    }

    #[test]
    fn nearest_player_returns_the_closest_connected_player() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![
            snapshot(1, Vec3::new(0.0, 0.0, 0.0), false),
            snapshot(2, Vec3::new(5.0, 0.0, 0.0), false),
        ];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local p = api.nearest_player(10, 0, 0)
                if p ~= nil and p.id == 2 then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("nearest_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected the closer player (id 2) to be picked as nearest: {edits:?}"
        );
    }

    #[test]
    fn find_creatures_and_nearest_creature_filter_by_kind_and_expose_health() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let sheep_id = creatures.spawn_one(CreatureKind::Sheep, Vec3::new(0.0, 5.0, 0.0), 1);
        let chicken_id = creatures.spawn_one(CreatureKind::Chicken, Vec3::new(1.0, 5.0, 1.0), 2);
        creatures.damage(sheep_id, 4.0);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = format!(
            r#"
            function on_tick(api)
                local sheep_nearby = api.find_creatures("sheep", 0, 5, 0, 5)
                local nearest_chicken = api.nearest_creature("chicken", 0, 5, 0)
                if #sheep_nearby == 1
                    and sheep_nearby[1].id == {sheep_id}
                    and math.abs(sheep_nearby[1].health - 8.0) < 0.001
                    and sheep_nearby[1].max_health == 12.0
                    and nearest_chicken ~= nil
                    and nearest_chicken.id == {chicken_id}
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module = Module::load("creature_query_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected find_creatures/nearest_creature/health fields to all check out: {edits:?}"
        );
    }

    #[test]
    fn player_table_exposes_speed_running_on_ground_and_water_state() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![PlayerSnapshot {
            id: 3,
            pos: Vec3::new(0.0, 0.0, 0.0),
            carrying_crystal: false,
            velocity: Vec3::new(3.0, -1.0, 4.0),
            on_ground: false,
            sprinting: true,
            in_water: true,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        }];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local p = api.players()[1]
                if math.abs(p.speed - 5.0) < 0.001
                    and p.vertical_speed == -1.0
                    and p.on_ground == false
                    and p.running == true
                    and p.in_water == true
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("player_state_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected speed/vertical_speed/on_ground/running/in_water to all check out: {edits:?}"
        );
    }

    #[test]
    fn on_block_break_fires_with_the_correct_event_fields() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_tick(api) end

            function on_block_break(api, event)
                if event.kind == "grass"
                    and event.x == 1
                    and event.y == 2
                    and event.z == 3
                    and event.player_id == 7
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("break_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let event = BlockBreakEvent {
            x: 1,
            y: 2,
            z: 3,
            block: BlockType::Grass,
            player_id: 7,
        };
        let outcome = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[event],
            NO_RESOURCES,
        );
        let edits = outcome.block_edits;

        assert!(host.modules[0].error.is_none(), "module errored: {:?}", host.modules[0].error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected on_block_break to see kind/x/y/z/player_id: {edits:?}"
        );
    }

    #[test]
    fn storm_summoner_module_starts_rain_and_spawns_a_sheep_when_sprinting_player_breaks_crystal() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let target_pos = Vec3::new(4.0, 5.0, 4.0);
        let players = vec![PlayerSnapshot {
            id: 9,
            pos: target_pos,
            carrying_crystal: false,
            velocity: Vec3::ZERO,
            on_ground: true,
            sprinting: true,
            in_water: false,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        }];
        let mut weather = WeatherState::new(1);
        assert_eq!(weather.current, Weather::Sunny);
        let mut time_of_day = 0.5;

        let source = std::fs::read_to_string("modules/storm_summoner.lua").unwrap();
        let mut module = Module::load("storm_summoner".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let event = BlockBreakEvent {
            x: 4,
            y: 5,
            z: 4,
            block: BlockType::Crystal,
            player_id: 9,
        };
        host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[event],
            NO_RESOURCES,
        );

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
        assert_eq!(
            weather.current,
            Weather::Rain,
            "expected the storm to start rain"
        );
        assert_eq!(
            creatures.snapshot_with_ids().len(),
            1,
            "expected a sheep to spawn near the player"
        );
    }

    #[test]
    fn storm_summoner_module_ignores_a_crystal_break_while_walking() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![PlayerSnapshot {
            id: 9,
            pos: Vec3::new(4.0, 5.0, 4.0),
            carrying_crystal: false,
            velocity: Vec3::ZERO,
            on_ground: true,
            sprinting: false,
            in_water: false,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        }];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = std::fs::read_to_string("modules/storm_summoner.lua").unwrap();
        let mut module = Module::load("storm_summoner".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let event = BlockBreakEvent {
            x: 4,
            y: 5,
            z: 4,
            block: BlockType::Crystal,
            player_id: 9,
        };
        host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[event],
            NO_RESOURCES,
        );

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
        assert_eq!(
            weather.current,
            Weather::Sunny,
            "a walking player's crystal break should not summon a storm"
        );
        assert_eq!(creatures.snapshot_with_ids().len(), 0);
    }

    #[test]
    fn jump_rain_module_starts_rain_only_on_the_tick_a_player_leaves_the_ground_while_rising() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut weather = WeatherState::new(1);

        let source = std::fs::read_to_string("modules/jump_rain.lua").unwrap();
        let mut module = Module::load("jump_rain".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let grounded = PlayerSnapshot {
            id: 0,
            pos: Vec3::new(0.0, 0.0, 0.0),
            carrying_crystal: false,
            velocity: Vec3::ZERO,
            on_ground: true,
            sprinting: false,
            in_water: false,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        };
        run_one_tick(&mut module, &world, &mut creatures, &[grounded], 0.5, &mut weather);
        assert!(
            module.error.is_none(),
            "module errored on grounded tick: {:?}",
            module.error
        );
        assert_eq!(
            weather.current,
            Weather::Sunny,
            "standing still shouldn't start rain"
        );

        let jumping = PlayerSnapshot {
            id: 0,
            pos: Vec3::new(0.0, 1.0, 0.0),
            carrying_crystal: false,
            velocity: Vec3::new(0.0, 5.0, 0.0),
            on_ground: false,
            sprinting: false,
            in_water: false,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        };
        run_one_tick(&mut module, &world, &mut creatures, &[jumping], 0.5, &mut weather);
        assert!(
            module.error.is_none(),
            "module errored on jump tick: {:?}",
            module.error
        );
        assert_eq!(
            weather.current,
            Weather::Rain,
            "expected rain to start the tick the player leaves the ground while rising"
        );
    }

    #[test]
    fn jump_rain_module_does_not_fire_for_a_player_first_seen_already_airborne() {
        // A player seen mid-jump on the very first tick (e.g. one who just
        // connected) has no recorded previous on_ground state yet, so this
        // must not be mistaken for "just jumped".
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut weather = WeatherState::new(1);

        let source = std::fs::read_to_string("modules/jump_rain.lua").unwrap();
        let mut module = Module::load("jump_rain".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let already_airborne = PlayerSnapshot {
            id: 0,
            pos: Vec3::new(0.0, 3.0, 0.0),
            carrying_crystal: false,
            velocity: Vec3::new(0.0, 5.0, 0.0),
            on_ground: false,
            sprinting: false,
            in_water: false,
            health: MAX_HEALTH,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: MAX_OXYGEN,
        };
        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &[already_airborne],
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert_eq!(
            weather.current,
            Weather::Sunny,
            "a player first observed already airborne shouldn't falsely trigger rain"
        );
    }

    #[test]
    fn spawn_creature_near_player_spawns_within_radius_of_the_target_player() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let target_pos = Vec3::new(10.0, 5.0, 10.0);
        let players = vec![snapshot(5, target_pos, false)];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                api.spawn_creature_near_player(5, "sheep", 3)
            end
        "#
        .to_string();
        let mut module = Module::load("spawn_near_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        let spawned = creatures.snapshot_with_ids();
        assert_eq!(spawned.len(), 1, "expected exactly one spawned creature");
        let (_, kind, pos, _, _) = spawned[0];
        assert_eq!(kind, 0, "expected a sheep (kind 0)");
        let dx = pos[0] - target_pos.x;
        let dz = pos[2] - target_pos.z;
        assert!(
            (dx * dx + dz * dz).sqrt() <= 3.0 + 0.001,
            "expected the spawn within radius 3 of the target player, got offset ({dx}, {dz})"
        );
    }

    #[test]
    fn start_rain_and_stop_rain_toggle_the_weather_state() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();

        let mut weather = WeatherState::new(1);
        assert_eq!(weather.current, Weather::Sunny);
        let mut module = Module::load(
            "rain_check".into(),
            "test".into(),
            "function on_tick(api) api.start_rain() end".to_string(),
        )
        .unwrap();
        module.enabled = true;
        run_one_tick(&mut module, &world, &mut creatures, &players, 0.5, &mut weather);
        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert_eq!(weather.current, Weather::Rain);

        let mut module = Module::load(
            "stop_rain_check".into(),
            "test".into(),
            "function on_tick(api) api.stop_rain() end".to_string(),
        )
        .unwrap();
        module.enabled = true;
        run_one_tick(&mut module, &world, &mut creatures, &players, 0.5, &mut weather);
        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert_eq!(weather.current, Weather::Sunny);
    }

    #[test]
    fn set_time_functions_move_the_real_clock() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let mut dawn_module = Module::load(
            "dawn_check".into(),
            "test".into(),
            "function on_tick(api) api.set_time_dawn() end".to_string(),
        )
        .unwrap();
        dawn_module.enabled = true;
        let (_, tod) = run_one_tick_with_time(
            &mut dawn_module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );
        assert!(dawn_module.error.is_none());
        assert_eq!(tod, 0.0);

        let mut night_module = Module::load(
            "night_check".into(),
            "test".into(),
            "function on_tick(api) api.set_time_night() end".to_string(),
        )
        .unwrap();
        night_module.enabled = true;
        let (_, tod) = run_one_tick_with_time(
            &mut night_module,
            &world,
            &mut creatures,
            &players,
            0.0,
            &mut weather,
        );
        assert!(night_module.error.is_none());
        assert_eq!(tod, 0.5);

        let mut custom_module = Module::load(
            "custom_time_check".into(),
            "test".into(),
            "function on_tick(api) api.set_time_of_day(0.25) end".to_string(),
        )
        .unwrap();
        custom_module.enabled = true;
        let (_, tod) = run_one_tick_with_time(
            &mut custom_module,
            &world,
            &mut creatures,
            &players,
            0.9,
            &mut weather,
        );
        assert!(custom_module.error.is_none());
        assert_eq!(tod, 0.25);
    }

    #[test]
    fn module_save_entry_round_trips_through_bincode() {
        // ModuleSaveEntry's field set is part of WorldSave's binary layout
        // (see world_api/schema.yaml's persistence.save_format) -- this
        // pins down that name/prompt/source/enabled all survive a real
        // bincode serialize/deserialize cycle, the same path save.rs uses.
        let entry = ModuleSaveEntry {
            name: "my_rule".to_string(),
            prompt: "do a thing".to_string(),
            source: "function on_tick(api) end".to_string(),
            enabled: true,
        };
        let bytes = bincode::serialize(&entry).unwrap();
        let restored: ModuleSaveEntry = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.name, entry.name);
        assert_eq!(restored.prompt, entry.prompt);
        assert_eq!(restored.source, entry.source);
        assert_eq!(restored.enabled, entry.enabled);
    }

    #[test]
    fn reloading_a_module_from_its_save_entry_resets_runtime_lua_state_but_keeps_enabled_and_source() {
        // Pins down the persistence boundary documented in
        // world_api/schema.yaml (persistence.not_saved): a module's
        // persistent Lua globals do NOT survive save/reload, only its
        // enabled flag and source code do. This module counts ticks in a
        // persistent global and only acts once the count reaches 3.
        let source = r#"
            count = count or 0
            function on_tick(api)
                count = count + 1
                if count >= 3 then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("counter".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        // Two ticks on the live module -- one short of the threshold, so
        // nothing has fired yet, but its persistent `count` global is now 2.
        for _ in 0..2 {
            run_one_tick(&mut module, &world, &mut creatures, &players, 0.5, &mut weather);
        }

        // Simulate a save/reload: round-trip through ModuleSaveEntry and
        // load a brand new Module from the saved source, exactly like
        // ScriptHost::load_from_save does.
        let entry = module.to_save_entry();
        assert!(entry.enabled, "enabled flag should have been saved as true");
        let mut reloaded = Module::load(entry.name, entry.prompt, entry.source).unwrap();
        reloaded.enabled = entry.enabled;

        // If `count` had survived, one more tick would bring it to 3 and
        // fire replace_block. It must not -- a fresh Lua VM means `count`
        // starts over at 0 (so this tick brings it to 1, not 3).
        let edits = run_one_tick(
            &mut reloaded,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );
        assert!(reloaded.error.is_none(), "module errored: {:?}", reloaded.error);
        assert!(
            edits.is_empty(),
            "runtime Lua state (the `count` global) must NOT survive a save/reload: {edits:?}"
        );
    }

    #[test]
    fn every_shipped_starter_module_still_loads_and_carries_an_api_version_tag() {
        // Regression guard for "preserve existing Lua rules": every .lua
        // file under modules/ (the starter rule library ScriptHost::scan_dir
        // loads for a brand new world) must still pass Module::load's
        // validation after any World API change, and -- since these were
        // retroactively tagged alongside the versioning work -- must carry
        // a recognizable api_version comment.
        let mut checked = 0;
        for entry in std::fs::read_dir("modules").unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }
            let name = path.file_stem().unwrap().to_str().unwrap().to_string();
            let source = std::fs::read_to_string(&path).unwrap();
            let module = Module::load(name.clone(), "test".into(), source)
                .unwrap_or_else(|e| panic!("modules/{name}.lua failed to load: {e}"));
            assert!(
                module.api_version().is_some(),
                "modules/{name}.lua should carry a -- api_version: tag"
            );
            checked += 1;
        }
        assert!(checked >= 5, "expected to find the known starter modules, found {checked}");
    }

    #[test]
    fn get_block_returns_the_same_snake_case_id_replace_block_accepts() {
        // Regression test: get_block used to return `name().to_lowercase()`
        // (the display name, e.g. "Oak Wood" -> "oak wood" with a space),
        // which didn't round-trip through replace_block/find_blocks (which
        // expect the snake_case id "oak_wood"). Any block whose display
        // name has more than one word would have exposed the bug -- oak
        // wood is the one this test pins down.
        let mut world = World::new(1);
        world.ensure_chunk_loaded(0, 0);
        world.set_block(5, 10, 5, BlockType::OakWood);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local kind = api.get_block(5, 10, 5)
                if kind == "oak_wood" and api.replace_block(5, 10, 5, kind) then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("get_block_id_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );

        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected get_block(5,10,5) == \"oak_wood\" and replace_block to round-trip it: {edits:?}"
        );
    }

    #[test]
    fn broadcast_messages_are_returned_from_run_tick_for_the_caller_to_relay() {
        // Regression test: api.broadcast used to only log server-side and
        // never actually reach players despite being documented (and named)
        // as a player-facing notification.
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_tick(api)
                api.broadcast("hello from a rule")
            end
        "#
        .to_string();
        let mut module = Module::load("broadcast_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[],
            NO_RESOURCES,
        );

        assert!(host.modules[0].error.is_none(), "module errored: {:?}", host.modules[0].error);
        assert_eq!(
            outcome.broadcasts,
            vec!["hello from a rule".to_string()],
            "expected the broadcast message to come back out of run_tick"
        );
    }

    #[test]
    fn module_load_rejects_a_source_defining_both_on_tick_and_on_cast() {
        let source = "function on_tick(api) end\nfunction on_cast(api, event) end".to_string();
        let err = match Module::load("both".into(), "test".into(), source) {
            Err(e) => e,
            Ok(_) => panic!("expected loading both on_tick and on_cast to fail"),
        };
        assert!(
            err.contains("both"),
            "expected an error naming the both-defined case: {err}"
        );
    }

    #[test]
    fn module_load_rejects_a_source_defining_neither_on_tick_nor_on_cast() {
        let source = "function on_death(api, event) end".to_string();
        let err = match Module::load("neither".into(), "test".into(), source) {
            Err(e) => e,
            Ok(_) => panic!("expected loading neither on_tick nor on_cast to fail"),
        };
        assert!(
            err.contains("on_tick") && err.contains("on_cast"),
            "expected an error naming both possible entry points: {err}"
        );
    }

    #[test]
    fn module_load_marks_an_on_cast_only_source_as_instant_and_on_tick_only_as_not() {
        let cast_only = Module::load(
            "spell".into(),
            "test".into(),
            "function on_cast(api, event) end".to_string(),
        )
        .unwrap();
        assert!(cast_only.is_instant);

        let tick_only = Module::load(
            "rule".into(),
            "test".into(),
            "function on_tick(api) end".to_string(),
        )
        .unwrap();
        assert!(!tick_only.is_instant);
    }

    #[test]
    fn run_cast_applies_its_effect_exactly_once_per_call_and_does_not_repeat_on_its_own() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = "function on_cast(api, event) api.replace_block(0, 0, 0, \"redstone\") end"
            .to_string();
        let module = Module::load("add_redstone".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        // A cast never fires from a regular tick -- only Run (run_cast).
        let tick_outcome = host.run_tick(&world, &mut creatures, &players, &mut time_of_day, &mut weather, &[], NO_RESOURCES);
        assert!(
            tick_outcome.block_edits.is_empty(),
            "an instant spell's on_cast must never fire from run_tick: {:?}",
            tick_outcome.block_edits
        );

        for _ in 0..2 {
            let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
            assert_eq!(
                outcome.block_edits,
                vec![(0, 0, 0, BlockType::RedStone)],
                "expected exactly one edit per Run click, not accumulated across calls"
            );
        }
    }

    #[test]
    fn run_cast_is_a_no_op_for_a_module_that_is_not_instant() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let module = Module::load(
            "a_rule".into(),
            "test".into(),
            "function on_tick(api) end".to_string(),
        )
        .unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
        assert!(outcome.block_edits.is_empty());
        assert!(outcome.crashes.is_empty());
    }

    #[test]
    fn run_cast_passes_the_caster_id_through_event_player_id() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_cast(api, event)
                if event.player_id == 7 then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let module = Module::load("caster_check".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 7, NO_RESOURCES);
        assert_eq!(outcome.block_edits, vec![(0, 0, 0, BlockType::RedStone)]);
    }

    #[test]
    fn run_cast_chains_a_death_it_causes_into_every_modules_on_death() {
        // Same chaining run_tick already does for tick/block-break deaths
        // (see redstone_module_spawns_a_healing_stone_after_three_deaths_in_one_spot),
        // now exercised for a death an instant spell's on_cast causes.
        let world = World::new(3);
        let mut creatures = Creatures::new();
        let sheep_id = creatures.spawn_one(CreatureKind::Sheep, Vec3::new(0.0, 5.0, 0.0), 1);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let killer_source =
            format!("function on_cast(api, event) api.destroy({sheep_id}) end");
        let killer = Module::load("smite".into(), "test".into(), killer_source).unwrap();

        let mut reactor = Module::load(
            "reactor".into(),
            "test".into(),
            "function on_tick(api) end\nfunction on_death(api, event) api.replace_block(0, 0, 0, \"redstone\") end"
                .to_string(),
        )
        .unwrap();
        reactor.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(killer);
        host.modules.push(reactor);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
        assert!(
            outcome.block_edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected the cast's death to chain into the reactor's on_death: {:?}",
            outcome.block_edits
        );
    }

    #[test]
    fn give_item_grants_a_collectible_resource_to_a_connected_player() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(3u32, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = "function on_cast(api, event) api.give_item(event.player_id, \"stone\", 100) end".to_string();
        let module = Module::load("give_stone".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 3, NO_RESOURCES);
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::GiveItem {
                player_id: 3u32,
                block: BlockType::Stone,
                amount: 100
            }]
        );
    }

    #[test]
    fn give_item_clamps_an_excessive_amount_and_rejects_a_non_collectible_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(0u32, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_cast(api, event)
                local clamped = api.give_item(event.player_id, "stone", 999999)
                local rejected = api.give_item(event.player_id, "crystal", 10)
                if clamped and not rejected then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let module = Module::load("give_edge_cases".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
        assert!(
            outcome.block_edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected the excessive amount to clamp (still true) and crystal to be rejected (false): {:?}",
            outcome.block_edits
        );
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::GiveItem {
                player_id: 0u32,
                block: BlockType::Stone,
                amount: MAX_ITEM_GRANT_AMOUNT
            }],
            "expected only the clamped stone grant, not the rejected crystal one"
        );
    }

    #[test]
    fn players_and_nearest_player_expose_health_poisoned_and_attribute_multipliers() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut player = snapshot(5, Vec3::ZERO, false);
        player.health = 42.0;
        player.poisoned = true;
        player.speed_multiplier = 2.0;
        player.jump_multiplier = 0.5;
        player.oxygen = 17.0;
        let players = vec![player];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local p = api.players()[1]
                local nearest = api.nearest_player(0, 0, 0)
                if p.health == 42.0 and p.poisoned == true
                    and p.speed_multiplier == 2.0 and p.jump_multiplier == 0.5
                    and p.oxygen == 17.0
                    and nearest.health == 42.0 and nearest.poisoned == true
                    and nearest.oxygen == 17.0
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("attr_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(&mut module, &world, &mut creatures, &players, 0.5, &mut weather);
        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected health/poisoned/speed_multiplier/jump_multiplier/oxygen to all round-trip: {edits:?}"
        );
    }

    #[test]
    fn damage_player_and_heal_player_queue_signed_health_deltas_and_check_connection() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(4, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_cast(api, event)
                local damaged_ok = api.damage_player(4, 10.0)
                local healed_ok = api.heal_player(4, 3.0)
                local missing_ok = api.damage_player(999, 1.0)
                if damaged_ok and healed_ok and not missing_ok then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let module = Module::load("health_check".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
        assert!(
            outcome.block_edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected both calls against player 4 to succeed and the one against 999 to fail: {:?}",
            outcome.block_edits
        );
        assert_eq!(
            outcome.player_effects,
            vec![
                PlayerEffect::Health { player_id: 4, delta: -10.0 },
                PlayerEffect::Health { player_id: 4, delta: 3.0 },
            ]
        );
    }

    #[test]
    fn set_poisoned_queues_the_flag_for_a_connected_player_only() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(2, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = "function on_cast(api, event) api.set_poisoned(2, true) end".to_string();
        let module = Module::load("poison_check".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::Poisoned { player_id: 2, poisoned: true }]
        );
    }

    #[test]
    fn set_player_speed_and_jump_clamp_the_multiplier_before_queuing() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(1, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_cast(api, event)
                api.set_player_speed(1, 999.0)
                api.set_player_jump(1, -50.0)
            end
        "#
        .to_string();
        let module = Module::load("multiplier_check".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, 0, NO_RESOURCES);
        assert_eq!(
            outcome.player_effects,
            vec![
                PlayerEffect::SpeedMultiplier { player_id: 1, multiplier: MAX_ATTRIBUTE_MULTIPLIER },
                PlayerEffect::JumpMultiplier { player_id: 1, multiplier: MIN_ATTRIBUTE_MULTIPLIER },
            ]
        );
    }

    #[test]
    fn get_resource_count_and_take_item_only_answer_for_the_host_player() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(HOST_PLAYER_ID, Vec3::ZERO, false), snapshot(9, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;
        let mut host_resources = NO_RESOURCES;
        let stone_index = COLLECTIBLE_BLOCKS.iter().position(|&b| b == BlockType::Stone).unwrap();
        host_resources[stone_index] = 5;

        let source = format!(
            r#"
            function on_cast(api, event)
                local host_count = api.get_resource_count({HOST_PLAYER_ID}, "stone")
                local remote_count = api.get_resource_count(9, "stone")
                local host_take_ok = api.take_item({HOST_PLAYER_ID}, "stone", 5)
                local remote_take_ok = api.take_item(9, "stone", 1)
                if host_count == 5 and remote_count == nil
                    and host_take_ok and not remote_take_ok
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let module = Module::load("resource_check".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, HOST_PLAYER_ID, host_resources);
        assert!(
            outcome.block_edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected host-only queries/takes to succeed and remote ones to fail: {:?}",
            outcome.block_edits
        );
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::TakeItem {
                player_id: HOST_PLAYER_ID,
                block: BlockType::Stone,
                amount: 5,
            }],
            "the failed remote take must not queue anything"
        );
    }

    #[test]
    fn take_item_refuses_to_take_more_than_the_host_currently_holds() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(HOST_PLAYER_ID, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;
        let mut host_resources = NO_RESOURCES;
        let stone_index = COLLECTIBLE_BLOCKS.iter().position(|&b| b == BlockType::Stone).unwrap();
        host_resources[stone_index] = 3;

        let source = format!(
            "function on_cast(api, event) if not api.take_item({HOST_PLAYER_ID}, \"stone\", 4) then api.replace_block(0, 0, 0, \"redstone\") end end"
        );
        let module = Module::load("take_too_much".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(0, &world, &mut creatures, &players, &mut time_of_day, &mut weather, HOST_PLAYER_ID, host_resources);
        assert!(
            outcome.block_edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "taking more than held should fail: {:?}",
            outcome.block_edits
        );
        assert!(
            outcome.player_effects.is_empty(),
            "a failed take must not queue a TakeItem effect: {:?}",
            outcome.player_effects
        );
    }

    #[test]
    fn spawn_creature_and_spawn_creature_near_player_accept_the_stone_golem_kind() {
        // api.*() queries read a snapshot taken before the Lua call starts
        // (see call_on_tick), so a creature spawned this same tick can't be
        // found by find_creatures/nearest_creature until the *next* tick --
        // this only checks that spawning itself accepts "stone_golem"
        // (a non-nil id back) rather than silently defaulting to sheep.
        // Kind round-tripping through queries is covered separately below
        // using a creature that already existed before the tick started.
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(0, Vec3::new(0.0, 5.0, 0.0), false)];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local a = api.spawn_creature("stone_golem", 5, 5, 5)
                local b = api.spawn_creature_near_player(0, "stone_golem", 3)
                if a ~= nil and b ~= nil then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("golem_spawn_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(&mut module, &world, &mut creatures, &players, 0.5, &mut weather);
        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected both spawn calls to accept \"stone_golem\" and return an id: {edits:?}"
        );
        assert_eq!(
            creatures
                .snapshot_with_ids()
                .iter()
                .filter(|(_, kind, ..)| *kind == CreatureKind::StoneGolem.to_u8())
                .count(),
            2,
            "expected both spawned creatures to actually be stone golems, not silently sheep"
        );
    }

    #[test]
    fn find_creatures_and_nearest_creature_report_the_stone_golem_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let golem_id = creatures.spawn_one(CreatureKind::StoneGolem, Vec3::new(5.0, 5.0, 5.0), 1);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = format!(
            r#"
            function on_tick(api)
                local found = api.find_creatures("stone_golem", 5, 5, 5, 1)
                local nearest = api.nearest_creature("stone_golem", 5, 5, 5)
                if #found == 1 and found[1].id == {golem_id} and found[1].kind == "stone_golem"
                    and found[1].max_health == 40.0
                    and nearest ~= nil and nearest.id == {golem_id}
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module = Module::load("golem_query_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let edits = run_one_tick(&mut module, &world, &mut creatures, &players, 0.5, &mut weather);
        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected find_creatures/nearest_creature to report kind \"stone_golem\" and the right max_health: {edits:?}"
        );
    }
}
