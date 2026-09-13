//! Small optional expedition loop. Progress belongs to the authoritative player account.
use crate::{
    crafting::Account,
    voxel::{BlockType, World, COLLECTIBLE_BLOCKS},
};
use glam::Vec3;
use serde::{Deserialize, Serialize};
pub type Cell = (i32, i32, i32);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub stage: u8,
    pub home: Option<Cell>,
    pub explored_depths: bool,
    pub crafted_tool: bool,
    pub recoveries: u32,
}
impl Progress {
    pub fn valid(&self) -> bool {
        self.stage <= 3
            && self.home.is_none_or(|(x, y, z)| {
                x.unsigned_abs() < 1_000_000
                    && z.unsigned_abs() < 1_000_000
                    && (1..crate::voxel::chunk::CHUNK_Y - 2).contains(&y)
            })
    }
    pub fn title(&self) -> &'static str {
        match self.stage {
            0 => "A place by the fire",
            1 => "Into the depths",
            2 => "Forge your own path",
            _ => "Wayfinder",
        }
    }
    pub fn objective(&self) -> &'static str {
        match self.stage {
            0 => "Bring 6 oak wood and 4 stone to a campkeeper.",
            1 => "Explore underground at Y 12 or below, then return to a campkeeper.",
            2 => "Craft a new axe, pickaxe or sword, then return to a campkeeper.",
            _ => "The world is yours. Build, explore, or write a new world rule.",
        }
    }
    pub fn reward(&self) -> &'static str {
        match self.stage {
            0 => "2 crystals + 20 mana",
            1 => "3 iron + 30 mana",
            2 => "2 redstone + 40 mana and the Wayfinder title",
            _ => "All three contracts completed",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Action {
    Rest { camp: Cell },
    Claim { camp: Cell },
}
impl Action {
    pub fn camp(self) -> Cell {
        match self {
            Self::Rest { camp } | Self::Claim { camp } => camp,
        }
    }
}

pub fn count(account: &Account, block: BlockType) -> u32 {
    COLLECTIBLE_BLOCKS
        .iter()
        .position(|b| *b == block)
        .map_or(0, |i| account.resources[i])
}
fn resource(account: &mut Account, block: BlockType, delta: i64) -> Result<(), String> {
    let i = COLLECTIBLE_BLOCKS
        .iter()
        .position(|b| *b == block)
        .ok_or("Unknown reward")?;
    let next = i64::from(account.resources[i]) + delta;
    account.resources[i] =
        u32::try_from(next).map_err(|_| "Insufficient supplies or inventory full")?;
    Ok(())
}
pub fn ready(account: &Account) -> bool {
    match account.adventure.stage {
        0 => count(account, BlockType::OakWood) >= 6 && count(account, BlockType::Stone) >= 4,
        1 => account.adventure.explored_depths,
        2 => account.adventure.crafted_tool,
        _ => false,
    }
}
pub fn clear_feet(world: &World, p: Cell) -> bool {
    if !(1..crate::voxel::chunk::CHUNK_Y-2).contains(&p.1)
        || p.0.unsigned_abs()>=1_000_000 || p.2.unsigned_abs()>=1_000_000 {return false;}
    world.get_block(p.0, p.1 - 1, p.2).is_solid()
        && (0..=1).all(|dy| world.get_block(p.0, p.1 + dy, p.2) == BlockType::Air)
}
pub fn feet(p: Cell) -> Vec3 {
    Vec3::new(p.0 as f32 + 0.5, p.1 as f32, p.2 as f32 + 0.5)
}
pub fn cell(p: Vec3) -> Cell {
    (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
}

/// Campkeepers are fixed, noncombat inhabitants derived from a campfire and a clear
/// adjacent standing place. Every peer derives the same NPC from authoritative blocks.
pub fn guide_position(world: &World, camp: Cell) -> Option<Cell> {
    if world.get_block(camp.0, camp.1, camp.2) != BlockType::Campfire {
        return None;
    }
    [(2, 0), (-2, 0), (0, 2), (0, -2)]
        .into_iter()
        .map(|(x, z)| (camp.0 + x, camp.1, camp.2 + z))
        .find(|p| clear_feet(world, *p))
}
pub fn guide_name(camp: Cell) -> &'static str {
    match (camp.0.wrapping_mul(31) ^ camp.2).rem_euclid(4) {
        0 => "Mira",
        1 => "Rowan",
        2 => "Ash",
        _ => "Tarin",
    }
}
pub fn can_use_camp(world: &World, position: Vec3, camp: Cell) -> bool {
    if !position.is_finite() || world.get_block(camp.0, camp.1, camp.2) != BlockType::Campfire {
        return false;
    }
    let eye = position + Vec3::Y * 1.62;
    let target = feet(camp) + Vec3::Y * 0.25;
    let distance = eye.distance(target);
    distance <= 6.0
        && crate::raycast::raycast(world, eye, target - eye, distance + 0.1)
            .is_some_and(|hit| hit.target == camp)
}
pub fn safe_camp(world: &World, camp: Cell) -> Option<Cell> {
    // Keep the respawn separate from the keeper and flame.
    [(0, -2), (0, 2), (-2, 0), (2, 0), (1, -1), (-1, -1)]
        .into_iter()
        .map(|(x, z)| (camp.0 + x, camp.1, camp.2 + z))
        .find(|p| clear_feet(world, *p))
}

/// Checks all conditions on a draft; failed claims cannot consume resources or rewards.
pub fn transact(
    world: &World,
    account: &mut Account,
    position: Vec3,
    health: f32,
    threatened: bool,
    action: Action,
) -> Result<String, String> {
    let camp = action.camp();
    if health <= 0.0 || !health.is_finite() {
        return Err("Recover before using a camp".into());
    }
    if !can_use_camp(world, position, camp) {
        return Err("Move within sight and six blocks of the campfire".into());
    }
    if threatened {
        return Err("A hostile creature is too close. Clear the camp first.".into());
    }
    let mut next = account.clone();
    let message = match action {
        Action::Rest { .. } => {
            next.adventure.home =
                Some(safe_camp(world, camp).ok_or("Clear a safe standing place beside the fire")?);
            "Rested: health restored, poison cured, and recovery camp set.".into()
        }
        Action::Claim { .. } => {
            if !ready(account) {
                return Err(if account.adventure.stage >= 3 {
                    "All campkeeper contracts are complete"
                } else {
                    "The current contract is not ready"
                }
                .into());
            }
            match account.adventure.stage {
                0 => {
                    resource(&mut next, BlockType::OakWood, -6)?;
                    resource(&mut next, BlockType::Stone, -4)?;
                    resource(&mut next, BlockType::Crystal, 2)?;
                }
                1 => resource(&mut next, BlockType::Iron, 3)?,
                2 => resource(&mut next, BlockType::RedStone, 2)?,
                _ => unreachable!(),
            }
            let reward = account.adventure.reward();
            next.mana = next
                .mana
                .checked_add(20 + 10 * u32::from(account.adventure.stage))
                .ok_or("Mana balance full")?;
            next.adventure.stage += 1;
            format!(
                "Contract complete: {reward}. {}",
                next.adventure.objective()
            )
        }
    };
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("Inventory revision limit reached")?;
    *account = next;
    Ok(message)
}

pub fn observe(world: &World, account: &mut Account, position: Vec3, health: f32) -> bool {
    let p = cell(position);
    if health > 0.0
        && position.is_finite()
        && account.adventure.stage == 1
        && !account.adventure.explored_depths
        && (2..=12).contains(&p.1)
        && world.get_block(p.0, p.1, p.2) == BlockType::Air
        && (3..=8).any(|y| world.get_block(p.0, p.1 + y, p.2).is_solid())
        && account.revision < u64::MAX
    {
        account.adventure.explored_depths = true;
        account.revision += 1;
        return true;
    }
    false
}

pub fn respawn_position(world: &mut World, home: Option<Cell>) -> Vec3 {
    let preferred = home.unwrap_or((0, world.terrain_height(0, 0) + 1, 0));
    crate::crafting::load_interaction_area(world, feet(preferred));
    for radius in 0i32..=8 {
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x.abs().max(z.abs()) != radius {
                    continue;
                }
                for dy in [0, 1, -1, 2, -2] {
                    let p = (preferred.0 + x, preferred.1 + dy, preferred.2 + z);
                    if (1..crate::voxel::chunk::CHUNK_Y - 2).contains(&p.1) && clear_feet(world, p)
                    {
                        return feet(p);
                    }
                }
            }
        }
    }
    // A destroyed/buried camp falls back to dry surface, without editing the world.
    for radius in 0i32..=24 {
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x.abs().max(z.abs()) != radius {
                    continue;
                }
                let (x, z) = (preferred.0 + x, preferred.2 + z);
                crate::crafting::load_interaction_area(world, Vec3::new(x as f32, 30., z as f32));
                for y in (1..crate::voxel::chunk::CHUNK_Y - 2).rev() {
                    if clear_feet(world, (x, y, z)) {
                        return feet((x, y, z));
                    }
                }
            }
        }
    }
    // Remain bounded in entirely edited worlds; the next frame's physics can settle.
    Vec3::new(
        preferred.0 as f32 + 0.5,
        crate::voxel::chunk::CHUNK_Y as f32 + 2.,
        preferred.2 as f32 + 0.5,
    )
}

pub fn crystal_light(account: &Account) -> bool {
    account.hotbar.entry() == Some(crate::equipment::Entry::Resource(BlockType::Crystal))
        && count(account, BlockType::Crystal) > 0
}

/// Ignore stale movement in flight until a recovering guest reports arrival.
pub fn accept_movement(health: f32, position: Vec3, recovery: Option<Vec3>) -> bool {
    health > 0.
        && position.is_finite()
        && position.abs().max_element() < 1_000_000.
        && recovery.is_none_or(|target| position.distance_squared(target) <= 9.)
}

/// Find a dry, unobstructed patch without modifying terrain or player blocks.
pub fn starter_camp(world: &mut World, position: Vec3) -> Option<Cell> {
    let origin = cell(position);
    crate::crafting::load_interaction_area(world, position);
    for radius in 4i32..=24 {
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx.abs().max(dz.abs()) != radius {
                    continue;
                }
                let (x, z) = (origin.0 + dx, origin.2 + dz);
                let y = world.terrain_height(x, z) + 1;
                if (y - origin.1).abs() > 5 {
                    continue;
                }
                // Cross-shaped clearing keeps the fire, guide and recovery space safe
                // while tolerating gentle terrain changes at the unused corners.
                if (-2..=2).all(|a| {
                    (-2..=2).all(|b| {
                        if a != 0 && b != 0 {
                            return true;
                        }
                        world.get_block(x + a, y - 1, z + b).is_solid()
                            && (0..4).all(|h| {
                                let block = world.get_block(x + a, y + h, z + b);
                                block == BlockType::Air || (h == 0 && block.def().cross)
                            })
                    })
                }) {
                    return Some((x, y, z));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    const CAMP: Cell = (8, 40, 8);
    #[test]
    fn dead_and_stale_guest_movement_cannot_undo_recovery() {
        let home = feet((8, 40, 6));
        assert!(!accept_movement(0., home, None));
        assert!(!accept_movement(100., Vec3::NAN, None));
        assert!(!accept_movement(100., Vec3::splat(1e7), None));
        assert!(!accept_movement(100., home + Vec3::X * 10., Some(home)));
        assert!(accept_movement(100., home + Vec3::X, Some(home)));
        assert!(accept_movement(100., home + Vec3::X * 10., None));
    }
    fn fixture() -> (World, Account, Vec3) {
        let mut w = World::new(42);
        w.ensure_chunk_loaded(0, 0);
        for x in 1..15 {
            for z in 1..15 {
                w.set_block(x, 39, z, BlockType::Stone);
                for y in 40..44 {
                    w.set_block(x, y, z, BlockType::Air);
                }
            }
        }
        w.set_block(CAMP.0, CAMP.1, CAMP.2, BlockType::Campfire);
        let mut a = Account::default();
        resource(&mut a, BlockType::OakWood, 6).unwrap();
        resource(&mut a, BlockType::Stone, 4).unwrap();
        (w, a, feet((8, 40, 5)))
    }
    #[test]
    fn ordinary_new_worlds_have_an_unobstructed_starter_camp() {
        for seed in [1, 2, 3, 7, 42, 99, 1234, 91827] {
            let mut world = World::new(seed);
            let p = Vec3::new(0.5, world.terrain_height(0, 0) as f32 + 2., 0.5);
            let camp =
                starter_camp(&mut world, p).unwrap_or_else(|| panic!("no camp for seed {seed}"));
            for a in -2..=2 {
                for b in -2..=2 {
                    if a == 0 || b == 0 {
                        world.set_block(camp.0 + a, camp.1, camp.2 + b, BlockType::Air);
                    }
                }
            }
            world.set_block(camp.0, camp.1, camp.2, BlockType::Campfire);
            assert!(guide_position(&world, camp).is_some());
            assert!(safe_camp(&world, camp).is_some());
        }
    }
    #[test]
    fn contracts_commit_once_and_preserve_inventory_on_failure() {
        let (w, mut a, p) = fixture();
        let initial = a.clone();
        assert!(transact(&w, &mut a, p, 100., false, Action::Claim { camp: CAMP }).is_ok());
        assert_eq!(a.adventure.stage, 1);
        assert_eq!(
            count(&a, BlockType::Crystal),
            count(&initial, BlockType::Crystal) + 2
        );
        assert_eq!(count(&a, BlockType::OakWood), 0);
        assert_eq!(count(&a, BlockType::Stone), 0);
        let once = a.clone();
        assert!(transact(&w, &mut a, p, 100., false, Action::Claim { camp: CAMP }).is_err());
        assert_eq!(a, once);
        a.adventure.explored_depths = true;
        transact(&w, &mut a, p, 100., false, Action::Claim { camp: CAMP }).unwrap();
        assert_eq!(a.adventure.stage, 2);
        assert!(!ready(&a));
        a.adventure.crafted_tool = true;
        transact(&w, &mut a, p, 100., false, Action::Claim { camp: CAMP }).unwrap();
        assert_eq!(a.adventure.title(), "Wayfinder");
        assert_eq!(a.mana, initial.mana + 90);
        let done = a.clone();
        assert!(transact(&w, &mut a, p, 100., false, Action::Claim { camp: CAMP }).is_err());
        assert_eq!(a, done);
    }
    #[test]
    fn rewards_reject_overflow_without_consuming_supplies() {
        let (w, a, p) = fixture();
        for variant in 0..3 {
            let mut a = a.clone();
            match variant {
                0 => a.mana = u32::MAX,
                1 => a.revision = u64::MAX,
                _ => {
                    let i = COLLECTIBLE_BLOCKS
                        .iter()
                        .position(|b| *b == BlockType::Crystal)
                        .unwrap();
                    a.resources[i] = u32::MAX;
                }
            }
            let before = a.clone();
            assert!(transact(&w, &mut a, p, 100., false, Action::Claim { camp: CAMP }).is_err());
            assert_eq!(a, before);
        }
    }
    #[test]
    fn camp_actions_require_life_range_sight_and_safety() {
        let (mut w, mut a, p) = fixture();
        let before = a.clone();
        for (pos, hp, danger) in [
            (p, 0., false),
            (p, f32::NAN, false),
            (p, 100., true),
            (p + Vec3::X * 20., 100., false),
            (Vec3::NAN, 100., false),
        ] {
            for action in [Action::Rest { camp: CAMP }, Action::Claim { camp: CAMP }] {
                assert!(transact(&w, &mut a, pos, hp, danger, action).is_err());
                assert_eq!(a, before);
            }
        }
        for y in 40..43 {
            w.set_block(8, y, 7, BlockType::Stone);
        }
        assert!(transact(&w, &mut a, p, 100., false, Action::Rest { camp: CAMP }).is_err());
        assert_eq!(a, before);
        for y in 40..43 {
            w.set_block(8, y, 7, BlockType::Air);
        }
        transact(&w, &mut a, p, 100., false, Action::Rest { camp: CAMP }).unwrap();
        assert!(clear_feet(&w, a.adventure.home.unwrap()));
        assert_eq!(a.resources, before.resources);
    }
    #[test]
    fn exploration_needs_active_contract_and_roof_and_crafting_needs_a_real_success() {
        let (mut w, mut a, _) = fixture();
        let pos = feet((8, 8, 8));
        for y in 8..17 {
            w.set_block(8, y, 8, BlockType::Air);
        }
        assert!(!observe(&w, &mut a, pos, 100.));
        a.adventure.stage = 1;
        assert!(!observe(&w, &mut a, pos, 100.));
        w.set_block(8, 12, 8, BlockType::Stone);
        assert!(!observe(&w, &mut a, pos, 0.));
        assert!(observe(&w, &mut a, pos, 100.));
        assert!(!observe(&w, &mut a, pos, 100.));
        a.adventure.stage = 2;
        let registry =
            crate::crafting::Registry::parse(include_str!("../data/crafting.json")).unwrap();
        let mut creatures = crate::creature::Creatures::new();
        let action = crate::crafting::Action::CraftGear(crate::equipment::Gear::Pickaxe);
        a.resources.fill(0);
        let before = a.clone();
        let revision = a.revision;
        assert!(registry
            .execute(&mut a, revision, &action, &w, &mut creatures, pos, &[pos])
            .is_err());
        assert_eq!(a, before);
        resource(&mut a, BlockType::Iron, 10).unwrap();
        resource(&mut a, BlockType::OakWood, 10).unwrap();
        a.mana = 100;
        let revision = a.revision;
        registry
            .execute(&mut a, revision, &action, &w, &mut creatures, pos, &[pos])
            .unwrap();
        assert!(a.adventure.crafted_tool);
    }
    #[test]
    fn recovery_moves_out_of_buried_camp_and_keeper_disappears_with_fire() {
        let (mut w, _, _) = fixture();
        w.set_block(8,crate::voxel::chunk::CHUNK_Y-1,6,BlockType::Stone);
        assert!(!clear_feet(&w,(8,crate::voxel::chunk::CHUNK_Y,6)),"out-of-world recovery feet cannot be saved");
        let home = (8, 40, 6);
        assert!(guide_position(&w, CAMP).is_some());
        assert_eq!(respawn_position(&mut w, Some(home)), feet(home));
        w.set_block(home.0, home.1, home.2, BlockType::Stone);
        let replacement = respawn_position(&mut w, Some(home));
        assert_ne!(replacement, feet(home));
        assert!(clear_feet(&w, cell(replacement)));
        w.set_block(CAMP.0, CAMP.1, CAMP.2, BlockType::Air);
        assert!(guide_position(&w, CAMP).is_none());
    }
    #[test]
    fn saved_accounts_preserve_journal_and_old_accounts_default_and_light_needs_stock() {
        let (_, mut a, _) = fixture();
        a.adventure = Progress {
            stage: 2,
            home: Some((8, 40, 6)),
            explored_depths: true,
            crafted_tool: false,
            recoveries: 3,
        };
        let mut save = crate::save::CraftingSave::default();
        save.host = a.clone();
        save.guests.insert("guest".into(), a.clone());
        let bytes = serde_json::to_vec(&save).unwrap();
        let saved: crate::save::CraftingSave = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.host, a);
        assert_eq!(saved.guests["guest"], a);
        let mut old = serde_json::to_value(&a).unwrap();
        old.as_object_mut().unwrap().remove("adventure");
        let legacy: Account = serde_json::from_value(old).unwrap();
        assert_eq!(legacy.adventure, Progress::default());
        a.hotbar.slots[0] = Some(crate::equipment::Entry::Resource(BlockType::Crystal));
        assert!(!crystal_light(&a));
        resource(&mut a, BlockType::Crystal, 1).unwrap();
        assert!(crystal_light(&a));
        a.hotbar.slots[0] = None;
        assert!(!crystal_light(&a));
    }
}
