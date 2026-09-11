//! Environment capabilities share the callback transaction and its budgets.
use super::transaction::CallbackTransaction;
use super::*;
use crate::voxel::chunk::{world_to_chunk, CHUNK_Y};

pub(super) fn fish_clear(tx: &CallbackTransaction, pos: Vec3) -> bool {
    let x = pos.x.floor() as i32;
    let z = pos.z.floor() as i32;
    [-3, 3].into_iter().all(|dx| {
        [-3, 3].into_iter().all(|dz| {
            tx.world
                .chunks
                .contains_key(&world_to_chunk(x + dx, z + dz))
        })
    }) && crate::water::fish_spawn_clear(|x, y, z| tx.get_block(x, y, z), pos)
}

fn campfire<'lua>(
    lua: &'lua Lua,
    tx: &CallbackTransaction,
    x: i32,
    y: i32,
    z: i32,
) -> mlua::Result<Option<Table<'lua>>> {
    if tx.get_block(x, y, z) != BlockType::Campfire {
        return Ok(None);
    }
    let t = lua.create_table()?;
    let burning = tx.get_block(x, y - 1, z).is_solid();
    t.set("x", x)?;
    t.set("y", y)?;
    t.set("z", z)?;
    t.set("burning", burning)?;
    t.set("light_radius", 8.0)?;
    t.set("light_active", burning && is_night(*tx.time.borrow()))?;
    t.set("requires_fuel", false)?;
    Ok(Some(t))
}

pub(super) fn creature_fields(
    t: &Table,
    tx: &CallbackTransaction,
    kind: u8,
    pos: [f32; 3],
) -> mlua::Result<()> {
    let k = CreatureKind::from_u8(kind);
    t.set("can_swim", k == CreatureKind::Fish)?;
    t.set("can_fly", k.is_dragon())?;
    t.set(
        "in_water",
        tx.get_block(
            pos[0].floor() as i32,
            pos[1].floor() as i32,
            pos[2].floor() as i32,
        ) == BlockType::Water,
    )?;
    Ok(())
}

pub(super) fn populate<'lua, 'scope>(
    lua: &'lua Lua,
    scope: &mlua::Scope<'lua, 'scope>,
    api: &Table<'lua>,
    tx: &'scope CallbackTransaction,
    blocks: &'scope Cell<u32>,
    spawns: &'scope Cell<u32>,
) -> mlua::Result<()> {
    api.set(
        "is_raining",
        tx.weather.borrow().current.has_rain_particles(),
    )?;
    api.set("campfire_light_radius", 8.0)?;
    api.set("fish_spawn_clearance", 3)?;
    api.set("place_campfire_near_player",scope.create_function(move|lua,(id,radius):(u32,f32)|{
        if blocks.get()==0 {return Ok(None);}
        let players=tx.players();
        let Some(player)=players.iter().find(|p|p.id==id) else {return Ok(None);};
        let radius=radius.clamp(2.0,8.0);let r=radius.ceil() as i32;
        let (cx,cy,cz)=(player.pos.x.floor() as i32,player.pos.y.floor() as i32,player.pos.z.floor() as i32);
        let mut candidates=Vec::new();
        for dx in -r..=r {for dz in -r..=r {
            let distance=(dx*dx+dz*dz) as f32;
            if distance>radius*radius || distance<4.0 {continue;}
            for dy in -4i32..=4 {candidates.push((dx*dx+dz*dz+dy*dy,cx+dx,cy+dy,cz+dz));}
        }}
        candidates.sort_unstable();
        for (_,x,y,z) in candidates {
            lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap().check();
            if !(1..CHUNK_Y-4).contains(&y) || !tx.world.chunks.contains_key(&world_to_chunk(x,z)) {continue;}
            let center=Vec3::new(x as f32+0.5,y as f32,z as f32+0.5);
            if players.iter().any(|p|(p.pos.x-center.x).powi(2)+(p.pos.z-center.z).powi(2)<4.0) {continue;}
            if !matches!(tx.get_block(x,y-1,z),BlockType::Grass|BlockType::Soil|BlockType::Sand|BlockType::Stone) {continue;}
            let existing=tx.get_block(x,y,z);
            if existing!=BlockType::Air && !(existing.def().only_on_top && existing!=BlockType::Campfire) {continue;}
            if (1..4).any(|dy|tx.get_block(x,y+dy,z)!=BlockType::Air) {continue;}
            blocks.set(blocks.get()-1);
            tx.blocks.borrow_mut().push((x,y,z,BlockType::Campfire));
            return campfire(lua,tx,x,y,z);
        }
        Ok(None)
    })?)?;
    api.set(
        "get_campfire",
        scope.create_function(move |lua, (x, y, z): (i32, i32, i32)| campfire(lua, tx, x, y, z))?,
    )?;
    api.set(
        "find_campfires",
        scope.create_function(move |lua, (x, y, z, radius): (i32, i32, i32, f32)| {
            let result = lua.create_table()?;
            let radius = radius.clamp(0.0, 12.0);
            let r = radius.ceil() as i32;
            let mut count = 0;
            for dx in -r..=r {
                for dy in -r..=r {
                    lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap().check();
                    for dz in -r..=r {
                        if (dx * dx + dy * dy + dz * dz) as f32 > radius * radius {
                            continue;
                        }
                        if let Some(t) = campfire(lua, tx, x + dx, y + dy, z + dz)? {
                            count += 1;
                            result.set(count, t)?;
                            if count >= MAX_FIND_RESULTS {
                                return Ok(result);
                            }
                        }
                    }
                }
            }
            Ok(result)
        })?,
    )?;
    api.set(
        "place_campfire",
        scope.create_function(move |_, (x, y, z): (i32, i32, i32)| {
            if blocks.get() == 0
                || !(1..CHUNK_Y - 4).contains(&y)
                || !tx.world.chunks.contains_key(&world_to_chunk(x, z))
                || !matches!(
                    tx.get_block(x, y - 1, z),
                    BlockType::Grass | BlockType::Soil | BlockType::Sand | BlockType::Stone
                )
                || (0..4).any(|dy| tx.get_block(x, y + dy, z) != BlockType::Air)
            {
                return Ok(false);
            }
            blocks.set(blocks.get() - 1);
            tx.blocks.borrow_mut().push((x, y, z, BlockType::Campfire));
            Ok(true)
        })?,
    )?;
    api.set(
        "get_water",
        scope.create_function(move |lua, (x, y, z): (i32, i32, i32)| {
            if !(0..CHUNK_Y).contains(&y) || tx.get_block(x, y, z) != BlockType::Water {
                return Ok(None);
            }
            let mut top = y;
            let mut bottom = y;
            while top + 1 < CHUNK_Y && tx.get_block(x, top + 1, z) == BlockType::Water {
                top += 1;
            }
            while bottom > 0 && tx.get_block(x, bottom - 1, z) == BlockType::Water {
                bottom -= 1;
            }
            let flow = if tx.world.generation.shape == crate::worldgen::Shape::Mainland {
                crate::voxel::terrain::current(
                    x,
                    z,
                    tx.world.seed,
                    top > crate::voxel::world::SEA_LEVEL,
                )
            } else {
                [0.0; 2]
            };
            let t = lua.create_table()?;
            t.set("x", x)?;
            t.set("y", y)?;
            t.set("z", z)?;
            t.set("surface_y", top + 1)?;
            t.set("depth", top - bottom + 1)?;
            t.set("flow_x", flow[0])?;
            t.set("flow_z", flow[1])?;
            t.set("flowing", flow != [0.0; 2])?;
            t.set("flow_speed", if flow == [0.0; 2] { 0.0 } else { 1.4 })?;
            t.set("visual_only", true)?;
            Ok(Some(t))
        })?,
    )?;
    api.set(
        "get_waterfalls",
        scope.create_function(move |lua, (x, y, z): (i32, i32, i32)| {
            let falls = crate::water::falls_at(
                |x, y, z| tx.get_block(x, y, z),
                |x, z| tx.world.chunks.contains_key(&world_to_chunk(x, z)),
                x,
                y,
                z,
            );
            let result = lua.create_table()?;
            for (i, f) in falls.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("x", x)?;
                t.set("y", y)?;
                t.set("z", z)?;
                t.set("bottom_y", f.bottom)?;
                t.set("height", f.lip.y - f.bottom)?;
                t.set("flow_x", f.direction.x)?;
                t.set("flow_z", f.direction.z)?;
                t.set("sound_radius", 48.0)?;
                t.set("visual_only", true)?;
                result.set(i + 1, t)?;
            }
            Ok(result)
        })?,
    )?;
    api.set(
        "can_spawn_fish",
        scope.create_function(move |_, (x, y, z): (f32, f32, f32)| {
            Ok(fish_clear(tx, Vec3::new(x, y, z)))
        })?,
    )?;
    api.set(
        "spawn_fish",
        scope.create_function(move |_, (x, y, z): (f32, f32, f32)| {
            if spawns.get() == 0 {
                return Ok(None);
            }
            spawns.set(spawns.get() - 1);
            let pos = Vec3::new(x, y, z);
            if !fish_clear(tx, pos) {
                return Ok(None);
            }
            let seed = tx.spawn_seed.get();
            tx.spawn_seed.set(seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
            Ok(tx
                .creatures
                .borrow_mut()
                .spawn(CreatureKind::Fish, pos, seed))
        })?,
    )?;
    let _ = lua;
    Ok(())
}
