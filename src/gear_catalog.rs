//! Append-only equipment definitions, shared by crafting, UI and authority.
use crate::{
    crafting::Account,
    equipment::{Entry, Gear},
    voxel::BlockType,
};

pub struct Definition {
    pub name: &'static str,
    pub description: &'static str,
    pub iron: u32,
    pub wood: u32,
    pub mana: u32,
    pub extra: (BlockType, u32),
    pub color: [f32; 3],
}
macro_rules! item {
    ($name:literal,$desc:literal,$iron:literal,$wood:literal,$mana:literal,$extra:ident,$n:literal,$color:expr) => {
        Definition {
            name: $name,
            description: $desc,
            iron: $iron,
            wood: $wood,
            mana: $mana,
            extra: (BlockType::$extra, $n),
            color: $color,
        }
    };
}
pub const DEFINITIONS: [Definition; 16] = [
    item!(
        "Forester axe",
        "Chops wood and plants with twice the harvesting power.",
        5,
        3,
        8,
        Resin,
        2,
        [0.35, 0.70, 0.30]
    ),
    item!(
        "Prospector pick",
        "Breaks stone and ore with twice the harvesting power.",
        6,
        2,
        10,
        Quartz,
        2,
        [0.85, 0.65, 0.25]
    ),
    item!(
        "Spade",
        "Clears soil and sand with three times the harvesting power.",
        2,
        3,
        4,
        Copper,
        1,
        [0.70, 0.45, 0.25]
    ),
    item!(
        "Sickle",
        "Harvests plants in one stroke; a light specialist gathering tool.",
        2,
        1,
        6,
        PlantFiber,
        3,
        [0.40, 0.80, 0.45]
    ),
    item!(
        "Spear",
        "Long melee reach: 4.8 blocks, 10 damage, 0.55s between thrusts.",
        3,
        4,
        6,
        Flax,
        2,
        [0.75, 0.80, 0.85]
    ),
    item!(
        "Dagger",
        "Quick close strikes: 2.4 blocks, 7 damage every 0.20s.",
        2,
        1,
        6,
        Obsidian,
        2,
        [0.40, 0.35, 0.55]
    ),
    item!(
        "Warhammer",
        "Heavy blows: 3 blocks, 25 damage every 1.1s.",
        8,
        2,
        12,
        Stone,
        6,
        [0.55, 0.60, 0.65]
    ),
    item!(
        "Longbow",
        "Long range: 36 blocks, 16 damage, 2 mana per shot every 1.0s.",
        2,
        6,
        10,
        PlantFiber,
        6,
        [0.65, 0.40, 0.20]
    ),
    item!(
        "Ember wand",
        "Heavy magic bolt: 18 blocks, 22 damage, 4 mana every 0.9s.",
        2,
        2,
        12,
        Ruby,
        2,
        [1.0, 0.30, 0.12]
    ),
    item!(
        "Tide wand",
        "Rapid magic bolt: 24 blocks, 8 damage, 1 mana every 0.30s.",
        2,
        2,
        12,
        Sapphire,
        2,
        [0.20, 0.65, 1.0]
    ),
    item!(
        "Life staff",
        "Left-click to restore 15 of your health for 5 mana, every 2s.",
        2,
        4,
        14,
        WildHerbs,
        6,
        [0.35, 1.0, 0.45]
    ),
    item!(
        "Survey lantern",
        "Hold to illuminate terrain and caves without spending mana.",
        2,
        1,
        10,
        EnchantedGlass,
        2,
        [1.0, 0.85, 0.35]
    ),
    item!(
        "Trail charm",
        "Hold to walk and sprint 30% faster.",
        2,
        1,
        10,
        Amber,
        2,
        [0.90, 0.60, 0.20]
    ),
    item!(
        "Leaping charm",
        "Hold for 35% stronger jumps.",
        2,
        1,
        10,
        Emerald,
        2,
        [0.45, 0.95, 0.40]
    ),
    item!(
        "Diving charm",
        "Hold to consume underwater oxygen four times more slowly.",
        2,
        1,
        10,
        Reeds,
        6,
        [0.25, 0.70, 0.90]
    ),
    item!(
        "Feather charm",
        "Hold to slow your descent to 3 blocks per second.",
        2,
        1,
        10,
        Cloth,
        4,
        [0.85, 0.80, 1.0]
    ),
];
pub const BOOK_NAMES: [&str; 4] = [
    "The Greenhand's Almanac",
    "The Warden's Arsenal",
    "The Living Elements",
    "The Wayfarer's Handbook",
];
pub const PATHS: [&str; 5] = ["Starter", "Harvesting", "Combat", "Magic", "Exploration"];

impl Gear {
    pub fn definition(self) -> &'static Definition {
        &DEFINITIONS[(self as usize).saturating_sub(4)]
    }
    pub fn book(self) -> Option<u8> {
        (self as usize >= 4).then(|| (self as u8 - 4) / 4)
    }
    pub fn known(self, account: &Account) -> bool {
        self.enabled() && self.book()
            .is_none_or(|b| account.adventure.recipe_books & (1 << b) != 0)
    }
    pub fn path(self) -> usize {
        self.book().map_or(0, |b| b as usize + 1)
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Axe => "Harvest wood and plants.",
            Self::Pickaxe => "Mine stone, ore and soil.",
            Self::Sword => "Melee: 12 damage, 3.2-block reach, every 0.35s.",
            Self::Bow => "Ranged: 9 damage, 20-block reach, 1 mana every 0.7s.",
            _ => self.definition().description,
        }
    }
    pub fn id(self) -> String {
        self.name().to_lowercase().replace(' ', "_")
    }
    pub fn ingredients(self, salvage: bool) -> Vec<(BlockType, u32)> {
        let Ok((iron, wood, _)) = crate::crafting::gear_formula(self, salvage) else {return vec![];};
        let mut result = vec![(BlockType::Iron, iron), (BlockType::OakWood, wood)];
        if self.book().is_some() {
            let (b, n) = self.definition().extra;
            result.push((b, if salvage { n / 2 } else { n }));
        }
        result.retain(|(_, n)| *n > 0);
        result
    }
    pub fn recipe_text(self, salvage: bool) -> String {
        self.ingredients(salvage)
            .into_iter()
            .map(|(b, n)| format!("{n} {}", b.name()))
            .collect::<Vec<_>>()
            .join(" + ")
    }
    /// Reach, damage, cooldown in ms, mana. All attacks use terrain occlusion.
    pub fn weapon(self) -> Option<(f32, f32, u64, u32)> {
        if !self.enabled() {return None;}
        Some(match self {
            Self::Sword => (3.2, 12., 350, 0),
            Self::Bow => (20., 9., 700, 1),
            Self::Spear => (4.8, 10., 550, 0),
            Self::Dagger => (2.4, 7., 200, 0),
            Self::Warhammer => (3., 25., 1100, 0),
            Self::Longbow => (36., 16., 1000, 2),
            Self::EmberWand => (18., 22., 900, 4),
            Self::TideWand => (24., 8., 300, 1),
            Self::LifeStaff => (0., 0., 2000, 5),
            _ => return None,
        })
    }
    pub fn harvest_power(self) -> u32 {
        match self {
            Self::ForesterAxe | Self::ProspectorPick => 2,
            Self::Spade => 3,
            Self::Sickle => u32::MAX,
            _ => 1,
        }
    }
}
pub fn held(account: &Account, gear: Gear) -> bool {
    account.hotbar.entry() == Some(Entry::Gear(gear)) && account.gear[gear as usize] > 0
}
pub fn oxygen_factor(account: &Account) -> f32 {
    if held(account, Gear::DivingCharm) {
        0.25
    } else {
        1.0
    }
}

pub const fn starter_counts() -> [u32; 20] {
    let mut counts = [0; 20];
    counts[0] = 1;
    counts[1] = 1;
    counts[2] = 1;
    counts
}
/// Legacy four-slot saves remain readable; new gear is appended with zero counts.
pub mod counts {
    use serde::Serialize;
    pub fn serialize<S: serde::Serializer>(v: &[u32; 20], s: S) -> Result<S::Ok, S::Error> {
        v.as_slice().serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u32; 20], D::Error> {
        struct Counts;
        impl<'de> serde::de::Visitor<'de> for Counts {
            type Value = [u32; 20];
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "up to 20 gear counts")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut result = [0; 20];
                for n in &mut result {
                    match seq.next_element()? {
                        Some(v) => *n = v,
                        None => return Ok(result),
                    }
                }
                if seq.next_element::<u32>()?.is_some() {
                    return Err(serde::de::Error::custom("Too many gear counts"));
                }
                Ok(result)
            }
        }
        d.deserialize_seq(Counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_equipment_migrates_and_all_items_roundtrip() {
        let a: Account = serde_json::from_str(r#"{"gear":[2,3,4,5]}"#).unwrap();
        assert_eq!(&a.gear[..4], &[2, 3, 4, 5]);
        assert_eq!(&a.gear[4..], &[0; 16]);
        assert_eq!(a.adventure.recipe_books, 0);
        for g in Gear::ALL {
            assert!(!g.description().is_empty());
            let mut a = a.clone();
            a.gear[g as usize] = 1;
            a.hotbar.assign(Some(Entry::Gear(g)));
            let restored: Account =
                serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
            assert_eq!(a, restored);
            assert!(!crate::held_item::parts(Some(Entry::Gear(g))).is_empty());
        }
        let bad = serde_json::json!({"gear":vec![0;21]});
        assert!(serde_json::from_value::<Account>(bad).is_err());
    }
    #[test]
    fn every_specialist_recipe_requires_discovery_and_spends_all_materials_atomically() {
        let r = crate::crafting::Registry::parse(include_str!("../data/crafting.json")).unwrap();
        let world = crate::voxel::World::new(1);
        let mut creatures = crate::creature::Creatures::new();
        for g in Gear::available().filter(|g|g.book().is_some()) {
            let mut a = Account::default();
            a.mana = 100;
            a.resources.fill(50);
            let action = crate::crafting::Action::CraftGear(g);
            let before = a.clone();
            assert!(r
                .execute(
                    &mut a,
                    0,
                    &action,
                    &world,
                    &mut creatures,
                    glam::Vec3::ZERO,
                    &[]
                )
                .is_err());
            assert_eq!(a, before);
            a.adventure.recipe_books = 1 << g.book().unwrap();
            let before = a.clone();
            r.execute(
                &mut a,
                0,
                &action,
                &world,
                &mut creatures,
                glam::Vec3::ZERO,
                &[],
            )
            .unwrap();
            assert_eq!(a.gear[g as usize], 1);
            for (b, n) in g.ingredients(false) {
                assert_eq!(
                    Entry::Resource(b).count(&a),
                    Entry::Resource(b).count(&before) - n
                );
            }
            let missing = g.definition().extra.0;
            let i = crate::voxel::COLLECTIBLE_BLOCKS
                .iter()
                .position(|b| *b == missing)
                .unwrap();
            a.resources[i] = 0;
            let before = a.clone();
            let revision = a.revision;
            assert!(r
                .execute(
                    &mut a,
                    revision,
                    &action,
                    &world,
                    &mut creatures,
                    glam::Vec3::ZERO,
                    &[]
                )
                .is_err());
            assert_eq!(a, before);
        }
    }
    #[test]
    fn utility_requires_owned_active_gear_and_weapons_have_distinct_tradeoffs() {
        let mut a = Account::default();
        a.hotbar.assign(Some(Entry::Gear(Gear::DivingCharm)));
        assert_eq!(oxygen_factor(&a), 1.);
        a.gear[Gear::DivingCharm as usize] = 1;
        assert_eq!(oxygen_factor(&a), 0.25);
        a.hotbar.select(1);
        assert_eq!(oxygen_factor(&a), 1.);
        assert!(Gear::Spear.weapon().unwrap().0 > Gear::Sword.weapon().unwrap().0);
        assert!(Gear::Warhammer.weapon().unwrap().2 > Gear::Dagger.weapon().unwrap().2);
        assert_eq!(Gear::ProspectorPick.harvest_power(), 2);
        assert_eq!(
            Gear::Spade.categories(),
            &[crate::equipment::Category::Soil]
        );
    }
}
