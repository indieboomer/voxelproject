//! Authoritative outlet: commit cargo removal only after the world accepts it.
use super::*;

pub fn eject_chests(
    world: &mut World,
    creatures: &mut crate::creature::Creatures,
    loot: &mut crate::loot::Effects,
    players: &[Vec3],
) {
    let b = balance();
    let tick = world.automation.tick;
    let candidates: Vec<_> = world
        .automation
        .devices
        .iter()
        .filter(|(_, d)| {
            matches!(d.kind, Kind::Chest | Kind::Smelter)
                && d.config.enabled
                && d.config.eject_contents
                && !d.outgoing().is_empty()
                && tick.saturating_sub(d.last_ejection_tick) >= b.def(d.kind).duration_ticks as u64
        })
        .map(|(&p, d)| {
            let (item, n) = d
                .outgoing()
                .iter()
                .nth(d.cursor as usize % d.outgoing().len())
                .unwrap();
            (
                p,
                item.clone(),
                (*n).min(b.def(d.kind).throughput),
                d.cursor,
            )
        })
        .collect();
    for (cell, item, amount, sequence) in candidates {
        let device = world.automation.devices.get_mut(&cell).unwrap();
        device.last_ejection_tick = tick;
        device.cursor = device.cursor.wrapping_add(1);
        let origin = center(cell) - Vec3::Y * 0.5;
        let loaded = world
            .chunks
            .contains_key(&crate::voxel::chunk::world_to_chunk(cell.0, cell.2));
        let creature = item.starts_with("creature:");
        let success = if !loaded {
            false
        } else if creature {
            // Reuse manual bound-creature release, including fish habitat and
            // population checks; the temporary account is only a reservation.
            let mut reserved = Account::default();
            reserved.production_goods.insert(item.clone(), 1);
            reserved.revision = tick;
            release(
                world,
                creatures,
                &mut reserved,
                origin,
                players,
                cell,
                &item,
            )
            .is_ok()
        } else {
            loot.eject(
                world,
                cell,
                &item,
                amount,
                tick.wrapping_add(sequence as u64),
            )
        };
        let device = world.automation.devices.get_mut(&cell).unwrap();
        let before = device.activity;
        if success {
            take(
                device.outgoing_mut(),
                &item,
                if creature { 1 } else { amount },
            );
            device.feedback(0);
            device.feedback(1);
            device.activity = if device.outgoing().is_empty() {
                Activity::Idle
            } else {
                Activity::Working
            };
        } else {
            device.activity = Activity::BlockedOutput;
        }
        if before != device.activity {
            device.feedback(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smelter_ejects_only_finished_metal_and_keeps_remaining_fuel_heat() {
        let (mut world, mut creatures, mut loot, p) = fixture();
        let mut smelter = Device::new(Kind::Smelter, p, 0);
        add(&mut smelter.items, "resource:copper_ore", 2);
        add(&mut smelter.items, "resource:coal", 1);
        world.automation.devices.insert(p, smelter);
        world.automation.step(balance(), &Registry::load().unwrap());
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert_eq!(loot.drops.len(), 1);
        assert_eq!(loot.drops[0].contents, vec![(BlockType::Copper, 2)]);
        assert_eq!(world.automation.devices[&p].fuel_heat, 6);
        assert!(world.automation.devices[&p].output.is_empty());
        let mut state = world.automation.clone();
        let mut account = Account::default();
        let feet = Vec3::new(8.5, 25.0, 12.5);
        apply(
            &world,
            &mut state,
            &mut account,
            feet,
            &[feet],
            &Action::Pack { cell: p },
            balance(),
            &Registry::load().unwrap(),
        )
        .unwrap();
        let loaded: Account =
            serde_json::from_str(&serde_json::to_string(&account).unwrap()).unwrap();
        assert_eq!(loaded.packed_devices[0].fuel_heat, 6);
    }
    fn fixture() -> (
        World,
        crate::creature::Creatures,
        crate::loot::Effects,
        Cell,
    ) {
        let mut world = World::new(1);
        let mut chunk = crate::voxel::chunk::Chunk::new(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_local(x, 24, z, BlockType::Stone);
            }
        }
        world.chunks.insert((0, 0), chunk);
        let p = (8, 25, 8);
        world
            .automation
            .devices
            .insert(p, Device::new(Kind::Chest, p, 0));
        world.automation.devices.get_mut(&p).unwrap().config.eject_contents = true;
        world.automation.tick = 10;
        (
            world,
            crate::creature::Creatures::new(),
            crate::loot::Effects::default(),
            p,
        )
    }
    #[test]
    fn chest_tosses_bags_and_pickup_is_atomic_for_resources_tools_and_elements() {
        for item in ["resource:stone", "item:axe", "element:earth"] {
            let (mut world, mut creatures, mut loot, p) = fixture();
            add(
                &mut world.automation.devices.get_mut(&p).unwrap().items,
                item,
                2,
            );
            eject_chests(&mut world, &mut creatures, &mut loot, &[]);
            assert!(world.automation.devices[&p].items.is_empty());
            assert_eq!(loot.drops.len(), 1);
            let drop = &loot.drops[0];
            assert!(drop.valid_machine());
            assert_eq!(
                drop.display_position(),
                Vec3::from_array(drop.launch.unwrap())
            );
            eject_chests(&mut world, &mut creatures, &mut loot, &[]);
            assert_eq!(loot.drops.len(), 1);
            loot.update(121.0, true);
            assert_eq!(loot.drops.len(), 1); // machine output never expires
            let landing = Vec3::from_array(loot.drops[0].pos);
            let mut account = Account::default();
            let initial = account_count(&account, item);
            account_add(&mut account, item, u32::MAX - initial).unwrap();
            let before = account.clone();
            loot.collect(&world, landing, &mut account);
            assert_eq!(account, before);
            assert_eq!(loot.drops.len(), 1);
            account = Account::default();
            loot.collect(&world, landing, &mut account);
            assert_eq!(account_count(&account, item), initial + 2);
            assert!(loot.drops.is_empty());
            loot.collect(&world, landing, &mut account);
            assert_eq!(account_count(&account, item), initial + 2);
        }
    }
    #[test]
    fn chest_releases_living_creatures_once_and_storage_mode_retains_contents() {
        let (mut world, mut creatures, mut loot, p) = fixture();
        let d = world.automation.devices.get_mut(&p).unwrap();
        add(&mut d.items, "creature:sheep", 1);
        assert!(d.config.eject_contents);
        assert!(!d
            .ports(balance())
            .iter()
            .any(|p| p.1 == Network::Matter && p.2 == Direction::Out));
        d.config.eject_contents = false;
        assert!(d
            .ports(balance())
            .iter()
            .any(|p| p.1 == Network::Matter && p.2 == Direction::Out));
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert!(creatures.snapshot().is_empty());
        world
            .automation
            .devices
            .get_mut(&p)
            .unwrap()
            .config
            .eject_contents = true;
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert_eq!(creatures.snapshot().len(), 1);
        assert!(world.automation.devices[&p].items.is_empty());
        assert!(loot.drops.is_empty());
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert_eq!(creatures.snapshot().len(), 1);
    }
    #[test]
    fn blocked_outlet_keeps_goods_until_room_exists() {
        let (mut world, mut creatures, mut loot, p) = fixture();
        add(
            &mut world.automation.devices.get_mut(&p).unwrap().items,
            "resource:stone",
            5,
        );
        assert!(loot.eject(&world, p, "resource:stone", 1, 0));
        loot.drops = vec![loot.drops[0].clone(); 48];
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert_eq!(world.automation.devices[&p].item_count(), 5);
        assert_eq!(
            world.automation.devices[&p].activity,
            Activity::BlockedOutput
        );
        loot.drops.clear();
        world.automation.tick = 20;
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert_eq!(world.automation.devices[&p].item_count(), 3);
        assert_eq!(loot.drops[0].contents, vec![(BlockType::Stone, 2)]);
        world.chunks.clear();
        world.automation.tick = 30;
        eject_chests(&mut world, &mut creatures, &mut loot, &[]);
        assert_eq!(world.automation.devices[&p].item_count(), 3);
    }
}
