use std::cell::{Cell, RefCell};
use std::fs;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glam::Vec3;
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table};
use serde::{Deserialize, Serialize};

use crate::creature::{CreatureKind, Creatures, DeathEvent};
use crate::daynight::is_night;
use crate::net::PlayerId;
use crate::voxel::block::BlockType;
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
        if !has_on_tick {
            return Err("module does not define a global `on_tick(api)` function".to_string());
        }

        let seed_hash = name.bytes().fold(0x1234_5678_9abc_def0u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });

        Ok(Module {
            name,
            prompt,
            source,
            enabled: false,
            error: None,
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

/// Maps a Lua-facing kind string to the internal `u8` kind code used by
/// `find_creatures`/`nearest_creature`. `None` means "any kind" (both the
/// filter argument itself, when it's "any", and an unrecognized string fall
/// through to matching everything rather than erroring).
fn creature_kind_filter(kind: &str) -> Option<u8> {
    match kind.to_ascii_lowercase().as_str() {
        "sheep" => Some(0),
        "chicken" => Some(1),
        _ => None,
    }
}

/// Horizontal-only speed (excludes fall/jump velocity), which is what
/// "speed" intuitively means for a rule checking how fast a player is
/// moving across the ground.
fn horizontal_speed(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
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
                c.set("kind", if *kind == 0 { "sheep" } else { "chicken" })?;
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
                    e.set("kind", if *k == 0 { "sheep" } else { "chicken" })?;
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
                e.set("kind", if *k == 0 { "sheep" } else { "chicken" })?;
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
            let kind = if kind.eq_ignore_ascii_case("chicken") {
                CreatureKind::Chicken
            } else {
                CreatureKind::Sheep
            };
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
                let kind = if kind.eq_ignore_ascii_case("chicken") {
                    CreatureKind::Chicken
                } else {
                    CreatureKind::Sheep
                };
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
            Ok(world.get_block(x, y, z).name().to_ascii_lowercase())
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
            weather_cell.borrow_mut().set(Weather::Clear);
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
        scope.create_function(|_, msg: String| {
            log::info!("[rule] {msg}");
            Ok(())
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
    let weather_cell = RefCell::new(&mut *input.weather);
    let time_cell = RefCell::new(&mut *input.time_of_day);
    let block_budget = Cell::new(MAX_BLOCK_EDITS_PER_CALL);
    let spawn_budget = Cell::new(MAX_SPAWNS_PER_CALL);
    let world = input.world;
    let kind_name = if event.kind.to_u8() == 0 {
        "sheep"
    } else {
        "chicken"
    };
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
    let weather_cell = RefCell::new(&mut *input.weather);
    let time_cell = RefCell::new(&mut *input.time_of_day);
    let block_budget = Cell::new(MAX_BLOCK_EDITS_PER_CALL);
    let spawn_budget = Cell::new(MAX_SPAWNS_PER_CALL);
    let world = input.world;
    let kind_name = event.block.name().to_ascii_lowercase();
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
    /// above caused. Returns the block edits requested this tick, for the
    /// caller to apply through the normal replicated path, plus one message
    /// per module that crashed and got auto-disabled *this call* (a rule
    /// can pass load-time validation -- valid syntax, defines `on_tick` --
    /// and still hit a runtime error the first time it actually executes,
    /// e.g. calling an undefined helper function; the caller surfaces these
    /// so a rule going silently dark isn't mistaken for "nothing happened").
    /// `time_of_day` is mutated in place if a rule calls
    /// `set_time_of_day`/`set_time_dawn`/`set_time_night`, the same way
    /// `weather` already is for the weather setters.
    pub fn run_tick(
        &mut self,
        world: &World,
        creatures: &mut Creatures,
        players: &[PlayerSnapshot],
        time_of_day: &mut f32,
        weather: &mut WeatherState,
        block_breaks: &[BlockBreakEvent],
    ) -> (Vec<(i32, i32, i32, BlockType)>, Vec<String>) {
        let mut block_edits = Vec::new();
        let mut death_events = Vec::new();
        let mut crashes = Vec::new();

        for module in &mut self.modules {
            let was_enabled = module.enabled;
            let mut input = TickInput {
                creatures: &mut *creatures,
                world,
                players,
                time_of_day: &mut *time_of_day,
                weather: &mut *weather,
                block_edits: &mut block_edits,
                death_events: &mut death_events,
            };
            module.run_tick(&mut input);
            if was_enabled && !module.enabled {
                crashes.push(crash_message(module));
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
                    block_edits: &mut block_edits,
                    death_events: &mut death_events,
                };
                module.run_block_break(&mut input, event);
                if was_enabled && !module.enabled {
                    crashes.push(crash_message(module));
                }
            }
        }

        for event in &death_events {
            for module in &mut self.modules {
                let was_enabled = module.enabled;
                let mut chained = Vec::new();
                let mut input = TickInput {
                    creatures: &mut *creatures,
                    world,
                    players,
                    time_of_day: &mut *time_of_day,
                    weather: &mut *weather,
                    block_edits: &mut block_edits,
                    death_events: &mut chained,
                };
                module.run_death(&mut input, event);
                if was_enabled && !module.enabled {
                    crashes.push(crash_message(module));
                }
            }
        }

        (block_edits, crashes)
    }

    pub fn save_entries(&self) -> Vec<ModuleSaveEntry> {
        self.modules.iter().map(Module::to_save_entry).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::World;

    fn make_creatures(seed: u32) -> Creatures {
        let world = World::new(seed);
        let mut creatures = Creatures::new();
        creatures.spawn_around(&world, Vec3::new(0.0, 0.0, 0.0), 6, seed);
        creatures
    }

    /// A player snapshot with the new movement/environment fields defaulted
    /// to "standing still on dry ground" -- most tests only care about
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
        let mut tod = time_of_day;
        let mut input = TickInput {
            creatures,
            world,
            players,
            time_of_day: &mut tod,
            weather,
            block_edits: &mut block_edits,
            death_events: &mut death_events,
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
        let mut tod = time_of_day;
        let mut input = TickInput {
            creatures,
            world,
            players,
            time_of_day: &mut tod,
            weather,
            block_edits: &mut block_edits,
            death_events: &mut death_events,
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
            let (edits, _crashes) = host.run_tick(
                &world,
                &mut creatures,
                &players,
                &mut time_of_day,
                &mut weather,
                &[],
            );
            all_edits.extend(edits);
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

        let (_edits, crashes) = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[],
        );

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
        let (edits, _crashes) = host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time_of_day,
            &mut weather,
            &[event],
        );

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
        }];
        let mut weather = WeatherState::new(1);
        assert_eq!(weather.current, Weather::Clear);
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
        );

        assert!(
            host.modules[0].error.is_none(),
            "module errored: {:?}",
            host.modules[0].error
        );
        assert_eq!(
            weather.current,
            Weather::Clear,
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
        };
        run_one_tick(&mut module, &world, &mut creatures, &[grounded], 0.5, &mut weather);
        assert!(
            module.error.is_none(),
            "module errored on grounded tick: {:?}",
            module.error
        );
        assert_eq!(
            weather.current,
            Weather::Clear,
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
            Weather::Clear,
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
        assert_eq!(weather.current, Weather::Clear);
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
        assert_eq!(weather.current, Weather::Clear);
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
}
