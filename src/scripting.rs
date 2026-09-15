use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;

use glam::Vec3;
use mlua::{Function, Lua, LuaOptions, MultiValue, StdLib, Table, Value};
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
use crate::script_budget::{ExecutionBudget, WorkUsage};
use crate::voxel::block::{BlockType, COLLECTIBLE_BLOCKS};
use crate::voxel::World;
use crate::weather::{Weather, WeatherState};
use crate::world_api_gen::{
    SCRIPT_BROADCASTS as MAX_BROADCASTS_PER_CALL, SCRIPT_BROADCAST_BYTES as MAX_BROADCAST_BYTES,
    SCRIPT_MEMORY_BYTES as MEMORY_LIMIT_BYTES, SCRIPT_SOURCE_BYTES as MAX_SOURCE_BYTES,
};

#[path = "script_api.rs"]
mod api;
#[path = "script_environment.rs"]
mod environment;
#[path = "script_world_edit.rs"]
mod world_edit;
#[path = "script_inventory.rs"]
mod inventory;
#[path = "script_automation.rs"]
mod automation_api;
#[path = "script_scheduler.rs"]
mod scheduler;
#[path = "script_transaction.rs"]
mod transaction;
use api::Callback;
use transaction::CallbackTransaction;

#[cfg(test)]
#[path = "script_transaction_tests.rs"]
mod transaction_tests;

/// How often (in seconds) modules get ticked. Real "tick" cadence rather
/// than every render frame, matching the README's tick/event model.
pub const TICK_INTERVAL: f32 = 0.1;

const MAX_API_COORDINATE: f64 = crate::world_api_gen::SCRIPT_COORDINATE_MAX as f64;
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
    pub finances: InventoryBalances,
    pub resources: [u32; COLLECTIBLE_BLOCKS.len()],
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InventoryBalances {
    pub adventure: crate::adventure::Progress,
    pub held: Option<crate::equipment::Entry>,
    pub mana: u32,
    pub elements: [u32; 5],
    pub items: [u32; 21],
}
impl InventoryBalances {
    pub fn from_account(account: &crate::crafting::Account) -> Self {
        Self {mana:account.mana,elements:account.elements,items:account.gear,
            adventure:account.adventure,held:account.hotbar.entry()}
    }
}

/// A player-caused block break (mining, not a rule's own `replace_block`),
/// collected by `App` so `ScriptHost::run_tick` can fire `on_block_break` to
/// every module -- not just whichever one, if any, caused it.
#[derive(Clone, Copy)]
pub struct BlockBreakEvent {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block: BlockType,
    pub player_id: PlayerId,
}

/// A player deliberately using the interact key (E) aimed at a block --
/// distinct from `BlockBreakEvent`: the block is never changed, only
/// reported, so a rule can react to "a player used/examined this" (e.g. a
/// crystal that heals on touch, a lever-like block that triggers something)
/// without the block being destroyed. Collected by `App` the same way
/// `BlockBreakEvent` is, so `ScriptHost::run_tick` can fire `on_interact` to
/// every module.
#[derive(Clone, Copy)]
pub struct InteractEvent {
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
    AutomationState { state: Box<crate::automation::State> },
    Inventory { player_id: PlayerId, balances: InventoryBalances, resources: [u32; COLLECTIBLE_BLOCKS.len()] },
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
    /// transaction balance check, including earlier accepted give/take
    /// commands, so funds are reserved before the apply side sees it.
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
    SpeedMultiplier {
        player_id: PlayerId,
        multiplier: f32,
    },
    /// `api.set_player_jump`.
    JumpMultiplier {
        player_id: PlayerId,
        multiplier: f32,
    },
    /// `api.teleport_player` -- moves a player directly, bypassing normal
    /// movement/collision entirely (the same way `replace_block` bypasses
    /// mining). Applied straight to the host's own `Player::position` for
    /// `HOST_PLAYER_ID`; for anyone else it's a targeted network message
    /// (see `ReliableMsg::Teleport`), since a remote player's position is
    /// simulated on their own machine, not the host's.
    Teleport { player_id: PlayerId, pos: Vec3 },
}

/// What a module needs from the live game each tick, plus the outputs it
/// can produce. Only what concrete rules actually need is exposed to Lua --
/// extend this as new rules require more of the World API.
pub struct TickInput<'a> {
    pub creatures: &'a mut Creatures,
    pub world: &'a World,
    pub players: &'a [PlayerSnapshot],
    /// Time and weather are copied into a callback transaction and updated
    /// here only after the callback succeeds.
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
    /// Host inventory at dispatch start. Transactions add the effects of
    /// earlier committed callbacks and their own queued give/take commands
    /// before answering resource queries or accepting another removal.
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
    budget: Rc<ExecutionBudget>,
    spawn_seed: Cell<u64>,
    /// Failed callbacks can mutate Lua globals even though effects roll
    /// back. Rebuild that VM before permitting another invocation.
    needs_reload: bool,
    // Runtime-only identity: pending work must never follow a shifted Vec
    // index onto another module. Durable IDs belong to the save migration.
    runtime_id: u64,
    activation_epoch: u64,
    last_work: WorkUsage,
}

impl Module {
    pub fn load(name: String, prompt: String, source: String) -> Result<Module, String> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(format!("module source exceeds {MAX_SOURCE_BYTES} bytes"));
        }
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::new().catch_rust_panics(false),
        )
        .map_err(|e| e.to_string())?;
        lua.set_memory_limit(MEMORY_LIMIT_BYTES)
            .map_err(|e| e.to_string())?;

        let budget = ExecutionBudget::install(&lua);
        budget
            .run(|| lua.load(&source).exec())
            .map_err(|e| format!("load error: {e}"))?;

        let has_on_tick = lua
            .globals()
            .raw_get::<_, Function>("on_tick")
            .map(|_| true)
            .unwrap_or(false);
        let has_on_cast = lua
            .globals()
            .raw_get::<_, Function>("on_cast")
            .map(|_| true)
            .unwrap_or(false);
        let is_instant =
            match (has_on_tick, has_on_cast) {
                (true, false) => false,
                (false, true) => true,
                (true, true) => return Err(
                    "module defines both `on_tick(api)` and `on_cast(api, event)` -- it must be \
                     exactly one: a continuous rule (on_tick) or an instant spell (on_cast), \
                     never both"
                        .to_string(),
                ),
                (false, false) => return Err(
                    "module does not define a global `on_tick(api)` function (for a continuous \
                     rule) or `on_cast(api, event)` function (for an instant spell)"
                        .to_string(),
                ),
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
            budget,
            spawn_seed: Cell::new(seed_hash),
            needs_reload: false,
            runtime_id: {
                static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            activation_epoch: 0,
            last_work: WorkUsage::default(),
        })
    }

    fn disable(&mut self, err: impl std::fmt::Display) {
        log::warn!("Module '{}' disabled: {}", self.name, err);
        self.error = Some(err.to_string());
        self.enabled = false;
        self.needs_reload = true;
    }

    fn execute(&mut self, input: &mut TickInput, event: Callback) -> mlua::Result<()> {
        let registry = self.lua.app_data_ref::<std::sync::Arc<crate::crafting::Registry>>().map(|r|r.clone()).unwrap_or_else(default_inventory_registry);
        self.last_work = WorkUsage::default();
        let initialization_work = if self.needs_reload {
            crate::world_api_gen::SCRIPT_INSTRUCTIONS
        } else {
            0
        };
        self.last_work.instructions = initialization_work;
        if self.needs_reload {
            let enabled = self.enabled;
            let seed = self.spawn_seed.get();
            let mut replacement =
                Self::load(self.name.clone(), self.prompt.clone(), self.source.clone())
                    .map_err(mlua::Error::RuntimeError)?;
            replacement.enabled = enabled;
            replacement.spawn_seed.set(seed);
            replacement.runtime_id = self.runtime_id;
            replacement.activation_epoch = self.activation_epoch;
            *self = replacement;
        }
        self.lua.set_app_data(registry);
        let result = self.budget.run(|| {
            let mut tx = CallbackTransaction::new(input, self.spawn_seed.get());
            if matches!(event, Callback::Tick) { tx.policy_owner = Some((self.runtime_id, self.activation_epoch)); }
            api::call(&self.lua, &tx, event)?;
            Ok(tx)
        });
        self.last_work = self.budget.usage();
        self.last_work.instructions += initialization_work;
        match result {
            Ok(tx) => {
                tx.commit(
                    input,
                    &self.spawn_seed,
                    !matches!(event, Callback::Death(_)),
                );
                Ok(())
            }
            Err(error) => {
                input.creatures.attack_policies.remove(&(self.runtime_id, self.activation_epoch));
                self.needs_reload = true;
                Err(error)
            }
        }
    }

    #[cfg(test)]
    pub fn run_tick(&mut self, input: &mut TickInput) {
        if !self.enabled {
            return;
        }
        let result = self.execute(input, Callback::Tick);
        if let Err(e) = result {
            self.disable(e);
        }
    }

    /// Fires `on_death` for one death event, if the module defines it. Runs
    /// with the same World API as `on_tick`, so a rule can react to a death
    /// by e.g. placing a block or spawning a creature.
    #[cfg(test)]
    pub fn run_death(&mut self, input: &mut TickInput, event: &DeathEvent) {
        if !self.enabled {
            return;
        }
        let result = self.execute(input, Callback::Death(event));
        if let Err(e) = result {
            self.disable(e);
        }
    }

    /// Fires `on_block_break` for one player-caused break, if the module
    /// defines it -- same World API and optionality as `on_death`.
    #[cfg(test)]
    pub fn run_block_break(&mut self, input: &mut TickInput, event: &BlockBreakEvent) {
        if !self.enabled {
            return;
        }
        let result = self.execute(input, Callback::BlockBreak(event));
        if let Err(e) = result {
            self.disable(e);
        }
    }

    /// Fires `on_interact` for one player-caused interact-key press, if the
    /// module defines it -- same World API and optionality as
    /// `on_block_break`.
    #[cfg(test)]
    pub fn run_interact(&mut self, input: &mut TickInput, event: &InteractEvent) {
        if !self.enabled {
            return;
        }
        let result = self.execute(input, Callback::Interact(event));
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
    #[cfg(test)]
    pub fn run_cast(&mut self, input: &mut TickInput, caster_id: PlayerId) -> Result<(), String> {
        let result = self.execute(input, Callback::Cast(caster_id));
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
    /// Backpressure notices; not script crashes and do not disable rules.
    pub warnings: Vec<String>,
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
        "wolf" => Some(3),
        "stinger" => Some(4),
        "cow" => Some(5),
        "goblin" => Some(6),
        "sunscorch" => Some(7),
        "zombie" => Some(8),
        "skeleton" => Some(9),
        "dragon_green" => Some(10),
        "dragon_red" => Some(11),
        "fish" => Some(12),
        "skeleton_sorcerer" => Some(13),
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
        3 => "wolf",
        4 => "stinger",
        5 => "cow",
        6 => "goblin",
        7 => "sunscorch",
        8 => "zombie",
        9 => "skeleton",
        10 => "dragon_green",
        11 => "dragon_red",
        12 => "fish",
        13 => "skeleton_sorcerer",
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
    } else if kind.eq_ignore_ascii_case("wolf") {
        CreatureKind::Wolf
    } else if kind.eq_ignore_ascii_case("stinger") {
        CreatureKind::Stinger
    } else if kind.eq_ignore_ascii_case("cow") {
        CreatureKind::Cow
    } else if kind.eq_ignore_ascii_case("goblin") {
        CreatureKind::Goblin
    } else if kind.eq_ignore_ascii_case("sunscorch") {
        CreatureKind::Sunscorch
    } else if kind.eq_ignore_ascii_case("zombie") {
        CreatureKind::Zombie
    } else if kind.eq_ignore_ascii_case("skeleton") {
        CreatureKind::Skeleton
    } else if kind.eq_ignore_ascii_case("dragon_green") {
        CreatureKind::DragonGreen
    } else if kind.eq_ignore_ascii_case("dragon_red") {
        CreatureKind::DragonRed
    } else if kind.eq_ignore_ascii_case("skeleton_sorcerer") {
        CreatureKind::SkeletonSorcerer
    } else if kind.eq_ignore_ascii_case("fish") {
        CreatureKind::Fish
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
    e.set("mana",p.finances.mana)?;
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

/// Owns the set of rule modules the host has loaded. Only meaningful on the
/// host -- joined clients never touch Lua at all; module-driven changes
/// reach them exactly the same way any other host-side state change does
/// (block edits replicate reliably, creature positions ride the existing
/// snapshot broadcast).
pub struct ScriptHost {
    pub spellbook: crate::spellbook::Spellbook,
    pub spell_cooldowns: std::collections::HashMap<crate::spellbook::SpellId,std::time::Instant>,
    pub inventory_registry: std::sync::Arc<crate::crafting::Registry>,
    last_cast_success: Option<(u64, PlayerId)>,
    pub modules: Vec<Module>,
    scheduler: scheduler::Scheduler,
    unloaded_entries: Vec<ModuleSaveEntry>,
}

impl ScriptHost {
    pub fn new() -> Self {
        Self {
            spellbook: Default::default(),
            spell_cooldowns: Default::default(),
            inventory_registry: default_inventory_registry(),
            modules: Vec::new(),
            scheduler: scheduler::Scheduler::default(),
            last_cast_success: None,
            unloaded_entries: Vec::new(),
        }
    }

    /// Restores modules exactly as they were saved (source included, so a
    /// world's rules keep working even if the on-disk `modules/` template
    /// they started from is later edited or removed).
    pub fn load_from_save(entries: &[ModuleSaveEntry]) -> Self {
        let mut host = Self::new();
        for e in entries {
            if host.modules.len() >= crate::world_api_gen::SCRIPT_MODULES_MAX {
                host.unloaded_entries.push(e.clone());
                continue;
            }
            match Module::load(e.name.clone(), e.prompt.clone(), e.source.clone()) {
                Ok(mut m) => {
                    m.enabled = e.enabled;
                    host.modules.push(m);
                }
                Err(err) => {
                    log::error!("Failed to reload module '{}': {err}", e.name);
                    host.unloaded_entries.push(e.clone());
                }
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
            if host.modules.len() >= crate::world_api_gen::SCRIPT_MODULES_MAX {
                log::warn!("Starter module limit reached");
                break;
            }
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
    pub fn add_generated(&mut self, module: Module) -> Result<usize, String> {
        if self.modules.len() >= crate::world_api_gen::SCRIPT_MODULES_MAX {
            return Err(format!(
                "World supports at most {} rule/spell modules; delete an unused module first",
                crate::world_api_gen::SCRIPT_MODULES_MAX
            ));
        }
        self.modules.push(module);
        Ok(self.modules.len() - 1)
    }

    /// Returns (name, new enabled state) so the caller can build a
    /// notification message, or `None` if the index no longer exists.
    pub fn toggle_at(&mut self, index: usize) -> Option<(String, bool)> {
        let m = self.modules.get_mut(index)?;
        m.enabled = !m.enabled;
        m.activation_epoch = m.activation_epoch.wrapping_add(1);
        self.scheduler.cancel(m.runtime_id);
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
        self.scheduler.cancel(m.runtime_id);
        log::info!("Module '{}' deleted", m.name);
        Some(m.name)
    }

    pub fn sync_attack_policies(&self, creatures: &mut Creatures) {
        creatures.attack_policies.retain(|&(id, epoch), _| self.modules.iter().any(|m|
            m.enabled && !m.is_instant && m.runtime_id == id && m.activation_epoch == epoch));
    }

    /// Enqueues ticks and player events, then fairly dispatches bounded work.
    /// Unstarted callbacks remain queued for subsequent host dispatches.
    /// Time/weather/creatures commit in place; queued effects are returned
    /// for the caller to apply before the next dispatch.
    #[allow(clippy::too_many_arguments)]
    pub fn run_tick(
        &mut self,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: &mut f32,
        weather: &mut WeatherState,
        block_breaks: &[BlockBreakEvent],
        interacts: &[InteractEvent],
        host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
    ) -> TickOutcome {
        self.sync_attack_policies(creatures);
        for death in creatures.combat_deaths.drain(..) {
            self.enqueue_combat_death(death);
        }
        self.enqueue_tick_events(block_breaks, interacts);
        self.dispatch_world(
            world,
            creatures,
            players,
            time_of_day,
            weather,
            host_resources,
        )
    }

    /// Enqueues one cast for its original recipient; it may run on a later dispatch.
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
        self.enqueue_cast(index, caster_id);
        self.last_cast_success = None;
        self.dispatch_world(
            world,
            creatures,
            players,
            time_of_day,
            weather,
            host_resources,
        )
    }

    /// Immediate Stage 1A casting. Re-resolves the supplied target before Lua and
    /// cancels unexecuted work so a refunded cast cannot execute on a later tick.
    #[allow(clippy::too_many_arguments)]
    pub fn run_targeted_cast(
        &mut self, index: usize, world: &World, creatures: &mut Creatures,
        players: &[PlayerSnapshot], time_of_day: &mut f32, weather: &mut WeatherState,
        caster_id: PlayerId, host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
        context: crate::spell_target::TargetContext,
    ) -> TickOutcome {
        self.last_cast_success = None;
        let validation = (|| {
            if !self.can_cast_immediately() { return Err("Rules are busy; try again".into()); }
            if !self.modules.get(index).is_some_and(|m|m.is_instant) {
                return Err("Select an instant spell".into());
            }
            let caster = players.iter().find(|p|p.id == caster_id).ok_or("Caster is not connected")?;
            if caster.health <= 0.0 { return Err("Defeated players cannot cast".into()); }
            context.validate(world, creatures, caster.pos + Vec3::Y * 1.62)
        })();
        let context = match validation {
            Ok(context) => context,
            Err(error) => return TickOutcome { warnings: vec![error], ..TickOutcome::default() },
        };
        self.enqueue_targeted_cast(index, caster_id, context);
        let mut outcome = self.dispatch_world(world, creatures, players, time_of_day, weather, host_resources);
        if !self.cast_succeeded(index, caster_id) {
            self.scheduler.cancel(self.modules[index].runtime_id);
            if outcome.crashes.is_empty() {
                outcome.warnings.push("Cast did not execute; try again".into());
            }
        }
        outcome
    }

    fn dispatch_world(
        &mut self,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: &mut f32,
        weather: &mut WeatherState,
        host_resources: [u32; COLLECTIBLE_BLOCKS.len()],
    ) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let mut deaths = Vec::new();
        let mut input = TickInput {
            creatures,
            world,
            players,
            time_of_day,
            weather,
            host_resources,
            block_edits: &mut outcome.block_edits,
            death_events: &mut deaths,
            broadcasts: &mut outcome.broadcasts,
            player_effects: &mut outcome.player_effects,
        };
        (outcome.crashes, outcome.warnings) = self.dispatch(&mut input);
        outcome
    }
    pub fn save_entries(&self) -> Vec<ModuleSaveEntry> {
        self.modules
            .iter()
            .map(Module::to_save_entry)
            .chain(self.unloaded_entries.iter().cloned())
            .collect()
    }
}

fn default_inventory_registry() -> std::sync::Arc<crate::crafting::Registry> {
    static REGISTRY: std::sync::OnceLock<std::sync::Arc<crate::crafting::Registry>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(||std::sync::Arc::new(crate::crafting::Registry::parse(include_str!("../data/crafting.json")).expect("embedded crafting registry"))).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{MAX_HEALTH, MAX_OXYGEN};
    use crate::voxel::World;

    /// `spawn_around` now picks each creature's kind from a weighted mix of
    /// every kind (see `creature.rs`'s `spawn_around`), so it no longer
    /// deterministically guarantees a sheep among a small count -- spawn one
    /// explicitly too, since several tests below need one to exist.
    fn make_creatures(seed: u32) -> Creatures {
        let world = World::new(seed);
        let mut creatures = Creatures::new();
        creatures.spawn_around(&world, Vec3::new(0.0, 0.0, 0.0), 6, seed);
        creatures.spawn_one(CreatureKind::Sheep, Vec3::new(0.0, 5.0, 0.0), seed as u64);
        creatures
    }

    fn targeted_fixture(source: &str) -> (ScriptHost, World, Creatures, PlayerSnapshot) {
        let mut host = ScriptHost::new();
        host.modules.push(Module::load("target test".into(), "manual".into(), source.into()).unwrap());
        let mut world = World::new(42);
        world.ensure_chunk_loaded(0, 0);
        for x in 0..16 { for z in 0..16 { for y in 65..75 {
            world.set_block(x, y, z, BlockType::Air);
        } } }
        (host, world, Creatures::new(), snapshot(0, Vec3::new(2.5, 68.38, 2.5), false))
    }

    fn targeted_run(host: &mut ScriptHost, world: &World, creatures: &mut Creatures,
        player: PlayerSnapshot, context: crate::spell_target::TargetContext) -> TickOutcome {
        host.run_targeted_cast(0, world, creatures, &[player], &mut 0.5,
            &mut WeatherState::new(42), player.id, NO_RESOURCES, context)
    }

    #[test]
    fn targeted_reference_heal_reuses_source_for_different_creatures_and_damage_works() {
        use crate::spell_target::TargetContext;
        let (mut host, world, mut creatures, player) = targeted_fixture(include_str!("../modules/target_heal.lua"));
        let a = creatures.spawn_one(CreatureKind::StoneGolem, Vec3::new(6.5,69.35,2.5), 1);
        let b = creatures.spawn_one(CreatureKind::StoneGolem, Vec3::new(2.5,69.35,6.5), 2);
        creatures.damage(a, 25.0); creatures.damage(b, 25.0);
        let before = creatures.snapshot_with_ids();
        for direction in [Vec3::X, Vec3::Z] {
            let context = TargetContext::resolve(&world, &creatures, player.pos + Vec3::Y*1.62, direction).unwrap();
            let out = targeted_run(&mut host,&world,&mut creatures,player,context);
            assert!(out.crashes.is_empty(), "{:?}", out.crashes);
            assert!(host.cast_succeeded(0,0));
        }
        for (id,_,_,health,_) in creatures.snapshot_with_ids() {
            assert_eq!(health, before.iter().find(|c|c.0==id).unwrap().3 + 20.0);
        }
        host.modules[0] = Module::load("damage".into(), "manual".into(), include_str!("../modules/target_damage.lua").into()).unwrap();
        let context = TargetContext::resolve(&world,&creatures,player.pos+Vec3::Y*1.62,Vec3::X).unwrap();
        targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(host.cast_succeeded(0,0));
        assert_eq!(creatures.snapshot_with_ids().iter().find(|c|c.0==a).unwrap().3,
            before.iter().find(|c|c.0==a).unwrap().3);
    }

    #[test]
    fn targeted_block_replacement_and_wrong_kind_are_transactional() {
        use crate::spell_target::TargetContext;
        let (mut host, mut world, mut creatures, player) = targeted_fixture(include_str!("../modules/target_stone.lua"));
        world.set_block(6,70,2,BlockType::Soil);
        let context = TargetContext::resolve(&world,&creatures,player.pos+Vec3::Y*1.62,Vec3::X).unwrap();
        assert_eq!(context.hit_normal, -Vec3::X);
        assert!((context.hit_position.x-6.0).abs()<0.001);
        let out = targeted_run(&mut host,&world,&mut creatures,player,context);
        assert_eq!(out.block_edits, [(6,70,2,BlockType::Stone)]);
        assert!(host.cast_succeeded(0,0));
        host.modules[0] = Module::load("heal".into(), "manual".into(), include_str!("../modules/target_heal.lua").into()).unwrap();
        let out = targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(!host.cast_succeeded(0,0));
        assert_eq!(out.crashes.len(),1);
        assert!(out.block_edits.is_empty());
        assert!(out.player_effects.is_empty());
    }

    #[test]
    fn targeted_stale_obstructed_moved_and_forged_casts_never_execute() {
        use crate::spell_target::{TargetContext, Target};
        let (mut host, mut world, mut creatures, player) = targeted_fixture("function on_cast(api,e) api.broadcast('executed') end");
        let id = creatures.spawn_one(CreatureKind::Sheep, Vec3::new(6.5,69.35,2.5), 1);
        let eye = player.pos + Vec3::Y*1.62;
        let context = TargetContext::resolve(&world,&creatures,eye,Vec3::X).unwrap();
        for bad in [TargetContext{target:Target::Creature{id:id+1},..context},
            TargetContext{origin:eye+Vec3::X,..context},
            TargetContext{facing:Vec3::splat(f32::NAN),..context},
            TargetContext{facing:Vec3::ZERO,..context}] {
            let out = targeted_run(&mut host,&world,&mut creatures,player,bad);
            assert!(!host.cast_succeeded(0,0)); assert!(out.broadcasts.is_empty());
        }
        world.set_block(4,70,2,BlockType::Stone);
        let out = targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(out.broadcasts.is_empty()); assert!(!host.cast_succeeded(0,0));
        world.set_block(4,70,2,BlockType::Air);
        creatures.damage(id,999.0);
        let out = targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(out.broadcasts.is_empty()); assert!(!host.cast_succeeded(0,0));
        assert!(TargetContext::resolve(&world,&creatures,eye,Vec3::ZERO).is_none());
        creatures.spawn_one(CreatureKind::Sheep,Vec3::new(24.5,69.35,2.5),2);
        assert!(TargetContext::resolve(&world,&creatures,eye,Vec3::X).is_none());
    }

    #[test]
    fn targeted_lua_error_rolls_back_damage_blocks_and_mana_commands() {
        use crate::spell_target::TargetContext;
        let source = "function on_cast(api,e) api.damage(e.target.id,20); api.replace_block(3,70,3,'stone'); api.give_mana(e.player_id,20); error('rollback') end";
        let (mut host, world, mut creatures, player) = targeted_fixture(source);
        creatures.spawn_one(CreatureKind::Sheep, Vec3::new(6.5,69.35,2.5), 1);
        let before = creatures.snapshot_with_ids();
        let context = TargetContext::resolve(&world,&creatures,player.pos+Vec3::Y*1.62,Vec3::X).unwrap();
        let out = targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(!host.cast_succeeded(0,0)); assert_eq!(out.crashes.len(),1);
        assert!(out.block_edits.is_empty()); assert!(out.player_effects.is_empty());
        assert_eq!(creatures.snapshot_with_ids(), before);
        let out = host.run_tick(&world,&mut creatures,&[player],&mut 0.5,&mut WeatherState::new(42),&[],&[],NO_RESOURCES);
        assert!(out.crashes.is_empty()); assert!(out.block_edits.is_empty());
        assert_eq!(creatures.snapshot_with_ids(),before);
    }

    #[test]
    fn targeted_context_recomputes_hit_and_rejects_changed_material_or_unloaded_visibility() {
        use crate::spell_target::TargetContext;
        let source = "function on_cast(api,e) assert(e.cast_id>0); assert(e.target.kind=='block'); assert(e.target.material=='soil'); assert(e.hit_position.x==6); assert(e.hit_normal.x==-1); assert(e.origin.x==2.5); assert(e.facing.x==1) end";
        let (mut host, mut world, mut creatures, player) = targeted_fixture(source);
        world.set_block(6,70,2,BlockType::Soil);
        let eye=player.pos+Vec3::Y*1.62;
        let mut context=TargetContext::resolve(&world,&creatures,eye,Vec3::X).unwrap();
        context.hit_position=Vec3::splat(f32::NAN);
        context.hit_normal=Vec3::splat(123.0);
        let out=targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(out.crashes.is_empty(),"{:?}",out.crashes);
        assert!(host.cast_succeeded(0,0));
        world.set_block(6,70,2,BlockType::Stone);
        let out=targeted_run(&mut host,&world,&mut creatures,player,context);
        assert!(!host.cast_succeeded(0,0)); assert!(!out.warnings.is_empty());
        world.set_block(6,70,2,BlockType::Air);
        creatures.spawn_one(CreatureKind::Sheep,Vec3::new(18.0,69.35,2.5),1);
        assert!(TargetContext::resolve(&world,&creatures,eye,Vec3::X).is_none());
        world.ensure_chunk_loaded(1,0);
        assert!(TargetContext::resolve(&world,&creatures,eye,Vec3::X).is_some());
    }

    #[test]
    fn spellbook_restored_definition_casts_without_replacing_or_saving_a_second_module() {
        use crate::spell_target::TargetContext;
        let (mut host,world,mut creatures,player)=targeted_fixture(include_str!("../modules/target_stone.lua"));
        let id=host.spellbook.remember(&host.modules[0],"Host").unwrap();
        host.spellbook=serde_json::from_slice(&serde_json::to_vec(&host.spellbook).unwrap()).unwrap();
        host.spellbook.revalidate();
        let original=host.save_entries();
        let mut world=world;world.set_block(6,70,2,BlockType::Soil);
        let context=TargetContext::resolve(&world,&creatures,player.pos+Vec3::Y*1.62,Vec3::X).unwrap();
        for _ in 0..2 {
            let index=host.add_generated(host.spellbook.get(id).unwrap().compiled().unwrap()).unwrap();
            let out=host.run_targeted_cast(index,&world,&mut creatures,&[player],&mut 0.5,
                &mut WeatherState::new(42),player.id,NO_RESOURCES,context);
            assert!(host.cast_succeeded(index,player.id));
            assert_eq!(out.block_edits,[(6,70,2,BlockType::Stone)]);
            host.remove(index);
        }
        assert_eq!(host.save_entries().len(),original.len());
        assert_eq!(host.save_entries()[0].source,original[0].source);
        assert_eq!(host.spellbook.spells.len(),1);assert_eq!(host.spellbook.spells[0].id,id);
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
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
            &[],
            NO_RESOURCES,
        );
        let crashes = outcome.crashes;

        assert!(
            !host.modules[0].enabled,
            "a module that errors at runtime should be auto-disabled"
        );
        assert_eq!(
            crashes.len(),
            1,
            "expected exactly one crash report: {crashes:?}"
        );
        assert!(
            crashes[0].contains("broken_helper"),
            "crash message should name the module: {}",
            crashes[0]
        );
    }

    #[test]
    fn sandbox_runaway_cases_finish_in_a_subprocess() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        for case in [
            "load",
            "load_pcall",
            "load_xpcall",
            "load_lookup",
            "tick",
            "death",
            "break",
            "interact",
            "cast",
            "memory",
            "api_calls",
            "native_work",
            "broadcasts",
            "broadcast_bytes",
        ] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "scripting::tests::sandbox_budget_subprocess_case",
                    "--nocapture",
                ])
                .env("VOXEL_SANDBOX_TEST_CASE", case)
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "sandbox case {case} failed: {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("sandbox case {case} exceeded external timeout");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn sandbox_budget_subprocess_case() {
        let Ok(case) = std::env::var("VOXEL_SANDBOX_TEST_CASE") else {
            return;
        };
        let infinite = "while true do end";
        let protected = "while true do pcall(function() while true do end end) end";
        let xprotected =
            "while true do xpcall(function() while true do end end, function(e) return e end) end";
        if case.starts_with("load") || case == "memory" {
            let body = match case.as_str() {
                "load" => infinite,
                "load_pcall" => protected,
                "load_xpcall" => xprotected,
                "load_lookup" => "setmetatable(_G, {__index = function() while true do end end})",
                "memory" => "local s = string.rep('x', 16777216)",
                _ => unreachable!(),
            };
            let source = if case == "load_lookup" {
                body.to_string()
            } else {
                format!("{body}\nfunction on_tick(api) end")
            };
            let result = Module::load("load_guard".into(), "test".into(), source);
            let error = result
                .err()
                .expect("invalid initialization should be rejected");
            if case == "load_lookup" {
                assert!(error.contains("does not define"), "{error}");
            } else {
                assert!(
                    error.contains("budget") || error.contains("memory"),
                    "{error}"
                );
            }
            return;
        }
        let (callback, body) = match case.as_str() {
            "tick" => ("on_tick", protected),
            "death" => ("on_death", xprotected),
            "break" => ("on_block_break", protected),
            "interact" => ("on_interact", xprotected),
            "cast" => ("on_cast", protected),
            "api_calls" => (
                "on_tick",
                "while true do pcall(function() api.distance(0,0,0,1,1,1) end) end",
            ),
            "native_work" => (
                "on_tick",
                "while true do pcall(function() api.find_blocks('unknown_kind',0,30,0,10) end) end",
            ),
            "broadcasts" => (
                "on_tick",
                "for i=1,9 do pcall(function() api.broadcast('hello') end) end",
            ),
            "broadcast_bytes" => (
                "on_tick",
                "pcall(function() api.broadcast(string.rep('x', 513)) end)",
            ),
            _ => panic!("unknown subprocess case"),
        };
        let mut source =
            format!("function {callback}(api, event) {body}; api.broadcast('escaped budget') end");
        if callback != "on_tick" && callback != "on_cast" {
            source.push_str("\nfunction on_tick(api) end");
        }
        let mut module = Module::load(case.clone(), "test".into(), source).unwrap();
        module.enabled = true;
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut weather = WeatherState::new(1);
        let mut time = 0.0;
        let mut blocks = Vec::new();
        let mut deaths = Vec::new();
        let mut broadcasts = Vec::new();
        let mut effects = Vec::new();
        let mut input = TickInput {
            creatures: &mut creatures,
            world: &world,
            players: &[],
            time_of_day: &mut time,
            weather: &mut weather,
            block_edits: &mut blocks,
            death_events: &mut deaths,
            broadcasts: &mut broadcasts,
            player_effects: &mut effects,
            host_resources: NO_RESOURCES,
        };
        match callback {
            "on_tick" => module.run_tick(&mut input),
            "on_cast" => {
                assert!(module.run_cast(&mut input, HOST_PLAYER_ID).is_err());
            }
            "on_death" => module.run_death(
                &mut input,
                &DeathEvent {
                    kind: CreatureKind::Sheep,
                    pos: Vec3::ZERO,
                },
            ),
            "on_block_break" => module.run_block_break(
                &mut input,
                &BlockBreakEvent {
                    x: 0,
                    y: 1,
                    z: 0,
                    block: BlockType::Stone,
                    player_id: HOST_PLAYER_ID,
                },
            ),
            "on_interact" => module.run_interact(
                &mut input,
                &InteractEvent {
                    x: 0,
                    y: 1,
                    z: 0,
                    block: BlockType::Stone,
                    player_id: HOST_PLAYER_ID,
                },
            ),
            _ => unreachable!(),
        }
        assert!(
            module.error.as_ref().is_some_and(|e| e.contains("budget")),
            "{:?}",
            module.error
        );
        if case == "api_calls" {
            assert!(module.error.as_ref().unwrap().contains("API call budget"));
        }
        if case.starts_with("broadcast") {
            assert!(module.error.as_ref().unwrap().contains("broadcast"));
        }
        if callback != "on_cast" {
            assert!(!module.enabled);
        }
        assert!(!broadcasts.iter().any(|s| s == "escaped budget"));
        assert!(broadcasts.len() <= MAX_BROADCASTS_PER_CALL as usize);
    }

    #[test]
    fn source_size_is_checked_before_compilation() {
        let source = format!(
            "function on_tick(api) end\n--{}",
            "x".repeat(MAX_SOURCE_BYTES)
        );
        let error = Module::load("oversized".into(), "test".into(), source)
            .err()
            .unwrap();
        assert!(error.contains("source exceeds"), "{error}");
    }

    #[test]
    fn documented_api_registry_matches_the_live_sandbox() {
        use crate::world_api_gen::{METHOD_NAMES, PROPERTY_NAMES};
        let mut source = String::from("function on_tick(api)\nlocal expected = {}\n");
        for name in METHOD_NAMES {
            source.push_str(&format!(
                "assert(type(api.{name}) == 'function'); expected.{name} = true\n"
            ));
        }
        for name in PROPERTY_NAMES {
            source.push_str(&format!(
                "assert(api.{name} ~= nil); expected.{name} = true\n"
            ));
        }
        source.push_str("for name in pairs(api) do assert(expected[name], name) end\nend");
        let mut module = Module::load("registry".into(), "test".into(), source).unwrap();
        module.enabled = true;
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut weather = WeatherState::new(1);
        run_one_tick(&mut module, &world, &mut creatures, &[], 0.0, &mut weather);
        assert!(module.enabled, "{:?}", module.error);
    }

    #[test]
    fn invalid_native_numbers_and_coordinates_are_rejected_before_mutation() {
        let mut creatures = Creatures::new();
        let world = World::new(1);
        let mut weather = WeatherState::new(1);
        for call in [
            "api.spawn_creature('sheep', 0/0, 0, 0)",
            "api.spawn_creature('sheep', '1e100', 0, 0)",
            "api.find_blocks('stone', 2147483647, 0, 0, 10)",
            "api.find_blocks('stone', 0, 0, 0, 0/0)",
            "api.set_time_of_day(1/0)",
        ] {
            let mut module = Module::load(
                "invalid_argument".into(),
                "test".into(),
                format!("function on_tick(api) {call} end"),
            )
            .unwrap();
            module.enabled = true;
            run_one_tick(&mut module, &world, &mut creatures, &[], 0.0, &mut weather);
            assert!(!module.enabled, "{call}");
            assert!(module.error.is_some(), "{call}");
            assert!(creatures.snapshot_with_ids().is_empty());
        }
    }

    #[test]
    fn ordinary_protected_errors_and_small_callbacks_still_work() {
        let source = "local ok = pcall(function() error('ordinary') end); assert(not ok)\nfunction on_tick(api) local ok = xpcall(function() error('ordinary') end, tostring); assert(not ok); api.broadcast('1e100') end";
        let mut module = Module::load("normal".into(), "test".into(), source.into()).unwrap();
        module.enabled = true;
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut weather = WeatherState::new(1);
        for _ in 0..20 {
            run_one_tick(&mut module, &world, &mut creatures, &[], 0.0, &mut weather);
            assert!(module.enabled, "{:?}", module.error);
        }
    }

    #[test]
    fn runaway_script_is_auto_disabled_by_the_time_budget() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        // Infinite loops are exercised in externally timed subprocesses.
        let source = "function on_tick(api) for i=1,1000000 do end end".to_string();
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
        let mut module =
            Module::load("creature_query_check".into(), "test".into(), source).unwrap();
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
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
            &[],
            NO_RESOURCES,
        );
        let edits = outcome.block_edits;

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected on_block_break to see kind/x/y/z/player_id: {edits:?}"
        );
    }

    #[test]
    fn on_interact_fires_with_the_correct_event_fields_and_never_changes_the_block() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_tick(api) end

            function on_interact(api, event)
                if event.kind == "crystal"
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
        let mut module = Module::load("interact_check".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let event = InteractEvent {
            x: 1,
            y: 2,
            z: 3,
            block: BlockType::Crystal,
            player_id: 7,
        };
        let outcome = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[],
            &[event],
            NO_RESOURCES,
        );
        let edits = outcome.block_edits;

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
        assert!(
            edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected on_interact to see kind/x/y/z/player_id: {edits:?}"
        );
    }

    #[test]
    fn on_interact_never_fires_from_a_block_break_and_vice_versa() {
        // The two events are collected and dispatched separately -- a
        // module that only defines one must never see the other's event.
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_tick(api) end
            function on_interact(api, event)
                api.replace_block(0, 0, 0, "redstone")
            end
        "#
        .to_string();
        let mut module = Module::load("interact_only".into(), "test".into(), source).unwrap();
        module.enabled = true;

        let mut host = ScriptHost::new();
        host.modules.push(module);

        let break_event = BlockBreakEvent {
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
            &[break_event],
            &[],
            NO_RESOURCES,
        );

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
        assert!(
            outcome.block_edits.is_empty(),
            "a block break must not fire on_interact: {:?}",
            outcome.block_edits
        );
    }

    #[test]
    fn storm_summoner_module_starts_rain_and_spawns_a_sheep_when_sprinting_player_breaks_crystal() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let target_pos = Vec3::new(4.0, 5.0, 4.0);
        let players = vec![PlayerSnapshot {
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
            &[],
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
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
            &[],
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
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &[grounded],
            0.5,
            &mut weather,
        );
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
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &[jumping],
            0.5,
            &mut weather,
        );
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
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()],
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
    fn undead_and_dragons_spawn_near_casting_player_through_world_api() {
        for kind in ["zombie","skeleton","dragon_green","dragon_red"] {
            let world=World::new(1);
            let mut creatures=Creatures::new();
            let players=vec![snapshot(5,Vec3::new(10.0,40.0,10.0),false)];
            let mut host=ScriptHost::new();
            let source=format!(r#"
                function on_cast(api,event)
                    local id=api.spawn_creature_near_player(event.player_id,"{kind}",6)
                    assert(id ~= nil)
                    local found=api.nearest_creature("{kind}",10,40,10)
                    assert(found.id==id and found.kind=="{kind}")
                    if found.can_fly then
                        assert(api.spawn_creature_near_player(event.player_id,"dragon_red",6)==nil)
                    end
                end
            "#);
            host.modules.push(Module::load("near_spawn".into(),String::new(),source).unwrap());
            let result=host.run_cast(0,&world,&mut creatures,&players,&mut 0.5,&mut WeatherState::new(1),5,[0;COLLECTIBLE_BLOCKS.len()]);
            assert!(result.crashes.is_empty(),"{kind}: {:?}",result.crashes);
            let spawned=creatures.snapshot_with_ids();
            assert_eq!(spawned.len(),1,"{kind}");
            assert_eq!(CreatureKind::from_u8(spawned[0].1),parse_creature_kind(kind));
            let pos=Vec3::from_array(spawned[0].2);
            assert!(Vec3::new(pos.x-players[0].pos.x,0.0,pos.z-players[0].pos.z).length()<=6.001);
            assert_eq!(pos.y,world.terrain_height(pos.x.floor() as i32,pos.z.floor() as i32) as f32+1.0);
        }
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
        run_one_tick(
            &mut module,
            &world,
            &mut creatures,
            &players,
            0.5,
            &mut weather,
        );
        assert!(module.error.is_none(), "module errored: {:?}", module.error);
        assert_eq!(weather.current, Weather::Rain);

        let mut module = Module::load(
            "stop_rain_check".into(),
            "test".into(),
            "function on_tick(api) api.stop_rain() end".to_string(),
        )
        .unwrap();
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
    fn reloading_a_module_from_its_save_entry_resets_runtime_lua_state_but_keeps_enabled_and_source(
    ) {
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
            run_one_tick(
                &mut module,
                &world,
                &mut creatures,
                &players,
                0.5,
                &mut weather,
            );
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
        assert!(
            reloaded.error.is_none(),
            "module errored: {:?}",
            reloaded.error
        );
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
        assert!(
            checked >= 5,
            "expected to find the known starter modules, found {checked}"
        );
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
            &[],
            NO_RESOURCES,
        );

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
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

        let source =
            "function on_cast(api, event) api.replace_block(0, 0, 0, \"redstone\") end".to_string();
        let module = Module::load("add_redstone".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        // A cast never fires from a regular tick -- only Run (run_cast).
        let tick_outcome = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[],
            &[],
            NO_RESOURCES,
        );
        assert!(
            tick_outcome.block_edits.is_empty(),
            "an instant spell's on_cast must never fire from run_tick: {:?}",
            tick_outcome.block_edits
        );

        for _ in 0..2 {
            let outcome = host.run_cast(
                0,
                &world,
                &mut creatures,
                &players,
                &mut time_of_day,
                &mut weather,
                0,
                NO_RESOURCES,
            );
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            7,
            NO_RESOURCES,
        );
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

        let killer_source = format!("function on_cast(api, event) api.destroy({sheep_id}) end");
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
        assert!(
            outcome
                .block_edits
                .iter()
                .any(|(_, _, _, b)| *b == BlockType::RedStone),
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

        let source =
            "function on_cast(api, event) api.give_item(event.player_id, \"stone\", 100) end"
                .to_string();
        let module = Module::load("give_stone".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            3,
            NO_RESOURCES,
        );
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
                local rejected = api.give_item(event.player_id, "water", 10)
                if clamped and not rejected then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let module = Module::load("give_edge_cases".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
        assert!(
            outcome.block_edits.iter().any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected the excessive amount to clamp (still true) and water to be rejected (false): {:?}",
            outcome.block_edits
        );
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::GiveItem {
                player_id: 0u32,
                block: BlockType::Stone,
                amount: MAX_ITEM_GRANT_AMOUNT
            }],
            "expected only the clamped stone grant, not the rejected water one"
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
        assert!(
            outcome
                .block_edits
                .iter()
                .any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected both calls against player 4 to succeed and the one against 999 to fail: {:?}",
            outcome.block_edits
        );
        assert_eq!(
            outcome.player_effects,
            vec![
                PlayerEffect::Health {
                    player_id: 4,
                    delta: -10.0
                },
                PlayerEffect::Health {
                    player_id: 4,
                    delta: 3.0
                },
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::Poisoned {
                player_id: 2,
                poisoned: true
            }]
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
        assert_eq!(
            outcome.player_effects,
            vec![
                PlayerEffect::SpeedMultiplier {
                    player_id: 1,
                    multiplier: MAX_ATTRIBUTE_MULTIPLIER
                },
                PlayerEffect::JumpMultiplier {
                    player_id: 1,
                    multiplier: MIN_ATTRIBUTE_MULTIPLIER
                },
            ]
        );
    }

    #[test]
    fn teleport_player_queues_a_destination_for_a_connected_player_and_fails_for_a_missing_one() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(1, Vec3::ZERO, false)];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;

        let source = r#"
            function on_cast(api, event)
                local ok = api.teleport_player(1, 10.0, 64.0, -5.0)
                local missing_ok = api.teleport_player(999, 0.0, 0.0, 0.0)
                if ok and not missing_ok then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let module = Module::load("teleport_check".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            0,
            NO_RESOURCES,
        );
        assert!(
            outcome
                .block_edits
                .iter()
                .any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected the call against player 1 to succeed and the one against 999 to fail: {:?}",
            outcome.block_edits
        );
        assert_eq!(
            outcome.player_effects,
            vec![PlayerEffect::Teleport {
                player_id: 1,
                pos: Vec3::new(10.0, 64.0, -5.0),
            }]
        );
    }

    #[test]
    fn resource_queries_distinguish_host_stock_from_empty_guest_stock() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![
            snapshot(HOST_PLAYER_ID, Vec3::ZERO, false),
            snapshot(9, Vec3::ZERO, false),
        ];
        let mut weather = WeatherState::new(1);
        let mut time_of_day = 0.5;
        let mut host_resources = NO_RESOURCES;
        let stone_index = COLLECTIBLE_BLOCKS
            .iter()
            .position(|&b| b == BlockType::Stone)
            .unwrap();
        host_resources[stone_index] = 5;

        let source = format!(
            r#"
            function on_cast(api, event)
                local host_count = api.get_resource_count({HOST_PLAYER_ID}, "stone")
                local remote_count = api.get_resource_count(9, "stone")
                local host_take_ok = api.take_item({HOST_PLAYER_ID}, "stone", 5)
                local remote_take_ok = api.take_item(9, "stone", 1)
                if host_count == 5 and remote_count == 0
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

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            HOST_PLAYER_ID,
            host_resources,
        );
        assert!(
            outcome
                .block_edits
                .iter()
                .any(|(_, _, _, b)| *b == BlockType::RedStone),
            "expected host count/removal and an empty guest inventory: {:?}",
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
        let stone_index = COLLECTIBLE_BLOCKS
            .iter()
            .position(|&b| b == BlockType::Stone)
            .unwrap();
        host_resources[stone_index] = 3;

        let source = format!(
            "function on_cast(api, event) if not api.take_item({HOST_PLAYER_ID}, \"stone\", 4) then api.replace_block(0, 0, 0, \"redstone\") end end"
        );
        let module = Module::load("take_too_much".into(), "test".into(), source).unwrap();
        let mut host = ScriptHost::new();
        host.modules.push(module);

        let outcome = host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            HOST_PLAYER_ID,
            host_resources,
        );
        assert!(
            outcome
                .block_edits
                .iter()
                .any(|(_, _, _, b)| *b == BlockType::RedStone),
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
            "expected find_creatures/nearest_creature to report kind \"stone_golem\" and the right max_health: {edits:?}"
        );
    }

    #[test]
    fn spawn_creature_and_spawn_creature_near_player_accept_the_wolf_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(0, Vec3::new(0.0, 5.0, 0.0), false)];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local a = api.spawn_creature("wolf", 5, 5, 5)
                local b = api.spawn_creature_near_player(0, "wolf", 3)
                if a ~= nil and b ~= nil then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("wolf_spawn_check".into(), "test".into(), source).unwrap();
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
            "expected both spawn calls to accept \"wolf\" and return an id: {edits:?}"
        );
        assert_eq!(
            creatures
                .snapshot_with_ids()
                .iter()
                .filter(|(_, kind, ..)| *kind == CreatureKind::Wolf.to_u8())
                .count(),
            2,
            "expected both spawned creatures to actually be wolves, not silently sheep"
        );
    }

    #[test]
    fn find_creatures_and_nearest_creature_report_the_wolf_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let wolf_id = creatures.spawn_one(CreatureKind::Wolf, Vec3::new(5.0, 5.0, 5.0), 1);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = format!(
            r#"
            function on_tick(api)
                local found = api.find_creatures("wolf", 5, 5, 5, 1)
                local nearest = api.nearest_creature("wolf", 5, 5, 5)
                if #found == 1 and found[1].id == {wolf_id} and found[1].kind == "wolf"
                    and found[1].max_health == 18.0
                    and nearest ~= nil and nearest.id == {wolf_id}
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module = Module::load("wolf_query_check".into(), "test".into(), source).unwrap();
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
            "expected find_creatures/nearest_creature to report kind \"wolf\" and the right max_health: {edits:?}"
        );
    }

    #[test]
    fn spawn_creature_and_spawn_creature_near_player_accept_the_stinger_and_cow_kinds() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(0, Vec3::new(0.0, 5.0, 0.0), false)];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local a = api.spawn_creature("stinger", 5, 5, 5)
                local b = api.spawn_creature_near_player(0, "stinger", 3)
                local c = api.spawn_creature("cow", 6, 5, 5)
                local d = api.spawn_creature_near_player(0, "cow", 3)
                if a ~= nil and b ~= nil and c ~= nil and d ~= nil then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module =
            Module::load("stinger_cow_spawn_check".into(), "test".into(), source).unwrap();
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
            "expected all four spawn calls to accept \"stinger\"/\"cow\" and return an id: {edits:?}"
        );
        let snapshot = creatures.snapshot_with_ids();
        assert_eq!(
            snapshot
                .iter()
                .filter(|(_, kind, ..)| *kind == CreatureKind::Stinger.to_u8())
                .count(),
            2,
            "expected both spawned creatures to actually be stingers, not silently sheep"
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|(_, kind, ..)| *kind == CreatureKind::Cow.to_u8())
                .count(),
            2,
            "expected both spawned creatures to actually be cows, not silently sheep"
        );
    }

    #[test]
    fn find_creatures_and_nearest_creature_report_the_stinger_and_cow_kinds() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let stinger_id = creatures.spawn_one(CreatureKind::Stinger, Vec3::new(5.0, 5.0, 5.0), 1);
        let cow_id = creatures.spawn_one(CreatureKind::Cow, Vec3::new(20.0, 5.0, 20.0), 2);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = format!(
            r#"
            function on_tick(api)
                local stinger = api.nearest_creature("stinger", 5, 5, 5)
                local cow = api.nearest_creature("cow", 20, 5, 20)
                if stinger ~= nil and stinger.id == {stinger_id} and stinger.kind == "stinger"
                    and stinger.max_health == 14.0
                    and cow ~= nil and cow.id == {cow_id} and cow.kind == "cow"
                    and cow.max_health == 20.0
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module =
            Module::load("stinger_cow_query_check".into(), "test".into(), source).unwrap();
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
            "expected nearest_creature to report kind \"stinger\"/\"cow\" and the right max_health: {edits:?}"
        );
    }

    #[test]
    fn spawn_creature_and_spawn_creature_near_player_accept_the_goblin_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(0, Vec3::new(0.0, 5.0, 0.0), false)];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local a = api.spawn_creature("goblin", 5, 5, 5)
                local b = api.spawn_creature_near_player(0, "goblin", 3)
                if a ~= nil and b ~= nil then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module = Module::load("goblin_spawn_check".into(), "test".into(), source).unwrap();
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
            "expected both spawn calls to accept \"goblin\" and return an id: {edits:?}"
        );
        assert_eq!(
            creatures
                .snapshot_with_ids()
                .iter()
                .filter(|(_, kind, ..)| *kind == CreatureKind::Goblin.to_u8())
                .count(),
            2,
            "expected both spawned creatures to actually be goblins, not silently sheep"
        );
    }

    #[test]
    fn find_creatures_and_nearest_creature_report_the_goblin_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let goblin_id = creatures.spawn_one(CreatureKind::Goblin, Vec3::new(5.0, 5.0, 5.0), 1);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = format!(
            r#"
            function on_tick(api)
                local found = api.find_creatures("goblin", 5, 5, 5, 1)
                local nearest = api.nearest_creature("goblin", 5, 5, 5)
                if #found == 1 and found[1].id == {goblin_id} and found[1].kind == "goblin"
                    and found[1].max_health == 24.0
                    and nearest ~= nil and nearest.id == {goblin_id}
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module = Module::load("goblin_query_check".into(), "test".into(), source).unwrap();
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
            "expected find_creatures/nearest_creature to report kind \"goblin\" and the right max_health: {edits:?}"
        );
    }

    #[test]
    fn spawn_creature_and_spawn_creature_near_player_accept_the_sunscorch_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let players = vec![snapshot(0, Vec3::new(0.0, 5.0, 0.0), false)];
        let mut weather = WeatherState::new(1);

        let source = r#"
            function on_tick(api)
                local a = api.spawn_creature("sunscorch", 5, 5, 5)
                local b = api.spawn_creature_near_player(0, "sunscorch", 3)
                if a ~= nil and b ~= nil then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        .to_string();
        let mut module =
            Module::load("sunscorch_spawn_check".into(), "test".into(), source).unwrap();
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
            "expected both spawn calls to accept \"sunscorch\" and return an id: {edits:?}"
        );
        assert_eq!(
            creatures
                .snapshot_with_ids()
                .iter()
                .filter(|(_, kind, ..)| *kind == CreatureKind::Sunscorch.to_u8())
                .count(),
            2,
            "expected both spawned creatures to actually be sunscorches, not silently sheep"
        );
    }

    #[test]
    fn find_creatures_and_nearest_creature_report_the_sunscorch_kind() {
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let sunscorch_id =
            creatures.spawn_one(CreatureKind::Sunscorch, Vec3::new(5.0, 5.0, 5.0), 1);
        let players: Vec<PlayerSnapshot> = Vec::new();
        let mut weather = WeatherState::new(1);

        let source = format!(
            r#"
            function on_tick(api)
                local found = api.find_creatures("sunscorch", 5, 5, 5, 1)
                local nearest = api.nearest_creature("sunscorch", 5, 5, 5)
                if #found == 1 and found[1].id == {sunscorch_id} and found[1].kind == "sunscorch"
                    and found[1].max_health == 28.0
                    and nearest ~= nil and nearest.id == {sunscorch_id}
                then
                    api.replace_block(0, 0, 0, "redstone")
                end
            end
        "#
        );
        let mut module =
            Module::load("sunscorch_query_check".into(), "test".into(), source).unwrap();
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
            "expected find_creatures/nearest_creature to report kind \"sunscorch\" and the right max_health: {edits:?}"
        );
    }
}

#[cfg(test)]
mod behavior_api_tests {
    use super::*;
    use crate::creature::{BehaviorMode, BehaviorTarget};

    #[test]
    fn dragon_rule_spawns_are_queryable_and_cannot_form_a_pack() {
        let mut creatures = Creatures::new();
        cast(&mut creatures, r#"
            local green = api.spawn_creature("dragon_green", 0, 40, 0)
            local red = api.spawn_creature("dragon_red", 400, 40, 0)
            local blocked = api.spawn_creature("dragon_red", 20, 40, 0)
            local found = api.find_creatures("dragon_green", 0, 40, 0, 1)
            if green ~= nil and red ~= nil and blocked == nil and #found == 1
                and found[1].kind == "dragon_green" and found[1].max_health == 240 then
                api.damage(green, 1)
                api.damage(red, 1)
            end
        "#);
        let snapshot = creatures.snapshot_with_ids();
        assert_eq!(snapshot.len(), 2);
        for (_, kind, _, health, _) in snapshot {
            assert!(CreatureKind::from_u8(kind).is_dragon());
            assert_eq!(health, 239.0);
        }
    }

    #[test]
    fn undead_spawn_and_query_through_the_rule_api() {
        let mut creatures = Creatures::new();
        cast(&mut creatures, r#"
            for _, kind in ipairs({"zombie", "skeleton"}) do
                local id = api.spawn_creature(kind, 5, 5, 5)
                local found = api.find_creatures(kind, 5, 5, 5, 1)
                if #found == 1 and found[1].id == id and found[1].kind == kind then
                    api.damage(id, 1)
                end
            end
        "#);
        let snapshot = creatures.snapshot_with_ids();
        assert_eq!(snapshot.len(), 2);
        for (_, kind, _, health, max_health) in snapshot {
            assert!(matches!(CreatureKind::from_u8(kind), CreatureKind::Zombie | CreatureKind::Skeleton));
            assert_eq!(health, max_health - 1.0);
        }
    }

    fn cast(creatures: &mut Creatures, code: &str) -> TickOutcome {
        let mut host = ScriptHost::new();
        host.modules.push(Module::load("behavior_test".into(), String::new(),
            format!("function on_cast(api, event) {code} end")).unwrap());
        host.run_cast(0, &World::new(1), creatures, &[], &mut 0.5,
            &mut WeatherState::new(1), 0, [0; COLLECTIBLE_BLOCKS.len()])
    }
    fn pair() -> (World, Creatures, u32, u32, Vec3) {
        let world = World::new(1);
        let pos = Vec3::new(0.5, world.terrain_height(0,0) as f32 + 1.0, 0.5);
        let mut creatures = Creatures::new();
        let a = creatures.spawn_one(CreatureKind::Sheep, pos, 1);
        let b = creatures.spawn_one(CreatureKind::Chicken, pos, 2);
        (world, creatures, a, b, pos)
    }
    #[test]
    fn selected_species_receives_real_damage_on_cooldown_and_death_event() {
        let (world, mut creatures, a, b, _) = pair();
        let result = cast(&mut creatures, &format!(
            "assert(api.select_target({a}, 'chicken', 32) == {b}); assert(api.attack({a})); local s=api.get_behavior({a}); assert(s.mode=='attack' and s.target_id=={b} and s.target_type=='creature')"));
        assert!(result.crashes.is_empty(), "{:?}", result.crashes);
        assert!(creatures.update(&world, 0.0, &[]).is_empty());
        assert_eq!(creatures.snapshot_with_ids().iter().find(|c| c.0==b).unwrap().3, 4.0);
        creatures.update(&world, 0.0, &[]);
        assert_eq!(creatures.snapshot_with_ids().iter().find(|c| c.0==b).unwrap().3, 4.0);
        creatures.update(&world, 2.0, &[]);
        creatures.update(&world, 2.0, &[]);
        assert!(!creatures.snapshot_with_ids().iter().any(|c| c.0==b));
        let mut host = ScriptHost::new();
        let mut rule=Module::load("death_observer".into(), String::new(),
            "function on_tick(api) end function on_death(api,e) api.broadcast(e.kind) end".into()).unwrap();
        rule.enabled=true; host.modules.push(rule);
        let out=host.run_tick(&world, &mut creatures, &[], &mut 0.5, &mut WeatherState::new(1), &[], &[], [0; COLLECTIBLE_BLOCKS.len()]);
        assert_eq!(out.broadcasts, ["chicken"]);
    }
    #[test]
    fn chase_does_not_attack_and_ignore_overrides_hostile_ai() {
        let (world, mut creatures, _, b, pos) = pair();
        let wolf = creatures.spawn_one(CreatureKind::Wolf, pos, 3);
        let out=cast(&mut creatures, &format!("assert(api.set_target({wolf}, 'creature', {b})); assert(api.chase_target({wolf}))"));
        assert!(out.crashes.is_empty());
        assert!(creatures.update(&world, 0.0, &[(7,pos)]).is_empty());
        assert_eq!(creatures.snapshot_with_ids().iter().find(|c| c.0==b).unwrap().3, 6.0);
        assert!(cast(&mut creatures, &format!("api.ignore({wolf})")).crashes.is_empty());
        assert!(creatures.update(&world, 0.0, &[(7,pos)]).is_empty());
        // Existing coordinate chase can still be used after ignore.
        cast(&mut creatures, &format!("api.chase({wolf}, 5, {}, 5)",pos.y));
        assert!(creatures.any_hunting());
        cast(&mut creatures, &format!("api.set_aggressive({wolf}, true)"));
        assert!(!creatures.update(&world, 0.0, &[(7,pos)]).is_empty());
    }
    #[test]
    fn behavior_drafts_rollback_and_validate_targets() {
        let (_, mut creatures, a, b, _) = pair();
        let out=cast(&mut creatures, &format!("api.set_target({a}, 'creature', {b}); api.attack({a}); api.die({b}); error('rollback')"));
        assert!(!out.crashes.is_empty());
        assert_eq!(creatures.behaviors[&a].mode, BehaviorMode::Auto);
        assert!(creatures.snapshot_with_ids().iter().any(|c| c.0==b));
        let out=cast(&mut creatures, &format!("assert(not api.set_target({a}, 'creature', {a})); assert(not api.attack({a})); assert(api.select_target({a}, 'wolf', 32)==nil); assert(api.get_behavior(99999)==nil); assert(api.die({b})); assert(not api.die({b}))"));
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        assert!(!creatures.snapshot_with_ids().iter().any(|c| c.0==b));
    }
    #[test]
    fn explicit_player_target_damages_selected_guest_and_stale_targets_clear() {
        let (world, mut creatures, a, _, pos) = pair();
        let mut host=ScriptHost::new();
        host.modules.push(Module::load("guest_target".into(), String::new(), format!(
            "function on_cast(api,e) assert(api.set_target({a}, 'player', 7)); assert(api.attack({a})) end"
        )).unwrap());
        let players=[PlayerSnapshot {
                finances: Default::default(),
                resources: [0; COLLECTIBLE_BLOCKS.len()], id: 7, pos, carrying_crystal: false, health: 20.0,
            sprinting: false, on_ground: true, velocity: Vec3::ZERO, in_water: false, poisoned: false,
            speed_multiplier: 1.0, jump_multiplier: 1.0, oxygen: 100.0 }];
        let out=host.run_cast(0,&world,&mut creatures,&players,&mut 0.5,&mut WeatherState::new(1),0,[0;COLLECTIBLE_BLOCKS.len()]);
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        assert_eq!(creatures.update(&world,0.0,&[(0,pos),(7,pos)]),[(7,2.0)]);
        creatures.update(&world,0.0,&[(0,pos)]);
        assert_eq!(creatures.behaviors[&a].target,None);
        assert!(creatures.update(&world,3.0,&[(0,pos)]).is_empty());
    }

    #[test]
    fn temporary_policies_compose_and_release_without_mutating_behavior() {
        let (world, mut creatures, _, prey, pos)=pair();
        let wolf=creatures.spawn_one(CreatureKind::Wolf,pos,3);
        let mut host=ScriptHost::new();
        for n in 0..2 {
            let mut m=Module::load(format!("protection{n}"),String::new(),
                "function on_tick(api) if api.weather=='rain' then api.protect_player(7,'wolf') end end".into()).unwrap();
            m.enabled=true;host.modules.push(m);
        }
        let players=[PlayerSnapshot { finances: Default::default(), resources:[0;COLLECTIBLE_BLOCKS.len()], id:7,pos,carrying_crystal:false,velocity:Vec3::ZERO,
            on_ground:true,sprinting:false,in_water:false,health:20.0,poisoned:false,speed_multiplier:1.0,jump_multiplier:1.0,oxygen:100.0 }];
        let mut weather=WeatherState::new(1);weather.set(Weather::Rain);
        let out=host.run_tick(&world,&mut creatures,&players,&mut 0.25,&mut weather,&[],&[],[0;COLLECTIBLE_BLOCKS.len()]);
        assert!(out.crashes.is_empty());assert_eq!(creatures.attack_policies.len(),2);
        assert!(creatures.update(&world,0.0,&[(7,pos)]).is_empty());
        assert!(creatures.behaviors[&wolf].aggressive);
        host.toggle_at(0);host.sync_attack_policies(&mut creatures);
        assert_eq!(creatures.attack_policies.len(),1);
        assert!(creatures.update(&world,0.0,&[(7,pos)]).is_empty());
        // Protection of a player must not pacify attacks against other creatures.
        creatures.behaviors.get_mut(&wolf).unwrap().mode=BehaviorMode::Attack;
        creatures.behaviors.get_mut(&wolf).unwrap().target=Some(BehaviorTarget::Creature(prey));
        creatures.update(&world,0.0,&[(7,pos)]);
        assert!(creatures.snapshot_with_ids().iter().find(|c|c.0==prey).map_or(true,|c|c.3<6.0));
        weather.set(Weather::Sunny);
        host.run_tick(&world,&mut creatures,&players,&mut 0.25,&mut weather,&[],&[],[0;COLLECTIBLE_BLOCKS.len()]);
        assert!(creatures.attack_policies.is_empty());
        assert_eq!(creatures.behaviors[&wolf].mode,BehaviorMode::Attack);
        assert_eq!(creatures.behaviors[&wolf].target,Some(BehaviorTarget::Creature(prey)));
    }

    #[test]
    fn failed_policy_callback_releases_previous_protection() {
        let (world,mut creatures,_,_,pos)=pair();
        let mut host=ScriptHost::new();
        let mut m=Module::load("fails".into(),String::new(),
            "local n=0 function on_tick(api) n=n+1 api.protect_player(7,'wolf') if n>1 then error('failed') end end".into()).unwrap();
        m.enabled=true;host.modules.push(m);
        let players=[PlayerSnapshot { finances: Default::default(), resources:[0;COLLECTIBLE_BLOCKS.len()],id:7,pos,carrying_crystal:false,velocity:Vec3::ZERO,
            on_ground:true,sprinting:false,in_water:false,health:20.0,poisoned:false,speed_multiplier:1.0,jump_multiplier:1.0,oxygen:100.0 }];
        for n in 0..2 {
            host.run_tick(&world,&mut creatures,&players,&mut 0.25,&mut WeatherState::new(1),&[],&[],[0;COLLECTIBLE_BLOCKS.len()]);
            assert_eq!(creatures.attack_policies.is_empty(),n==1);
        }
    }

    #[test]
    fn behavior_save_roundtrip_preserves_selected_target_and_mode() {
        let (_, mut creatures, a, b, _) = pair();
        cast(&mut creatures, &format!("api.set_target({a}, 'creature', {b}); api.attack({a})"));
        let save=crate::save::CraftingSave { creatures: Some(creatures.snapshot_with_ids()), behaviors: creatures.behaviors.clone(), ..Default::default() };
        let saved: crate::save::CraftingSave=serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
        let mut restored=Creatures::new();
        restored.restore_saved(saved.creatures.as_ref().unwrap(), 1);
        restored.behaviors=saved.behaviors;
        assert_eq!(restored.behaviors[&a].mode, BehaviorMode::Attack);
        assert_eq!(restored.behaviors[&a].target, Some(BehaviorTarget::Creature(b)));
        let old: crate::save::CraftingSave=serde_json::from_str("{}").unwrap();
        assert!(old.behaviors.is_empty());
    }
}
