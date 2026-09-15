use super::*;

fn shaping_fixture() -> Fixture {
    let mut f=Fixture::new();
    for x in -1..=0 { f.world.chunks.insert((x,0),crate::voxel::chunk::Chunk::new(x,0)); }
    f
}

#[test]
fn creature_magic_reads_staged_changes_respects_walls_and_rolls_back() {
    let mut f=shaping_fixture();
    f.creatures=Creatures::new();f.creatures.spawn_one(CreatureKind::Sheep,Vec3::new(4.,30.,4.),1);
    let before=f.creatures.snapshot_with_ids();
    let body=r#"
        assert(api.apply_creature_status(1,'slow',10))
        assert(api.apply_creature_status(1,'stun',3))
        assert(api.get_creature(1).slow_seconds==10 and api.get_creature(1).stun_seconds==3)
        assert(not api.apply_creature_status(1,'slow',31))
        assert(not api.apply_creature_status(1,'invented',2))
        assert(not api.apply_creature_status(999,'slow',2))
        assert(not api.push_creature(1,9,0,0))
        assert(api.fill_box(7,30,3,7,32,5,'stone')==9)
        assert(api.push_creature(1,4,0,0))
        local c=api.get_creature(1);assert(c.x>4 and c.x<6.4)
        assert(api.apply_creature_status(1,'stun',0));assert(api.get_creature(1).stun_seconds==0)
    "#;
    let mut failed=module("on_cast",&format!("{body}\nerror('rollback')"));f.invoke(&mut failed,"on_cast");
    assert!(failed.error.as_ref().unwrap().contains("rollback"),"{:?}",failed.error);
    assert_eq!(f.creatures.snapshot_with_ids(),before);assert!(f.creatures.magic_statuses.is_empty());
    let mut m=module("on_cast",body);f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
    assert_eq!(f.creatures.magic_statuses[&1].slow,10.);
    assert!(f.creatures.snapshot_with_ids()[0].2[0]>4.);
}

#[test]
fn world_shaping_documented_examples_execute_the_requested_behaviors() {
    let platform=include_str!("../world_api/examples/stone_platform.lua");
    let rain=include_str!("../world_api/examples/rain_softens_soil.lua");
    let shrine=include_str!("../world_api/examples/crystal_healing_shrine.lua");
    for source in [platform,rain,shrine] {
        assert!(crate::world_api_validate::validate_source(source).is_empty());
        Module::load("example".into(),"example".into(),source.into()).unwrap();
    }
    let mut f=shaping_fixture();f.players[0].pos=Vec3::new(8.,30.,8.);
    let mut m=Module::load("platform".into(),"platform".into(),platform.into()).unwrap();
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_none(),"{:?}",m.error);assert_eq!(out.block_edits.len(),25);
    assert!(out.block_edits.iter().all(|&(_,y,_,b)| y==29 && b==BlockType::Stone));

    f.world.set_block(8,29,8,BlockType::Soil);f.world.set_block(9,29,8,BlockType::Soil);
    f.world.set_block(9,31,8,BlockType::Stone);f.weather.set(Weather::Rain);
    let mut m=Module::load("rain".into(),"rain".into(),rain.into()).unwrap();m.enabled=true;
    let (out,_)=f.invoke(&mut m,"on_tick");assert!(m.error.is_none(),"{:?}",m.error);
    assert_eq!(out.block_edits,vec![(8,29,8,BlockType::Mud)]);
    f.weather.set(Weather::Sunny);
    assert!(f.invoke(&mut m,"on_tick").0.block_edits.is_empty());

    f.creatures.damage(1,6.);f.players[0].finances.elements[crate::crafting::Element::Life as usize]=1;
    let source=shrine.replace("function on_tick(api) end", "function on_cast(api,event) on_interact(api,{kind='crystal',x=1,y=30,z=0,player_id=event.player_id}) end");
    let mut m=Module::load("shrine".into(),"shrine".into(),source).unwrap();
    let (out,_)=f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
    assert_eq!(f.creatures.snapshot_with_ids().into_iter().find(|c|c.0==1).unwrap().3,10.);
    assert!(out.player_effects.iter().any(|e|matches!(e,PlayerEffect::Inventory{balances,..} if balances.elements[crate::crafting::Element::Life as usize]==0)));
}

#[test]
fn world_shaping_queries_read_staged_materials_roofs_and_players() {
    let mut f=shaping_fixture();
    let body=r#"
        local p=api.get_player(event.player_id); assert(p and p.id==event.player_id)
        assert(api.get_player(900)==nil and api.get_creature(900)==nil)
        assert(api.teleport_player(p.id,2,20,2)); assert(api.get_player(p.id).y==20)
        assert(#api.get_block_kinds()>80)
        local info=api.get_block_info('iron_ore'); assert(info.ore and info.solid and info.collectible)
        info.hardness=999; assert(api.get_block_info('iron_ore').hardness~=999)
        assert(api.get_block_info('invented_ore')==nil)
        assert(api.block_matches('coal','ore') and api.block_matches('oak_wood','wood'))
        assert(not api.block_matches('stone','unknown'))
        assert(api.is_block_loaded(-1,10,1) and not api.is_block_loaded(16,10,1))
        assert(api.surface_height(1,1)==nil and api.surface_height(99,99)==nil)
        assert(api.is_exposed_to_sky(99,10,99)==nil)
        assert(api.fill_box(3,10,3,1,10,1,'stone','air')==9)
        assert(api.surface_height(1,1)==10)
        assert(api.is_exposed_to_sky(1,11,1)==true and api.is_exposed_to_sky(1,10,1)==false)
        assert(api.fill_box(1,10,1,3,10,3,'stone')==0)
        assert(api.fill_box(1,10,1,3,10,3,'iron_ore','stone')==9)
        assert(api.fill_sphere(8,10,8,1,'coal','air')==7)
        assert(#api.find_blocks('ore',8,10,8,1)==7)
        assert(api.get_block(8,10,8)=='coal' and api.get_block(9,11,8)=='air')
    "#;
    let mut m=module("on_cast",body);
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_none(),"{:?}",m.error);assert_eq!(out.block_edits.len(),25);
    let mut m=module("on_cast",&format!("{body}\nerror('rollback')"));
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_some());assert!(out.block_edits.is_empty() && out.player_effects.is_empty());
}

#[test]
fn world_shaping_is_atomic_and_shares_existing_edit_limits() {
    let mut f=shaping_fixture();
    let mut m=module("on_tick",r#"
        assert(api.fill_box(0,10,0,7,10,3,'stone')==32)
        assert(api.fill_box(0,10,0,7,10,3,'stone')==0)
        assert(api.fill_box(10,10,10,10,10,10,'stone')==nil)
        assert(not api.replace_block(10,10,10,'stone'))
    "#);
    let (out,_)=f.invoke(&mut m,"on_tick");
    assert!(m.error.is_none(),"{:?}",m.error);assert_eq!(out.block_edits.len(),32);
    let mut m=module("on_cast",r#"
        assert(api.fill_box(0,10,0,9,12,9,'stone')==300)
        assert(api.fill_sphere(12,12,12,0,'gold_ore')==nil)
        assert(api.get_block(12,12,12)=='air')
    "#);
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_none(),"{:?}",m.error);assert_eq!(out.block_edits.len(),300);
}

#[test]
fn world_shaping_rejects_protected_unloaded_and_invalid_regions_without_partial_edits() {
    let mut f=shaping_fixture();
    f.world.set_block(4,10,4,BlockType::Bedrock);
    f.world.automation.devices.insert((8,10,8),crate::automation::Device::new(crate::automation::Kind::Chest,(8,10,8),0));
    let mut m=module("on_cast",r#"
        assert(api.fill_box(3,10,4,4,10,4,'stone')==nil)
        assert(api.fill_box(7,10,8,8,10,8,'stone')==nil)
        assert(api.fill_box(15,10,1,16,10,1,'stone')==nil)
        assert(api.fill_box(0,10,0,15,25,15,'stone')==nil)
        assert(api.fill_sphere(1,1,1,8,'stone')==nil)
        assert(api.fill_box(0,10,0,0,10,0,'bedrock')==nil)
        assert(api.fill_box(0,10,0,0,10,0,'unknown')==nil)
        assert(api.fill_box(0,10,0,0,10,0,'stone','unknown')==nil)
        assert(api.fill_box(0,-1,0,0,0,0,'stone')==nil)
        assert(api.get_block(3,10,4)=='air' and api.get_block(7,10,8)=='air')
        assert(api.fill_box(0,10,0,9,12,9,'stone','air')==298)
    "#);
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_none(),"{:?}",m.error);
    assert_eq!(out.block_edits.len(),298);
}

#[test]
fn world_shaping_native_limits_cannot_be_caught_and_abort_prior_writes() {
    let mut f=shaping_fixture();
    let mut m=module("on_cast",r#"
        assert(api.fill_box(0,10,0,7,10,3,'stone')==32)
        pcall(function() api.fill_box(0,10,0,15,25,15,'stone') end)
        api.broadcast('must not commit')
    "#);
    let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_some());assert!(out.block_edits.is_empty() && out.broadcasts.is_empty());
    for call in ["api.fill_sphere(0,10,0,0/0,'stone')",
        "api.fill_box(1000001,10,0,1000001,10,0,'stone')", "api.heal_creature(1,1/0)"] {
        let mut m=module("on_cast",call);let (out,_)=f.invoke(&mut m,"on_cast");
        assert!(m.error.is_some(),"{call}");assert!(out.block_edits.is_empty());
    }
}

#[test]
fn creature_healing_reads_own_writes_rolls_back_and_survives_save_restore() {
    let mut f=shaping_fixture();
    f.creatures.damage(1,5.);
    let before=f.creatures.snapshot_with_ids();
    let body=r#"
        local c=api.get_creature(1); assert(c.health<c.max_health)
        assert(api.heal_creature(1,2)); assert(api.get_creature(1).health==c.health+2)
        assert(api.heal_creature(1,999)); assert(api.get_creature(1).health==c.max_health)
        assert(not api.heal_creature(999,1) and not api.heal_creature(1,-1))
        api.destroy(2); assert(api.get_creature(2)==nil and not api.heal_creature(2,99))
    "#;
    let mut m=module("on_cast",&format!("{body}\nerror('rollback')"));f.invoke(&mut m,"on_cast");
    assert!(m.error.is_some());assert_eq!(f.creatures.snapshot_with_ids(),before);
    let mut m=module("on_cast",body);f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
    let saved=f.creatures.snapshot_with_ids();
    let mut restored=Creatures::new();restored.restore_saved(&saved,1);
    let c=restored.snapshot_with_ids().into_iter().find(|c|c.0==1).unwrap();assert_eq!(c.3,c.4);
}

#[test]
fn documented_wayfinder_ward_uses_current_progress_and_selection() {
    let source=include_str!("../docs/examples/wayfinder_crystal_ward.lua");
    assert!(crate::world_api_validate::validate_source(source).is_empty());
    let mut f=Fixture::new();f.players[0].finances.adventure.stage=3;
    f.players[0].finances.held=Some(crate::equipment::Entry::Resource(BlockType::Crystal));
    f.resources[COLLECTIBLE_BLOCKS.iter().position(|b|*b==BlockType::Crystal).unwrap()]=1;
    let mut m=Module::load("wayfinder ward".into(),"Protect experienced crystal holders from goblins".into(),source.into()).unwrap();m.enabled=true;
    f.invoke(&mut m,"on_tick");assert!(m.error.is_none(),"{:?}",m.error);
    assert!(f.creatures.attack_policies.values().flatten().any(|p|matches!(p,crate::creature::AttackPolicy::ProtectPlayer(0,k) if *k==CreatureKind::Goblin as u8)));
    f.players[0].finances.held=None;
    f.invoke(&mut m,"on_tick");assert!(f.creatures.attack_policies.is_empty());
}

#[test]
fn equipped_spell_survives_staged_mana_changes() {
    let mut f=Fixture::new();
    let mut account=crate::crafting::Account::default();
    account.known_spells.push(42);
    account.hotbar.assign(Some(crate::equipment::Entry::Spell(42)));
    f.players[0].finances=InventoryBalances::from_account(&account);
    let mut m=module("on_cast",r#"
        assert(api.get_equipped_item(0)=='spell:42')
        assert(api.give_mana(0,1))
        assert(api.get_equipped_item(0)=='spell:42')
    "#);
    f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
    account.known_spells.clear();
    f.players[0].finances=InventoryBalances::from_account(&account);
    let mut m=module("on_cast","assert(api.get_equipped_item(0)==nil)");
    f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
}

#[test]
fn journal_and_equipment_queries_preserve_progress_across_staged_inventory_changes() {
    let mut f=Fixture::new();
    f.players[0].finances.adventure=crate::adventure::Progress{stage:2,home:Some((4,30,8)),explored_depths:true,recoveries:3,..Default::default()};
    f.players[0].finances.held=Some(crate::equipment::Entry::Resource(BlockType::Crystal));
    let crystal=COLLECTIBLE_BLOCKS.iter().position(|b|*b==BlockType::Crystal).unwrap();f.players[0].resources[crystal]=1;f.resources[crystal]=1;
    let mut guest=f.players[0];guest.id=7;f.players.push(guest);
    let body=r#"
        for _,id in ipairs({0,7}) do
            local journal=api.get_player_journal(id)
            assert(journal.stage==2 and journal.explored_depths and journal.recoveries==3)
            assert(#journal.quests==20 and journal.quests_completed==0)
            assert(journal.quests[20].npc=='Necromancer' and journal.quests[20].target==1)
            journal.quests[1].completed=true
            assert(not api.get_player_journal(id).quests[1].completed)
            assert(journal.home.x==4 and journal.home.y==30 and journal.home.z==8)
            journal.stage=3; journal.home.x=900
            assert(api.get_player_journal(id).stage==2 and api.get_player_journal(id).home.x==4)
            assert(api.get_equipped_item(id)=='crystal')
            assert(api.take_item(id,'crystal',1)); assert(api.get_equipped_item(id)==nil)
            assert(api.give_mana(id,1)); assert(api.get_player_journal(id).recoveries==3)
            assert(api.give_item(id,'crystal',1)); assert(api.get_equipped_item(id)=='crystal')
        end
        assert(api.get_player_journal(99)==nil and api.get_equipped_item(99)==nil)
    "#;
    let mut m=module("on_cast",body);let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_none(),"{:?}",m.error);
    assert!(out.player_effects.iter().any(|e|matches!(e,PlayerEffect::Inventory{balances,..} if balances.adventure.recoveries==3)));
    let mut m=module("on_cast",&format!("{body}\nerror('rollback')"));let (out,_)=f.invoke(&mut m,"on_cast");
    assert!(m.error.is_some());assert!(out.player_effects.is_empty());
}

#[test]
fn inventory_economy_reads_staged_balances_for_host_and_guest_and_rolls_back() {
    let mut f=Fixture::new();
    f.players[0].finances=InventoryBalances {mana:100,elements:[0;5],items:crate::gear_catalog::starter_counts(),..Default::default()};
    let mut guest=f.players[0];guest.id=7;guest.resources=[0;COLLECTIBLE_BLOCKS.len()];f.players.push(guest);
    let body=r#"
        for _,id in ipairs({0,7}) do
            assert(api.give_item(id,'iron',3)); assert(api.give_item(id,'oak_wood',2))
            assert(api.craft_item(id,'pickaxe')); assert(api.get_item_count(id,'pickaxe')==2)
            assert(api.has_item(id,'pickaxe',2)); assert(api.get_mana(id)==94)
            assert(api.decompose_item(id,'pickaxe')); assert(api.get_mana(id)==91)
            assert(api.get_inventory(id).iron==2 and api.get_resource_count(id,'oak_wood')==1)
            assert(api.give_item(id,'stone',2)); assert(api.decompose_resource(id,'stone',2))
            assert(api.get_element_count(id,'Earth')==2)
            assert(api.convert_elements_to_mana(id,'earth',2))
            assert(api.take_mana(id,1)); assert(api.give_element(id,'fire',4)); assert(api.take_element(id,'Fire',1))
            local inventory=api.get_player_inventory(id)
            assert(inventory.mana==90 and inventory.elements.fire==3 and inventory.elements.earth==0)
            assert(inventory.items.pickaxe==1 and inventory.items.sword==1)
            inventory.mana=999;inventory.elements.fire=999
            assert(api.get_mana(id)==90 and api.get_element_count(id,'fire')==3)
            local recipe=api.get_item_recipe('pickaxe')
            assert(recipe.create.mana==6 and recipe.decompose.resources.iron==2)
            assert(api.get_resource_elements('stone').earth==1)
            assert(not api.take_mana(id,100)); assert(not api.craft_item(id,'pickaxe'))
            assert(api.get_mana(id)==90)
        end
        assert(api.get_mana(99)==nil and api.get_item_count(7,'bow')==nil)
        assert(not api.give_element(7,'light',1) and not api.give_mana(7,0))
    "#;
    let mut m=module("on_cast",body);
    let (out,_)=f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
    assert!(out.player_effects.iter().any(|e|matches!(e,PlayerEffect::Inventory{player_id:7,..})));
    let mut m=module("on_cast",&format!("{body}\nerror('rollback')"));
    let (out,_)=f.invoke(&mut m,"on_cast");assert!(m.error.is_some());assert!(out.player_effects.is_empty());
}

#[test]
fn inventory_uses_configured_registry_and_rejects_overflow() {
    let mut f=Fixture::new();
    f.players[0].finances=InventoryBalances{mana:u32::MAX,elements:[2,0,0,0,0],items:[0;21],..Default::default()};
    let mut registry=crate::crafting::Registry::parse(include_str!("../data/crafting.json")).unwrap();registry.conversion_rate=3;
    let mut m=module("on_cast",r#"
        assert(not api.give_mana(0,1))
        assert(not api.convert_elements_to_mana(0,'earth',2))
        assert(api.get_element_count(0,'earth')==2)
        assert(api.take_mana(0,6))
        assert(api.convert_elements_to_mana(0,'earth',2))
        assert(api.get_mana(0)==4294967295 and api.get_element_count(0,'earth')==0)
        assert(not api.decompose_item(0,'sword'))
    "#);
    m.lua.set_app_data(std::sync::Arc::new(registry));
    let (_,_) = f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
}

#[test]
fn inventory_balances_flow_between_committed_callbacks() {
    let mut f=Fixture::new();
    let mut guest=f.players[0];guest.id=7;guest.finances=InventoryBalances{mana:10,..Default::default()};f.players.push(guest);
    let mut host=ScriptHost::new();
    host.modules.push(module("on_tick","assert(api.give_mana(7,2)); assert(api.give_item(7,'axe',1)); assert(api.give_element(7,'life',3))"));
    host.modules.push(module("on_tick","assert(api.get_player_inventory(7).mana==12); assert(api.has_item(7,'axe')); assert(api.get_element_count(7,'life')==3); assert(api.take_item(7,'axe',1)); assert(api.take_mana(7,2))"));
    for m in &mut host.modules {m.enabled=true;assert!(crate::world_api_validate::validate_source(&m.source).is_empty());}
    let out=host.run_tick(&f.world,&mut f.creatures,&f.players,&mut f.time,&mut f.weather,&[],&[],f.resources);
    assert!(out.crashes.is_empty(),"{:?}",out.crashes);
    let last=out.player_effects.iter().rev().find_map(|e|match e {PlayerEffect::Inventory{player_id:7,balances,..}=>Some(balances),_=>None}).unwrap();
    assert_eq!(last.mana,10);assert_eq!(last.items[0],0);assert_eq!(last.elements[3],3);
}

#[test]
fn nearby_campfire_search_uses_edited_ground_avoids_players_and_rolls_back() {
    let mut f=environment_fixture();
    f.players[0].pos=Vec3::new(8.5,32.0,8.5);
    let mut guest=f.players[0];guest.id=7;guest.pos=Vec3::new(10.5,32.0,8.5);f.players.push(guest);
    let chunk=f.world.chunks.get_mut(&(0,0)).unwrap();
    for x in 0..16 {for z in 0..16 {
        chunk.set_local(x,31,z,BlockType::Stone);
        chunk.set_local(x,32,z,BlockType::ShortGrass);
    }}
    let mut m=module("on_cast",r#"
        local fire=api.place_campfire_near_player(0,6)
        assert(fire and fire.y==32 and fire.burning)
        for _,p in ipairs(api.players()) do
            assert((p.x-fire.x-0.5)^2+(p.z-fire.z-0.5)^2>=4)
        end
        assert(api.get_block(fire.x,fire.y,fire.z)=='campfire')
    "#);
    let (out,_)=f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);
    assert_eq!(out.block_edits.len(),1);
    let mut bad=module("on_cast","assert(api.place_campfire_near_player(0,6)); error('rollback')");
    let (out,_)=f.invoke(&mut bad,"on_cast");assert!(bad.error.is_some());assert!(out.block_edits.is_empty());
    for x in 0..16 {for z in 0..16 {f.world.chunks.get_mut(&(0,0)).unwrap().set_local(x,32,z,BlockType::Water);}}
    let mut m=module("on_cast","assert(api.place_campfire_near_player(0,6)==nil); assert(api.place_campfire_near_player(99,2)==nil)");
    let (out,_)=f.invoke(&mut m,"on_cast");assert!(m.error.is_none(),"{:?}",m.error);assert!(out.block_edits.is_empty());
}

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
    assert_eq!(docs.split("```lua\n").skip(1).count(),4);
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

#[test]
fn automation_lua_changes_are_visible_in_callback_and_rollback_on_error() {
    use crate::automation::{Device,Kind};
    let mut fixture=Fixture::new();
    fixture.world.automation.devices.insert((2,1,0),Device::new(Kind::Workshop,(2,1,0),0));
    let body=r#"
        assert(#api.get_devices() == 1)
        assert(api.configure_device(2,1,0,{recipe="stone"}))
        assert(api.set_device_enabled(2,1,0,false))
        local d=api.get_device(2,1,0)
        assert(d.recipe == "stone" and not d.enabled)
        assert(not api.replace_block(2,1,0,"air"))
    "#;
    let mut failing=module("on_tick",&format!("{body}\nerror('rollback')"));
    let (outcome,_)=fixture.invoke(&mut failing,"on_tick");
    assert!(outcome.player_effects.is_empty());
    assert!(fixture.world.automation.devices[&(2,1,0)].config.enabled);
    let mut successful=module("on_tick",body);
    let (outcome,_)=fixture.invoke(&mut successful,"on_tick");
    assert!(outcome.player_effects.iter().any(|effect|matches!(effect,PlayerEffect::AutomationState{state} if state.devices[&(2,1,0)].config.recipe=="stone" && !state.devices[&(2,1,0)].config.enabled)));
}

#[test]
fn automation_lua_placement_pays_resources_and_failed_callback_refunds() {
    let mut fixture=Fixture::new();
    let body=format!(r#"
        assert(api.place_device({},"signal",2,1,0,0))
        assert(api.get_device(2,1,0).kind == "signal")
        assert(not api.place_device({},"signal",2,1,0,0))
    "#,HOST_PLAYER_ID,HOST_PLAYER_ID);
    let mut failed=module("on_cast",&format!("{body}\nerror('rollback')"));
    let (outcome,_)=fixture.invoke(&mut failed,"on_cast");assert!(outcome.player_effects.is_empty());
    let mut success=module("on_cast",&body);
    let (outcome,_)=fixture.invoke(&mut success,"on_cast");
    let stone=COLLECTIBLE_BLOCKS.iter().position(|b|*b==BlockType::Stone).unwrap();
    assert!(outcome.player_effects.iter().any(|e|matches!(e,PlayerEffect::Inventory{resources,..} if resources[stone]==4)));
    assert!(outcome.player_effects.iter().any(|e|matches!(e,PlayerEffect::AutomationState{state} if state.devices.len()==1)));
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
                finances: Default::default(),
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
