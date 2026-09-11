//! Deterministic recipes and host-side transactions. No LLM or UI dependencies.
use crate::creature::{CreatureDraft, CreatureKind, Creatures};
use crate::voxel::{BlockType, World, COLLECTIBLE_BLOCKS};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Element {
    Earth,
    Fire,
    Water,
    Life,
    Death,
}
impl Element {
    pub const ALL: [Self; 5] = [
        Self::Earth,
        Self::Fire,
        Self::Water,
        Self::Life,
        Self::Death,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
}
pub type Composition = [u32; 5];

/// Authority queries must read generated terrain even when a guest is outside the host's view.
pub fn load_interaction_area(world: &mut World, position: Vec3) {
    if !position.is_finite() || position.abs().max_element() > 1_000_000.0 {
        return;
    }
    let cx = (position.x.floor() as i32).div_euclid(crate::voxel::chunk::CHUNK_X);
    let cz = (position.z.floor() as i32).div_euclid(crate::voxel::chunk::CHUNK_Z);
    for x in cx - 1..=cx + 1 {
        for z in cz - 1..=cz + 1 {
            world.ensure_chunk_loaded(x, z);
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Slot {
    pub element: Element,
    pub amount: i64,
}
pub type Formula = [Option<Slot>; 5];
pub fn normalize(slots: &Formula) -> Result<Vec<Slot>, String> {
    let slots: Vec<_> = slots.iter().flatten().copied().collect();
    if slots
        .iter()
        .any(|s| s.amount <= 0 || s.amount > u32::MAX as i64)
    {
        return Err("Amounts must be positive 32-bit integers".into());
    }
    Ok(slots)
}
pub fn totals(slots: &[Slot]) -> Result<Composition, String> {
    let mut result = [0u32; 5];
    for s in slots {
        if s.amount <= 0 || s.amount > u32::MAX as i64 {
            return Err("Invalid amount".into());
        }
        result[s.element.index()] = result[s.element.index()]
            .checked_add(s.amount as u32)
            .ok_or("Element total overflow")?;
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub gear: [u32;4],
    pub hotbar: crate::equipment::Hotbar,
    pub elements: Composition,
    pub mana: u32,
    #[serde(with = "resource_counts")]
    pub resources: [u32; COLLECTIBLE_BLOCKS.len()],
    pub revision: u64,
}
impl Default for Account {
    fn default() -> Self {
        Self {
            gear: [1,1,1,0],
            hotbar: crate::equipment::Hotbar::default(),
            elements: [0; 5],
            mana: 0,
            resources: [0; COLLECTIBLE_BLOCKS.len()],
            revision: 0,
        }
    }
}
// Append-only resource slots. Legacy JSON saves contain 23 entries; missing new slots are zero.
// A bounded sequence also avoids serde's fixed-array limit and hostile length allocations.
mod resource_counts {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &[u32; COLLECTIBLE_BLOCKS.len()],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u32; COLLECTIBLE_BLOCKS.len()], D::Error> {
        struct Counts;
        impl<'de> serde::de::Visitor<'de> for Counts {
            type Value = [u32; COLLECTIBLE_BLOCKS.len()];
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "at most {} resource counts", COLLECTIBLE_BLOCKS.len())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = [0; COLLECTIBLE_BLOCKS.len()];
                for value in &mut values {
                    match seq.next_element()? {
                        Some(n) => *value = n,
                        None => return Ok(values),
                    }
                }
                if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("Too many resource counts"));
                }
                Ok(values)
            }
        }
        deserializer.deserialize_seq(Counts)
    }
}
impl Account {
    pub fn add_elements(&mut self, composition: Composition) -> Result<(), String> {
        let mut next = self.elements;
        for i in 0..5 {
            next[i] = next[i]
                .checked_add(composition[i])
                .ok_or("Element inventory full")?;
        }
        self.elements = next;
        Ok(())
    }
    pub fn can_afford_elements(&self, cost: Composition) -> bool {
        (0..5).all(|i| self.elements[i] >= cost[i])
    }
    pub fn consume_elements(&mut self, cost: Composition) -> Result<(), String> {
        if !self.can_afford_elements(cost) {
            let i = (0..5).find(|&i| self.elements[i] < cost[i]).unwrap();
            return Err(format!("Insufficient {:?}", Element::ALL[i]));
        }
        for (i, amount) in cost.iter().enumerate() {
            self.elements[i] -= amount;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Item,
    Block,
    Resource,
    Creature,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub kind: ObjectKind,
    pub id: String,
    pub quantity: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub inputs: Vec<Slot>,
    pub output: Output,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectComposition {
    pub kind: ObjectKind,
    pub id: String,
    pub elements: Composition,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub mana_costs: [u32; 5],
    pub conversion_rate: u32,
    pub recipes: Vec<Recipe>,
    pub compositions: Vec<ObjectComposition>,
}
pub fn creature_kind(id: &str) -> Option<CreatureKind> {
    Some(match id {
        "sheep" => CreatureKind::Sheep,
        "chicken" => CreatureKind::Chicken,
        "cow" => CreatureKind::Cow,
        "wolf" => CreatureKind::Wolf,
        "stinger" => CreatureKind::Stinger,
        "goblin" => CreatureKind::Goblin,
        "stone_golem" => CreatureKind::StoneGolem,
        "sunscorch" => CreatureKind::Sunscorch,
        "zombie" => CreatureKind::Zombie,
        "skeleton" => CreatureKind::Skeleton,
        "dragon_green" => CreatureKind::DragonGreen,
        "dragon_red" => CreatureKind::DragonRed,
        _ => return None,
    })
}
fn resource_index(id: &str) -> Option<usize> {
    let b = BlockType::from_name(id)?;
    COLLECTIBLE_BLOCKS.iter().position(|&x| x == b)
}
impl Registry {
    pub fn load() -> Result<Self, String> {
        // Embedded fallback keeps packaged builds independent of the working directory.
        let text = match std::fs::read_to_string(crate::runtime_paths::resource("data/crafting.json")) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                include_str!("../data/crafting.json").into()
            }
            Err(e) => return Err(format!("Cannot read crafting registry: {e}")),
        };
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let registry: Self =
            serde_json::from_str(text).map_err(|e| format!("Crafting registry: {e}"))?;
        registry.validate()?;
        Ok(registry)
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        let mut formulas = std::collections::HashSet::new();
        if self.conversion_rate == 0 {
            return Err("Conversion rate must be positive".into());
        }
        for r in &self.recipes {
            if r.id.is_empty() || !ids.insert(&r.id) || !formulas.insert(&r.inputs) {
                return Err(format!("Duplicate recipe id or formula: {}", r.id));
            }
            if !(1..=5).contains(&r.inputs.len()) {
                return Err(format!("Invalid slot count: {}", r.id));
            }
            totals(&r.inputs)?;
            if r.output.quantity == 0
                || (r.output.kind == ObjectKind::Creature
                    && r.output.quantity as usize > crate::world_api_gen::SCRIPT_CREATURES_MAX)
            {
                return Err(format!("Invalid output quantity: {}", r.id));
            }
            let valid = if r.output.kind == ObjectKind::Creature {
                creature_kind(&r.output.id).is_some()
            } else {
                resource_index(&r.output.id).is_some()
            };
            if !valid {
                return Err(format!("Unknown output: {}", r.output.id));
            }
        }
        let mut compositions = std::collections::HashSet::new();
        for c in &self.compositions {
            let kind = if c.kind == ObjectKind::Creature {
                "creature"
            } else {
                "block"
            };
            if !compositions.insert((kind, &c.id)) {
                return Err(format!("Duplicate composition: {}", c.id));
            }
            let valid = if c.kind == ObjectKind::Creature {
                creature_kind(&c.id).is_some()
            } else {
                BlockType::from_name(&c.id).is_some()
            };
            if !valid {
                return Err(format!("Unknown composition object: {}", c.id));
            }
        }
        // Extraction must never recover more of any element than crafting spent.
        // This component-wise invariant also prevents profit through longer recipe cycles.
        for recipe in &self.recipes {
            if recipe.output.kind == ObjectKind::Creature {
                continue;
            }
            let cost = totals(&recipe.inputs)?;
            let recovered = self.composition(recipe.output.kind, &recipe.output.id);
            if !compositions.contains(&("block", &recipe.output.id)) {
                return Err(format!("Missing output composition: {}", recipe.output.id));
            }
            for i in 0..5 {
                if recovered[i]
                    .checked_mul(recipe.output.quantity)
                    .is_none_or(|n| n > cost[i])
                {
                    return Err(format!(
                        "Recipe creates extractable elements: {}",
                        recipe.id
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn composition(&self, kind: ObjectKind, id: &str) -> Composition {
        self.compositions
            .iter()
            .find(|c| {
                c.id == id && (c.kind == ObjectKind::Creature) == (kind == ObjectKind::Creature)
            })
            .map_or([0; 5], |c| c.elements)
    }
    pub fn matched(&self, slots: &Formula) -> Result<&Recipe, String> {
        let inputs = normalize(slots)?;
        self.recipes
            .iter()
            .find(|r| r.inputs == inputs)
            .ok_or("Unknown formula".into())
    }
    pub fn mana_cost(&self, slots: &Formula) -> u32 {
        let n = slots.iter().flatten().count();
        if n == 0 {
            0
        } else {
            self.mana_costs[n - 1]
        }
    }
    // Run on a private account; the caller commits only after output preparation succeeds.
    fn prepare(&self, account: &mut Account, action: &Action) -> Result<Option<Output>, String> {
        match action {
            Action::Craft(slots) => {
                let r = self.matched(slots)?;
                account.consume_elements(totals(&r.inputs)?)?;
                account.mana = account
                    .mana
                    .checked_sub(self.mana_cost(slots))
                    .ok_or("Insufficient mana")?;
                if r.output.kind != ObjectKind::Creature {
                    let i = resource_index(&r.output.id).ok_or("Unknown output")?;
                    account.resources[i] = account.resources[i]
                        .checked_add(r.output.quantity)
                        .ok_or("Inventory full")?;
                }
                Ok(Some(r.output.clone()))
            }
            Action::Convert { element, amount } => {
                let cost = totals(&[Slot {
                    element: *element,
                    amount: *amount,
                }])?;
                account.consume_elements(cost)?;
                let mana = (*amount as u32)
                    .checked_mul(self.conversion_rate)
                    .ok_or("Mana overflow")?;
                account.mana = account.mana.checked_add(mana).ok_or("Mana balance full")?;
                Ok(None)
            }
            Action::Extract { block, amount } => {
                if *amount == 0 {
                    return Err("Amount must be positive".into());
                }
                let i = COLLECTIBLE_BLOCKS
                    .iter()
                    .position(|b| b == block)
                    .ok_or("Not a resource")?;
                let mut comp = self.composition(ObjectKind::Resource, block.id());
                if comp == [0; 5] {
                    return Err("No elemental composition".into());
                }
                for n in &mut comp {
                    *n = n.checked_mul(*amount).ok_or("Element overflow")?;
                }
                account.resources[i] = account.resources[i]
                    .checked_sub(*amount)
                    .ok_or("Insufficient resources")?;
                account.add_elements(comp)?;
                Ok(None)
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn execute(
        &self,
        account: &mut Account,
        revision: u64,
        action: &Action,
        world: &World,
        creatures: &mut Creatures,
        position: Vec3,
        players: &[Vec3],
    ) -> Result<String, String> {
        let (next, draft, output) = self.prepare_transaction(
            account, revision, action, world, creatures, position, players,
        )?;
        if let Some(draft) = draft {
            draft.commit(creatures);
        }
        *account = next;
        Ok(output.map_or_else(
            || "Conversion complete".into(),
            |o| format!("Created {} x{}", o.id, o.quantity),
        ))
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_transaction(
        &self,
        account: &Account,
        revision: u64,
        action: &Action,
        world: &World,
        creatures: &Creatures,
        position: Vec3,
        players: &[Vec3],
    ) -> Result<(Account, Option<CreatureDraft>, Option<Output>), String> {
        if revision != account.revision {
            return Err("Inventory changed; review and try again".into());
        }
        let mut next = account.clone();
        let output = self.prepare(&mut next, action)?;
        let mut draft = None;
        if let Some(o) = &output {
            if o.kind == ObjectKind::Creature {
                let mut d = CreatureDraft::new(creatures);
                let mut occupied = players.to_vec();
                for _ in 0..o.quantity {
                    let pos = spawn_position(world, creatures, position, &occupied).ok_or(
                        "Invalid spawn location: clear a nearby 3 x 3 x 3 space on solid ground",
                    )?;
                    d.spawn(
                        creature_kind(&o.id).ok_or("Unknown creature")?,
                        pos,
                        account.revision,
                    )
                    .ok_or("Creature limit reached")?;
                    occupied.push(pos);
                }
                draft = Some(d);
            }
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("Revision limit reached")?;
        Ok((next, draft, output))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn preview(
        &self,
        account: &Account,
        action: &Action,
        world: &World,
        creatures: &Creatures,
        position: Vec3,
        players: &[Vec3],
    ) -> Result<(), String> {
        self.prepare_transaction(
            account,
            account.revision,
            action,
            world,
            creatures,
            position,
            players,
        )
        .map(|_| ())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Craft(Formula),
    Convert { element: Element, amount: i64 },
    Extract { block: BlockType, amount: u32 },
}

fn spawn_position(
    world: &World,
    creatures: &Creatures,
    position: Vec3,
    players: &[Vec3],
) -> Option<Vec3> {
    if !position.is_finite() || position.abs().max_element() > 1_000_000.0 {
        return None;
    }
    let occupied = creatures.snapshot();
    for (dx, dz) in [(4, 0), (-4, 0), (0, 4), (0, -4), (4, 4), (-4, -4)] {
        let x = position.x.floor() as i32 + dx;
        let z = position.z.floor() as i32 + dz;
        for y in ((position.y as i32 - 4).max(1)..=(position.y as i32 + 4).min(44)).rev() {
            let pos = Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5);
            if players.iter().any(|p| p.distance(pos) < 3.0)
                || occupied
                    .iter()
                    .any(|c| Vec3::from_array(c.0).distance(pos) < 3.0)
            {
                continue;
            }
            let clear = (-1..=1).all(|ox| {
                (-1..=1).all(|oz| {
                    world.is_solid(x + ox, y - 1, z + oz)
                        && (0..3)
                            .all(|oy| world.get_block(x + ox, y + oy, z + oz) == BlockType::Air)
                })
            });
            if clear {
                return Some(pos);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_authority_loads_distant_terrain_and_saved_edits_before_querying() {
        let mut world = World::new(1);
        let pos = Vec3::new(500.5, 10.0, -500.5);
        world.edits.insert((500, 10, -501), BlockType::Bricks);
        assert_eq!(world.get_block(500, 10, -501), BlockType::Air);
        load_interaction_area(&mut world, pos);
        assert_eq!(world.get_block(500, 10, -501), BlockType::Bricks);
        let count = world.chunks.len();
        load_interaction_area(&mut world, Vec3::NAN);
        assert_eq!(count, world.chunks.len());
    }
    fn registry() -> Registry {
        Registry::parse(include_str!("../data/crafting.json")).unwrap()
    }
    fn formula(pairs: &[(Element, i64)]) -> Formula {
        let mut slots = [None; 5];
        for (i, &(element, amount)) in pairs.iter().enumerate() {
            slots[i] = Some(Slot { element, amount });
        }
        slots
    }
    fn rich() -> Account {
        Account {
            elements: [100; 5],
            mana: 300,
            ..Account::default()
        }
    }
    fn flat_world() -> World {
        let mut world = World::new(1);
        for x in -1..=1 {
            for z in -1..=1 {
                let mut chunk = crate::voxel::chunk::Chunk::new(x, z);
                for bx in 0..16 {
                    for bz in 0..16 {
                        chunk.set_local(bx, 9, bz, BlockType::Stone);
                    }
                }
                world.chunks.insert((x, z), chunk);
            }
        }
        world
    }
    fn execute(
        reg: &Registry,
        account: &mut Account,
        action: Action,
        world: &World,
        creatures: &mut Creatures,
    ) -> Result<String, String> {
        reg.execute(
            account,
            account.revision,
            &action,
            world,
            creatures,
            Vec3::new(0.5, 10.0, 0.5),
            &[],
        )
    }
    #[test]
    fn exact_order_quantities_repeated_elements_and_gap_normalization() {
        let r = registry();
        assert_eq!(
            r.matched(&formula(&[(Element::Earth, 2), (Element::Fire, 1)]))
                .unwrap()
                .output
                .id,
            "bricks"
        );
        assert_eq!(
            r.matched(&formula(&[(Element::Fire, 1), (Element::Earth, 2)]))
                .unwrap()
                .output
                .id,
            "basalt"
        );
        assert!(r
            .matched(&formula(&[(Element::Earth, 3), (Element::Fire, 1)]))
            .is_err());
        let repeated = formula(&[(Element::Earth, 1), (Element::Earth, 1)]);
        assert_eq!(r.matched(&repeated).unwrap().output.id, "cobblestone");
        assert_eq!(
            totals(&normalize(&repeated).unwrap()).unwrap(),
            [2, 0, 0, 0, 0]
        );
        let mut gaps = [None; 5];
        gaps[1] = repeated[0];
        gaps[4] = repeated[1];
        assert_eq!(
            r.matched(&gaps).unwrap().id,
            r.matched(&repeated).unwrap().id
        );
        assert!(r.matched(&[None; 5]).is_err());
    }
    #[test]
    fn nonpositive_and_overflowing_amounts_are_rejected() {
        for n in [0, -1, i64::MIN, u32::MAX as i64 + 1] {
            assert!(normalize(&formula(&[(Element::Earth, n)])).is_err());
        }
        assert!(totals(
            &normalize(&formula(&[
                (Element::Earth, u32::MAX as i64),
                (Element::Earth, 1)
            ]))
            .unwrap()
        )
        .is_err());
    }
    #[test]
    fn mana_depends_only_on_occupied_slots() {
        let r = registry();
        for (i, cost) in [0, 2, 4, 16, 256].into_iter().enumerate() {
            assert_eq!(
                r.mana_cost(&formula(&vec![(Element::Earth, 999); i + 1])),
                cost
            );
        }
        let mut gaps = [None; 5];
        gaps[0] = Some(Slot {
            element: Element::Fire,
            amount: 1,
        });
        gaps[4] = gaps[0];
        assert_eq!(r.mana_cost(&gaps), 2);
    }
    #[test]
    fn failed_material_mana_inventory_and_spawn_checks_are_atomic() {
        let reg = registry();
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let bricks = Action::Craft(formula(&[(Element::Earth, 2), (Element::Fire, 1)]));
        for (mut account, action, expected) in [
            (Account::default(), bricks.clone(), "Insufficient Earth"),
            (
                Account { mana: 0, ..rich() },
                bricks.clone(),
                "Insufficient mana",
            ),
            (
                {
                    let mut a = rich();
                    a.resources[resource_index("bricks").unwrap()] = u32::MAX;
                    a
                },
                bricks,
                "Inventory full",
            ),
            (
                rich(),
                Action::Craft(formula(&[(Element::Life, 2)])),
                "Invalid spawn location",
            ),
            (
                rich(),
                Action::Craft(formula(&[(Element::Death, 99)])),
                "Unknown formula",
            ),
        ] {
            let before = account.clone();
            let error = execute(&reg, &mut account, action, &world, &mut creatures).unwrap_err();
            assert!(error.starts_with(expected), "{error}");
            assert_eq!(account, before);
            assert!(creatures.snapshot().is_empty());
        }
    }
    #[test]
    fn successful_craft_consumes_exact_materials_and_mana_and_grants_output() {
        let mut account = rich();
        let mut creatures = Creatures::new();
        execute(
            &registry(),
            &mut account,
            Action::Craft(formula(&[(Element::Earth, 2), (Element::Fire, 1)])),
            &World::new(1),
            &mut creatures,
        )
        .unwrap();
        assert_eq!(account.elements, [98, 99, 100, 100, 100]);
        assert_eq!(account.mana, 298);
        assert_eq!(account.resources[resource_index("bricks").unwrap()], 1);
        assert_eq!(account.revision, 1);
    }
    #[test]
    fn creatures_spawn_on_clear_ground_and_replicate_through_existing_snapshot() {
        let mut account = rich();
        let mut creatures = Creatures::new();
        let reg = registry();
        let world = flat_world();
        let action = Action::Craft(formula(&[
            (Element::Earth, 5),
            (Element::Life, 2),
            (Element::Fire, 1),
        ]));
        reg.preview(
            &account,
            &action,
            &world,
            &creatures,
            Vec3::new(0.5, 10.0, 0.5),
            &[],
        )
        .unwrap();
        assert!(creatures.snapshot().is_empty());
        execute(&reg, &mut account, action, &world, &mut creatures).unwrap();
        assert_eq!(account.elements, [95, 99, 100, 98, 100]);
        assert_eq!(account.mana, 296);
        let snapshot = creatures.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].1, CreatureKind::StoneGolem.to_u8());
        assert_eq!(snapshot[0].0[1], 10.0);
    }
    #[test]
    fn exhausted_creature_budget_and_occupied_ground_consume_nothing() {
        let reg = registry();
        let world = flat_world();
        let mut creatures = Creatures::new();
        let mut account = rich();
        for i in 0..crate::world_api_gen::SCRIPT_CREATURES_MAX {
            creatures.spawn_one(
                CreatureKind::Sheep,
                Vec3::new(100.0, 10.0, i as f32),
                i as u64,
            );
        }
        let before = account.clone();
        let snapshot = creatures.snapshot_with_ids();
        let action = Action::Craft(formula(&[(Element::Life, 2)]));
        assert_eq!(
            execute(&reg, &mut account, action.clone(), &world, &mut creatures).unwrap_err(),
            "Creature limit reached"
        );
        assert_eq!(account, before);
        assert_eq!(snapshot, creatures.snapshot_with_ids());
        let mut creatures = Creatures::new();
        let positions: Vec<_> = [(4, 0), (-4, 0), (0, 4), (0, -4), (4, 4), (-4, -4)]
            .iter()
            .map(|&(x, z)| Vec3::new(x as f32 + 0.5, 10.0, z as f32 + 0.5))
            .collect();
        assert!(reg
            .execute(
                &mut account,
                0,
                &action,
                &world,
                &mut creatures,
                Vec3::new(0.5, 10.0, 0.5),
                &positions
            )
            .is_err());
        assert_eq!(account, before);
    }
    #[test]
    fn duplicate_ids_and_duplicate_formulas_fail_loading() {
        let mut r = registry();
        r.recipes.push(r.recipes[0].clone());
        assert!(Registry::parse(&serde_json::to_string(&r).unwrap())
            .unwrap_err()
            .contains("Duplicate"));
        r.recipes.last_mut().unwrap().id = "different_id".into();
        assert!(r.validate().unwrap_err().contains("Duplicate"));
    }
    #[test]
    fn extraction_and_explicit_conversion_are_atomic_and_configurable() {
        let mut r = registry();
        r.conversion_rate = 3;
        let mut a = Account::default();
        let mut creatures = Creatures::new();
        let world = World::new(1);
        a.resources[resource_index("stone").unwrap()] = 2;
        execute(
            &r,
            &mut a,
            Action::Extract {
                block: BlockType::Stone,
                amount: 2,
            },
            &world,
            &mut creatures,
        )
        .unwrap();
        assert_eq!(a.elements, [2, 0, 0, 0, 0]);
        assert_eq!(a.mana, 0);
        execute(
            &r,
            &mut a,
            Action::Convert {
                element: Element::Earth,
                amount: 2,
            },
            &world,
            &mut creatures,
        )
        .unwrap();
        assert_eq!(a.elements, [0; 5]);
        assert_eq!(a.mana, 6);
        for amount in [-1, 0, 1] {
            let before = a.clone();
            assert!(execute(
                &r,
                &mut a,
                Action::Convert {
                    element: Element::Earth,
                    amount
                },
                &world,
                &mut creatures
            )
            .is_err());
            assert_eq!(a, before);
        }
        a.elements[0] = 1;
        a.mana = u32::MAX;
        let before = a.clone();
        assert!(execute(
            &r,
            &mut a,
            Action::Convert {
                element: Element::Earth,
                amount: 1
            },
            &world,
            &mut creatures
        )
        .is_err());
        assert_eq!(a, before);
    }
    #[test]
    fn network_requests_cannot_choose_balances_and_replays_cannot_purchase_twice() {
        use crate::net::{decode, encode, Packet, ReliableMsg};
        let reg = registry();
        let mut account = rich();
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let packet = Packet::Reliable {
            id: 1,
            msg: ReliableMsg::CraftRequest {
                revision: 0,
                action: Action::Craft(formula(&[(Element::Earth, 1), (Element::Water, 1)])),
            },
        };
        let bytes = encode(&packet);
        for attempt in 0..2 {
            let Packet::Reliable {
                msg: ReliableMsg::CraftRequest { revision, action },
                ..
            } = decode(&bytes).unwrap()
            else {
                panic!()
            };
            let result = reg.execute(
                &mut account,
                revision,
                &action,
                &world,
                &mut creatures,
                Vec3::ZERO,
                &[],
            );
            assert_eq!(result.is_ok(), attempt == 0);
        }
        assert_eq!(account.elements[0], 99);
        assert_eq!(account.resources[resource_index("stone").unwrap()], 1);
        let snapshot = Packet::Reliable {
            id: 2,
            msg: ReliableMsg::CraftState {
                account: account.clone(),
                feedback: Some("Created stone".into()),
            },
        };
        let Packet::Reliable {
            msg: ReliableMsg::CraftState {
                account: replica, ..
            },
            ..
        } = decode(&encode(&snapshot)).unwrap()
        else {
            panic!()
        };
        assert_eq!(account, replica);
        let mut poor = Account::default();
        assert!(reg
            .execute(
                &mut poor,
                0,
                &Action::Craft(formula(&[(Element::Earth, 1)])),
                &world,
                &mut creatures,
                Vec3::ZERO,
                &[]
            )
            .is_err());
        assert_eq!(poor, Account::default());
    }
    #[test]
    fn composition_lookup_and_element_api_check_overflow_without_partial_mutation() {
        let r = registry();
        assert_eq!(r.composition(ObjectKind::Item, "stone"), [1, 0, 0, 0, 0]);
        assert_eq!(
            r.composition(ObjectKind::Creature, "sheep"),
            [0, 0, 0, 2, 0]
        );
        assert_eq!(r.composition(ObjectKind::Resource, "missing"), [0; 5]);
        let mut a = rich();
        a.elements[4] = u32::MAX;
        let before = a.clone();
        assert!(a.add_elements([1, 0, 0, 0, 1]).is_err());
        assert_eq!(a, before);
        assert!(a.consume_elements([101, 0, 0, 0, 0]).is_err());
        assert_eq!(a, before);
    }

    #[test]
    fn multi_creature_outputs_reserve_distinct_positions_and_rollback_the_entire_batch() {
        let mut reg = registry();
        let index = reg.recipes.iter().position(|r| r.id == "sheep").unwrap();
        let world = flat_world();
        let action = Action::Craft(formula(&[(Element::Life, 2)]));
        for (quantity, success) in [(2, true), (7, false)] {
            reg.recipes[index].output.quantity = quantity;
            reg.validate().unwrap();
            let mut account = rich();
            let before = account.clone();
            let mut creatures = Creatures::new();
            assert_eq!(
                execute(&reg, &mut account, action.clone(), &world, &mut creatures).is_ok(),
                success
            );
            if success {
                let snapshot = creatures.snapshot();
                assert_eq!(snapshot.len(), 2);
                assert!(
                    Vec3::from_array(snapshot[0].0).distance(Vec3::from_array(snapshot[1].0))
                        >= 3.0
                );
            } else {
                assert_eq!(account, before);
                assert!(creatures.snapshot().is_empty());
            }
        }
    }
}

#[cfg(test)]
mod resource_balance_tests {
    use super::*;
    #[test]
    fn all_resource_formulas_execute_without_recycling_profit() {
        let registry = Registry::parse(include_str!("../data/crafting.json")).unwrap();
        assert_eq!(COLLECTIBLE_BLOCKS.len(), 84);
        let world = World::new(5);
        let mut creatures = Creatures::new();
        for (index, block) in COLLECTIBLE_BLOCKS.iter().enumerate() {
            let info = crate::voxel::resource_catalog::info(*block);
            assert_eq!(
                crate::voxel::resource_catalog::RESOURCES[index].block,
                *block
            );
            let comp = registry.composition(ObjectKind::Resource, block.id());
            assert!(
                comp.iter().sum::<u32>() > 0,
                "{} has no composition",
                block.id()
            );
            let recipe = registry.recipes.iter().find(|r| r.output.id == block.id());
            assert_eq!(
                recipe.is_some(),
                info.source != "natural",
                "{} source mismatch",
                block.id()
            );
            if let Some(recipe) = recipe {
                assert!((2..=3).contains(&recipe.inputs.len()));
                assert_ne!(recipe.output.kind, ObjectKind::Item);
                let mut slots = [None; 5];
                for (i, s) in recipe.inputs.iter().enumerate() {
                    slots[i] = Some(*s);
                }
                let initial = totals(&recipe.inputs).unwrap();
                let mana = registry.mana_cost(&slots);
                let mut account = Account {
                    elements: initial,
                    mana,
                    ..Default::default()
                };
                registry
                    .execute(
                        &mut account,
                        0,
                        &Action::Craft(slots),
                        &world,
                        &mut creatures,
                        Vec3::ZERO,
                        &[],
                    )
                    .unwrap();
                assert_eq!(account.resources[index], recipe.output.quantity);
                registry
                    .execute(
                        &mut account,
                        1,
                        &Action::Extract {
                            block: *block,
                            amount: recipe.output.quantity,
                        },
                        &world,
                        &mut creatures,
                        Vec3::ZERO,
                        &[],
                    )
                    .unwrap();
                assert_eq!(account.resources[index], 0);
                assert_eq!(account.mana, 0);
                assert!(account.elements.iter().zip(initial).all(|(a, b)| *a <= b));
            }
        }
        let bytes = crate::net::encode(&crate::net::Packet::Reliable {
            id: 1,
            msg: crate::net::ReliableMsg::CraftRegistry(registry),
        });
        assert!(bytes.len() < crate::transport::MAX_PACKET_BYTES);
        assert!(crate::net::decode(&bytes).is_some());
    }
    #[test]
    fn legacy_inventory_expands_without_shifting_counts() {
        let legacy =
            serde_json::json!({"resources":(1..=23).collect::<Vec<u32>>(),"mana":17,"revision":9});
        let account: Account = serde_json::from_value(legacy).unwrap();
        assert_eq!(account.resources[..23], (1..=23).collect::<Vec<u32>>());
        assert!(account.resources[23..].iter().all(|n| *n == 0));
        assert_eq!(account.mana, 17);
        let previous: Account =
            serde_json::from_value(serde_json::json!({"resources":(1..=72).collect::<Vec<u32>>()}))
                .unwrap();
        assert_eq!(previous.resources[71], 72);
        assert!(previous.resources[72..].iter().all(|n| *n == 0));
        let before_crystals: Account = serde_json::from_value(
            serde_json::json!({"resources":(1..=82).collect::<Vec<u32>>()}),
        ).unwrap();
        assert_eq!(before_crystals.resources[..82], (1..=82).collect::<Vec<u32>>());
        assert_eq!(COLLECTIBLE_BLOCKS[82], BlockType::Crystal);
        assert_eq!(before_crystals.resources[82], 0);
        assert_eq!(BlockType::Fern as u8, 76);
        let before_snow: Account=serde_json::from_value(serde_json::json!({"resources":(1..=83).collect::<Vec<u32>>()})).unwrap();
        assert_eq!(before_snow.resources[..83],(1..=83).collect::<Vec<u32>>());
        assert_eq!(COLLECTIBLE_BLOCKS[83],BlockType::Snow);
        assert_eq!(before_snow.resources[83],0);
        let mut expanded = account;
        expanded.resources[COLLECTIBLE_BLOCKS.len() - 1] = 55;
        let bytes = bincode::serialize(&expanded).unwrap();
        assert_eq!(bincode::deserialize::<Account>(&bytes).unwrap(), expanded);
        let too_many = serde_json::json!({"resources":vec![0;COLLECTIBLE_BLOCKS.len()+1]});
        assert!(serde_json::from_value::<Account>(too_many).is_err());
        assert_eq!(BlockType::RedStone as u8, 26);
        assert_eq!(BlockType::IronOre as u8, 27);
    }
    #[test]
    fn registry_rejects_element_multiplication() {
        let mut registry = Registry::parse(include_str!("../data/crafting.json")).unwrap();
        registry
            .recipes
            .iter_mut()
            .find(|r| r.output.id == "iron")
            .unwrap()
            .output
            .quantity = 2;
        assert!(registry
            .validate()
            .unwrap_err()
            .contains("creates extractable elements"));
    }
}

#[cfg(test)]
mod resource_progression_tests {
    use super::*;
    #[test]
    fn iron_ore_and_coal_can_fund_iron_without_starter_mana() {
        let registry = Registry::parse(include_str!("../data/crafting.json")).unwrap();
        let mut account = Account::default();
        account.resources[resource_index("iron_ore").unwrap()] = 1;
        account.resources[resource_index("coal").unwrap()] = 1;
        let world = World::new(1);
        let mut creatures = Creatures::new();
        for block in [BlockType::IronOre, BlockType::Coal] {
            let rev = account.revision;
            registry
                .execute(
                    &mut account,
                    rev,
                    &Action::Extract { block, amount: 1 },
                    &world,
                    &mut creatures,
                    Vec3::ZERO,
                    &[],
                )
                .unwrap();
        }
        let rev = account.revision;
        registry
            .execute(
                &mut account,
                rev,
                &Action::Convert {
                    element: Element::Fire,
                    amount: 2,
                },
                &world,
                &mut creatures,
                Vec3::ZERO,
                &[],
            )
            .unwrap();
        let mut slots = [None; 5];
        slots[0] = Some(Slot {
            element: Element::Earth,
            amount: 3,
        });
        slots[1] = Some(Slot {
            element: Element::Fire,
            amount: 2,
        });
        let rev = account.revision;
        registry
            .execute(
                &mut account,
                rev,
                &Action::Craft(slots),
                &world,
                &mut creatures,
                Vec3::ZERO,
                &[],
            )
            .unwrap();
        assert_eq!(account.resources[resource_index("iron").unwrap()], 1);
        assert_eq!(account.mana, 0);
    }
}
