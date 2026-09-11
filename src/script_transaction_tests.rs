use super::*;

fn environment_fixture()->Fixture {
    let mut f=Fixture::new();
    let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
    for x in 0..16 {for z in 0..16 {chunk.set_local(x,24,z,BlockType::Stone);}}
    for x in 5..=11 {for z in 5..=11 {for y in 30..=32 {chunk.set_local(x,y,z,BlockType::Water);}}}
    // Isolated source above a lower receiving pool.
    chunk.set_local(2,35,2,BlockType::Water);
    chunk.set_local(3,28,2,BlockType::Water);
    f.world.chunks.insert((0,0),chunk);
    f
}

#[test]
fn documented_environment_examples_load_and_execute() {
    let docs=include_str!("../docs/prompting.md");
    assert_eq!(docs.split("```lua\n").skip(1).count(),3);
    for (i,part) in docs.split("```lua\n").skip(1).enumerate() {
        let source=part.split("```").next().unwrap();
        let mut m=Module::load(format!("documented-{i}"),"documentation".into(),source.into()).unwrap();
        let mut f=environment_fixture();
        let callback=if source.contains("function on_tick") {"on_tick"}else{"on_cast"};
        for _ in 0..11 {f.invoke(&mut m,callback);assert!(m.error.is_none(),"example {i}: {:?}",m.error);}
    }
}

#[test]
fn environment_api_reads_staged_edits_and_properties_and_spawns_safe_fish() {
    let mut f=environment_fixture();
    let mut m=module("on_cast",r#"
        assert(api.campfire_light_radius==8 and api.fish_spawn_clearance==3)
        assert(api.place_campfire(3,25,12))
        assert(not api.place_campfire(3,25,12))
        assert(not api.place_campfire(1000,25,1000))
        api.set_time_night()
        api.set_weather('storm')
        assert(api.is_raining)
        local fire=api.get_campfire(3,25,12)
        assert(fire.burning and fire.light_active and not fire.requires_fuel)
        assert(#api.find_campfires(3,25,12,2)==1)
        local water=api.get_water(8,31,8)
        assert(water.surface_y==33 and water.depth==3 and water.visual_only)
        assert(api.get_water(0,25,0)==nil)
        assert(api.can_spawn_fish(8.5,31.5,8.5))
        local id=api.spawn_fish(8.5,31.5,8.5)
        assert(id~=nil)
        local fish=api.nearest_creature('fish',8,31,8)
        assert(fish.id==id and fish.can_swim and fish.in_water and not fish.can_fly)
        assert(api.replace_block(5,31,5,'air'))
        assert(not api.can_spawn_fish(8.5,31.5,8.5))
        assert(api.spawn_fish(8.5,31.5,8.5)==nil)
        local falls=api.get_waterfalls(2,35,2)
        assert(#falls==1 and falls[1].height==7 and falls[1].bottom_y==29)
        assert(falls[1].flow_x==1 and falls[1].sound_radius==48)
        api.replace_block(3,32,2,'stone')
        assert(#api.get_waterfalls(2,35,2)==0)
    "#);
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_none(),"{:?}",m.error);
    assert!(out.block_edits.iter().any(|b|b.3==BlockType::Campfire));
    assert_eq!(f.creatures.snapshot_with_ids().iter().filter(|c|c.1==CreatureKind::Fish.to_u8()).count(),1);
}

#[test]
fn environment_actions_roll_back_and_scans_cannot_evade_native_budget() {
    let mut f=environment_fixture();
    let initial=f.creatures.snapshot_with_ids().len();
    let mut m=module("on_cast","assert(api.place_campfire(3,25,12)); assert(api.spawn_fish(8.5,31.5,8.5)); error('rollback')");
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_some());assert!(out.block_edits.is_empty());
    assert_eq!(f.creatures.snapshot_with_ids().len(),initial);
    let mut m=module("on_cast","api.broadcast('rollback'); for i=1,10 do pcall(function() api.find_campfires(0,25,0,12) end) end");
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.as_deref().unwrap().contains("native work budget"));
    assert!(out.broadcasts.is_empty());
    for call in ["api.get_water(0/0,0,0)","api.spawn_fish('1e100',0,0)","api.get_waterfalls(2000000,0,0)"] {
        let mut m=module("on_cast",call);f.invoke(&mut m,"on_cast");assert!(m.error.is_some());
    }
}

#[test]
fn native_work_exhaustion_cannot_be_caught_and_rolls_back() {
    let mut fixture = Fixture::new();
    let mut m = module(
        "on_cast",
        r#"
        api.broadcast('must roll back')
        for i=1,8 do
            pcall(function() api.find_blocks('unknown_kind', 0, 30, 0, 10) end)
        end
        api.set_time_night()
    "#,
    );
    let (out, _) = fixture.invoke(&mut m, "on_cast");
    assert!(m.error.as_deref().unwrap().contains("native work budget"));
    assert!(out.broadcasts.is_empty());
    assert_eq!(fixture.time, 0.25);
}

#[test]
fn creature_capacity_rejects_spawns_without_reserving_ids_and_destroy_frees_space() {
    let mut fixture = Fixture::new();
    let cap = crate::world_api_gen::SCRIPT_CREATURES_MAX;
    for _ in 3..cap {
        fixture
            .creatures
            .spawn_one(CreatureKind::Sheep, Vec3::ZERO, 1);
    }
    let mut m = module(
        "on_cast",
        &format!(
            r#"
        assert(api.spawn_creature('wolf', 0, 30, 0) == nil)
        api.destroy(1)
        assert(api.spawn_creature('wolf', 0, 30, 0) == {})
        assert(api.spawn_creature('wolf', 0, 30, 0) == nil)
    "#,
            cap + 1
        ),
    );
    fixture.invoke(&mut m, "on_cast");
    assert!(m.error.is_none(), "{:?}", m.error);
    assert_eq!(fixture.creatures.snapshot_with_ids().len(), cap);
}

struct Fixture {
    world: World,
    creatures: Creatures,
    players: Vec<PlayerSnapshot>,
    weather: WeatherState,
    time: f32,
    resources: [u32; COLLECTIBLE_BLOCKS.len()],
}

impl Fixture {
    fn new() -> Self {
        let mut creatures = Creatures::new();
        for x in 0..3 {
            creatures.spawn_one(
                CreatureKind::Sheep,
                Vec3::new(x as f32, 30.0, 0.0),
                x as u64,
            );
        }
        let mut resources = [0; COLLECTIBLE_BLOCKS.len()];
        resources[COLLECTIBLE_BLOCKS
            .iter()
            .position(|b| *b == BlockType::Stone)
            .unwrap()] = 5;
        Self {
            world: World::new(1),
            creatures,
            weather: WeatherState::new(1),
            time: 0.25,
            resources,
            players: vec![PlayerSnapshot {
                resources: [0; COLLECTIBLE_BLOCKS.len()],
                id: HOST_PLAYER_ID,
                pos: Vec3::ZERO,
                carrying_crystal: false,
                velocity: Vec3::ZERO,
                on_ground: true,
                sprinting: false,
                in_water: false,
                health: 100.0,
                poisoned: false,
                speed_multiplier: 1.0,
                jump_multiplier: 1.0,
                oxygen: 100.0,
            }],
        }
    }

    fn invoke(&mut self, module: &mut Module, callback: &str) -> (TickOutcome, Vec<DeathEvent>) {
        let mut outcome = TickOutcome::default();
        let mut deaths = Vec::new();
        let mut input = TickInput {
            creatures: &mut self.creatures,
            world: &self.world,
            players: &self.players,
            time_of_day: &mut self.time,
            weather: &mut self.weather,
            block_edits: &mut outcome.block_edits,
            death_events: &mut deaths,
            broadcasts: &mut outcome.broadcasts,
            player_effects: &mut outcome.player_effects,
            host_resources: self.resources,
        };
        match callback {
            "on_tick" => module.run_tick(&mut input),
            "on_cast" => {
                let _ = module.run_cast(&mut input, HOST_PLAYER_ID);
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
        (outcome, deaths)
    }

    fn host_tick(&mut self, host: &mut ScriptHost) -> TickOutcome {
        host.run_tick(
            &self.world,
            &mut self.creatures,
            &self.players,
            &mut self.time,
            &mut self.weather,
            &[],
            &[],
            self.resources,
        )
    }
}

fn module(callback: &str, body: &str) -> Module {
    let extra = if callback != "on_tick" && callback != "on_cast" {
        "function on_tick(api) end"
    } else {
        ""
    };
    let mut module = Module::load(
        "transaction_test".into(),
        "test".into(),
        format!("{extra}\nfunction {callback}(api, event)\n{body}\nend"),
    )
    .unwrap();
    module.enabled = true;
    module
}

const MUTATIONS: &str = r#"
    api.chase(1, 20, 30, 0)
    api.damage(2, 2)
    api.destroy(3)
    local id = api.spawn_creature('wolf', 4, 30, 0)
    assert(id == 4)
    api.damage(id, 1)
    api.replace_block(1, 30, 1, 'redstone')
    api.give_item(0, 'stone', 2)
    assert(api.take_item(0, 'stone', 3))
    api.damage_player(0, 20)
    api.heal_player(0, 5)
    api.set_poisoned(0, true)
    api.set_player_speed(0, 2)
    api.set_player_jump(0, 2)
    api.teleport_player(0, 4, 30, 0)
    api.set_weather('storm')
    api.set_time_of_day(0.75)
    api.broadcast('committed')
"#;

#[test]
fn every_callback_discards_all_effects_on_error_or_budget_exhaustion() {
    for callback in [
        "on_tick",
        "on_cast",
        "on_death",
        "on_block_break",
        "on_interact",
    ] {
        for failure in [
            "error('deliberate failure')",
            "pcall(function() for i=1,1000000 do end end)",
            "local memory = string.rep('x', 16777216)",
        ] {
            let mut fixture = Fixture::new();
            let before = fixture.creatures.snapshot_with_ids();
            let weather = fixture.weather.clone();
            let mut module = module(callback, &format!("{MUTATIONS}\n{failure}"));
            let seed = module.spawn_seed.get();
            let (outcome, deaths) = fixture.invoke(&mut module, callback);
            assert!(module.error.is_some(), "{callback}: expected failure");
            assert!(outcome.block_edits.is_empty());
            assert!(outcome.player_effects.is_empty());
            assert!(outcome.broadcasts.is_empty());
            assert!(deaths.is_empty());
            assert_eq!(fixture.creatures.snapshot_with_ids(), before);
            assert!(!fixture.creatures.any_hunting());
            assert_eq!(
                fixture.weather, weather,
                "weather RNG/timer must roll back too"
            );
            assert_eq!(fixture.time, 0.25);
            assert_eq!(module.spawn_seed.get(), seed);
            assert_eq!(
                fixture
                    .creatures
                    .spawn_one(CreatureKind::Sheep, Vec3::ZERO, 0),
                4
            );
        }
    }
}

#[test]
fn every_callback_commits_successful_effects_once() {
    for callback in [
        "on_tick",
        "on_cast",
        "on_death",
        "on_block_break",
        "on_interact",
    ] {
        let mut fixture = Fixture::new();
        let mut module = module(callback, MUTATIONS);
        let (outcome, deaths) = fixture.invoke(&mut module, callback);
        assert!(module.error.is_none(), "{:?}", module.error);
        assert_eq!(outcome.block_edits, vec![(1, 30, 1, BlockType::RedStone)]);
        assert_eq!(outcome.broadcasts, vec!["committed"]);
        assert_eq!(outcome.player_effects.len(), 8);
        assert_eq!(deaths.len(), if callback == "on_death" { 0 } else { 1 });
        let creatures = fixture.creatures.snapshot_with_ids();
        assert_eq!(creatures.len(), 3);
        assert!(!creatures.iter().any(|c| c.0 == 3));
        assert_eq!(creatures.iter().find(|c| c.0 == 2).unwrap().3, 10.0);
        assert_eq!(creatures.iter().find(|c| c.0 == 4).unwrap().3, 17.0);
        assert!(fixture.creatures.any_hunting());
        assert_eq!(fixture.weather.current, Weather::Storm);
        assert_eq!(fixture.time, 0.75);
    }
}

#[test]
fn queries_observe_the_callbacks_own_staged_writes() {
    let mut fixture = Fixture::new();
    let body = r#"
        assert(api.get_resource_count(0, 'stone') == 5)
        assert(api.take_item(0, 'stone', 5))
        assert(not api.take_item(0, 'stone', 1))
        assert(api.get_resource_count(0, 'stone') == 0)
        assert(api.give_item(0, 'stone', 2))
        assert(api.take_item(0, 'stone', 1))
        assert(api.get_resource_count(0, 'stone') == 1)
        assert(api.replace_block(7, 30, 7, 'redstone'))
        assert(api.get_block(7, 30, 7) == 'redstone')
        assert(#api.find_blocks('redstone', 7, 30, 7, 0) == 1)
        api.replace_block(7, 30, 7, 'air')
        assert(#api.find_blocks('redstone', 7, 30, 7, 0) == 0)
        assert(not api.replace_block(0, -1, 0, 'stone'))
        api.set_weather('rain'); assert(api.weather == 'rain')
        api.set_time_of_day(0.75); assert(api.time_of_day == 0.75 and api.is_night)
        local id = api.spawn_creature('wolf', 9, 30, 9)
        assert(#api.creatures() == 4)
        api.damage(id, 3)
        assert(api.nearest_creature('wolf', 9, 30, 9).health == 15)
        api.destroy(id)
        assert(#api.find_creatures('wolf', 9, 30, 9, 2) == 0)
        assert(api.nearest_creature('wolf', 9, 30, 9) == nil)
        api.damage_player(0, 100); api.heal_player(0, 12)
        api.set_poisoned(0, true); api.set_player_speed(0, 2); api.set_player_jump(0, 3)
        api.teleport_player(0, 9, 30, 9)
        local p = api.players()[1]
        assert(p.health == 12 and p.poisoned and p.speed_multiplier == 2 and p.jump_multiplier == 3)
        assert(api.nearest_player(9, 30, 9).distance == 0)
    "#;
    let mut module = module("on_cast", body);
    fixture.invoke(&mut module, "on_cast");
    assert!(module.error.is_none(), "{:?}", module.error);
}

#[test]
fn later_modules_see_prior_commits_but_not_failed_callbacks() {
    let mut fixture = Fixture::new();
    let mut host = ScriptHost::new();
    host.modules.push(module("on_tick", "assert(api.take_item(0,'stone',3)); api.replace_block(7,30,7,'stone'); api.set_weather('rain')"));
    host.modules.push(module("on_tick", "assert(api.take_item(0,'stone',2)); api.replace_block(7,30,7,'redstone'); api.set_weather('storm'); api.destroy(1); error('abort')"));
    host.modules.push(module(
        "on_tick",
        r#"
        assert(api.get_resource_count(0,'stone') == 2)
        assert(api.get_block(7,30,7) == 'stone' and api.weather == 'rain')
        assert(#api.creatures() == 3)
        assert(api.take_item(0,'stone',2))
        api.broadcast('observed committed state')
    "#,
    ));
    host.modules
        .push(module("on_death", "api.broadcast('unexpected death')"));
    let outcome = fixture.host_tick(&mut host);
    assert_eq!(outcome.crashes.len(), 1);
    assert_eq!(outcome.block_edits, vec![(7, 30, 7, BlockType::Stone)]);
    assert_eq!(outcome.broadcasts, vec!["observed committed state"]);
    assert_eq!(outcome.player_effects.len(), 2);
    assert_eq!(fixture.creatures.snapshot_with_ids().len(), 3);
}

#[test]
fn failed_lua_state_is_rebuilt_before_rule_reactivation_or_cast_retry() {
    for callback in ["on_tick", "on_cast"] {
        let mut fixture = Fixture::new();
        let mut module = module(
            callback,
            r#"
            counter = (counter or 0) + 1
            assert(counter == 1, 'failed VM was reused')
            api.broadcast('must not leak')
            error('intentional failure')
        "#,
        );
        for _ in 0..2 {
            module.enabled = true;
            let (outcome, _) = fixture.invoke(&mut module, callback);
            assert!(module
                .error
                .as_ref()
                .unwrap()
                .contains("intentional failure"));
            assert!(outcome.broadcasts.is_empty());
        }
    }
}

#[test]
fn successful_death_events_commit_but_failing_death_handlers_do_not() {
    let mut fixture = Fixture::new();
    let mut host = ScriptHost::new();
    host.modules.push(module("on_tick", "api.destroy(1)"));
    host.modules.push(module(
        "on_death",
        "api.destroy(2); api.replace_block(7,30,7,'redstone'); error('abort handler')",
    ));
    host.modules.push(module(
        "on_death",
        "assert(#api.creatures() == 2); api.broadcast('one committed death')",
    ));
    let outcome = fixture.host_tick(&mut host);
    assert_eq!(outcome.crashes.len(), 1);
    assert_eq!(outcome.broadcasts, vec!["one committed death"]);
    assert!(outcome.block_edits.is_empty());
    assert_eq!(fixture.creatures.snapshot_with_ids().len(), 2);
}

#[test]
fn committed_inventory_effects_match_the_real_player_balance() {
    let mut fixture = Fixture::new();
    let mut player = crate::player::Player::new(Vec3::ZERO);
    player.add_resources(BlockType::Stone, u32::MAX - 1);
    fixture.resources = player.resources_snapshot();
    let mut module = module(
        "on_cast",
        r#"
        api.give_item(0, 'stone', 3)
        assert(api.get_resource_count(0, 'stone') == 4294967295)
        assert(api.take_item(0, 'stone', 4294967295))
        assert(not api.take_item(0, 'stone', 1))
        api.give_item(0, 'stone', 4)
        assert(api.get_resource_count(0, 'stone') == 4)
    "#,
    );
    let (outcome, _) = fixture.invoke(&mut module, "on_cast");
    assert!(module.error.is_none(), "{:?}", module.error);
    for effect in outcome.player_effects {
        match effect {
            PlayerEffect::GiveItem { block, amount, .. } => player.add_resources(block, amount),
            PlayerEffect::TakeItem { block, amount, .. } => {
                assert!(player.take_resources(block, amount))
            }
            _ => panic!("unexpected effect"),
        }
    }
    assert_eq!(player.resource_count(BlockType::Stone), 4);
}

#[test]
fn a_failed_instant_spell_does_not_dispatch_phantom_deaths() {
    let mut fixture = Fixture::new();
    let mut host = ScriptHost::new();
    host.modules
        .push(module("on_cast", "api.destroy(1); error('abort cast')"));
    host.modules
        .push(module("on_death", "api.broadcast('phantom death')"));
    let outcome = host.run_cast(
        0,
        &fixture.world,
        &mut fixture.creatures,
        &fixture.players,
        &mut fixture.time,
        &mut fixture.weather,
        HOST_PLAYER_ID,
        fixture.resources,
    );
    assert_eq!(outcome.crashes.len(), 1);
    assert!(outcome.broadcasts.is_empty());
    assert_eq!(fixture.creatures.snapshot_with_ids().len(), 3);
}

#[test]
fn guest_inventory_queries_are_transactional_and_distinguish_absence_from_invalid() {
    let mut f = Fixture::new();
    let mut guest = f.players[0];
    guest.id = 7;
    let crystal = COLLECTIBLE_BLOCKS.iter().position(|b| *b == BlockType::Crystal).unwrap();
    guest.resources[crystal] = 2;
    f.players.push(guest);
    let mut m = module("on_cast", r#"
        assert(api.get_resource_count(7, 'crystal') == 2)
        assert(api.has_resource(7, 'crystal'))
        assert(api.has_item(7, 'crystal', 3) == false)
        assert(api.has_resource(7, 'iron') == false)
        assert(api.has_resource(99, 'crystal') == nil)
        assert(api.has_item(7, 'water') == nil)
        assert(api.has_item(7, 'crystal', 0) == nil)
        local inv = api.get_inventory(7)
        assert(inv.crystal == 2 and inv.iron == 0)
        inv.crystal = 1000
        assert(api.get_resource_count(7, 'crystal') == 2)
        assert(api.take_item(7, 'crystal', 2))
        assert(api.has_resource(7, 'crystal') == false)
        assert(not api.take_item(7, 'crystal', 1))
        assert(api.give_item(7, 'crystal', 4))
        assert(api.has_item(7, 'crystal', 4))
        assert(api.get_inventory(7).crystal == 4)
        assert(api.get_resource_count(0, 'crystal') == 0)
    "#);
    let (out, _) = f.invoke(&mut m, "on_cast");
    assert!(m.error.is_none(), "{:?}", m.error);
    assert_eq!(out.player_effects, [
        PlayerEffect::TakeItem { player_id: 7, block: BlockType::Crystal, amount: 2 },
        PlayerEffect::GiveItem { player_id: 7, block: BlockType::Crystal, amount: 4 },
    ]);
    let mut failed=module("on_cast", "api.take_item(7, 'crystal', 2); error('rollback')");
    let (out, _) = f.invoke(&mut failed, "on_cast");
    assert!(failed.error.is_some());
    assert!(out.player_effects.is_empty());
    assert_eq!(f.players[1].resources[crystal], 2);
}
