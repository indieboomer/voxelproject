//! Data-driven properties for world resources.
use crate::voxel::BlockType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceDefinition {
    pub edible: bool,
    pub raw_health_change: i16,
    pub cooked_health_change: i16,
    pub cookable: bool,
    pub cooked_resource: Option<BlockType>,
    pub fuel_value: u16,
    pub plantable: bool,
    pub renewable_source: bool,
}

const EMPTY: ResourceDefinition = ResourceDefinition {
    edible: false, raw_health_change: 0, cooked_health_change: 0,
    cookable: false, cooked_resource: None, fuel_value: 0,
    plantable: false, renewable_source: false,
};

pub fn definition(block: BlockType) -> ResourceDefinition {
    use BlockType::*;
    match block {
        Meat => ResourceDefinition { edible: true, raw_health_change: 8, cooked_health_change: 0, cookable: true, cooked_resource: Some(CookedMeat), ..EMPTY },
        CookedMeat => ResourceDefinition { edible: true, raw_health_change: 25, cooked_health_change: 25, ..EMPTY },
        Pumpkin => ResourceDefinition { edible: true, raw_health_change: 10, cooked_health_change: 20, cookable: true, plantable: true, renewable_source: true, ..EMPTY },
        WildHerbs => ResourceDefinition { edible: true, raw_health_change: 5, cooked_health_change: 5, renewable_source: true, ..EMPTY },
        BrownMushroom => ResourceDefinition { edible: true, raw_health_change: 6, cooked_health_change: 6, renewable_source: true, ..EMPTY },
        OakWood | BirchWood | SpruceWood => ResourceDefinition { fuel_value: 1, ..EMPTY },
        Coal => ResourceDefinition { fuel_value: 4, ..EMPTY },
        _ => EMPTY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn definitions_cover_food_and_fuel_without_engine_special_cases() {
        assert!(definition(BlockType::Meat).edible);
        assert_eq!(definition(BlockType::Meat).cooked_resource, Some(BlockType::CookedMeat));
        assert_eq!(definition(BlockType::Coal).fuel_value, 4);
        assert!(!definition(BlockType::Stone).edible);
    }
}
