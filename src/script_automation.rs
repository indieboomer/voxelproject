//! Automation capabilities participate in the same all-or-nothing Lua callback.
use super::transaction::CallbackTransaction;
use super::*;
use crate::automation::{self, Action, Device, Face, Kind, Routing, SignalMode};

fn inventory_table<'lua>(
    lua: &'lua Lua,
    items: &automation::Inventory,
) -> mlua::Result<Table<'lua>> {
    let t = lua.create_table()?;
    for (id, n) in items {
        t.set(id.as_str(), *n)?;
    }
    Ok(t)
}
fn snapshot<'lua>(lua: &'lua Lua, d: &Device) -> mlua::Result<Table<'lua>> {
    let t = lua.create_table()?;
    t.set("x", d.cell.0)?;
    t.set("y", d.cell.1)?;
    t.set("z", d.cell.2)?;
    t.set("kind", d.kind.id())?;
    t.set("enabled", d.config.enabled)?;
    t.set("eject_contents", d.config.eject_contents)?;
    t.set("mana", d.mana)?;
    t.set("fuel_heat", d.fuel_heat)?;
    t.set("height", d.kind.height())?;
    t.set("powered_ticks", d.powered_ticks)?;
    t.set("signal", d.signal)?;
    t.set("activity", format!("{:?}", d.activity))?;
    t.set("recipe", d.config.recipe.as_str())?;
    t.set(
        "element",
        automation::element_id(d.config.element).trim_start_matches("element:"),
    )?;
    t.set("rotation", d.rotation)?;
    t.set("reserve", d.config.reserve)?;
    t.set("items", inventory_table(lua, &d.items)?)?;
    t.set("output", inventory_table(lua, &d.output)?)?;
    t.set(
        "progress",
        d.batch
            .as_ref()
            .map_or(0.0, |b| b.elapsed as f32 / b.duration as f32),
    )?;
    Ok(t)
}
fn charge_budget(budget: &Cell<u32>) -> bool {
    if budget.get() == 0 {
        false
    } else {
        budget.set(budget.get() - 1);
        true
    }
}
fn face(name: &str) -> Option<Face> {
    Face::ALL
        .into_iter()
        .find(|f| format!("{f:?}").eq_ignore_ascii_case(name))
}
pub(super) fn populate<'lua, 'scope>(
    _: &'lua Lua,
    scope: &mlua::Scope<'lua, 'scope>,
    api: &Table<'lua>,
    tx: &'scope CallbackTransaction,
    budget: &'scope Cell<u32>,
) -> mlua::Result<()>
where
    'lua: 'scope,
{
    api.set(
        "get_device",
        scope.create_function(move |lua, (x, y, z): (i32, i32, i32)| {
            tx.automation
                .borrow()
                .device_at((x, y, z))
                .map(|d| snapshot(lua, d))
                .transpose()
        })?,
    )?;
    api.set(
        "get_devices",
        scope.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            for (i, d) in tx.automation.borrow().devices.values().enumerate() {
                lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap().check();
                t.set(i + 1, snapshot(lua, d)?)?;
            }
            Ok(t)
        })?,
    )?;
    api.set(
        "set_device_enabled",
        scope.create_function(move |_, (x, y, z, enabled): (i32, i32, i32, bool)| {
            let mut state = tx.automation.borrow_mut();
            let Some(d) = state.devices.get_mut(&(x, y, z)) else {
                return Ok(false);
            };
            if d.config.enabled == enabled {
                return Ok(true);
            }
            if !charge_budget(budget) {
                return Ok(false);
            }
            d.config.enabled = enabled;
            d.config_revision = d.config_revision.saturating_add(1);
            tx.automation_changed.set(true);
            Ok(true)
        })?,
    )?;
    api.set(
        "configure_device",
        scope.create_function(move |lua, (x, y, z, settings): (i32, i32, i32, Table)| {
            let mut state = tx.automation.borrow_mut();
            let Some(d) = state.devices.get(&(x, y, z)) else {
                return Ok(false);
            };
            let mut c = d.config.clone();
            let revision = d.config_revision;
            macro_rules! field {
                ($name:ident,$ty:ty) => {
                    if let Some(v) = settings.get::<_, Option<$ty>>(stringify!($name))? {
                        c.$name = v;
                    }
                };
            }
            field!(enabled, bool);
            field!(recipe, String);
            field!(reserve, u32);
            field!(filter, String);
            field!(sensor_item, String);
            field!(lower, u32);
            field!(upper, u32);
            field!(invert, bool);
            field!(valve_matter, bool);
            field!(eject_contents, bool);
            if let Some(element) = settings.get::<_, Option<String>>("element")? {
                let Some(i) = (0..5)
                    .find(|&i| automation::element_id(i).trim_start_matches("element:") == element)
                else {
                    return Ok(false);
                };
                c.element = i;
            }
            if let Some(value) = settings.get::<_, Option<String>>("input")? {
                let Some(f) = face(&value) else {
                    return Ok(false);
                };
                c.input = f;
            }
            if let Some(outputs) = settings.get::<_, Option<Vec<String>>>("outputs")? {
                let Some(faces) = outputs.iter().map(|s| face(s)).collect::<Option<Vec<_>>>()
                else {
                    return Ok(false);
                };
                c.outputs = faces;
            }
            if let Some(mode) = settings.get::<_, Option<String>>("routing")? {
                c.routing = match mode.as_str() {
                    "round_robin" => Routing::RoundRobin,
                    "priority" => Routing::Priority,
                    "filter" => Routing::Filter,
                    _ => return Ok(false),
                };
            }
            if let Some(mode) = settings.get::<_, Option<String>>("signal_mode")? {
                c.signal_mode = match mode.as_str() {
                    "state" => SignalMode::State,
                    "pulse" => SignalMode::Pulse,
                    "numeric" => SignalMode::Numeric,
                    _ => return Ok(false),
                };
            }
            if let Some(value) = settings.get::<_, Option<i32>>("sensor_x")? {
                c.sensor_target.0 = value;
            }
            if let Some(value) = settings.get::<_, Option<i32>>("sensor_y")? {
                c.sensor_target.1 = value;
            }
            if let Some(value) = settings.get::<_, Option<i32>>("sensor_z")? {
                c.sensor_target.2 = value;
            }
            if !charge_budget(budget) {
                return Ok(false);
            }
            let registry = lua
                .app_data_ref::<std::sync::Arc<crate::crafting::Registry>>()
                .unwrap();
            if state
                .configure((x, y, z), c, revision, automation::balance(), &registry)
                .is_err()
            {
                return Ok(false);
            }
            tx.automation_changed.set(true);
            Ok(true)
        })?,
    )?;
    api.set(
        "place_device",
        scope.create_function(
            move |lua,
                  (player_id, kind, x, y, z, rotation): (
                PlayerId,
                String,
                i32,
                i32,
                i32,
                Option<u8>,
            )| {
                let Some(kind) = Kind::parse(&kind) else {
                    return Ok(false);
                };
                if tx.get_block(x, y, z) != BlockType::Air || !charge_budget(budget) {
                    return Ok(false);
                }
                let Some(player) = tx.players().into_iter().find(|p| p.id == player_id) else {
                    return Ok(false);
                };
                let Some(mut account) = tx.inventory(player_id) else {
                    return Ok(false);
                };
                let players = tx.players().iter().map(|p| p.pos).collect::<Vec<_>>();
                let mut state = tx.automation.borrow_mut();
                let registry = lua
                    .app_data_ref::<std::sync::Arc<crate::crafting::Registry>>()
                    .unwrap();
                let action = Action::Place {
                    kind,
                    cell: (x, y, z),
                    rotation: rotation.unwrap_or(0),
                    packed: None,
                };
                if automation::apply(
                    tx.world,
                    &mut state,
                    &mut account,
                    player.pos,
                    &players,
                    &action,
                    automation::balance(),
                    &registry,
                )
                .is_err()
                {
                    return Ok(false);
                }
                tx.stage_inventory(player_id, &account);
                tx.automation_changed.set(true);
                Ok(true)
            },
        )?,
    )?;
    Ok(())
}
