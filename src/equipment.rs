//! Inventory references and authoritative player interactions.
use crate::{
    crafting::Account,
    voxel::{BlockType, World, COLLECTIBLE_BLOCKS},
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gear {
    Axe,
    Pickaxe,
    Sword,
    /// Reserved for old saves; unavailable in gameplay.
    Bow,
}
impl Gear {
    pub const ALL: [Self; 3] = [Self::Axe, Self::Pickaxe, Self::Sword];
    pub fn name(self) -> &'static str {
        match self {
            Self::Axe => "Axe",
            Self::Pickaxe => "Pickaxe",
            Self::Sword => "Sword",
            Self::Bow => "Bow",
        }
    }
    pub fn categories(self) -> &'static [Category] {
        match self {
            Self::Axe => &[Category::Wood, Category::Plant],
            Self::Pickaxe => &[Category::Stone, Category::Ore, Category::Soil],
            _ => &[],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Resource(BlockType),
    Gear(Gear),
}
impl Entry {
    pub fn name(self) -> &'static str {
        match self {
            Self::Resource(b) => b.name(),
            Self::Gear(g) => g.name(),
        }
    }
    pub fn count(self, account: &Account) -> u32 {
        match self {
            Self::Resource(b) => COLLECTIBLE_BLOCKS
                .iter()
                .position(|v| *v == b)
                .map_or(0, |i| account.resources[i]),
            Self::Gear(Gear::Bow) => 0,
            Self::Gear(g) => account.gear[g as usize],
        }
    }
    pub fn resource(self) -> Option<BlockType> {
        if let Self::Resource(b) = self {
            Some(b)
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotbar {
    pub slots: [Option<Entry>; 9],
    pub active: usize,
    pub revision: u64,
}
impl Default for Hotbar {
    fn default() -> Self {
        let mut slots = [None; 9];
        for (i, g) in Gear::ALL.into_iter().enumerate() {
            slots[i] = Some(Entry::Gear(g));
        }
        Self {
            slots,
            active: 0,
            revision: 0,
        }
    }
}
impl Hotbar {
    pub fn entry(&self) -> Option<Entry> {
        self.slots.get(self.active).copied().flatten()
    }
    pub fn select(&mut self, slot: usize) {
        if slot < 9 && self.active != slot {
            self.active = slot;
            self.revision += 1;
        }
    }
    pub fn cycle(&mut self, delta: i32) {
        for n in 1..=9 {
            let slot = (self.active as i32 + n * delta.signum()).rem_euclid(9) as usize;
            if self.slots[slot].is_some() {
                self.select(slot);
                break;
            }
        }
    }
    pub fn assign(&mut self, entry: Option<Entry>) {
        if self.active < 9 {
            self.slots[self.active] = entry;
            self.revision += 1;
        }
    }
}
/// Material semantics and an explicit, data-defined hand-gathering rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Wood,
    Stone,
    Ore,
    Soil,
    Plant,
}
impl BlockType {
    pub fn hand_pickable(self) -> bool {
        self == BlockType::Campfire || crate::voxel::resource_catalog::RESOURCES
            .iter()
            .find(|r| r.block == self)
            .is_some_and(|r| r.hand_pickable)
    }
    pub fn harvest_category(self) -> Category {
        match crate::voxel::resource_catalog::RESOURCES
            .iter()
            .find(|r| r.block == self)
            .map(|r| r.harvest_category)
        {
            Some("wood") => Category::Wood,
            Some("ore") => Category::Ore,
            Some("soil") => Category::Soil,
            Some("plant") => Category::Plant,
            _ => Category::Stone,
        }
    }
    pub fn required_tool(self) -> Gear {
        if matches!(self.harvest_category(), Category::Wood | Category::Plant) {
            Gear::Axe
        } else {
            Gear::Pickaxe
        }
    }
}
pub fn can_mine(entry: Option<Entry>, block: BlockType, account: &Account) -> bool {
    !block.is_unbreakable()
        && match entry {
            None => block.hand_pickable(),
            Some(e) => {
                e.count(account) > 0
                    && matches!(e,Entry::Gear(g) if g.categories().contains(&block.harvest_category()))
            }
        }
}
/// Preserve old save discriminants, but remove retired equipment from all accounts.
pub fn remove_bow(account: &mut Account) {
    let mut changed = account.gear[Gear::Bow as usize] != 0;
    account.gear[Gear::Bow as usize] = 0;
    for slot in &mut account.hotbar.slots {
        if *slot == Some(Entry::Gear(Gear::Bow)) {
            *slot = None;
            changed = true;
        }
    }
    if changed {
        account.hotbar.revision += 1;
        account.revision += 1;
    }
}
pub fn accept_hotbar(account: &mut Account, hotbar: &Hotbar) -> Result<(), String> {
    if hotbar.slots.contains(&Some(Entry::Gear(Gear::Bow))) {
        return Err("Bow is currently unavailable".into());
    }
    if hotbar.active >= 9 {
        return Err("Invalid hotbar slot".into());
    }
    if hotbar.revision < account.hotbar.revision {
        return Err("Active item changed; try again".into());
    }
    if hotbar.revision == account.hotbar.revision {
        return if *hotbar == account.hotbar {
            Ok(())
        } else {
            Err("Hotbar revision mismatch".into())
        };
    }
    for (i, entry) in hotbar.slots.iter().enumerate() {
        if let Some(e) = entry {
            if e.count(account) == 0 && account.hotbar.slots[i] != *entry {
                return Err("Assign an owned item".into());
            }
        }
    }
    account.hotbar = hotbar.clone();
    account.revision += 1;
    Ok(())
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Action {
    Mine,
    Place,
    Attack,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub hotbar: Hotbar,
    pub item: Option<Entry>,
    pub target: Option<(i32, i32, i32)>,
    pub direction: [f32; 3],
    pub action: Action,
}
#[derive(Default)]
pub struct Mining {
    pub target: Option<((i32, i32, i32), BlockType, Option<Entry>)>,
    pub hits: u32,
    pub last_use: Option<std::time::Instant>,
}
pub fn overlaps(pos: Vec3, block: (i32, i32, i32)) -> bool {
    let b = Vec3::new(block.0 as f32, block.1 as f32, block.2 as f32);
    pos.x + 0.3 > b.x
        && pos.x - 0.3 < b.x + 1.0
        && pos.z + 0.3 > b.z
        && pos.z - 0.3 < b.z + 1.0
        && pos.y + 1.8 > b.y
        && pos.y < b.y + 1.0
}
/// Validates everything before changing resources. Returns an edit for the host to apply/replicate.
pub fn block_action(
    world: &World,
    account: &mut Account,
    mining: &mut Mining,
    feet: Vec3,
    players: &[Vec3],
    intent: &Intent,
) -> Result<Option<((i32, i32, i32), BlockType, BlockType)>, String> {
    accept_hotbar(account, &intent.hotbar)?;
    if intent.item != account.hotbar.entry() {
        return Err("Active item mismatch".into());
    }
    if !feet.is_finite() || feet.abs().max_element() > 1_000_000.0 {
        return Err("Invalid position".into());
    }
    let dir = Vec3::from_array(intent.direction);
    if !dir.is_finite() || dir.length_squared() < 0.5 {
        return Err("Invalid aim".into());
    }
    let hit = crate::raycast::raycast(world, feet + Vec3::Y * 1.62, dir, 6.0)
        .ok_or("No block in reach")?;
    if Some(hit.target) != intent.target {
        return Err("Target changed or is obstructed".into());
    }
    let old = world.get_block(hit.target.0, hit.target.1, hit.target.2);
    let entry = intent.item;
    if entry.is_some_and(|e| e.count(account) == 0) {
        return Err("Empty stack".into());
    }
    if mining
        .last_use
        .is_some_and(|t| t.elapsed().as_millis() < 160)
    {
        return Err("".into());
    }
    match intent.action {
        Action::Mine => {
            if !can_mine(entry, old, account) {
                mining.target = None;
                mining.hits = 0;
                return Err(if old.is_unbreakable() {
                    "Unbreakable".into()
                } else {
                    format!("Requires a {}", old.required_tool().name().to_lowercase())
                });
            }
            // Stage the complete reward before changing either inventory. Campfires
            // salvage their stone ring/logs and release stored Fire essence.
            let mut resources = account.resources;
            let mut elements = account.elements;
            let drops: &[(BlockType, u32)] = if old == BlockType::Campfire {
                &[(BlockType::Stone, 2), (BlockType::OakWood, 2)]
            } else {
                &[(old, 1)]
            };
            for &(block, count) in drops {
                let i = COLLECTIBLE_BLOCKS.iter().position(|b| *b == block)
                    .ok_or("Cannot gather this block")?;
                resources[i] = resources[i].checked_add(count).ok_or("Inventory full")?;
            }
            if old == BlockType::Campfire {
                let fire = crate::crafting::Element::Fire as usize;
                elements[fire] = elements[fire].checked_add(4).ok_or("Element inventory full")?;
            }
            let key = (hit.target, old, entry);
            if mining.target != Some(key) {
                mining.target = Some(key);
                mining.hits = 0;
            }
            mining.hits += 1;
            mining.last_use = Some(std::time::Instant::now());
            if mining.hits < if entry.is_none() { 1 } else { old.hardness() } {
                return Ok(None);
            }
            mining.target = None;
            mining.hits = 0;
            account.resources = resources;
            account.elements = elements;
            account.revision += 1;
            Ok(Some((hit.target, BlockType::Air, old)))
        }
        Action::Place => {
            let block = entry
                .and_then(Entry::resource)
                .ok_or("Select a resource to build")?;
            let p = hit.place;
            if (p.0 - hit.target.0).abs() + (p.1 - hit.target.1).abs() + (p.2 - hit.target.2).abs()
                != 1
                || !(0..crate::voxel::chunk::CHUNK_Y).contains(&p.1)
                || players.iter().any(|v| overlaps(*v, p))
                || overlaps(feet, p)
            {
                return Err("Cannot place here".into());
            }
            let previous = world.get_block(p.0, p.1, p.2);
            if !matches!(previous, BlockType::Air | BlockType::Water) {
                return Err("Space is occupied".into());
            }
            if block.def().only_on_top && !world.get_block(p.0, p.1 - 1, p.2).is_solid() {
                return Err("Needs solid ground".into());
            }
            let i = COLLECTIBLE_BLOCKS
                .iter()
                .position(|b| *b == block)
                .ok_or("Not placeable")?;
            account.resources[i] -= 1;
            account.revision += 1;
            mining.last_use = Some(std::time::Instant::now());
            Ok(Some((p, block, previous)))
        }
        Action::Attack => Err("Use a weapon against a creature".into()),
    }
}

/// Sword melee is blocked by terrain. Retired bow requests are rejected.
pub fn attack(
    world: &World,
    creatures: &mut crate::creature::Creatures,
    account: &mut Account,
    state: &mut Mining,
    feet: Vec3,
    intent: &Intent,
) -> Result<(), String> {
    accept_hotbar(account, &intent.hotbar)?;
    if intent.item != account.hotbar.entry() {
        return Err("Active item mismatch".into());
    }
    match intent.item {
        Some(Entry::Gear(Gear::Sword)) if account.gear[Gear::Sword as usize] > 0 => (),
        _ => return Err("Equip a weapon".into()),
    };
    let dir = Vec3::from_array(intent.direction).normalize_or_zero();
    if !feet.is_finite()
        || feet.abs().max_element() > 1_000_000.0
        || !dir.is_finite()
        || dir == Vec3::ZERO
    {
        return Err("Invalid aim".into());
    }
    if state
        .last_use
        .is_some_and(|t| t.elapsed().as_millis() < 350)
    {
        return Err("".into());
    }
    state.last_use = Some(std::time::Instant::now());
    state.target = None;
    state.hits = 0;
    let eye = feet + Vec3::Y * 1.62;
    let reach = 3.2;
    let best = creatures.weapon_target(world, eye, dir, reach);
    if let Some(id) = best {
        if let Some(death) = creatures.damage(id, 12.0) {
            creatures.player_kills.push(death);
            creatures.combat_deaths.push(death);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(block: BlockType, entry: Option<Entry>) -> (World, Account, Intent, Vec3) {
        let mut world = World::new(71);
        world.ensure_chunk_loaded(0, 0);
        for x in 0..9 {
            for y in 39..44 {
                for z in 0..9 {
                    world.set_block(x, y, z, BlockType::Air);
                }
            }
        }
        world.set_block(4, 41, 2, block);
        let mut account = Account::default();
        account.hotbar.slots[0] = entry;
        if let Some(Entry::Resource(b)) = entry {
            account.resources[COLLECTIBLE_BLOCKS.iter().position(|v| *v == b).unwrap()] = 2;
        }
        let intent = Intent {
            hotbar: account.hotbar.clone(),
            item: entry,
            target: Some((4, 41, 2)),
            direction: Vec3::X.to_array(),
            action: Action::Mine,
        };
        (world, account, intent, Vec3::new(1.5, 40.0, 2.5))
    }
    #[test]
    fn sword_kill_queues_renderable_collectible_loot_once() {
        let (mut world, mut account, mut intent, feet) = fixture(BlockType::Stone, Some(Entry::Gear(Gear::Sword)));
        for x in 0..9 { for z in 0..9 { world.set_block(x,39,z,BlockType::Stone); } }
        intent.action=Action::Attack;
        let mut creatures=crate::creature::Creatures::new();
        creatures.spawn_one(crate::creature::CreatureKind::Wolf,Vec3::new(3.0,40.8,2.5),1);
        for _ in 0..20 {
            attack(&world,&mut creatures,&mut account,&mut Mining::default(),feet,&intent).unwrap();
            if creatures.snapshot_with_ids().is_empty() { break; }
        }
        assert!(creatures.snapshot_with_ids().is_empty());
        assert_eq!(creatures.player_kills.len(),1);
        let mut loot=crate::loot::Effects::default();
        for death in creatures.player_kills.drain(..) { loot.spawn(&world,death.kind,death.pos); }
        assert_eq!(loot.drops.len(),1);
        assert!(!loot.mesh(|_|true).indices.is_empty());
        loot.update(2.0,true);
        let before=account.clone();
        for drop in loot.drops.clone() { loot.collect(&world,Vec3::from_array(drop.pos),&mut account); }
        assert!(loot.drops.is_empty());
        for (block,amount) in crate::loot::rewards(crate::creature::CreatureKind::Wolf) {
            assert_eq!(Entry::Resource(block).count(&account),Entry::Resource(block).count(&before)+amount);
        }
    }
    #[test]
    fn empty_slot_picks_each_soft_resource_once_and_rejects_hard_materials() {
        for b in COLLECTIBLE_BLOCKS {
            let (world, mut account, intent, feet) = fixture(b, None);
            let before = account.clone();
            let result = block_action(
                &world,
                &mut account,
                &mut Mining::default(),
                feet,
                &[],
                &intent,
            );
            if b.hand_pickable() {
                assert_eq!(result.unwrap(), Some(((4, 41, 2), BlockType::Air, b)));
                assert_eq!(Entry::Resource(b).count(&account), 1);
                assert_eq!(account.hotbar.entry(), None);
            } else {
                assert!(result.is_err(), "{}", b.name());
                assert_eq!(account, before);
            }
        }
    }
    #[test]
    fn depleted_resource_and_weapons_do_not_become_bare_hands() {
        for entry in [Entry::Resource(BlockType::Stone), Entry::Gear(Gear::Sword)] {
            let (world, mut account, intent, feet) = fixture(BlockType::Pumpkin, Some(entry));
            account.resources.fill(0);
            let before = account.clone();
            assert!(block_action(
                &world,
                &mut account,
                &mut Mining::default(),
                feet,
                &[],
                &intent
            )
            .is_err());
            assert_eq!(account, before);
        }
    }
    #[test]
    fn old_bow_slots_become_empty_hands_and_bow_requests_are_rejected() {
        let mut account = Account::default();
        account.gear[3] = 1;
        account.hotbar.slots[3] = Some(Entry::Gear(Gear::Bow));
        account.hotbar.active = 3;
        remove_bow(&mut account);
        assert_eq!(account.hotbar.entry(), None);
        assert_eq!(account.gear[3], 0);
        let migrated = account.clone();
        remove_bow(&mut account);
        assert_eq!(account, migrated);
        let restored: Account =
            serde_json::from_str(&serde_json::to_string(&account).unwrap()).unwrap();
        assert_eq!(restored, account);
        let mut forged = account.hotbar.clone();
        forged.assign(Some(Entry::Gear(Gear::Bow)));
        assert!(accept_hotbar(&mut account, &forged).is_err());
        assert!(!Gear::ALL.contains(&Gear::Bow));
    }
    #[test]
    fn all_materials_have_semantic_tool_mapping_and_weapons_never_mine() {
        let account = Account::default();
        for b in COLLECTIBLE_BLOCKS {
            let info = crate::voxel::resource_catalog::info(b);
            assert!(matches!(
                info.harvest_category,
                "wood" | "ore" | "soil" | "plant" | "stone"
            ));
            assert!(
                can_mine(Some(Entry::Gear(b.required_tool())), b, &account),
                "{}",
                b.name()
            );
            for entry in [
                None,
                Some(Entry::Gear(Gear::Sword)),
                Some(Entry::Gear(Gear::Bow)),
                Some(Entry::Resource(b)),
            ] {
                assert_eq!(
                    can_mine(entry, b, &account),
                    entry.is_none() && b.hand_pickable()
                );
            }
        }
        assert_eq!(BlockType::OakWood.harvest_category(), Category::Wood);
        assert_eq!(BlockType::IronOre.harvest_category(), Category::Ore);
        assert_eq!(BlockType::Grass.harvest_category(), Category::Soil);
        assert_eq!(BlockType::Fern.harvest_category(), Category::Plant);
    }
    #[test]
    fn invalid_mining_never_changes_inventory_or_world() {
        for entry in [
            None,
            Some(Entry::Gear(Gear::Axe)),
            Some(Entry::Gear(Gear::Sword)),
            Some(Entry::Gear(Gear::Bow)),
            Some(Entry::Resource(BlockType::Stone)),
        ] {
            let (world, mut account, intent, feet) = fixture(BlockType::Stone, entry);
            let before = account.clone();
            assert!(block_action(
                &world,
                &mut account,
                &mut Mining::default(),
                feet,
                &[],
                &intent
            )
            .is_err());
            assert_eq!(account, before);
            assert_eq!(world.get_block(4, 41, 2), BlockType::Stone);
        }
    }
    #[test]
    fn host_counts_hits_and_awards_once_then_rejects_stale_target() {
        let (mut world, mut account, intent, feet) =
            fixture(BlockType::Stone, Some(Entry::Gear(Gear::Pickaxe)));
        let mut state = Mining::default();
        for n in 1..=BlockType::Stone.hardness() {
            state.last_use = None;
            let result =
                block_action(&world, &mut account, &mut state, feet, &[], &intent).unwrap();
            if n < BlockType::Stone.hardness() {
                assert!(result.is_none());
            } else {
                let (p, b, _) = result.unwrap();
                world.set_block(p.0, p.1, p.2, b);
            }
        }
        assert_eq!(Entry::Resource(BlockType::Stone).count(&account), 1);
        assert!(block_action(&world, &mut account, &mut state, feet, &[], &intent).is_err());
        assert_eq!(Entry::Resource(BlockType::Stone).count(&account), 1);
    }
    #[test]
    fn campfire_salvage_is_atomic_and_awarded_once() {
        for entry in [None, Some(Entry::Gear(Gear::Pickaxe))] {
            let (mut world, mut account, intent, feet) = fixture(BlockType::Campfire, entry);
            let before = account.clone();
            let mut state = Mining::default();
            let hits = if entry.is_none() { 1 } else { BlockType::Campfire.hardness() };
            for n in 1..=hits {
                state.last_use = None;
                let result = block_action(&world, &mut account, &mut state, feet, &[], &intent).unwrap();
                if n < hits {
                    assert!(result.is_none());
                    assert_eq!(account, before);
                } else {
                    let (p, block, old) = result.unwrap();
                    assert_eq!(old, BlockType::Campfire);
                    world.set_block(p.0, p.1, p.2, block);
                }
            }
            for block in [BlockType::Stone, BlockType::OakWood] {
                assert_eq!(Entry::Resource(block).count(&account), Entry::Resource(block).count(&before) + 2);
            }
            let fire = crate::crafting::Element::Fire.index();
            assert_eq!(account.elements[fire], before.elements[fire] + 4);
            let after = account.clone();
            state.last_use = None;
            assert!(block_action(&world, &mut account, &mut state, feet, &[], &intent).is_err());
            assert_eq!(account, after);
        }
        for full in 0..3 {
            let (world, mut account, intent, feet) = fixture(BlockType::Campfire, None);
            if full == 2 { account.elements[crate::crafting::Element::Fire.index()] = u32::MAX; }
            else {
                let block = [BlockType::Stone, BlockType::OakWood][full];
                let i = COLLECTIBLE_BLOCKS.iter().position(|b| *b == block).unwrap();
                account.resources[i] = u32::MAX;
            }
            let before = account.clone();
            assert!(block_action(&world, &mut account, &mut Mining::default(), feet, &[], &intent).is_err());
            assert_eq!(account, before);
            assert_eq!(world.get_block(4, 41, 2), BlockType::Campfire);
        }
    }
    #[test]
    fn placement_consumes_exactly_one_and_failures_are_atomic() {
        let (mut world, mut account, mut intent, feet) =
            fixture(BlockType::Stone, Some(Entry::Resource(BlockType::OakWood)));
        intent.action = Action::Place;
        let result = block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet,
            &[],
            &intent,
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.0, (3, 41, 2));
        assert_eq!(result.1, BlockType::OakWood);
        assert_eq!(Entry::Resource(BlockType::OakWood).count(&account), 1);
        world.set_block(result.0 .0, result.0 .1, result.0 .2, result.1);
        let before = account.clone();
        assert!(block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet,
            &[],
            &intent
        )
        .is_err());
        assert_eq!(account, before);
        let (world, mut account, mut intent, feet) =
            fixture(BlockType::Stone, Some(Entry::Resource(BlockType::OakWood)));
        intent.action = Action::Place;
        let before = account.clone();
        assert!(block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet,
            &[Vec3::new(3.5, 40.0, 2.5)],
            &intent
        )
        .is_err());
        assert_eq!(account, before);
        for entry in [
            None,
            Some(Entry::Gear(Gear::Axe)),
            Some(Entry::Gear(Gear::Pickaxe)),
            Some(Entry::Gear(Gear::Sword)),
            Some(Entry::Gear(Gear::Bow)),
        ] {
            let (world, mut account, mut intent, feet) = fixture(BlockType::Stone, entry);
            intent.action = Action::Place;
            let before = account.clone();
            assert!(block_action(
                &world,
                &mut account,
                &mut Mining::default(),
                feet,
                &[],
                &intent
            )
            .is_err());
            assert_eq!(account, before);
        }
    }
    #[test]
    fn depleted_assignment_recovers_and_wheel_wraps_empty_slots() {
        let mut account = Account::default();
        account.hotbar.slots = [None; 9];
        account.hotbar.slots[0] = Some(Entry::Resource(BlockType::Stone));
        account.hotbar.slots[8] = Some(Entry::Gear(Gear::Axe));
        assert_eq!(account.hotbar.entry().unwrap().count(&account), 0);
        account.hotbar.cycle(-1);
        assert_eq!(account.hotbar.active, 8);
        account.hotbar.cycle(1);
        assert_eq!(account.hotbar.active, 0);
        account.resources[COLLECTIBLE_BLOCKS
            .iter()
            .position(|b| *b == BlockType::Stone)
            .unwrap()] = 5;
        assert_eq!(account.hotbar.entry().unwrap().count(&account), 5);
        account.hotbar.select(9);
        assert_eq!(account.hotbar.active, 0);
        account.hotbar.slots = [None; 9];
        account.hotbar.cycle(1);
        assert_eq!(account.hotbar.active, 0);
    }
    #[test]
    fn forged_items_out_of_range_and_obstructed_targets_are_rejected() {
        let (mut world, mut account, mut intent, feet) =
            fixture(BlockType::Stone, Some(Entry::Gear(Gear::Pickaxe)));
        let before = account.clone();
        intent.item = Some(Entry::Gear(Gear::Axe));
        assert!(block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet,
            &[],
            &intent
        )
        .is_err());
        intent.item = account.hotbar.entry();
        assert!(block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet - Vec3::X * 10.0,
            &[],
            &intent
        )
        .is_err());
        world.set_block(2, 41, 2, BlockType::OakWood);
        assert!(block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet,
            &[],
            &intent
        )
        .is_err());
        assert_eq!(account, before);
    }
    #[test]
    fn reordered_equipment_and_intents_agree_and_persist_for_reconnection() {
        let (world, mut account, mut intent, feet) =
            fixture(BlockType::Stone, Some(Entry::Gear(Gear::Axe)));
        let mut latest = account.hotbar.clone();
        latest.select(1);
        intent.hotbar = latest.clone();
        intent.item = latest.entry();
        // The action arrives before the standalone equipment packet.
        let bytes = bincode::serialize(&crate::net::ReliableMsg::ItemAction(intent)).unwrap();
        let crate::net::ReliableMsg::ItemAction(intent) = bincode::deserialize(&bytes).unwrap()
        else {
            panic!()
        };
        block_action(
            &world,
            &mut account,
            &mut Mining::default(),
            feet,
            &[],
            &intent,
        )
        .unwrap();
        accept_hotbar(&mut account, &latest).unwrap();
        let mut stale = latest.clone();
        stale.revision = 0;
        assert!(accept_hotbar(&mut account, &stale).is_err());
        let mut save = crate::save::CraftingSave::default();
        save.guests.insert("steam:123".into(), account.clone());
        let restored: crate::save::CraftingSave =
            serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
        assert_eq!(restored.guests["steam:123"], account);
        let mut forged = latest.clone();
        forged.assign(Some(Entry::Resource(BlockType::Gold)));
        assert!(accept_hotbar(&mut account, &forged).is_err());
    }
    #[test]
    fn weapons_hit_creatures_but_not_through_walls() {
        for gear in [Gear::Sword] {
            let (mut world, mut account, mut intent, feet) =
                fixture(BlockType::Stone, Some(Entry::Gear(gear)));
            intent.action = Action::Attack;
            let mut creatures = crate::creature::Creatures::new();
            creatures.spawn_one(
                crate::creature::CreatureKind::Wolf,
                Vec3::new(3.0, 40.8, 2.5),
                1,
            );
            let before = creatures.snapshot_with_ids()[0].3;
            attack(
                &world,
                &mut creatures,
                &mut account,
                &mut Mining::default(),
                feet,
                &intent,
            )
            .unwrap();
            assert!(creatures.snapshot_with_ids()[0].3 < before);
            world.set_block(2, 41, 2, BlockType::Stone);
            let before = creatures.snapshot_with_ids()[0].3;
            attack(
                &world,
                &mut creatures,
                &mut account,
                &mut Mining::default(),
                feet,
                &intent,
            )
            .unwrap();
            assert_eq!(creatures.snapshot_with_ids()[0].3, before);
        }
    }
}
