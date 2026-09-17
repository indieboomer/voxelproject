//! Technical checks run outside the decision provider, at transaction boundaries.
use super::{Action, Effects};
use crate::{
    crafting, equipment,
    voxel::{BlockType, COLLECTIBLE_BLOCKS},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub invariant: String,
    pub tick: u64,
    pub event_reference: String,
    pub expected: String,
    pub observed: String,
    pub basis: String,
    pub classification: String,
    pub severity: String,
    pub confidence: String,
    pub reproduced: bool,
}
impl Finding {
    pub fn new(tick: u64, invariant: &str, expected: &str, observed: String) -> Self {
        Self {
            invariant: invariant.into(),
            tick,
            event_reference: format!("events.jsonl:outcome.tick={tick}"),
            expected: expected.into(),
            observed,
            basis: "transaction invariant".into(),
            classification: "uncertain".into(),
            severity: "high".into(),
            confidence: "high for invariant violation; cause requires review".into(),
            reproduced: false,
        }
    }
}

// Signed balances avoid overflow in the observer and do not call game mutation code.
fn balances(a: &crafting::Account) -> Vec<i64> {
    a.resources
        .iter()
        .chain(a.elements.iter())
        .chain(a.gear.iter())
        .copied()
        .chain(std::iter::once(a.mana))
        .map(i64::from)
        .collect()
}
fn resource(delta: &mut [i64], block: BlockType, amount: i64) {
    if let Some(i) = COLLECTIBLE_BLOCKS.iter().position(|b| *b == block) {
        delta[i] += amount;
    }
}

/// Evaluate the direct block changes before the host commits them. Later host callbacks
/// may repair this condition; reports keep the cause uncertain pending reproduction.
pub fn support(tick: u64, world: &crate::voxel::World, effects: &Effects) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (cell, new, old) in &effects.edits {
        if !old.is_solid() || new.is_solid() {
            continue;
        }
        let above = (cell.0, cell.1 + 1, cell.2);
        let block = effects
            .edits
            .iter()
            .find(|(p, _, _)| *p == above)
            .map_or_else(
                || world.get_block(above.0, above.1, above.2),
                |(_, b, _)| *b,
            );
        if block.def().only_on_top {
            let mut finding = Finding::new(
                tick,
                "decoration_support",
                "Blocks marked only_on_top must sit on solid ground",
                format!(
                    "Direct edit at {cell:?} leaves {} at {above:?} above {}",
                    block.id(),
                    new.id()
                ),
            );
            finding.basis = "src/voxel/block.rs: BlockDef::only_on_top contract; equipment placement support validator".into();
            finding.severity = "low".into();
            findings.push(finding);
        }
    }
    findings
}

/// Player collision box contract: 0.6 wide, 1.8 tall, anchored at the feet.
pub fn embedded(world: &crate::voxel::World, feet: glam::Vec3) -> bool {
    if !feet.is_finite() {
        return false;
    }
    let min = feet - glam::Vec3::new(0.3, 0.0, 0.3);
    let max = feet + glam::Vec3::new(0.3, 1.8, 0.3) - glam::Vec3::splat(0.0001);
    for x in min.x.floor() as i32..=max.x.floor() as i32 {
        for y in min.y.floor() as i32..=max.y.floor() as i32 {
            for z in min.z.floor() as i32..=max.z.floor() as i32 {
                if world.get_block(x, y, z).is_solid() {
                    return true;
                }
            }
        }
    }
    false
}

pub fn transaction(
    tick: u64,
    action: &Action,
    before: &crafting::Account,
    after: &crafting::Account,
    rejected: bool,
    effects: &Effects,
    registry: &crafting::Registry,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    if rejected {
        if before != after || !effects.edits.is_empty() {
            findings.push(Finding::new(
                tick,
                "rejected_action_atomicity",
                "Rejected actions preserve inventory and emit no block edits",
                format!(
                    "Inventory changed: {}; edits: {:?}",
                    before != after,
                    effects.edits
                ),
            ));
        }
        return findings;
    }
    let initial = balances(before);
    let actual: Vec<i64> = balances(after)
        .iter()
        .zip(&initial)
        .map(|(a, b)| a - b)
        .collect();
    let mut expected = vec![0i64; initial.len()];
    let element = COLLECTIBLE_BLOCKS.len();
    let gear = element + 5;
    let mana = gear + crate::equipment::Gear::ALL.len();
    let checked = match action {
        Action::Wait
        | Action::Move { .. }
        | Action::Look { .. }
        | Action::Equip { .. }
        | Action::Interact { .. } => true,
        Action::Attack => {
            if let Some(equipment::Entry::Gear(g)) = before.hotbar.entry() {
                if let Some((_, _, _, cost)) = g.weapon() {
                    expected[mana] -= i64::from(cost);
                }
            }
            true
        }
        Action::Mine { .. } => {
            for (_, new, old) in &effects.edits {
                if *new != BlockType::Air {
                    continue;
                }
                if *old == BlockType::Campfire {
                    resource(&mut expected, BlockType::Stone, 2);
                    resource(&mut expected, BlockType::OakWood, 2);
                    expected[element + crafting::Element::Fire as usize] += 4;
                } else {
                    resource(&mut expected, *old, 1);
                }
            }
            true
        }
        Action::Place { .. } => {
            for (_, block, _) in &effects.edits {
                resource(&mut expected, *block, -1);
            }
            true
        }
        Action::Craft { recipe } => {
            match recipe {
                crafting::Action::EquipTorch(_) => {}
                crafting::Action::BindSheep => {
                    let slots = [
                        Some(crafting::Slot {
                            element: crafting::Element::Life,
                            amount: 2,
                        }),
                        None,
                        None,
                        None,
                        None,
                    ];
                    expected[element + crafting::Element::Life as usize] -= 2;
                    expected[mana] -= i64::from(registry.mana_cost(&slots));
                }
                crafting::Action::Craft(slots) => {
                    let Ok(recipe) = registry.matched(slots) else {
                        return vec![Finding::new(
                            tick,
                            "known_recipe",
                            "Accepted crafting uses a known recipe",
                            "Unknown recipe accepted".into(),
                        )];
                    };
                    for slot in &recipe.inputs {
                        expected[element + slot.element as usize] -= slot.amount;
                    }
                    expected[mana] -= i64::from(registry.mana_cost(slots));
                    if recipe.output.kind != crafting::ObjectKind::Creature {
                        if let Some(block) = BlockType::from_name(&recipe.output.id) {
                            resource(&mut expected, block, i64::from(recipe.output.quantity));
                        }
                    }
                }
                crafting::Action::Convert { element: e, amount } => {
                    expected[element + *e as usize] -= *amount;
                    expected[mana] += amount.saturating_mul(i64::from(registry.conversion_rate));
                }
                crafting::Action::Extract { block, amount } => {
                    resource(&mut expected, *block, -i64::from(*amount));
                    for (i, n) in registry
                        .composition(crafting::ObjectKind::Resource, block.id())
                        .iter()
                        .enumerate()
                    {
                        expected[element + i] += i64::from(*n) * i64::from(*amount);
                    }
                    expected[mana] -= i64::from(registry.mana_charge(*amount));
                }
                crafting::Action::CraftGear(g) | crafting::Action::SalvageGear(g) => {
                    let salvage = matches!(recipe, crafting::Action::SalvageGear(_));
                    let Ok((_, _, cost)) = crafting::gear_formula(*g, salvage) else {
                        return findings;
                    };
                    let sign = if salvage { 1 } else { -1 };
                    for (block, n) in g.ingredients(salvage) {
                        resource(&mut expected, block, sign * i64::from(n));
                    }
                    expected[gear + *g as usize] -= sign;
                    expected[mana] -= i64::from(registry.mana_charge(cost));
                }
            }
            true
        }
        // Device and loot balances need their own cross-inventory ledgers.
        Action::Device { .. } | Action::Collect => false,
    };
    if checked && actual != expected {
        findings.push(Finding::new(tick, "resource_balance",
            "Resource, element, equipment and mana deltas equal the recorded action's costs and rewards",
            format!("Expected deltas {expected:?}; observed {actual:?} (resources, elements, gear, mana)")));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equipment;
    #[test]
    fn detects_free_tool_and_rejected_spending_without_trusting_outcome_text() {
        let registry = crafting::Registry::load().unwrap();
        let before = crafting::Account::default();
        let mut after = before.clone();
        after.gear[equipment::Gear::Axe as usize] += 1;
        let action = Action::Craft {
            recipe: crafting::Action::CraftGear(equipment::Gear::Axe),
        };
        assert_eq!(
            transaction(
                1,
                &action,
                &before,
                &after,
                false,
                &Effects::default(),
                &registry
            )[0]
            .invariant,
            "resource_balance"
        );
        assert_eq!(
            transaction(
                1,
                &action,
                &before,
                &after,
                true,
                &Effects::default(),
                &registry
            )[0]
            .invariant,
            "rejected_action_atomicity"
        );
        assert!(transaction(
            1,
            &action,
            &before,
            &before,
            true,
            &Effects::default(),
            &registry
        )
        .is_empty());
    }
}
