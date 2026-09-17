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
    /// Mana-strung bow; each shot costs one mana.
    Bow,
    ForesterAxe,
    ProspectorPick,
    Spade,
    Sickle,
    Spear,
    Dagger,
    Warhammer,
    Longbow,
    EmberWand,
    TideWand,
    LifeStaff,
    SurveyLantern,
    TrailCharm,
    LeapingCharm,
    DivingCharm,
    FeatherCharm,
    Torch,
}
impl Gear {
    pub fn enabled(self) -> bool {
        !matches!(self, Self::Bow | Self::Longbow)
    }
    pub fn available() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter().filter(|g| g.enabled())
    }
    pub const ALL: [Self; 21] = [
        Self::Axe,
        Self::Pickaxe,
        Self::Sword,
        Self::Bow,
        Self::ForesterAxe,
        Self::ProspectorPick,
        Self::Spade,
        Self::Sickle,
        Self::Spear,
        Self::Dagger,
        Self::Warhammer,
        Self::Longbow,
        Self::EmberWand,
        Self::TideWand,
        Self::LifeStaff,
        Self::SurveyLantern,
        Self::TrailCharm,
        Self::LeapingCharm,
        Self::DivingCharm,
        Self::FeatherCharm,
        Self::Torch,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Axe => "Axe",
            Self::Pickaxe => "Pickaxe",
            Self::Sword => "Sword",
            Self::Bow => "Bow",
            _ => self.definition().name,
        }
    }
    pub fn categories(self) -> &'static [Category] {
        match self {
            Self::Axe => &[Category::Wood, Category::Plant],
            Self::Pickaxe => &[Category::Stone, Category::Ore, Category::Soil],
            Self::ForesterAxe => &[Category::Wood, Category::Plant],
            Self::ProspectorPick => &[Category::Stone, Category::Ore],
            Self::Spade => &[Category::Soil],
            Self::Sickle => &[Category::Plant],
            _ => &[],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Resource(BlockType),
    Gear(Gear),
    Spell(crate::spellbook::SpellId),
}
impl Entry {
    pub fn name(self) -> &'static str {
        match self {
            Self::Resource(b) => b.name(),
            Self::Gear(g) => g.name(),
            Self::Spell(_) => "Spell",
        }
    }
    pub fn count(self, account: &Account) -> u32 {
        match self {
            Self::Resource(b) => COLLECTIBLE_BLOCKS
                .iter()
                .position(|v| *v == b)
                .map_or(0, |i| account.resources[i]),
            Self::Gear(g) => {
                if g.enabled() {
                    account.gear[g as usize]
                } else {
                    0
                }
            }
            Self::Spell(id) => u32::from(account.known_spells.contains(&id)),
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
        for (i, g) in [Gear::Axe, Gear::Pickaxe, Gear::Sword]
            .into_iter()
            .enumerate()
        {
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
    /// Only bindings change; selecting an empty slot never switches tools implicitly.
    pub fn prune_unavailable(&mut self, account: &Account) -> bool {
        let mut changed = false;
        for slot in &mut self.slots {
            if slot.is_some_and(|e| e.count(account) == 0 || e == Entry::Gear(Gear::Torch)) {
                *slot = None;
                changed = true;
            }
        }
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        changed
    }
    pub fn entry(&self) -> Option<Entry> {
        self.slots
            .get(self.active)
            .copied()
            .flatten()
            .filter(|e| !matches!(e,Entry::Gear(g) if !g.enabled() || *g==Gear::Torch))
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
        self == BlockType::Campfire
            || crate::voxel::resource_catalog::RESOURCES
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
pub fn accept_hotbar(account: &mut Account, hotbar: &Hotbar) -> Result<(), String> {
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
            if *e == Entry::Gear(Gear::Torch) && account.hotbar.slots[i] != *entry {
                return Err("Equip torches in the left-hand inventory slot".into());
            }
            if e.count(account) == 0 {
                return Err("Assign an owned item".into());
            }
        }
    }
    account.hotbar = hotbar.clone();
    account.revision += 1;
    Ok(())
}

/// Merge optimistic slot selection with authoritative balances. Never advance the
/// client's account revision: a later server inventory packet must remain usable.
pub fn receive_inventory(
    current: &mut Account,
    mut incoming: Account,
    ready: bool,
) -> Option<bool> {
    if ready && incoming.revision < current.revision {
        return None;
    }
    let mut hotbar = if ready && current.hotbar.revision > incoming.hotbar.revision {
        current.hotbar.clone()
    } else {
        incoming.hotbar.clone()
    };
    let pruned = hotbar.prune_unavailable(&incoming);
    incoming.hotbar = hotbar;
    *current = incoming;
    Some(pruned)
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
    block_action_at(
        world,
        account,
        mining,
        feet,
        players,
        intent,
        std::time::Instant::now(),
    )
}

/// Shared validator with an explicit clock for deterministic development playback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn block_action_at(
    world: &World,
    account: &mut Account,
    mining: &mut Mining,
    feet: Vec3,
    players: &[Vec3],
    intent: &Intent,
    now: std::time::Instant,
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
    // Placement uses a slightly shorter reach than mining so the player
    // cannot build through an intervening surface; mining keeps the full
    // interaction distance.
    let reach = if matches!(intent.action, Action::Place) { 5.5 } else { 6.0 };
    let hit = crate::raycast::raycast(world, feet + Vec3::Y * 1.62, dir, reach)
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
        .is_some_and(|t| now.saturating_duration_since(t).as_millis() < 160)
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
                let i = COLLECTIBLE_BLOCKS
                    .iter()
                    .position(|b| *b == block)
                    .ok_or("Cannot gather this block")?;
                resources[i] = resources[i].checked_add(count).ok_or("Inventory full")?;
            }
            if old == BlockType::Campfire {
                let fire = crate::crafting::Element::Fire as usize;
                elements[fire] = elements[fire]
                    .checked_add(4)
                    .ok_or("Element inventory full")?;
            }
            let key = (hit.target, old, entry);
            if mining.target != Some(key) {
                mining.target = Some(key);
                mining.hits = 0;
            }
            mining.hits = mining.hits.saturating_add(match entry {
                Some(Entry::Gear(g)) => g.harvest_power(),
                _ => 1,
            });
            mining.last_use = Some(now);
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
            if crate::food::inventory_only(block) {
                return Err(
                    "Eat this food in inventory [I]; raw meat can be cooked at a campfire".into(),
                );
            }
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
            mining.last_use = Some(now);
            Ok(Some((p, block, previous)))
        }
        Action::Attack => Err("Use a weapon against a creature".into()),
    }
}

/// Sword melee and mana-strung bow shots are both blocked by terrain.
pub fn attack(
    world: &World,
    creatures: &mut crate::creature::Creatures,
    account: &mut Account,
    state: &mut Mining,
    feet: Vec3,
    intent: &Intent,
) -> Result<(), String> {
    attack_at(
        world,
        creatures,
        account,
        state,
        feet,
        intent,
        std::time::Instant::now(),
    )
}

/// Shared validator with an explicit clock for deterministic development playback.
#[allow(clippy::too_many_arguments)]
pub(crate) fn attack_at(
    world: &World,
    creatures: &mut crate::creature::Creatures,
    account: &mut Account,
    state: &mut Mining,
    feet: Vec3,
    intent: &Intent,
    now: std::time::Instant,
) -> Result<(), String> {
    accept_hotbar(account, &intent.hotbar)?;
    if intent.item != account.hotbar.entry() {
        return Err("Active item mismatch".into());
    }
    let Some(Entry::Gear(gear)) = intent.item else {
        return Err("Equip a weapon".into());
    };
    let (reach, damage, cooldown, mana) = gear
        .weapon()
        .filter(|_| account.gear[gear as usize] > 0)
        .ok_or("Equip a weapon")?;
    let dir = Vec3::from_array(intent.direction).normalize_or_zero();
    if !feet.is_finite()
        || feet.abs().max_element() > 1_000_000.0
        || !dir.is_finite()
        || dir == Vec3::ZERO
    {
        return Err("Invalid aim".into());
    }
    if account.mana < mana {
        return Err(format!("{} needs {mana} mana per use", gear.name()));
    }
    if state
        .last_use
        .is_some_and(|t| now.saturating_duration_since(t).as_millis() < cooldown as u128)
    {
        return Err("".into());
    }
    state.last_use = Some(now);
    state.target = None;
    state.hits = 0;
    let eye = feet + Vec3::Y * 1.62;
    if mana > 0 {
        account.mana -= mana;
        account.revision = account.revision.saturating_add(1);
    }
    if gear == Gear::LifeStaff {
        return Ok(());
    }
    let best = creatures.weapon_target(world, eye, dir, reach);
    if let Some(id) = best {
        if let Some(death) = creatures.damage(id, damage) {
            crate::quests::kill(account, death.kind);
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
        let (mut world, mut account, mut intent, feet) =
            fixture(BlockType::Stone, Some(Entry::Gear(Gear::Sword)));
        for x in 0..9 {
            for z in 0..9 {
                world.set_block(x, 39, z, BlockType::Stone);
            }
        }
        intent.action = Action::Attack;
        let mut creatures = crate::creature::Creatures::new();
        creatures.spawn_one(
            crate::creature::CreatureKind::Wolf,
            Vec3::new(3.0, 40.8, 2.5),
            1,
        );
        for _ in 0..20 {
            attack(
                &world,
                &mut creatures,
                &mut account,
                &mut Mining::default(),
                feet,
                &intent,
            )
            .unwrap();
            if creatures.snapshot_with_ids().is_empty() {
                break;
            }
        }
        assert!(creatures.snapshot_with_ids().is_empty());
        assert_eq!(creatures.player_kills.len(), 1);
        assert_eq!(account.adventure.quests.counts[5], 1);
        let mut loot = crate::loot::Effects::default();
        for death in creatures.player_kills.drain(..) {
            loot.spawn(&world, death.kind, death.pos);
        }
        assert_eq!(loot.drops.len(), 1);
        assert!(!loot.mesh(|_| true).indices.is_empty());
        loot.update(2.0, true);
        let before = account.clone();
        for drop in loot.drops.clone() {
            loot.collect(&world, Vec3::from_array(drop.pos), &mut account);
        }
        assert!(loot.drops.is_empty());
        for (block, amount) in crate::loot::rewards(crate::creature::CreatureKind::Wolf) {
            assert_eq!(
                Entry::Resource(block).count(&account),
                Entry::Resource(block).count(&before) + amount
            );
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
    fn owned_bow_survives_save_and_unowned_bow_is_rejected() {
        let mut account = Account::default();
        account.gear[3] = 1;
        account.hotbar.slots[3] = Some(Entry::Gear(Gear::Bow));
        account.hotbar.active = 3;
        let restored: Account =
            serde_json::from_str(&serde_json::to_string(&account).unwrap()).unwrap();
        assert_eq!(restored, account);
        assert_eq!(account.hotbar.entry(), None);
        account.gear[3] = 0;
        account.hotbar.slots[3] = None;
        let mut forged = account.hotbar.clone();
        forged.assign(Some(Entry::Gear(Gear::Bow)));
        assert!(accept_hotbar(&mut account, &forged).is_err());
        assert!(Gear::ALL.contains(&Gear::Bow));
    }
    #[test]
    fn specialist_weapons_use_authoritative_damage_mana_and_cooldowns() {
        for gear in [
            Gear::Spear,
            Gear::Dagger,
            Gear::Warhammer,
            Gear::EmberWand,
            Gear::TideWand,
            Gear::LifeStaff,
        ] {
            let (world, mut a, mut intent, feet) = fixture(BlockType::Air, Some(Entry::Gear(gear)));
            a.gear[gear as usize] = 1;
            a.mana = 20;
            intent.action = Action::Attack;
            let mut creatures = crate::creature::Creatures::new();
            creatures.spawn_one(
                crate::creature::CreatureKind::StoneGolem,
                feet + Vec3::new(1.5, 0.8, 0.),
                1,
            );
            let hp = creatures.snapshot_with_ids()[0].3;
            let now = std::time::Instant::now();
            let mut state = Mining::default();
            attack_at(
                &world,
                &mut creatures,
                &mut a,
                &mut state,
                feet,
                &intent,
                now,
            )
            .unwrap();
            let (_, damage, _, mana) = gear.weapon().unwrap();
            assert_eq!(a.mana, 20 - mana);
            assert_eq!(creatures.snapshot_with_ids()[0].3, hp - damage);
            let before = a.clone();
            assert!(attack_at(
                &world,
                &mut creatures,
                &mut a,
                &mut state,
                feet,
                &intent,
                now
            )
            .is_err());
            assert_eq!(a, before);
        }
    }
    #[test]
    fn specialist_harvesting_is_faster_without_duplicate_rewards() {
        for (gear, block) in [
            (Gear::ForesterAxe, BlockType::OakWood),
            (Gear::ProspectorPick, BlockType::Stone),
            (Gear::Spade, BlockType::Soil),
            (Gear::Sickle, BlockType::CherryLeaves),
        ] {
            let (world, mut a, intent, feet) = fixture(block, Some(Entry::Gear(gear)));
            a.gear[gear as usize] = 1;
            let before = Entry::Resource(block).count(&a);
            let now = std::time::Instant::now();
            let mut state = Mining::default();
            let strikes = block.hardness().div_ceil(gear.harvest_power());
            for n in 0..strikes {
                let edit = block_action_at(
                    &world,
                    &mut a,
                    &mut state,
                    feet,
                    &[],
                    &intent,
                    now + std::time::Duration::from_millis(n as u64 * 200),
                )
                .unwrap();
                assert_eq!(edit.is_some(), n + 1 == strikes);
            }
            assert_eq!(Entry::Resource(block).count(&a), before + 1);
        }
    }
    #[test]
    fn retired_bows_cannot_attack_or_craft_even_when_owned() {
        for gear in [Gear::Bow, Gear::Longbow] {
            let (world, mut a, mut intent, feet) = fixture(BlockType::Air, Some(Entry::Gear(gear)));
            a.gear[gear as usize] = 1;
            a.mana = 100;
            intent.action = Action::Attack;
            let before = a.clone();
            let mut creatures = crate::creature::Creatures::new();
            assert!(attack(
                &world,
                &mut creatures,
                &mut a,
                &mut Mining::default(),
                feet,
                &intent
            )
            .is_err());
            assert_eq!(a, before);
            assert!(crate::crafting::gear_formula(gear, false).is_err());
            assert!(!Gear::available().any(|g| g == gear));
        }
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
            let hits = if entry.is_none() {
                1
            } else {
                BlockType::Campfire.hardness()
            };
            for n in 1..=hits {
                state.last_use = None;
                let result =
                    block_action(&world, &mut account, &mut state, feet, &[], &intent).unwrap();
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
                assert_eq!(
                    Entry::Resource(block).count(&account),
                    Entry::Resource(block).count(&before) + 2
                );
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
            if full == 2 {
                account.elements[crate::crafting::Element::Fire.index()] = u32::MAX;
            } else {
                let block = [BlockType::Stone, BlockType::OakWood][full];
                let i = COLLECTIBLE_BLOCKS.iter().position(|b| *b == block).unwrap();
                account.resources[i] = u32::MAX;
            }
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
    fn depleted_assignment_clears_and_restock_does_not_restore_it() {
        let mut account = Account::default();
        account.hotbar.slots = [None; 9];
        account.hotbar.slots[0] = Some(Entry::Resource(BlockType::Stone));
        account.hotbar.slots[8] = Some(Entry::Gear(Gear::Axe));
        account.gear[Gear::Axe as usize] = 1;
        assert_eq!(account.hotbar.entry().unwrap().count(&account), 0);
        assert!(account.prune_hotbar());
        assert_eq!(account.hotbar.entry(), None);
        account.hotbar.cycle(-1);
        assert_eq!(account.hotbar.active, 8);
        account.hotbar.cycle(1);
        assert_eq!(account.hotbar.active, 8);
        account.resources[COLLECTIBLE_BLOCKS
            .iter()
            .position(|b| *b == BlockType::Stone)
            .unwrap()] = 5;
        assert_eq!(account.hotbar.slots[0], None);
        account.hotbar.select(9);
        assert_eq!(account.hotbar.active, 8);
        account.hotbar.slots = [None; 9];
        account.hotbar.cycle(1);
        assert_eq!(account.hotbar.active, 8);
    }
    #[test]
    fn unavailable_gear_resources_and_spells_clear_every_binding_once() {
        let mut account = Account::default();
        account.gear.fill(0);
        account.known_spells = vec![42];
        account.gear[Gear::Axe as usize] = 1;
        account.hotbar.active = 3;
        account.hotbar.slots = [
            Some(Entry::Resource(BlockType::Stone)),
            Some(Entry::Gear(Gear::Sword)),
            Some(Entry::Spell(99)),
            Some(Entry::Spell(99)),
            Some(Entry::Spell(42)),
            Some(Entry::Gear(Gear::Axe)),
            None,
            None,
            None,
        ];
        let before = (account.revision, account.hotbar.revision);
        assert!(account.prune_hotbar());
        assert_eq!(account.hotbar.active, 3);
        assert_eq!(&account.hotbar.slots[..4], &[None; 4]);
        assert_eq!(account.hotbar.slots[4], Some(Entry::Spell(42)));
        assert_eq!(account.hotbar.slots[5], Some(Entry::Gear(Gear::Axe)));
        assert_eq!(
            (account.revision, account.hotbar.revision),
            (before.0 + 1, before.1 + 1)
        );
        let cleaned = account.clone();
        assert!(!account.prune_hotbar());
        assert_eq!(account, cleaned);
    }
    #[test]
    fn inventory_reconciliation_prunes_optimistic_slots_without_inventing_account_revisions() {
        let mut server = Account::default();
        server.revision = 10;
        server.hotbar.revision = 4;
        server.hotbar.slots = [None; 9];
        server.gear[Gear::Axe as usize] = 1;
        server.known_spells = vec![42];
        let mut client = server.clone();
        client.hotbar.revision = 20;
        client.hotbar.slots[0] = Some(Entry::Resource(BlockType::Stone));
        client.hotbar.slots[1] = Some(Entry::Gear(Gear::Axe));
        client.hotbar.slots[2] = Some(Entry::Spell(42));
        client.hotbar.active = 2;
        server.revision = 11;
        assert_eq!(
            receive_inventory(&mut client, server.clone(), true),
            Some(true)
        );
        assert_eq!(client.revision, 11);
        assert_eq!(client.hotbar.slots[0], None);
        assert_eq!(client.hotbar.slots[1], Some(Entry::Gear(Gear::Axe)));
        assert_eq!(client.hotbar.entry(), Some(Entry::Spell(42))); // Catalog may still be in flight.
        accept_hotbar(&mut server, &client.hotbar).unwrap();
        assert_eq!(
            receive_inventory(&mut client, server.clone(), true),
            Some(false)
        );
        server.known_spells.clear();
        server.revision += 1;
        server.prune_hotbar();
        assert_eq!(
            receive_inventory(&mut client, server.clone(), true),
            Some(false)
        );
        assert_eq!(client.hotbar.entry(), None);
        let mut stale = server.clone();
        stale.revision -= 1;
        stale.known_spells = vec![42];
        stale.hotbar.assign(Some(Entry::Spell(42)));
        let before = client.clone();
        assert_eq!(receive_inventory(&mut client, stale, true), None);
        assert_eq!(client, before);
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
