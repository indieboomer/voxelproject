use super::transaction::CallbackTransaction;
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Callback<'a> {
    Tick,
    Cast(PlayerId),
    Death(&'a DeathEvent),
    BlockBreak(&'a BlockBreakEvent),
    Interact(&'a InteractEvent),
}

impl Callback<'_> {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tick => "on_tick",
            Self::Cast(_) => "on_cast",
            Self::Death(_) => "on_death",
            Self::BlockBreak(_) => "on_block_break",
            Self::Interact(_) => "on_interact",
        }
    }
}

pub(super) fn call(lua: &Lua, tx: &CallbackTransaction, event: Callback) -> mlua::Result<()> {
    let Some(function) = lua.globals().raw_get::<_, Option<Function>>(event.name())? else {
        return Ok(());
    };
    let instant = matches!(event, Callback::Cast(_));
    let block_budget = Cell::new(if instant {
        MAX_BLOCK_EDITS_PER_CAST
    } else {
        MAX_BLOCK_EDITS_PER_CALL
    });
    let spawn_budget = Cell::new(if instant {
        MAX_SPAWNS_PER_CAST
    } else {
        MAX_SPAWNS_PER_CALL
    });
    lua.scope(|scope| {
        let api = lua.create_table()?;
        populate_api(lua, scope, &api, tx, &block_budget, &spawn_budget)?;
        if matches!(event, Callback::Tick) {
            return function.call::<_, ()>(api);
        }
        let table = lua.create_table()?;
        match event {
            Callback::Cast(player_id) => table.set("player_id", player_id)?,
            Callback::Death(event) => {
                table.set("kind", creature_kind_name(event.kind.to_u8()))?;
                table.set("x", event.pos.x)?;
                table.set("y", event.pos.y)?;
                table.set("z", event.pos.z)?;
            }
            Callback::BlockBreak(event) => {
                table.set("kind", event.block.id())?;
                table.set("x", event.x)?;
                table.set("y", event.y)?;
                table.set("z", event.z)?;
                table.set("player_id", event.player_id)?;
            }
            Callback::Interact(event) => {
                table.set("kind", event.block.id())?;
                table.set("x", event.x)?;
                table.set("y", event.y)?;
                table.set("z", event.z)?;
                table.set("player_id", event.player_id)?;
            }
            Callback::Tick => unreachable!(),
        }
        function.call::<_, ()>((api, table))
    })
}

fn populate_api<'lua, 'scope>(
    lua: &'lua Lua,
    scope: &mlua::Scope<'lua, 'scope>,
    api: &Table<'lua>,
    tx: &'scope CallbackTransaction,
    block_budget: &'scope Cell<u32>,
    spawn_budget: &'scope Cell<u32>,
) -> mlua::Result<()> {
    let world = tx.world;
    let creatures_cell = &tx.creatures;
    let block_edits_cell = &tx.blocks;
    let death_events_cell = &tx.deaths;
    let broadcasts_cell = &tx.broadcasts;
    let player_effects_cell = &tx.effects;
    let weather_cell = &tx.weather;
    let time_cell = &tx.time;
    let spawn_seed = &tx.spawn_seed;
    let time_of_day = *tx.time.borrow();
    let night = is_night(time_of_day);
    let weather_name = tx.weather.borrow().current.name();
    api.set("time_of_day", time_of_day)?;
    api.set("is_night", night)?;
    api.set("weather", weather_name)?;

    api.set(
        "players",
        scope.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            for (i, p) in tx.players().iter().enumerate() {
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
            for (i, (id, kind, pos, health, max_health)) in
                creatures_cell.borrow().snapshot.iter().enumerate()
            {
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
                for (id, k, pos, health, max_health) in creatures_cell.borrow().snapshot.iter() {
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
        scope.create_function(move |lua, (kind, x, y, z): (String, f32, f32, f32)| {
            let kind_filter = creature_kind_filter(&kind);
            let center = Vec3::new(x, y, z);
            let creatures = creatures_cell.borrow();
            let nearest = creatures
                .snapshot
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
        })?,
    )?;

    api.set(
        "nearest_player",
        scope.create_function(move |lua, (x, y, z): (f32, f32, f32)| {
            let center = Vec3::new(x, y, z);
            let players = tx.players();
            let nearest = players
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
            creatures_cell.borrow_mut().chase(id, Vec3::new(x, y, z));
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
                .spawn(kind, Vec3::new(x, y, z), seed);
            Ok(id)
        })?,
    )?;

    api.set(
        "spawn_creature_near_player",
        scope.create_function(move |_, (player_id, kind, radius): (u32, String, f32)| {
            if spawn_budget.get() == 0 {
                return Ok(None);
            }
            let Some(target) = tx.players().into_iter().find(|p| p.id == player_id) else {
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
                .spawn(kind, Vec3::new(sx, sy, sz), seed);
            Ok(id)
        })?,
    )?;

    api.set(
        "get_block",
        scope.create_function(move |_, (x, y, z): (i32, i32, i32)| {
            Ok(tx.get_block(x, y, z).id().to_string())
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
                let is_match: Box<dyn Fn(BlockType) -> bool> = if kind.eq_ignore_ascii_case("wood")
                {
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
                        lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap().check();
                        for dz in -r..=r {
                            if count >= MAX_FIND_RESULTS {
                                break 'search;
                            }
                            if (dx * dx + dy * dy + dz * dz) as f32 > radius_sq {
                                continue;
                            }
                            let (x, y, z) = (cx + dx, cy + dy, cz + dz);
                            if is_match(tx.get_block(x, y, z)) {
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
            if !(0..crate::voxel::chunk::CHUNK_Y).contains(&y) {
                return Ok(false);
            }
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
            *time_cell.borrow_mut() = t.rem_euclid(1.0);
            Ok(())
        })?,
    )?;

    api.set(
        "set_time_dawn",
        scope.create_function(move |_, ()| {
            *time_cell.borrow_mut() = 0.0;
            Ok(())
        })?,
    )?;

    api.set(
        "set_time_night",
        scope.create_function(move |_, ()| {
            *time_cell.borrow_mut() = 0.5;
            Ok(())
        })?,
    )?;

    let broadcast_count = Cell::new(0);
    api.set(
        "broadcast",
        scope.create_function(move |lua, msg: String| {
            if broadcast_count.get() >= MAX_BROADCASTS_PER_CALL || msg.len() > MAX_BROADCAST_BYTES {
                lua.app_data_ref::<Rc<ExecutionBudget>>()
                    .unwrap()
                    .exhaust("script exceeded its broadcast count or message size budget");
            }
            broadcast_count.set(broadcast_count.get() + 1);

            broadcasts_cell.borrow_mut().push(msg);
            Ok(())
        })?,
    )?;

    api.set(
        "give_item",
        scope.create_function(
            move |_, (player_id, kind, amount): (PlayerId, String, u32)| {
                if !tx.players().iter().any(|p| p.id == player_id) {
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
                    .push(PlayerEffect::GiveItem {
                        player_id,
                        block,
                        amount,
                    });
                Ok(true)
            },
        )?,
    )?;

    api.set(
        "take_item",
        scope.create_function(
            move |_, (player_id, kind, amount): (PlayerId, String, u32)| {
                // The transaction includes prior accepted give/take
                // commands, so two removals cannot spend the same funds.
                // Remote inventories are still not tracked on the host.
                if player_id != HOST_PLAYER_ID {
                    return Ok(false);
                }
                let Some(block) = BlockType::from_name(&kind) else {
                    return Ok(false);
                };
                let Some(i) = COLLECTIBLE_BLOCKS.iter().position(|&b| b == block) else {
                    return Ok(false);
                };
                if tx.resource_count(i) < amount {
                    return Ok(false);
                }
                player_effects_cell
                    .borrow_mut()
                    .push(PlayerEffect::TakeItem {
                        player_id,
                        block,
                        amount,
                    });
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
            Ok(Some(tx.resource_count(i)))
        })?,
    )?;

    api.set(
        "damage_player",
        scope.create_function(move |_, (player_id, amount): (PlayerId, f32)| {
            if !tx.players().iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell.borrow_mut().push(PlayerEffect::Health {
                player_id,
                delta: -amount,
            });
            Ok(true)
        })?,
    )?;

    api.set(
        "heal_player",
        scope.create_function(move |_, (player_id, amount): (PlayerId, f32)| {
            if !tx.players().iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell.borrow_mut().push(PlayerEffect::Health {
                player_id,
                delta: amount,
            });
            Ok(true)
        })?,
    )?;

    api.set(
        "set_poisoned",
        scope.create_function(move |_, (player_id, poisoned): (PlayerId, bool)| {
            if !tx.players().iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::Poisoned {
                    player_id,
                    poisoned,
                });
            Ok(true)
        })?,
    )?;

    api.set(
        "set_player_speed",
        scope.create_function(move |_, (player_id, multiplier): (PlayerId, f32)| {
            if !tx.players().iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            let multiplier = multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::SpeedMultiplier {
                    player_id,
                    multiplier,
                });
            Ok(true)
        })?,
    )?;

    api.set(
        "set_player_jump",
        scope.create_function(move |_, (player_id, multiplier): (PlayerId, f32)| {
            if !tx.players().iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            let multiplier = multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::JumpMultiplier {
                    player_id,
                    multiplier,
                });
            Ok(true)
        })?,
    )?;

    api.set(
        "teleport_player",
        scope.create_function(move |_, (player_id, x, y, z): (PlayerId, f32, f32, f32)| {
            if !tx.players().iter().any(|p| p.id == player_id) {
                return Ok(false);
            }
            player_effects_cell
                .borrow_mut()
                .push(PlayerEffect::Teleport {
                    player_id,
                    pos: Vec3::new(x, y, z),
                });
            Ok(true)
        })?,
    )?;

    // Guard the actual registered functions, including future additions.
    // Scoped wrappers preserve the API callbacks' borrowed world lifetimes.
    let methods = api
        .clone()
        .pairs::<String, Value>()
        .collect::<mlua::Result<Vec<_>>>()?;
    for (name, value) in methods {
        let Value::Function(function) = value else {
            continue;
        };
        let budget = lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap().clone();
        let method_name = name.clone();
        let properties = api.clone();
        let numeric_args = crate::world_api_gen::NUMERIC_ARGUMENTS
            .iter()
            .find(|(method, _)| *method == name)
            .map_or(&[][..], |(_, args)| *args);
        let coordinate_args = crate::world_api_gen::COORDINATE_ARGUMENTS
            .iter()
            .find(|(method, _)| *method == name)
            .map_or(&[][..], |(_, args)| *args);
        api.set(name, scope.create_function(move |lua, args: MultiValue| {
            budget.charge_api_call();
            for (index, value) in args.iter().enumerate() {
                if !numeric_args.contains(&index) { continue; }
                // mlua accepts numeric strings too; check the converted
                // value so "nan"/"1e100" cannot bypass f32 validation.
                if let Some(number) = lua.coerce_number(value.clone())? {
                    if !number.is_finite() || number.abs() > f32::MAX as f64 {
                        return Err(mlua::Error::RuntimeError(format!(
                            "{method_name}: numeric arguments must be finite f32 values"
                        )));
                    }
                    if coordinate_args.contains(&index) && number.abs() > MAX_API_COORDINATE {
                        return Err(mlua::Error::RuntimeError(format!(
                            "{method_name}: coordinates must be within +/-{MAX_API_COORDINATE} blocks"
                        )));
                    }
                }
            }
            // Charge candidate scans before entering native code. Even empty
            // queries consume work; returned-result limits alone do not bound it.
            let native_work = if method_name == "find_blocks" {
                let radius = args.get(4).cloned().map(|v| lua.coerce_number(v)).transpose()?.flatten().unwrap_or(0.0);
                let side = 2 * (radius as f32).clamp(0.0, MAX_FIND_RADIUS).ceil() as u32 + 1;
                side.saturating_pow(3).saturating_mul(1 + tx.blocks.borrow().len() as u32)
            } else {
                // Conservative charge covers creature/player scans and staged
                // effect lookups, including methods with constant-time fast paths.
                tx.native_scan_cost()
            };
            budget.charge_native_work(native_work);
            let result = function.call::<_, MultiValue>(args);
            properties.raw_set("time_of_day", *tx.time.borrow())?;
            properties.raw_set("is_night", is_night(*tx.time.borrow()))?;
            properties.raw_set("weather", tx.weather.borrow().current.name())?;
            budget.check();
            result
        })?)?;
    }
    Ok(())
}
