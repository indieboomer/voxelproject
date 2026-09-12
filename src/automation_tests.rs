use super::*;
fn recipes() -> Registry {
    Registry::load().unwrap()
}
#[test]
fn storage_chest_large_transfers_are_atomic_and_pack_without_loss() {
    let mut world=World::new(42); let mut state=State::default();let mut account=Account::default();
    let cell=(0,40,0);let feet=Vec3::new(0.5,40.0,3.5);let r=recipes();let b=balance();
    account_add(&mut account,"resource:oak_wood",4).unwrap();
    apply(&world,&mut state,&mut account,feet,&[],&Action::Place{kind:Kind::Chest,cell,rotation:0,packed:None},b,&r).unwrap();
    assert!(!state.devices[&cell].config.eject_contents);
    world.automation=state.clone();
    account_add(&mut account,"resource:stone",2_000_000).unwrap();
    apply(&world,&mut state,&mut account,feet,&[],&Action::Deposit{cell,item:"resource:stone".into(),amount:2_000_000},b,&r).unwrap();
    let snapshot=state.clone();let balances=account.clone();
    assert!(apply(&world,&mut state,&mut account,feet,&[],&Action::Withdraw{cell,item:"resource:stone".into(),amount:2_000_001},b,&r).is_err());
    assert_eq!(state,snapshot);assert_eq!(account,balances);
    apply(&world,&mut state,&mut account,feet,&[],&Action::Withdraw{cell,item:"resource:stone".into(),amount:1_000_000},b,&r).unwrap();
    apply(&world,&mut state,&mut account,feet,&[],&Action::Pack{cell},b,&r).unwrap();
    assert_eq!(account.packed_devices[0].items["resource:stone"],1_000_000);
    assert_eq!(account_count(&account,"resource:stone"),1_000_000);
    validate_account(&account,b,&r).unwrap();
}

#[test]
fn new_sustained_devices_start_charged_then_stop_and_packing_does_not_refill() {
    for kind in [Kind::Lantern,Kind::DarkAltar,Kind::Shrine] {
        let world=World::new(42);let mut state=State::default();let mut account=Account::default();
        let cell=(0,40,0);let feet=Vec3::new(0.5,40.0,3.5);let b=balance();let r=recipes();
        for (id,n) in &b.def(kind).cost {account_add(&mut account,id,*n).unwrap();}
        apply(&world,&mut state,&mut account,feet,&[],&Action::Place{kind,cell,rotation:0,packed:None},b,&r).unwrap();
        assert_eq!(state.devices[&cell].mana,30);
        let mut pulses=0;
        for _ in 0..30*b.def(kind).duration_ticks {pulses+=state.step(b,&r).len();}
        assert_eq!(pulses,if kind==Kind::Lantern {0}else{30});
        assert!(state.step(b,&r).is_empty());assert_eq!(state.devices[&cell].activity,Activity::InsufficientMana);
        apply(&world,&mut state,&mut account,feet,&[],&Action::Pack{cell},b,&r).unwrap();
        apply(&world,&mut state,&mut account,feet,&[],&Action::Place{kind,cell,rotation:0,packed:Some(0)},b,&r).unwrap();
        assert!(state.step(b,&r).is_empty());assert_eq!(state.devices[&cell].activity,Activity::InsufficientMana);
    }
}
#[test]
fn sustained_parts_pay_once_preserve_runtime_and_pause_without_effects() {
    let b=balance(); let mut r=recipes();
    for kind in [Kind::Lantern,Kind::DarkAltar,Kind::Shrine] {
        let p=(0,30,0); let mut s=State::default(); install(&mut s,kind,p);
        assert!(s.step(b,&r).is_empty());
        assert_eq!(s.devices[&p].activity,Activity::InsufficientMana);
        s.devices.get_mut(&p).unwrap().mana=1;
        let pulses=s.step(b,&r);
        assert_eq!(pulses.len(),usize::from(kind!=Kind::Lantern));
        assert_eq!(s.devices[&p].mana,0);
        let ticks=s.devices[&p].powered_ticks;
        let saved=serde_json::to_string(&s).unwrap();
        s=serde_json::from_str(&saved).unwrap(); s.validate(b,&r).unwrap();
        s.devices.get_mut(&p).unwrap().config.enabled=false;
        assert!(s.step(b,&r).is_empty());
        assert_eq!(s.devices[&p].powered_ticks,ticks);
        s.devices.get_mut(&p).unwrap().config.enabled=true;
        for _ in 0..ticks {assert!(s.step(b,&r).is_empty());assert_eq!(s.devices[&p].activity,Activity::Working);}
        assert!(s.step(b,&r).is_empty());
        assert_eq!(s.devices[&p].activity,Activity::InsufficientMana);
        r.mana_free=true;s.step(b,&r);
        assert_eq!(s.devices[&p].activity,Activity::Working);
        assert_eq!(s.devices[&p].mana,0);r.mana_free=false;
    }
}
#[test]
fn aura_range_healing_cap_deaths_and_fixed_tick_catchup() {
    use crate::creature::{Creatures,CreatureKind};
    let p=(0,30,0);let mut c=Creatures::new();
    let near=c.spawn_one(CreatureKind::Sheep,center(p)+Vec3::X*6.0,1);
    let far=c.spawn_one(CreatureKind::Sheep,center(p)+Vec3::X*6.01,2);
    let max=CreatureKind::Sheep.max_health();
    let health=|c:&Creatures,id| c.snapshot_with_ids().into_iter().find(|v|v.0==id).unwrap().3;
    let mut s=State::default();install(&mut s,Kind::DarkAltar,p);
    s.devices.get_mut(&p).unwrap().mana=2;
    let mut clock=Clock::default();let r=recipes();
    clock.advance_with(1.0,&mut s,balance(),&r,|a|apply_auras(&mut c,a));
    assert_eq!(health(&c,near),max-3.0);assert_eq!(health(&c,far),max);
    clock.advance_with(0.1,&mut s,balance(),&r,|a|apply_auras(&mut c,a));
    assert_eq!(health(&c,near),max-6.0);
    for _ in 0..4 {apply_auras(&mut c,&[(p,Kind::Shrine)]);}
    assert_eq!(health(&c,near),max);
    c.damage(near,max-1.0);apply_auras(&mut c,&[(p,Kind::DarkAltar)]);
    assert_eq!(c.combat_deaths.len(),1);
    assert_eq!(c.snapshot_with_ids().len(),1);
}
#[test]
fn tall_parts_reserve_all_cells_and_personal_charge_is_atomic() {
    let b=balance();let r=recipes();let p=(0,30,0);let feet=Vec3::new(0.5,30.0,3.5);
    let mut world=World::new(42);let mut s=State::default();let mut a=Account::default();
    a.packed_devices.push(Device::new(Kind::Lantern,p,0));a.mana=20;
    let place=Action::Place{kind:Kind::Lantern,cell:p,rotation:0,packed:Some(0)};
    install(&mut s,Kind::Chest,(0,32,0));
    assert!(apply(&world,&mut s,&mut a,feet,&[],&place,b,&r).is_err());
    assert_eq!(a.packed_devices.len(),1);s.devices.clear();
    assert!(apply(&world,&mut s,&mut a,feet,&[Vec3::new(0.5,32.0,0.5)],&place,b,&r).is_err());
    apply(&world,&mut s,&mut a,feet,&[],&place,b,&r).unwrap();
    world.automation=s.clone();
    for y in 30..33 {assert_eq!(world.get_block(0,y,0),BlockType::AutomationDevice);}
    world.set_block(0,32,0,BlockType::Stone);
    assert_eq!(world.get_block(0,32,0),BlockType::AutomationDevice);
    assert_eq!(s.device_at((0,32,0)).unwrap().cell,p);
    apply(&world,&mut s,&mut a,feet,&[],&Action::Charge{cell:p,amount:10},b,&r).unwrap();
    assert_eq!(a.mana,10);assert_eq!(s.devices[&p].mana,10);
    assert!(apply(&world,&mut s,&mut a,feet,&[],&Action::Charge{cell:p,amount:60},b,&r).is_err());
    assert_eq!(a.mana,10);assert_eq!(s.devices[&p].mana,10);
    apply(&world,&mut s,&mut a,feet,&[],&Action::Pack{cell:p},b,&r).unwrap();
    assert_eq!(a.packed_devices[0].mana,10);assert!(s.device_at((0,32,0)).is_none());
}
#[test]
fn collector_connections_power_all_sustained_parts() {
    for kind in [Kind::Lantern,Kind::DarkAltar,Kind::Shrine] {
        let mut s=State::default();install(&mut s,Kind::Collector,(0,30,0));
        install(&mut s,kind,(1,30,0));let r=recipes();
        let mut active=false;
        for _ in 0..30 {s.step(balance(),&r);active |= s.devices[&(1,30,0)].activity==Activity::Working;}
        assert!(active,"{kind:?}");
    }
}
#[test]
fn smelter_resource_port_accepts_ore_and_wood_from_a_feeding_chest() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    install(&mut s, Kind::Chest, (1, 30, -1));
    install(&mut s, Kind::Smelter, (1, 30, 0));
    install(&mut s, Kind::Chest, (2, 30, 0));
    let chest = s.devices.get_mut(&(1, 30, -1)).unwrap();
    chest.rotation = 1;
    chest.config.eject_contents = false;
    add(&mut chest.items, "resource:copper_ore", 2);
    add(&mut chest.items, "resource:oak_wood", 1);
    s.devices
        .get_mut(&(1, 30, 0))
        .unwrap()
        .config
        .eject_contents = false;
    for _ in 0..5 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&(2, 30, 0)].items["resource:copper"], 2);
    assert_eq!(s.devices[&(1, 30, 0)].fuel_heat, 0);
    assert!(s.devices[&(1, 30, -1)].items.is_empty());
}
#[test]
fn smelter_converts_every_metal_ore_on_the_first_tick() {
    let b = balance();
    let r = recipes();
    let p = (0, 30, 0);
    for (ore, metal) in &b.smelting.recipes {
        let mut s = State::default();
        install(&mut s, Kind::Smelter, p);
        let d = s.devices.get_mut(&p).unwrap();
        add(&mut d.items, ore, 1);
        d.mana = 1;
        s.step(b, &r);
        assert_eq!(s.devices[&p].output[metal], 1);
        assert!(s.devices[&p].items.is_empty());
        assert_eq!(s.devices[&p].mana, 0);
        assert!(s.devices[&p].batch.is_none());
        s.step(b, &r);
        assert_eq!(s.devices[&p].output[metal], 1);
        s.validate(b, &r).unwrap();
    }
}
#[test]
fn smelter_fuels_are_efficient_and_leftover_heat_survives_saving() {
    let b = balance();
    let r = recipes();
    let p = (0, 30, 0);
    for (fuel, yield_count) in [
        ("resource:oak_wood", 2),
        ("resource:spruce_wood", 2),
        ("resource:coal", 8),
    ] {
        let mut s = State::default();
        install(&mut s, Kind::Smelter, p);
        let d = s.devices.get_mut(&p).unwrap();
        add(&mut d.items, "resource:copper_ore", 1);
        add(&mut d.items, fuel, 1);
        s.step(b, &r);
        assert_eq!(s.devices[&p].fuel_heat, yield_count - 1);
        assert_eq!(s.devices[&p].output["resource:copper"], 1);
        s = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        s.validate(b, &r).unwrap();
        add(
            &mut s.devices.get_mut(&p).unwrap().items,
            "resource:copper_ore",
            yield_count,
        );
        for _ in 0..10 {
            s.step(b, &r);
        }
        assert_eq!(s.devices[&p].output["resource:copper"], yield_count);
        assert_eq!(s.devices[&p].items["resource:copper_ore"], 1);
        assert_eq!(s.devices[&p].fuel_heat, 0);
        assert_eq!(s.devices[&p].activity, Activity::MissingFuel);
    }
}
#[test]
fn smelter_blockage_never_consumes_fuel_and_mana_free_mode_preserves_it() {
    let b = balance();
    let mut r = recipes();
    let p = (0, 30, 0);
    let mut s = State::default();
    install(&mut s, Kind::Smelter, p);
    install(&mut s, Kind::Chest, (1, 30, 0));
    let d = s.devices.get_mut(&p).unwrap();
    d.config.eject_contents = false;
    add(&mut d.items, "resource:iron_ore", 2);
    add(&mut d.items, "resource:coal", 1);
    add(
        &mut s.devices.get_mut(&(1, 30, 0)).unwrap().items,
        "resource:stone",
        b.def(Kind::Chest).item_capacity,
    );
    for _ in 0..20 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&p].activity, Activity::BlockedOutput);
    assert_eq!(s.devices[&p].items["resource:coal"], 1);
    assert_eq!(s.devices[&p].items["resource:iron_ore"], 2);
    s.devices.get_mut(&(1, 30, 0)).unwrap().items.clear();
    r.mana_free = true;
    s.step(b, &r);
    s.step(b, &r);
    assert_eq!(s.devices[&(1, 30, 0)].items["resource:iron"], 2);
    assert_eq!(s.devices[&p].items["resource:coal"], 1);
    assert_eq!(s.devices[&p].fuel_heat, 0);
}
#[test]
fn collector_powers_smelter_and_feeds_a_chest_without_material_duplication() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    install(&mut s, Kind::Collector, (0, 30, 0));
    install(&mut s, Kind::Smelter, (1, 30, 0));
    install(&mut s, Kind::Chest, (2, 30, 0));
    let d = s.devices.get_mut(&(1, 30, 0)).unwrap();
    d.config.eject_contents = false;
    add(&mut d.items, "resource:copper_ore", 10);
    for _ in 0..110 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&(2, 30, 0)].items["resource:copper"], 10);
    assert_eq!(s.devices.values().map(Device::item_count).sum::<u32>(), 10);
}
#[test]
fn machine_event_counters_capture_transfer_even_when_stock_is_unchanged() {
    let mut s = State::default();
    let b = balance();
    let r = recipes();
    for x in 0..3 {
        install(&mut s, Kind::Channel, (x, 30, 0));
    }
    for x in 0..2 {
        add(
            &mut s.devices.get_mut(&(x, 30, 0)).unwrap().items,
            "resource:stone",
            1,
        );
    }
    s.step(b, &r);
    assert_eq!(s.devices[&(1, 30, 0)].item_count(), 1);
    assert_eq!(s.devices[&(1, 30, 0)].feedback_events[1], 2);
    let decoded: State = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(
        decoded.devices[&(1, 30, 0)].feedback_events,
        s.devices[&(1, 30, 0)].feedback_events
    );
}
#[test]
fn mana_free_machine_production_needs_no_balance_and_toggle_restores_costs() {
    let mut r = recipes();
    r.mana_free = true;
    let b = balance();
    let p = (0, 30, 0);
    let mut s = State::default();
    install(&mut s, Kind::Condenser, p);
    s.step(b, &r);
    assert_eq!(s.devices[&p].batch.as_ref().unwrap().mana_cost, 0);
    r.mana_free = false;
    for _ in 0..b.def(Kind::Condenser).duration_ticks {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&p].output[element_id(0)], 1);
    s.step(b, &r);
    assert_eq!(s.devices[&p].activity, Activity::InsufficientMana);
    assert_eq!(s.devices[&p].mana, 0);
}
fn install(state: &mut State, kind: Kind, p: Cell) {
    state.devices.insert(p, Device::new(kind, p, 0));
}
#[test]
fn collector_clusters_share_supply_and_conversion_always_loses_mana() {
    let b = balance();
    let r = recipes();
    let mut one = State::default();
    let mut many = State::default();
    install(&mut one, Kind::Collector, (0, 30, 0));
    for z in 0..8 {
        install(&mut many, Kind::Collector, (0, 30, z));
    }
    for _ in 0..100 {
        one.step(b, &r);
        many.step(b, &r);
    }
    assert_eq!(
        one.devices.values().map(|d| d.mana).sum::<u32>(),
        many.devices.values().map(|d| d.mana).sum::<u32>()
    );
    let mut invalid = b.clone();
    invalid.dissipator_yields[0] = invalid.condenser_costs[0];
    assert!(invalid.validate().is_err());
}
#[test]
fn ordered_workshop_ignores_arrival_order_and_pays_once() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    let p = (0, 30, 0);
    install(&mut s, Kind::Workshop, p);
    let d = s.devices.get_mut(&p).unwrap();
    d.config.recipe = "stone".into();
    d.mana = 20;
    add(&mut d.items, element_id(2), 1);
    add(&mut d.items, element_id(0), 1);
    s.step(b, &r);
    let paid = s.devices[&p].mana;
    assert_eq!(paid, 18);
    assert!(s.devices[&p].items.is_empty());
    for _ in 0..b.def(Kind::Workshop).duration_ticks {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&p].mana, paid);
    assert_eq!(s.devices[&p].output["resource:stone"], 1);
}
#[test]
fn fixed_ticks_and_save_load_preserve_inflight_state() {
    let b = balance();
    let r = recipes();
    let mut a = State::default();
    install(&mut a, Kind::Condenser, (0, 30, 0));
    a.devices.get_mut(&(0, 30, 0)).unwrap().mana = 60;
    let mut other = a.clone();
    let mut clock = Clock::default();
    let mut clock2 = Clock::default();
    for _ in 0..30 {
        clock.advance(0.1, &mut a, b, &r);
    }
    for _ in 0..60 {
        clock2.advance(0.05, &mut other, b, &r);
    }
    assert_eq!(a, other);
    let mut loaded: State = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
    loaded.validate(b, &r).unwrap();
    assert_eq!(loaded, a);
    for _ in 0..20 {
        loaded.step(b, &r);
        a.step(b, &r);
    }
    assert_eq!(loaded, a);
}

#[test]
fn complete_collector_to_chest_loop_stops_and_resumes_with_hysteresis() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    for (x, kind) in [
        Kind::Collector,
        Kind::Vessel,
        Kind::Condenser,
        Kind::Workshop,
        Kind::Valve,
        Kind::Chest,
    ]
    .into_iter()
    .enumerate()
    {
        install(&mut s, kind, (x as i32, 30, 0));
    }
    s.devices.get_mut(&(2, 30, 0)).unwrap().config.element = 3;
    s.devices.get_mut(&(3, 30, 0)).unwrap().config.recipe = "sheep".into();
    let valve = s.devices.get_mut(&(4, 30, 0)).unwrap();
    valve.config.valve_matter = true;
    valve.config.invert = true;
    install(&mut s, Kind::Sensor, (4, 31, 0));
    let sensor = s.devices.get_mut(&(4, 31, 0)).unwrap();
    sensor.config.sensor_target = (5, 30, 0);
    sensor.config.sensor_item = "creature:sheep".into();
    sensor.config.lower = 1;
    sensor.config.upper = 3;
    for _ in 0..15000 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&(5, 30, 0)].items["creature:sheep"], 3);
    assert!(!s.devices[&(4, 30, 0)].open());
    assert_eq!(s.devices[&(3, 30, 0)].activity, Activity::BlockedOutput);
    let batch = s.devices[&(3, 30, 0)].batch.clone();
    for _ in 0..100 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&(3, 30, 0)].batch, batch);
    take(
        &mut s.devices.get_mut(&(5, 30, 0)).unwrap().items,
        "creature:sheep",
        1,
    );
    s.step(b, &r);
    assert!(!s.devices[&(4, 30, 0)].open()); // still above lower threshold
    take(
        &mut s.devices.get_mut(&(5, 30, 0)).unwrap().items,
        "creature:sheep",
        1,
    );
    s.step(b, &r);
    assert!(s.devices[&(4, 30, 0)].open());
    for _ in 0..2000 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&(5, 30, 0)].items["creature:sheep"], 3);
    s.validate(b, &r).unwrap();
}

#[test]
fn blocked_paid_batch_resumes_without_second_payment() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    let p = (0, 30, 0);
    install(&mut s, Kind::Workshop, p);
    let d = s.devices.get_mut(&p).unwrap();
    d.config.recipe = "stone".into();
    d.mana = 20;
    add(&mut d.items, element_id(0), 1);
    add(&mut d.items, element_id(2), 1);
    s.step(b, &r);
    assert_eq!(s.devices[&p].mana, 18);
    // New arrivals occupy all storage after ingredients have been reserved.
    add(&mut s.devices.get_mut(&p).unwrap().items, element_id(4), 64);
    for _ in 0..100 {
        s.step(b, &r);
    }
    assert_eq!(s.devices[&p].activity, Activity::BlockedOutput);
    assert_eq!(s.devices[&p].mana, 18);
    assert!(s.devices[&p].output.is_empty());
    take(&mut s.devices.get_mut(&p).unwrap().items, element_id(4), 1);
    s.step(b, &r);
    assert_eq!(s.devices[&p].output["resource:stone"], 1);
    assert_eq!(s.devices[&p].mana, 18);
    assert!(s.devices[&p].batch.is_none());
}

#[test]
fn competing_branches_conserve_mana_and_items_and_obey_throughput() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    install(&mut s, Kind::Conduit, (0, 30, 0));
    let d = s.devices.get_mut(&(0, 30, 0)).unwrap();
    d.mana = 12;
    d.config.outputs = vec![Face::East, Face::South];
    install(&mut s, Kind::Vessel, (1, 30, 0));
    install(&mut s, Kind::Vessel, (0, 30, 1));
    s.devices.get_mut(&(0, 30, 1)).unwrap().rotation = 1;
    s.step(b, &r);
    assert_eq!(s.devices[&(0, 30, 0)].mana, 9);
    for _ in 0..20 {
        s.step(b, &r);
        assert_eq!(s.devices.values().map(|d| d.mana).sum::<u32>(), 12);
    }
    assert!(s.devices[&(1, 30, 0)].mana > 0);
    assert!(s.devices[&(0, 30, 1)].mana > 0);
    let mut s = State::default();
    install(&mut s, Kind::Splitter, (0, 30, 0));
    add(
        &mut s.devices.get_mut(&(0, 30, 0)).unwrap().items,
        "resource:stone",
        12,
    );
    install(&mut s, Kind::Chest, (1, 30, 0));
    install(&mut s, Kind::Chest, (0, 30, 1));
    s.devices.get_mut(&(0, 30, 1)).unwrap().rotation = 1;
    for _ in 0..12 {
        s.step(b, &r);
        assert_eq!(s.devices.values().map(Device::item_count).sum::<u32>(), 12);
    }
    assert_eq!(s.devices[&(1, 30, 0)].item_count(), 6);
    assert_eq!(s.devices[&(0, 30, 1)].item_count(), 6);
}

#[test]
fn rotation_removal_vertical_channels_and_filter_reconnect() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    install(&mut s, Kind::Channel, (0, 30, 0));
    install(&mut s, Kind::Channel, (0, 31, 0));
    s.devices.get_mut(&(0, 30, 0)).unwrap().config.outputs = vec![Face::Up];
    s.devices.get_mut(&(0, 31, 0)).unwrap().config.input = Face::Down;
    install(&mut s, Kind::Chest, (1, 31, 0));
    add(
        &mut s.devices.get_mut(&(0, 30, 0)).unwrap().items,
        "resource:stone",
        1,
    );
    s.step(b, &r);
    assert_eq!(s.devices[&(0, 31, 0)].item_count(), 1);
    assert_eq!(s.devices[&(1, 31, 0)].item_count(), 0);
    s.devices.get_mut(&(0, 31, 0)).unwrap().rotation = 1;
    s.step(b, &r);
    assert_eq!(s.devices[&(0, 31, 0)].item_count(), 1);
    s.devices.get_mut(&(0, 31, 0)).unwrap().rotation = 0;
    let chest = s.devices.remove(&(1, 31, 0)).unwrap();
    s.step(b, &r);
    assert_eq!(s.devices[&(0, 31, 0)].item_count(), 1);
    s.devices.insert(chest.cell, chest);
    s.step(b, &r);
    assert_eq!(s.devices[&(1, 31, 0)].item_count(), 1);
    let mut s = State::default();
    install(&mut s, Kind::Splitter, (0, 30, 0));
    install(&mut s, Kind::Chest, (0, 30, 1));
    s.devices.get_mut(&(0, 30, 1)).unwrap().rotation = 1;
    let d = s.devices.get_mut(&(0, 30, 0)).unwrap();
    d.config.routing = Routing::Filter;
    d.config.filter = "resource:stone".into();
    add(&mut d.items, "resource:stone", 1);
    s.step(b, &r);
    assert_eq!(s.devices[&(0, 30, 0)].item_count(), 1); // primary east disconnected
    install(&mut s, Kind::Chest, (1, 30, 0));
    s.step(b, &r);
    assert_eq!(s.devices[&(1, 30, 0)].item_count(), 1);
}

#[test]
fn packing_and_failed_placement_preserve_inventory_and_progress() {
    let world = World::new(42);
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    let mut account = Account::default();
    let p = (0, 30, 0);
    let feet = Vec3::new(0.5, 30.0, 3.5);
    install(&mut s, Kind::Condenser, p);
    s.devices.get_mut(&p).unwrap().mana = 60;
    s.step(b, &r);
    let original = s.devices[&p].clone();
    apply(
        &world,
        &mut s,
        &mut account,
        feet,
        &[feet],
        &Action::Pack { cell: p },
        b,
        &r,
    )
    .unwrap();
    assert!(s.devices.is_empty());
    assert_eq!(account.packed_devices[0], original);
    let saved = serde_json::to_string(&account).unwrap();
    let mut account: Account = serde_json::from_str(&saved).unwrap();
    let before = account.clone();
    assert!(apply(
        &world,
        &mut s,
        &mut account,
        feet,
        &[feet],
        &Action::Place {
            kind: Kind::Condenser,
            cell: (99, 30, 0),
            rotation: 0,
            packed: Some(0)
        },
        b,
        &r
    )
    .is_err());
    assert_eq!(account, before);
    assert!(s.devices.is_empty());
    apply(
        &world,
        &mut s,
        &mut account,
        feet,
        &[feet],
        &Action::Place {
            kind: Kind::Condenser,
            cell: p,
            rotation: 0,
            packed: Some(0),
        },
        b,
        &r,
    )
    .unwrap();
    assert_eq!(s.devices[&p], original);
    assert!(account.packed_devices.is_empty());
}

#[test]
fn conversion_cycle_loses_actual_mana_for_each_element() {
    let b = balance();
    let r = recipes();
    for i in 0..5 {
        let mut s = State::default();
        install(&mut s, Kind::Condenser, (0, 30, 0));
        install(&mut s, Kind::Dissipator, (1, 30, 0));
        s.devices.get_mut(&(0, 30, 0)).unwrap().config.element = i;
        s.devices.get_mut(&(0, 30, 0)).unwrap().mana = b.condenser_costs[i];
        s.devices.get_mut(&(1, 30, 0)).unwrap().config.element = i;
        for _ in 0..100 {
            s.step(b, &r);
        }
        assert_eq!(
            s.devices.values().map(|d| d.mana).sum::<u32>(),
            b.dissipator_yields[i]
        );
        assert_eq!(s.devices.values().map(Device::item_count).sum::<u32>(), 0);
    }
}

#[test]
fn signal_branch_numeric_pulse_and_disconnect_clear_valves() {
    let b = balance();
    let r = recipes();
    let mut s = State::default();
    install(&mut s, Kind::Chest, (3, 30, 0));
    add(
        &mut s.devices.get_mut(&(3, 30, 0)).unwrap().items,
        "resource:stone",
        10,
    );
    install(&mut s, Kind::Sensor, (0, 32, 0));
    install(&mut s, Kind::Signal, (0, 31, 0));
    install(&mut s, Kind::Signal, (1, 31, 0));
    install(&mut s, Kind::Valve, (0, 30, 0));
    install(&mut s, Kind::Valve, (1, 30, 0));
    let c = &mut s.devices.get_mut(&(0, 32, 0)).unwrap().config;
    c.sensor_target = (3, 30, 0);
    c.signal_mode = SignalMode::Pulse;
    s.step(b, &r);
    assert!(s.devices[&(0, 30, 0)].open());
    assert!(s.devices[&(1, 30, 0)].open());
    s.step(b, &r);
    assert!(!s.devices[&(0, 30, 0)].open());
    s.devices.get_mut(&(0, 32, 0)).unwrap().config.signal_mode = SignalMode::Numeric;
    s.step(b, &r);
    assert_eq!(s.devices[&(1, 30, 0)].signal, 10);
    s.devices.remove(&(0, 31, 0));
    s.step(b, &r);
    assert_eq!(s.devices[&(1, 30, 0)].signal, 0);
}

#[test]
fn failed_creature_release_keeps_the_figurine() {
    let world = World::new(42);
    let mut creatures = crate::creature::Creatures::new();
    let mut account = Account::default();
    account.production_goods.insert("creature:sheep".into(), 1);
    let before = account.clone();
    assert!(release(
        &world,
        &mut creatures,
        &mut account,
        Vec3::new(0.5, 30.0, 0.5),
        &[],
        (0, 30, 0),
        "creature:sheep"
    )
    .is_err());
    assert_eq!(account, before);
    assert!(creatures.snapshot().is_empty());
}

#[test]
fn bound_creature_release_consumes_exactly_one_and_spawns_once() {
    let mut world = World::new(42);
    world.ensure_chunk_loaded(0, 0);
    for x in 3..=5 {
        for z in 3..=5 {
            world.set_block(x, 29, z, BlockType::Stone);
            for y in 30..35 {
                world.set_block(x, y, z, BlockType::Air);
            }
        }
    }
    let mut creatures = crate::creature::Creatures::new();
    let mut account = Account::default();
    account.production_goods.insert("creature:sheep".into(), 1);
    let feet = Vec3::new(0.5, 30.0, 0.5);
    release(
        &world,
        &mut creatures,
        &mut account,
        feet,
        &[feet],
        (0, 30, 0),
        "creature:sheep",
    )
    .unwrap();
    assert_eq!(account_count(&account, "creature:sheep"), 0);
    assert_eq!(creatures.snapshot().len(), 1);
    assert!(release(
        &world,
        &mut creatures,
        &mut account,
        feet,
        &[feet],
        (0, 30, 0),
        "creature:sheep"
    )
    .is_err());
    assert_eq!(creatures.snapshot().len(), 1);
}
