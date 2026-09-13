//! Optional, independent contracts. Only authoritative gameplay writes progress.
use crate::{
    adventure::{self, Cell},
    crafting::{Account, Element},
    equipment::{Entry, Gear},
    voxel::BlockType,
};
use serde::{Deserialize, Serialize};

pub const NAMES: [&str; 6] = [
    "Sage",
    "Elf Ranger",
    "Warrior",
    "Merchant",
    "Fire Sorceress",
    "Necromancer",
];
pub const GREETINGS: [&str; 6] = [
    "Start with the ordinary. Even stone remembers the elements.",
    "The paths need watching, and the watch needs supplies.",
    "Make your own blade. Then learn when to use it.",
    "A little storage and a steady supply make a settlement.",
    "Fire is more than a weapon. Give someone a warm place to stop.",
    "Nothing is wasted. What? Even I enjoy watching sheep.",
];
pub struct Quest {
    pub npc: u8,
    pub title: &'static str,
    pub objective: &'static str,
    pub target: u32,
}
pub const QUESTS: [Quest; 20] = [
    Quest {
        npc: 0,
        title: "A Solid Beginning",
        objective: "Bring 10 stone.",
        target: 10,
    },
    Quest {
        npc: 0,
        title: "The Five Foundations",
        objective: "Bring one each of Earth, Fire, Water, Life and Death.",
        target: 5,
    },
    Quest {
        npc: 0,
        title: "Shape the World",
        objective: "Craft stone with Earth 1, then Water 1 [C].",
        target: 1,
    },
    Quest {
        npc: 0,
        title: "A Little Reserve",
        objective: "Place a Mana Vessel [B] and store at least 10 mana in it.",
        target: 1,
    },
    Quest {
        npc: 1,
        title: "Wood for the Watch",
        objective: "Bring 12 oak wood.",
        target: 12,
    },
    Quest {
        npc: 1,
        title: "Hungry Shadows",
        objective: "Defeat two wolves.",
        target: 2,
    },
    Quest {
        npc: 1,
        title: "Ready for the Wilds",
        objective: "Craft a bow [C], assign it in I and select its hotbar slot.",
        target: 1,
    },
    Quest {
        npc: 2,
        title: "Your First Blade",
        objective: "Craft a sword [C]. Starter equipment does not count.",
        target: 1,
    },
    Quest {
        npc: 2,
        title: "Restless Bones",
        objective: "Defeat three skeletons.",
        target: 3,
    },
    Quest {
        npc: 2,
        title: "Back to the Grave",
        objective: "Defeat three zombies.",
        target: 3,
    },
    Quest {
        npc: 2,
        title: "A Sharper Lesson",
        objective: "Defeat one goblin.",
        target: 1,
    },
    Quest {
        npc: 3,
        title: "Room for More",
        objective: "Place a chest [B].",
        target: 1,
    },
    Quest {
        npc: 3,
        title: "Fresh Supplies",
        objective: "Bring 20 oak wood and 10 stone.",
        target: 30,
    },
    Quest {
        npc: 3,
        title: "From Ore to Ingots",
        objective: "Produce three metal ingots in an Ore Smelter you placed or supplied.",
        target: 3,
    },
    Quest {
        npc: 4,
        title: "A Handful of Sparks",
        objective: "Bring three Fire elements.",
        target: 3,
    },
    Quest {
        npc: 4,
        title: "A Place to Warm Your Hands",
        objective: "Light a campfire with the Sorceress: 3 wood + 2 stone.",
        target: 1,
    },
    Quest {
        npc: 4,
        title: "Light Against the Dark",
        objective: "Place a Mana Lantern [B], then charge it so it shines.",
        target: 1,
    },
    Quest {
        npc: 5,
        title: "Nothing Is Wasted",
        objective: "Bring three Death elements.",
        target: 3,
    },
    Quest {
        npc: 5,
        title: "Borrowed Power",
        objective: "Process one Death in an Element Dissipator you placed or supplied.",
        target: 1,
    },
    Quest {
        npc: 5,
        title: "An Unexpected Interest",
        objective: "Craft a bound sheep figurine [C], or produce one in your Workshop.",
        target: 1,
    },
];
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub completed: u32,
    pub counts: [u32; 20],
    /// Latest placed vessel/lantern; latest placed or supplied smelter/dissipator/workshop.
    pub machines: [Option<Cell>; 5],
    pub observed: [u64; 3],
}
impl Progress {
    pub fn record(&mut self, id: usize, amount: u32) {
        if let Some(q) = QUESTS.get(id) {
            self.counts[id] = self.counts[id].saturating_add(amount).min(q.target);
        }
    }
    pub fn done(&self, id: usize) -> bool {
        id < 20 && self.completed & (1 << id) != 0
    }
    pub fn valid(&self) -> bool {
        self.completed < 1 << 20
            && self
                .counts
                .iter()
                .zip(QUESTS.iter())
                .all(|(n, q)| *n <= q.target)
            && self
                .machines
                .iter()
                .flatten()
                .all(|&p| crate::automation::valid_cell(p))
    }
}
pub fn progress(a: &Account, id: usize) -> u32 {
    let wood = adventure::count(a, BlockType::OakWood);
    let stone = adventure::count(a, BlockType::Stone);
    match id {
        0 => stone.min(10),
        1 => a.elements.iter().filter(|&&n| n > 0).count() as u32,
        4 => wood.min(12),
        12 => wood.min(20) + stone.min(10),
        14 => a.elements[Element::Fire.index()].min(3),
        17 => a.elements[Element::Death.index()].min(3),
        _ => a.adventure.quests.counts.get(id).copied().unwrap_or(0),
    }
}
pub fn claim(a: &mut Account, npc: u8, id: usize) -> Result<String, String> {
    let q = QUESTS.get(id).ok_or("Unknown quest")?;
    if q.npc != npc {
        return Err("Speak to this quest's giver".into());
    }
    if a.adventure.quests.done(id) {
        return Err("Already rewarded".into());
    }
    if progress(a, id) < q.target {
        return Err("Objective is not complete".into());
    }
    let mut next = a.clone();
    let (wood, stone) = match id {
        0 => (0, 10),
        4 => (12, 0),
        12 => (20, 10),
        _ => (0, 0),
    };
    pay_supplies(&mut next, wood, stone)?;
    if id == 1 {
        for n in &mut next.elements {
            *n -= 1;
        }
    }
    if id == 14 {
        next.elements[1] -= 3;
    }
    if id == 17 {
        next.elements[4] -= 3;
    }
    next.mana = next.mana.checked_add(20).ok_or("Mana balance full")?;
    next.adventure.quests.completed |= 1 << id;
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("Revision limit reached")?;
    *a = next;
    Ok(format!("{} completed: +20 mana", q.title))
}
pub fn pay_supplies(a: &mut Account, wood: u32, stone: u32) -> Result<(), String> {
    for (block, n) in [(BlockType::OakWood, wood), (BlockType::Stone, stone)] {
        let i = crate::voxel::COLLECTIBLE_BLOCKS
            .iter()
            .position(|b| *b == block)
            .unwrap();
        a.resources[i] = a.resources[i]
            .checked_sub(n)
            .ok_or("Not enough wood or stone")?;
    }
    Ok(())
}
pub fn observe(a: &mut Account, state: &crate::automation::State) -> bool {
    let before = a.adventure.quests;
    if a.hotbar.entry() == Some(Entry::Gear(Gear::Bow)) && a.gear[Gear::Bow as usize] > 0 {
        a.adventure.quests.record(6, 1);
    }
    for (slot, kind, id) in [
        (0, crate::automation::Kind::Vessel, 3),
        (1, crate::automation::Kind::Lantern, 16),
    ] {
        if let Some(d) = a.adventure.quests.machines[slot]
            .and_then(|p| state.devices.get(&p))
            .filter(|d| d.kind == kind)
        {
            if (slot == 0 && d.mana >= 10)
                || (slot == 1 && d.activity == crate::automation::Activity::Working)
            {
                a.adventure.quests.record(id, 1);
            }
        }
    }
    for (slot, kind, id) in [
        (2, crate::automation::Kind::Smelter, 13),
        (3, crate::automation::Kind::Dissipator, 18),
        (4, crate::automation::Kind::Workshop, 19),
    ] {
        if let Some(d) = a.adventure.quests.machines[slot]
            .and_then(|p| state.devices.get(&p))
            .filter(|d| d.kind == kind)
        {
            let n = d.quest_production[slot - 2];
            let old = a.adventure.quests.observed[slot - 2];
            a.adventure
                .quests
                .record(id, n.saturating_sub(old).min(u32::MAX as u64) as u32);
            a.adventure.quests.observed[slot - 2] = n;
        }
    }
    if a.adventure.quests != before {
        a.revision = a.revision.saturating_add(1);
        true
    } else {
        false
    }
}
pub fn machine_action(
    a: &mut Account,
    state: &crate::automation::State,
    action: &crate::automation::Action,
) {
    observe(a, state);
    use crate::automation::{Action, Kind};
    let p = action.cell();
    if matches!(action, Action::Pack { .. }) {
        for cell in &mut a.adventure.quests.machines {
            if *cell == Some(p) {
                *cell = None;
            }
        }
        return;
    }
    let Some(d) = state.devices.get(&p) else {
        return;
    };
    let placed = matches!(action, Action::Place { .. });
    if placed && d.kind == Kind::Chest {
        a.adventure.quests.record(11, 1);
    }
    let slot = match d.kind {
        Kind::Vessel if placed => 0,
        Kind::Lantern if placed => 1,
        Kind::Smelter => 2,
        Kind::Dissipator => 3,
        Kind::Workshop => 4,
        _ => return,
    };
    if !placed && !matches!(action,Action::Deposit{amount,..} if *amount>0) {
        return;
    }
    if a.adventure.quests.machines[slot] != Some(p) || placed {
        a.adventure.quests.machines[slot] = Some(p);
        if slot >= 2 {
            a.adventure.quests.observed[slot - 2] = d.quest_production[slot - 2];
        }
    }
    observe(a, state);
}
pub fn kill(a: &mut Account, kind: crate::creature::CreatureKind) {
    use crate::creature::CreatureKind::*;
    let id = match kind {
        Wolf => 5,
        Skeleton => 8,
        Zombie => 9,
        Goblin => 10,
        _ => return,
    };
    a.adventure.quests.record(id, 1);
    a.revision = a.revision.saturating_add(1);
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Action {
    Claim { npc: u8, quest: u8 },
    LightCampfire,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        automation::{self, Device, Kind, State},
        crafting::{Action as CraftAction, Registry},
        creature::Creatures,
        voxel::World,
    };
    fn rich() -> Account {
        let mut a = Account::default();
        a.resources.fill(100);
        a.elements.fill(100);
        a.mana = 1000;
        a
    }
    #[test]
    fn every_contract_claims_once_with_atomic_delivery_and_overflow_rejection() {
        for (id, q) in QUESTS.iter().enumerate() {
            let mut a = rich();
            a.adventure.quests.record(id, q.target);
            let before = a.clone();
            assert!(claim(&mut a, (q.npc + 1) % 6, id).is_err());
            assert_eq!(a, before);
            a.mana = u32::MAX;
            let full = a.clone();
            assert!(claim(&mut a, q.npc, id).is_err());
            assert_eq!(a, full);
            a.mana = 100;
            claim(&mut a, q.npc, id).unwrap();
            assert_eq!(a.mana, 120);
            let paid = a.clone();
            assert!(claim(&mut a, q.npc, id).is_err());
            assert_eq!(a, paid);
            assert!(a.adventure.quests.valid());
            if id == 0 {
                assert_eq!(adventure::count(&a, BlockType::Stone), 90);
            }
            if id == 12 {
                assert_eq!(adventure::count(&a, BlockType::Stone), 90);
                assert_eq!(adventure::count(&a, BlockType::OakWood), 80);
            }
            if id == 1 {
                assert_eq!(a.elements, [99; 5]);
            }
        }
        let mut empty = Account::default();
        for (id, q) in QUESTS.iter().enumerate() {
            assert!(claim(&mut empty, q.npc, id).is_err());
        }
    }
    #[test]
    fn real_crafting_is_required_and_bound_sheep_stays_in_inventory() {
        let registry = Registry::load().unwrap();
        let world = World::new(12);
        let mut creatures = Creatures::new();
        let mut a = rich();
        assert_eq!(progress(&a, 7), 0);
        for action in [
            CraftAction::CraftGear(Gear::Sword),
            CraftAction::CraftGear(Gear::Bow),
            CraftAction::BindSheep,
            CraftAction::Craft([
                Some(crate::crafting::Slot {
                    element: Element::Earth,
                    amount: 1,
                }),
                Some(crate::crafting::Slot {
                    element: Element::Water,
                    amount: 1,
                }),
                None,
                None,
                None,
            ]),
        ] {
            registry
                .execute(
                    &mut a.clone(),
                    u64::MAX,
                    &action,
                    &world,
                    &mut creatures,
                    glam::Vec3::ZERO,
                    &[],
                )
                .unwrap_err();
            let rev = a.revision;
            registry
                .execute(
                    &mut a,
                    rev,
                    &action,
                    &world,
                    &mut creatures,
                    glam::Vec3::ZERO,
                    &[],
                )
                .unwrap();
        }
        for id in [2, 7, 19] {
            assert_eq!(progress(&a, id), 1);
        }
        assert!(creatures.snapshot().is_empty());
        assert_eq!(a.production_goods["creature:sheep"], 1);
        a.hotbar.assign(Some(Entry::Gear(Gear::Bow)));
        observe(&mut a, &State::default());
        assert_eq!(progress(&a, 6), 1);
        let saved: Account = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert_eq!(saved, a);
    }
    #[test]
    fn machinery_counts_completed_production_and_does_not_replay_saved_or_packed_cycles() {
        let r = Registry::load().unwrap();
        let b = automation::balance();
        let mut state = State::default();
        let mut a = rich();
        for (i, kind) in [
            Kind::Vessel,
            Kind::Lantern,
            Kind::Smelter,
            Kind::Dissipator,
            Kind::Workshop,
            Kind::Chest,
        ]
        .into_iter()
        .enumerate()
        {
            let cell = (i as i32 * 4, 40, 0);
            state.devices.insert(cell, Device::new(kind, cell, 0));
            machine_action(
                &mut a,
                &state,
                &automation::Action::Place {
                    kind,
                    cell,
                    rotation: 0,
                    packed: None,
                },
            );
        }
        state.devices.get_mut(&(0, 40, 0)).unwrap().mana = 10;
        machine_action(
            &mut a,
            &state,
            &automation::Action::Charge {
                cell: (0, 40, 0),
                amount: 10,
            },
        );
        assert_eq!(progress(&a, 3), 1);
        state.devices.get_mut(&(4, 40, 0)).unwrap().mana = 1;
        let smelter = state.devices.get_mut(&(8, 40, 0)).unwrap();
        smelter.config.eject_contents = false;
        smelter.items.insert("resource:copper_ore".into(), 3);
        smelter.items.insert("resource:coal".into(), 3);
        let dissipator = state.devices.get_mut(&(12, 40, 0)).unwrap();
        dissipator.config.element = 4;
        dissipator.items.insert("element:death".into(), 1);
        let workshop = state.devices.get_mut(&(16, 40, 0)).unwrap();
        workshop.config.recipe = r
            .recipes
            .iter()
            .find(|r| r.output.id == "sheep")
            .unwrap()
            .id
            .clone();
        workshop.items.insert("element:life".into(), 2);
        workshop.mana = 100;
        assert_eq!(progress(&a, 13), 0);
        assert_eq!(progress(&a, 18), 0);
        assert_eq!(progress(&a, 19), 0);
        for _ in 0..100 {
            state.step(b, &r);
            observe(&mut a, &state);
        }
        for id in [11, 16, 18, 19] {
            assert_eq!(progress(&a, id), 1, "quest {id}");
        }
        assert_eq!(progress(&a, 13), 3);
        let before = a.clone();
        assert!(!observe(&mut a, &state));
        assert_eq!(a, before);
        let mut newcomer = rich();
        machine_action(
            &mut newcomer,
            &state,
            &automation::Action::Deposit {
                cell: (8, 40, 0),
                item: "resource:copper_ore".into(),
                amount: 1,
            },
        );
        observe(&mut newcomer, &state);
        assert_eq!(progress(&newcomer, 13), 0);
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(!observe(&mut a, &restored));
        state.devices.remove(&(8, 40, 0));
        machine_action(
            &mut newcomer,
            &state,
            &automation::Action::Pack { cell: (8, 40, 0) },
        );
        assert_eq!(newcomer.adventure.quests.machines[2], None);
    }
}
