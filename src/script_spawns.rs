//! Quest NPCs and ordinary loot bags use the existing host-owned save/network paths.
use super::*;

#[derive(Clone, Default)]
pub struct SpawnScene {
    pub npcs: Vec<crate::npc::Npc>,
    pub loot: Vec<crate::loot::Drop>,
}
impl SpawnScene {
    pub fn capture(npcs: &[crate::npc::Npc], loot: &[crate::loot::Drop]) -> Self {
        Self {
            npcs: npcs.to_vec(),
            loot: loot.to_vec(),
        }
    }
    pub fn apply(&mut self, effect: &PlayerEffect) {
        match effect {
            PlayerEffect::SpawnNpc(npc) => self.npcs.push(npc.clone()),
            PlayerEffect::SpawnLootBag(drop) => self.loot.push(drop.clone()),
            _ => {}
        }
    }
}

fn error(message: &str) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

fn clear_cell(tx: &CallbackTransaction, x: i32, y: i32, z: i32) -> bool {
    (1..crate::voxel::chunk::CHUNK_Y - 2).contains(&y)
        && x.unsigned_abs() < 1_000_000
        && z.unsigned_abs() < 1_000_000
        && tx
            .world
            .chunks
            .contains_key(&crate::voxel::chunk::world_to_chunk(x, z))
        && tx.get_block(x, y - 1, z).is_solid()
        && tx.get_block(x, y, z) == BlockType::Air
        && tx.get_block(x, y + 1, z) == BlockType::Air
}

pub(super) fn populate<'lua, 'scope>(
    lua: &'lua Lua,
    scope: &mlua::Scope<'lua, 'scope>,
    api: &Table<'lua>,
    tx: &'scope CallbackTransaction,
    spawn_budget: &'scope Cell<u32>,
) -> mlua::Result<()> {
    api.set("spawn_npc", scope.create_function(move |_, (kind,x,y,z): (String,i32,i32,i32)| {
        let kind = crate::npc::API_KINDS.iter().position(|k| *k == kind)
            .ok_or_else(|| error("unknown quest NPC kind; use sage, elf_ranger, warrior, merchant, fire_sorceress or necromancer"))?;
        if spawn_budget.get() == 0 { return Ok(false); }
        spawn_budget.set(spawn_budget.get()-1);
        let position = Vec3::new(x as f32+0.5, y as f32, z as f32+0.5);
        if tx.scene.borrow().npcs.len() >= 256 || !clear_cell(tx,x,y,z)
            || tx.scene.borrow().npcs.iter().any(|n| Vec3::from_array(n.position).distance(position) < 1.2)
            || tx.players().iter().any(|p| p.pos.distance(position) < 1.2) {
            return Ok(false);
        }
        let npc = crate::npc::Npc { kind:kind as u8, home:(x,y,z), position:position.to_array(),
            facing:0., walking:false, phase:0., wait:2., step:0, target:None };
        tx.scene.borrow_mut().npcs.push(npc.clone());
        tx.effects.borrow_mut().push(PlayerEffect::SpawnNpc(npc));
        Ok(true)
    })?)?;
    api.set(
        "spawn_loot_bag",
        scope.create_function(move |_, (x, y, z, contents): (i32, i32, i32, Table)| {
            let mut items = Vec::new();
            let mut total = 0u32;
            // Bound parsing even for a huge/adversarial table; no metamethods are invoked.
            for pair in contents.pairs::<Value, Value>() {
                if items.len() >= 16 {
                    return Err(error("loot bags accept at most 16 resource entries"));
                }
                let (name, amount) = pair?;
                let Value::String(name) = name else {
                    return Err(error("loot keys must be canonical resource names"));
                };
                let name = name.to_str()?;
                if name.len() > 64 {
                    return Err(error("invalid resource name"));
                }
                let block = BlockType::from_name(name)
                    .filter(|b| b.id() == name && COLLECTIBLE_BLOCKS.contains(b))
                    .ok_or_else(|| error("unknown or non-collectible resource in loot bag"))?;
                let Value::Integer(amount) = amount else {
                    return Err(error("loot amounts must be integers"));
                };
                if !(1..=256).contains(&amount) {
                    return Err(error("loot amounts must be 1..256"));
                }
                total += amount as u32;
                if total > 1024 {
                    return Err(error("loot bag total exceeds 1024 items"));
                }
                items.push((block, amount as u32));
            }
            if items.is_empty() {
                return Err(error("loot bag contents cannot be empty"));
            }
            items.sort_by_key(|(b, _)| b.id());
            if spawn_budget.get() == 0 {
                return Ok(false);
            }
            spawn_budget.set(spawn_budget.get() - 1);
            let pos = [x as f32 + 0.5, y as f32 + 0.3, z as f32 + 0.5];
            if tx.scene.borrow().loot.len() >= 48
                || !clear_cell(tx, x, y, z)
                || tx
                    .scene
                    .borrow()
                    .loot
                    .iter()
                    .any(|d| Vec3::from_array(d.pos).distance(Vec3::from_array(pos)) < 0.5)
            {
                return Ok(false);
            }
            let drop = crate::loot::Drop {
                pos,
                contents: items,
                age: 0.,
                cargo: Default::default(),
                machine: false,
                launch: None,
            };
            tx.scene.borrow_mut().loot.push(drop.clone());
            tx.effects
                .borrow_mut()
                .push(PlayerEffect::SpawnLootBag(drop));
            Ok(true)
        })?,
    )?;
    api.set(
        "find_npcs",
        scope.create_function(move |lua, (x, y, z, radius): (f32, f32, f32, f32)| {
            let center = Vec3::new(x, y, z);
            let result = lua.create_table()?;
            for npc in tx
                .scene
                .borrow()
                .npcs
                .iter()
                .filter(|n| Vec3::from_array(n.position).distance(center) <= radius.clamp(0., 32.))
                .take(64)
            {
                let row = lua.create_table()?;
                row.set("kind", crate::npc::API_KINDS[npc.kind as usize])?;
                row.set("x", npc.position[0])?;
                row.set("y", npc.position[1])?;
                row.set("z", npc.position[2])?;
                row.set("home_x", npc.home.0)?;
                row.set("home_y", npc.home.1)?;
                row.set("home_z", npc.home.2)?;
                row.set("quests", true)?;
                result.raw_push(row)?;
            }
            Ok(result)
        })?,
    )?;
    api.set(
        "find_loot_bags",
        scope.create_function(move |lua, (x, y, z, radius): (f32, f32, f32, f32)| {
            let center = Vec3::new(x, y, z);
            let result = lua.create_table()?;
            for drop in tx
                .scene
                .borrow()
                .loot
                .iter()
                .filter(|d| Vec3::from_array(d.pos).distance(center) <= radius.clamp(0., 32.))
            {
                let row = lua.create_table()?;
                row.set("x", drop.pos[0])?;
                row.set("y", drop.pos[1])?;
                row.set("z", drop.pos[2])?;
                row.set("age", drop.age)?;
                row.set("machine", drop.machine)?;
                let contents = lua.create_table()?;
                for (block, amount) in &drop.contents {
                    contents.set(block.id(), *amount)?;
                }
                row.set("contents", contents)?;
                result.raw_push(row)?;
            }
            Ok(result)
        })?,
    )?;
    let _ = lua;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        world: World,
        creatures: Creatures,
        host: ScriptHost,
        time: f32,
        weather: WeatherState,
    }
    impl Fixture {
        fn new() -> Self {
            let mut world = World::new(1);
            let mut chunk = crate::voxel::chunk::Chunk::new(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_local(x, 9, z, BlockType::Stone);
                }
            }
            world.chunks.insert((0, 0), chunk);
            Self {
                world,
                creatures: Creatures::new(),
                host: ScriptHost::new(),
                time: 0.25,
                weather: WeatherState::new(1),
            }
        }
        fn run(&mut self, body: &str, tick: bool) -> TickOutcome {
            let callback = if tick { "on_tick" } else { "on_cast" };
            let source = format!("function {callback}(api) {body} end");
            assert!(crate::world_api_validate::validate_source(&source).is_empty());
            let mut m = Module::load("scene-test".into(), "test".into(), source).unwrap();
            m.enabled = true;
            self.host.modules.clear();
            self.host.modules.push(m);
            if tick {
                self.host.run_tick(
                    &self.world,
                    &mut self.creatures,
                    &[],
                    &mut self.time,
                    &mut self.weather,
                    &[],
                    &[],
                    [0; COLLECTIBLE_BLOCKS.len()],
                )
            } else {
                self.host.run_cast(
                    0,
                    &self.world,
                    &mut self.creatures,
                    &[],
                    &mut self.time,
                    &mut self.weather,
                    0,
                    [0; COLLECTIBLE_BLOCKS.len()],
                )
            }
        }
    }

    #[test]
    fn quest_spawn_all_types_use_existing_npcs_and_reads_staged_ground() {
        let mut f = Fixture::new();
        let out = f.run(
            r#"
            local kinds={'sage','elf_ranger','warrior','merchant','fire_sorceress','necromancer'}
            for i,k in ipairs(kinds) do
                assert(api.spawn_npc(k,i*2,10,3))
                assert(not api.spawn_npc(k,i*2,10,3))
            end
            assert(#api.find_npcs(8,10,3,32)==6)
            assert(api.find_npcs(8,10,3,0.9)[1].kind=='merchant')
            assert(api.replace_block(5,19,5,'stone'))
            assert(api.spawn_npc('merchant',5,20,5))
            assert(not api.spawn_npc('merchant',5,22,5))
            assert(not api.spawn_npc('merchant',1000,10,1000))
            assert(api.replace_block(14,10,14,'stone'))
            assert(not api.spawn_npc('merchant',14,10,14))
            assert(not api.spawn_loot_bag(14,10,14,{stone=1}))
        "#,
            false,
        );
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        let npcs: Vec<_> = out
            .player_effects
            .iter()
            .filter_map(|e| {
                if let PlayerEffect::SpawnNpc(n) = e {
                    Some(n.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(npcs.len(), 7);
        assert!(npcs.iter().all(crate::npc::Npc::valid));
        assert_eq!(
            npcs.iter().take(6).map(|n| n.kind).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4, 5]
        );
        assert!(crate::npc::can_talk(
            &f.world,
            &npcs[3],
            Vec3::new(8.5, 10., 5.5)
        ));
        let packet = crate::net::ReliableMsg::Npcs(npcs.clone());
        let bytes = bincode::serialize(&packet).unwrap();
        let crate::net::ReliableMsg::Npcs(restored) = bincode::deserialize(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!(restored, npcs);
    }

    #[test]
    fn quest_spawn_and_bags_rollback_invalid_input_and_share_tick_budget() {
        let mut f = Fixture::new();
        for bad in ["api.spawn_npc('merchant_boss',4,10,4)",
            "api.spawn_loot_bag(4,10,4,{stone=0})", "api.spawn_loot_bag(4,10,4,{stone=257})",
            "api.spawn_loot_bag(4,10,4,{stone=1.5})", "api.spawn_loot_bag(4,10,4,{air=1})",
            "api.spawn_loot_bag(4,10,4,{coins=1})", "api.spawn_loot_bag(4,10,4,{})",
            "api.spawn_loot_bag(4,10,4,{stone=256,amber=256,soil=256,clay=256,sand=1})",
            "local t={}; for _,k in ipairs({'stone','soil','sand','amber','clay','ash','basalt','birch_wood','oak_wood','spruce_wood','coal','copper','tin','gold','silver','iron','crystal'}) do t[k]=1 end; api.spawn_loot_bag(4,10,4,t)",
            "api.spawn_npc('merchant',1000001,10,4)",
            "api.spawn_loot_bag(4,0/0,4,{stone=1})",
            "error('abort')"] {
            let out=f.run(&format!("assert(api.spawn_npc('merchant',2,10,2)); assert(api.spawn_loot_bag(6,10,6,{{stone=2}})); {bad}"),false);
            assert!(!out.crashes.is_empty(),"{bad}");
            assert!(out.player_effects.is_empty(),"{bad}");
        }
        let out = f.run(
            r#"
            assert(api.spawn_npc('merchant',2,10,2))
            assert(api.spawn_loot_bag(4,10,4,{stone=2}))
            assert(api.spawn_creature('chicken',6,10,6))
            assert(not api.spawn_npc('merchant',1000,10,1000))
            assert(not api.spawn_loot_bag(8,10,8,{stone=2}))
            assert(#api.find_npcs(2,10,2,32)==1)
            assert(#api.find_loot_bags(4,10,4,32)==1)
        "#,
            true,
        );
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        assert_eq!(out.player_effects.len(), 2);
    }

    #[test]
    fn quest_spawn_bags_save_network_pickup_and_expiry_use_existing_behavior() {
        let mut f = Fixture::new();
        let out = f.run(
            r#"
            assert(api.spawn_npc('merchant',2,10,2))
            assert(api.spawn_loot_bag(4,10,4,{amber=math.random(1,3),stone=2}))
            assert(not api.spawn_loot_bag(4,10,4,{stone=1}))
            local bags=api.find_loot_bags(4,10,4,32)
            assert(#bags==1 and bags[1].contents.stone==2 and bags[1].age==0)
        "#,
            false,
        );
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        let mut scene = SpawnScene::default();
        for e in &out.player_effects {
            scene.apply(e);
        }
        let save = crate::save::CraftingSave {
            npcs: scene.npcs.clone(),
            loot: Some(scene.loot.clone()),
            ..Default::default()
        };
        let restored: crate::save::CraftingSave =
            serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
        assert_eq!(restored.npcs, scene.npcs);
        assert_eq!(restored.loot.as_ref().unwrap(), &scene.loot);
        let packet = crate::net::UnreliableMsg::Snapshot {
            world_revision: 0,
            creature_targets: vec![],
            loot: scene.loot.clone(),
            allow_guest_prompting: false,
            time_of_day: 0.25,
            weather: 0,
            players: vec![],
            creatures: vec![],
            creature_vitals: vec![],
        };
        let crate::net::UnreliableMsg::Snapshot { loot, .. } =
            bincode::deserialize(&bincode::serialize(&packet).unwrap()).unwrap()
        else {
            panic!()
        };
        assert_eq!(loot, scene.loot);
        let mut effects = crate::loot::Effects::restore_machine(loot);
        let mut account = crate::crafting::Account::default();
        let feet = Vec3::new(4.5, 10., 4.5);
        assert!(effects.collect(&f.world, feet, &mut account).is_empty());
        effects.update(2.0, true);
        assert!(!effects.collect(&f.world, feet, &mut account).is_empty());
        assert!(effects.collect(&f.world, feet, &mut account).is_empty());
        let mut effects = crate::loot::Effects::restore_machine(scene.loot);
        effects.update(119.0, true);
        assert_eq!(effects.drops.len(), 1);
        effects.update(1.0, true);
        assert!(effects.drops.is_empty());
    }

    #[test]
    fn quest_spawn_caps_and_cross_callback_reads_include_committed_spawns() {
        let mut f = Fixture::new();
        let first=f.run("assert(api.spawn_npc('merchant',2,10,2)); assert(api.spawn_loot_bag(4,10,4,{stone=1}))",false);
        for e in &first.player_effects {
            f.host.scene.apply(e);
        }
        let out=f.run("assert(#api.find_npcs(2,10,2,32)==1); assert(not api.spawn_npc('merchant',2,10,2)); assert(not api.spawn_loot_bag(4,10,4,{stone=2}))",false);
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        let npc = f.host.scene.npcs[0].clone();
        let bag = f.host.scene.loot[0].clone();
        f.host.scene.npcs = vec![npc; 256];
        f.host.scene.loot = vec![bag; 48];
        let out=f.run("assert(not api.spawn_npc('merchant',8,10,8)); assert(not api.spawn_loot_bag(8,10,8,{stone=1}))",false);
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        assert!(out.player_effects.is_empty());
        f.host.scene = SpawnScene::default();
        f.host.modules.clear();
        for source in [
            "function on_tick(api) assert(api.spawn_npc('merchant',2,10,2)); assert(api.spawn_loot_bag(4,10,4,{stone=1})) end",
            "function on_tick(api) assert(#api.find_npcs(2,10,2,32)==1); assert(#api.find_loot_bags(4,10,4,32)==1); assert(not api.spawn_npc('merchant',2,10,2)) end"
        ] { let mut m=Module::load("test".into(),"".into(),source.into()).unwrap();m.enabled=true;f.host.modules.push(m); }
        let out = f.host.run_tick(
            &f.world,
            &mut f.creatures,
            &[],
            &mut f.time,
            &mut f.weather,
            &[],
            &[],
            [0; COLLECTIBLE_BLOCKS.len()],
        );
        assert!(out.crashes.is_empty(), "{:?}", out.crashes);
        assert_eq!(out.player_effects.len(), 2);
    }
}
