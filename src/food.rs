//! Food transactions shared by inventory eating and campfire cooking.
use crate::{
    crafting::Account,
    player::MAX_HEALTH,
    voxel::{BlockType, COLLECTIBLE_BLOCKS},
};

pub const MAX_COOK_BATCH: u32 = 64;
pub const CAMPFIRE_DISH: &str = "harvest:campfire_dish";
pub const CAMPFIRE_HERBAL_DISH: &str = "harvest:campfire_herbal_dish";
pub const CAMPFIRE_POISONOUS_DISH: &str = "harvest:campfire_poisonous_dish";
pub fn harvest_name(item: &str) -> String { match item { "harvest:milk" => "Milk", "harvest:wool" => "Wool", "harvest:cooked_milk" => "Cooked Milk", "harvest:cooked_egg" => "Fried Egg", "harvest:cooked_honey" => "Melted Honey", "harvest:egg" => "Egg", "harvest:honey" => "Honey", "harvest:cooked_pumpkin" => "Cooked Pumpkin", "harvest:cooked_mushroom" | "harvest:fired_mushroom" => "Cooked Mushroom", "harvest:cooked_glowcap" | "harvest:fired_glowcap" => "Cooked Glowcap", CAMPFIRE_DISH => "Campfire Dish", CAMPFIRE_HERBAL_DISH => "Herbal Purifying Stew", CAMPFIRE_POISONOUS_DISH => "Toxic Campfire Dish", _ => item.strip_prefix("harvest:").unwrap_or(item) }.into() }
pub fn harvest_healing(item: &str) -> Option<f32> {
    Some(match item { "harvest:egg" => 0., "harvest:cooked_egg" => 8., "harvest:milk" => 4., "harvest:cooked_milk" => 12., "harvest:honey" => 12., "harvest:cooked_honey" => 8., "harvest:cooked_pumpkin" => 20., "harvest:cooked_mushroom" | "harvest:fired_mushroom" => 8., "harvest:glowcap" | "harvest:cooked_glowcap" | "harvest:fired_glowcap" => 0., CAMPFIRE_DISH | CAMPFIRE_HERBAL_DISH | CAMPFIRE_POISONOUS_DISH => 24., _ => return None })
}
pub fn harvest_satiety(item: &str) -> f32 {
    match item { "harvest:egg" => 0., "harvest:cooked_egg" => 12., "harvest:milk" => 10., "harvest:cooked_milk" => 18., "harvest:honey" => 16., "harvest:cooked_honey" => 10., "harvest:cooked_pumpkin" => 30., "harvest:cooked_mushroom" => 12., CAMPFIRE_DISH | CAMPFIRE_HERBAL_DISH => 24., _ => 0. }
}
pub fn cook_harvest(account: &mut Account, item: &str, amount: u32) -> Result<(), String> {
    if !(1..=MAX_COOK_BATCH).contains(&amount) { return Err("Cook between 1 and 64 items at a time".into()); }
    let cooked = match item { "harvest:egg" => "harvest:cooked_egg", "harvest:milk" => "harvest:cooked_milk", "harvest:honey" => "harvest:cooked_honey", "harvest:mushroom" => "harvest:cooked_mushroom", "harvest:glowcap" => "harvest:cooked_glowcap", _ => return Err("This resource cannot be cooked".into()) };
    let raw_count = account.production_goods.get(item).copied().unwrap_or(0);
    if raw_count < amount { return Err("Not enough raw ingredients".into()); }
    let cooked_count = account.production_goods.get(cooked).copied().unwrap_or(0);
    let new_count = cooked_count.checked_add(amount).ok_or("Cooked stack is full")?;
    if raw_count == amount { account.production_goods.remove(item); } else { account.production_goods.insert(item.into(), raw_count - amount); }
    account.production_goods.insert(cooked.into(), new_count);
    Ok(())
}

/// Consume a small mixed batch and place the locally generated campfire dish in
/// the player's production inventory. The transaction is performed on the
/// cloned account by the adventure layer, so a failed ingredient check is
/// atomic.
pub fn cook_batch(account: &mut Account, ingredients: &[String]) -> Result<&'static str, String> {
    if !(1..=4).contains(&ingredients.len()) { return Err("A dish needs 1 to 4 ingredients".into()); }
    let purifying = ingredients.iter().any(|ingredient| ingredient == "block:WildHerbs");
    let poisonous = ingredients.iter().any(|ingredient| ingredient == "block:Glowcap");
    for ingredient in ingredients {
        if let Some(block_name) = ingredient.strip_prefix("block:") {
            let block = match block_name {
                "Meat" => BlockType::Meat,
                "Pumpkin" => BlockType::Pumpkin,
                "BrownMushroom" => BlockType::BrownMushroom,
                "Glowcap" => BlockType::Glowcap,
                "WildHerbs" => BlockType::WildHerbs,
                _ => return Err("That ingredient cannot be cooked".into()),
            };
            let index = COLLECTIBLE_BLOCKS.iter().position(|candidate| *candidate == block).ok_or("Ingredient unavailable")?;
            account.resources[index] = account.resources[index].checked_sub(1).ok_or("Not enough ingredients")?;
        } else if ingredient.starts_with("harvest:") {
            let count = account.production_goods.get(ingredient).copied().unwrap_or(0);
            if count == 0 { return Err(format!("Not enough {}", harvest_name(ingredient))); }
            if count == 1 { account.production_goods.remove(ingredient); } else { account.production_goods.insert(ingredient.clone(), count - 1); }
        } else {
            return Err("That ingredient cannot be cooked".into());
        }
    }
    let dish = match (purifying, poisonous) {
        (true, false) => CAMPFIRE_HERBAL_DISH,
        (false, true) => CAMPFIRE_POISONOUS_DISH,
        // Herbs neutralize Glowcap when both are in the same dish.
        (true, true) | (false, false) => CAMPFIRE_DISH,
    };
    let current = account.production_goods.get(dish).copied().unwrap_or(0);
    account.production_goods.insert(dish.into(), current.checked_add(1).ok_or("Dish stack is full")?);
    Ok(dish)
}
pub fn cook_pumpkin(account: &mut Account, amount: u32) -> Result<(), String> {
    if !(1..=MAX_COOK_BATCH).contains(&amount) { return Err("Cook between 1 and 64 pumpkins at a time".into()); }
    let raw = crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b| *b == BlockType::Pumpkin).ok_or("Pumpkin resource unavailable")?;
    let remaining = account.resources[raw].checked_sub(amount).ok_or("Not enough raw pumpkins")?;
    let cooked = account.production_goods.get("harvest:cooked_pumpkin").copied().unwrap_or(0).checked_add(amount).ok_or("Cooked stack is full")?;
    account.resources[raw] = remaining;
    account.production_goods.insert("harvest:cooked_pumpkin".into(), cooked);
    Ok(())
}
pub fn cook_mushroom(account: &mut Account, block: BlockType, amount: u32) -> Result<(), String> {
    if !(1..=MAX_COOK_BATCH).contains(&amount) { return Err("Cook between 1 and 64 mushrooms at a time".into()); }
    if !matches!(block, BlockType::BrownMushroom | BlockType::Glowcap) { return Err("This mushroom cannot be cooked".into()); }
    let raw = crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b| *b == block).ok_or("Mushroom resource unavailable")?;
    let remaining = account.resources[raw].checked_sub(amount).ok_or("Not enough mushrooms")?;
    account.resources[raw] = remaining;
    let id = if block == BlockType::Glowcap { "harvest:cooked_glowcap" } else { "harvest:cooked_mushroom" };
    account.production_goods.insert(id.into(), account.production_goods.get(id).copied().unwrap_or(0).checked_add(amount).ok_or("Cooked stack is full")?);
    Ok(())
}
pub fn healing(block: BlockType) -> Option<f32> {
    let definition = crate::resource_defs::definition(block);
    definition.edible.then_some(definition.raw_health_change.max(0) as f32)
}
pub fn satiety(block: BlockType) -> f32 {
    match block {
        BlockType::CookedMeat => 32.,
        BlockType::Meat => 16.,
        BlockType::Pumpkin => 18.,
        BlockType::WildHerbs | BlockType::BrownMushroom => 10.,
        _ => 0.,
    }
}
pub fn inventory_only(block: BlockType) -> bool {
    matches!(block, BlockType::Meat | BlockType::CookedMeat)
}
fn index(block: BlockType) -> usize {
    COLLECTIBLE_BLOCKS
        .iter()
        .position(|b| *b == block)
        .expect("food is a resource")
}

/// Consumes exactly one owned food and returns the heal amount for the host to apply.
/// Every rejection leaves inventory unchanged; the account revision rejects duplicate requests.
pub fn eat(
    account: &mut Account,
    health: f32,
    block: BlockType,
    revision: u64,
) -> Result<f32, String> {
    if account.revision != revision {
        return Err("Inventory changed; try eating again".into());
    }
    let amount = healing(block).ok_or("This resource cannot be eaten")?;
    if !health.is_finite() || health <= 0. || health >= MAX_HEALTH {
        return Err("Recover before eating".into());
    }
    let i = index(block);
    let count = account.resources[i]
        .checked_sub(1)
        .ok_or("No food left in this stack")?;
    let next = account
        .revision
        .checked_add(1)
        .ok_or("Inventory revision limit reached")?;
    account.resources[i] = count;
    account.revision = next;
    Ok(amount.min(MAX_HEALTH - health))
}

/// Called only after campfire access and revision validation, inside the camp transaction.
pub fn cook(account: &mut Account, amount: u32) -> Result<(), String> {
    if !(1..=MAX_COOK_BATCH).contains(&amount) {
        return Err("Cook between 1 and 64 pieces at a time".into());
    }
    let raw = index(BlockType::Meat);
    let cooked = index(BlockType::CookedMeat);
    let remaining = account.resources[raw]
        .checked_sub(amount)
        .ok_or("Not enough raw meat")?;
    let output = account.resources[cooked]
        .checked_add(amount)
        .ok_or("Cooked meat stack is full")?;
    account.resources[raw] = remaining;
    account.resources[cooked] = output;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn food_heals_consumes_once_and_caps_health() {
        for (food, hp) in [
            (BlockType::Meat, 8.),
            (BlockType::CookedMeat, 25.),
            (BlockType::Pumpkin, 10.),
            (BlockType::WildHerbs, 5.),
            (BlockType::BrownMushroom, 6.),
        ] {
            let mut a = Account::default();
            a.resources[index(food)] = 2;
            let rev = a.revision;
            assert_eq!(eat(&mut a, 50., food, rev), Ok(hp));
            assert_eq!(a.resources[index(food)], 1);
            let before = a.clone();
            assert!(eat(&mut a, 50. + hp, food, rev).is_err());
            assert_eq!(a, before);
            assert_eq!(eat(&mut a, MAX_HEALTH - 1., food, rev + 1), Ok(1.));
            assert_eq!(a.resources[index(food)], 0);
        }
    }
    #[test]
    fn rejected_meals_keep_inventory_unchanged() {
        for (hp, food, count, rev) in [
            (100., BlockType::Meat, 1, 0),
            (0., BlockType::Meat, 1, 0),
            (f32::NAN, BlockType::Meat, 1, 0),
            (50., BlockType::Stone, 1, 0),
            (50., BlockType::Meat, 0, 0),
            (50., BlockType::Meat, 1, u64::MAX),
        ] {
            let mut a = Account::default();
            a.resources[index(food)] = count;
            a.revision = rev;
            let before = a.clone();
            assert!(eat(&mut a, hp, food, rev).is_err());
            assert_eq!(a, before);
        }
    }
    #[test]
    fn cooking_rejects_invalid_batches_and_overflow_atomically() {
        for (raw, cooked, amount) in [(2, 0, 0), (100, 0, 65), (1, 0, 2), (2, u32::MAX, 1)] {
            let mut a = Account::default();
            a.resources[index(BlockType::Meat)] = raw;
            a.resources[index(BlockType::CookedMeat)] = cooked;
            let before = a.clone();
            assert!(cook(&mut a, amount).is_err());
            assert_eq!(a, before);
        }
    }
    #[test]
    fn pumpkins_are_edible_raw_and_gain_value_when_cooked() {
        let mut account = Account::default();
        let index = crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b| *b == BlockType::Pumpkin).unwrap();
        account.resources[index] = 2;
        cook_pumpkin(&mut account, 1).unwrap();
        assert_eq!(account.resources[index], 1);
        assert_eq!(account.production_goods.get("harvest:cooked_pumpkin"), Some(&1));
        assert_eq!(harvest_healing("harvest:cooked_pumpkin"), Some(20.));
        assert!(harvest_satiety("harvest:cooked_pumpkin") > satiety(BlockType::Pumpkin));
    }
    #[test]
    fn legacy_inventory_and_food_roundtrip() {
        let mut a = Account::default();
        a.resources[index(BlockType::Meat)] = 3;
        a.resources[index(BlockType::CookedMeat)] = 7;
        let bytes = bincode::serialize(&a).unwrap();
        assert_eq!(bincode::deserialize::<Account>(&bytes).unwrap(), a);
        let mut old = serde_json::to_value(&a).unwrap();
        old["resources"].as_array_mut().unwrap().truncate(84);
        let restored: Account = serde_json::from_value(old).unwrap();
        assert_eq!(&restored.resources[..84], &a.resources[..84]);
        assert_eq!(restored.resources[index(BlockType::Meat)], 0);
        assert_eq!(restored.resources[index(BlockType::CookedMeat)], 0);
    }
}
