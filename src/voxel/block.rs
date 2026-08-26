use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockType {
    Air = 0,
    Grass = 1,
    Dirt = 2,
    Stone = 3,
    Sand = 4,
    Wood = 5,
    Leaves = 6,
    Water = 7,
    /// Pickup-able: breaking it sets the player's `carrying_crystal` flag
    /// instead of going to the hotbar. Rare natural spawn + host-placeable.
    Crystal = 8,
    /// Passive effect (see `player.rs`): standing on mud slows movement.
    /// Placed by rules via `api.replace_block`, e.g. "rain turns soil to
    /// mud near trees".
    Mud = 9,
    /// Passive effect (see `creature.rs`'s `heal_near`): slowly heals
    /// nearby creatures each tick. Placed by rules via `api.spawn_block` /
    /// `api.replace_block`.
    RedStone = 10,
}

impl BlockType {
    pub fn is_solid(self) -> bool {
        !matches!(self, BlockType::Air | BlockType::Water)
    }

    /// How strongly a surface picks up sky reflections and a sun specular
    /// glint, from 0 (fully matte: grass, dirt, wood, leaves...) to 1
    /// (mirror-like). Water is the standout case; stone/crystal/redstone get
    /// a subtle sheen since they read as harder, smoother materials.
    pub fn reflectivity(self) -> f32 {
        match self {
            BlockType::Water => 0.9,
            BlockType::Crystal => 0.5,
            BlockType::Stone | BlockType::RedStone => 0.15,
            _ => 0.0,
        }
    }

    /// Whether a face against this block should be culled (i.e. this block
    /// fills its full cube and is not see-through).
    pub fn is_opaque(self) -> bool {
        !matches!(self, BlockType::Air | BlockType::Water | BlockType::Leaves)
    }

    pub fn from_hotbar_index(i: usize) -> Option<BlockType> {
        match i {
            0 => Some(BlockType::Grass),
            1 => Some(BlockType::Dirt),
            2 => Some(BlockType::Stone),
            3 => Some(BlockType::Sand),
            4 => Some(BlockType::Wood),
            5 => Some(BlockType::Leaves),
            6 => Some(BlockType::Crystal),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BlockType::Air => "Air",
            BlockType::Grass => "Grass",
            BlockType::Dirt => "Dirt",
            BlockType::Stone => "Stone",
            BlockType::Sand => "Sand",
            BlockType::Wood => "Wood",
            BlockType::Leaves => "Leaves",
            BlockType::Water => "Water",
            BlockType::Crystal => "Crystal",
            BlockType::Mud => "Mud",
            BlockType::RedStone => "RedStone",
        }
    }

    /// Case-insensitive lookup by name, for the Lua `replace_block` action.
    pub fn from_name(name: &str) -> Option<BlockType> {
        match name.to_ascii_lowercase().as_str() {
            "air" => Some(BlockType::Air),
            "grass" => Some(BlockType::Grass),
            "dirt" => Some(BlockType::Dirt),
            "stone" => Some(BlockType::Stone),
            "sand" => Some(BlockType::Sand),
            "wood" => Some(BlockType::Wood),
            "leaves" => Some(BlockType::Leaves),
            "water" => Some(BlockType::Water),
            "crystal" => Some(BlockType::Crystal),
            "mud" => Some(BlockType::Mud),
            "redstone" => Some(BlockType::RedStone),
            _ => None,
        }
    }
}
