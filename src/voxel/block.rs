use serde::{Deserialize, Serialize};

use super::atlas_tiles::*;
use super::block_defs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockType {
    Air = 0,
    // -- textures/blocks.csv roster, in CSV row order --
    Grass,
    Sand,
    SpruceWood,
    Stone,
    Water,
    OakLeaves,
    Soil,
    Bedrock,
    Cobblestone,
    Basalt,
    CherryWood,
    CherryLeaves,
    BirchWood,
    BirchLeaves,
    GoldOre,
    DiamondOre,
    EmeraldOre,
    CopperOre,
    OakWood,
    SpruceLeaves,
    Pumpkin,
    Bricks,
    ShortGrass,
    // -- blocks kept outside the CSV roster for existing gameplay mechanics
    // (see player.rs's mud slow-down, creature.rs/scripting.rs's redstone
    // heal-near-creatures, and the crystal carry/storm-summoner rules).
    // Not part of the new asset set, so they render with a flat placeholder
    // texture instead of a real tile.
    /// Pickup-able: breaking it sets the player's `carrying_crystal` flag
    /// instead of going to the hotbar. Rare natural spawn + host-placeable.
    Crystal,
    /// Passive effect (see `player.rs`): standing on mud slows movement.
    /// Placed by rules via `api.replace_block`, e.g. "rain turns soil to
    /// mud near trees".
    Mud,
    /// Passive effect (see `creature.rs`'s `heal_near`): slowly heals
    /// nearby creatures each tick. Placed by rules via `api.spawn_block` /
    /// `api.replace_block`.
    RedStone,
}

/// Per-block static properties, transcribed 1:1 from `textures/blocks.csv`
/// (plus the three non-CSV extras above, which use the placeholder tile).
/// `only_on_top`/`single_item` match the CSV's "only on top"/"single items"
/// columns: `only_on_top` blocks can only ever sit on a solid block below
/// and render as a thin non-cube cross (see `cross`) instead of a full cube;
/// `single_item` blocks are scattered individually by world generation
/// rather than in clusters/veins (see `world.rs`).
pub struct BlockDef {
    pub display_name: &'static str,
    /// Material category from the CSV's "resource type" column (e.g.
    /// "wood", "gold_ore", "biomass"). Kept for completeness/future use
    /// (crafting categories, tooltips) -- nothing reads it yet.
    #[allow(dead_code)]
    pub resource_type: &'static str,
    /// 1.0 = fully opaque; less than 1.0 = translucent (currently only
    /// water). Drives `is_opaque`'s face-culling behavior.
    pub opacity: f32,
    /// Self-illumination strength, 0..1. Purely a rendering glow (see
    /// `Vertex::emission` / `shader.wgsl`) -- it does not cast light onto
    /// neighboring blocks.
    pub roughness: f32,
    pub emission: f32,
    /// Hits with the default (invisible) tool needed to break this block.
    /// 0 = unbreakable (bedrock only).
    pub hardness: u32,
    pub tile_top: u8,
    pub tile_side: u8,
    pub tile_bottom: u8,
    pub only_on_top: bool,
    /// World generation scatters this block as isolated single cells
    /// rather than clusters/veins (see `world.rs`'s `scatter_brick_ruins`
    /// vs. `scatter_veins`/`VEINS`). Not read generically -- each
    /// generator function is hardcoded to the right block set -- but kept
    /// so the CSV's "single items" column has a named home.
    #[allow(dead_code)]
    pub single_item: bool,
    /// Alpha-tested (cutout) rather than treated as fully solid -- leaves,
    /// whose atlas tiles have real transparency baked in from the source
    /// texture.
    pub cutout: bool,
    /// Rendered as a thin double-sided cross (two crossed quads) instead of
    /// a full cube, and excluded from collision -- currently only
    /// short_grass.
    pub cross: bool,
}

/// Block types the player can gather (by breaking) and place again, shown
/// in the Resources HUD panel; hotbar keys 1-8 select the first eight.
/// Air, Water, Bedrock, and Crystal are deliberately excluded: the first
/// three can't be broken into a carryable resource, and Crystal is a
/// special one-shot pickup tracked via `Player::carrying_crystal` instead
/// of a stackable material.
pub const COLLECTIBLE_BLOCKS: [BlockType; 23] = [
    BlockType::Grass,
    BlockType::Soil,
    BlockType::Stone,
    BlockType::Sand,
    BlockType::OakWood,
    BlockType::OakLeaves,
    BlockType::Cobblestone,
    BlockType::Bricks,
    BlockType::SpruceWood,
    BlockType::SpruceLeaves,
    BlockType::CherryWood,
    BlockType::CherryLeaves,
    BlockType::BirchWood,
    BlockType::BirchLeaves,
    BlockType::Basalt,
    BlockType::GoldOre,
    BlockType::DiamondOre,
    BlockType::EmeraldOre,
    BlockType::CopperOre,
    BlockType::Pumpkin,
    BlockType::ShortGrass,
    BlockType::Mud,
    BlockType::RedStone,
];

impl BlockType {
    /// CSV-roster blocks delegate to the generated `block_defs::csv_def`
    /// (see `tools/build_atlas.py`); only the non-CSV extras (Air, Mud,
    /// RedStone, Crystal) are hand-written here.
    pub fn def(self) -> BlockDef {
        if let Some(def) = block_defs::csv_def(self) {
            return def;
        }
        use BlockType::*;
        match self {
            Air => BlockDef {
                display_name: "Air",
                resource_type: "air",
                opacity: 0.0,
                roughness: 1.0,
                emission: 0.0,
                hardness: 0,
                tile_top: TILE_WHITE,
                tile_side: TILE_WHITE,
                tile_bottom: TILE_WHITE,
                only_on_top: false,
                single_item: false,
                cutout: false,
                cross: false,
            },
            Crystal => BlockDef {
                display_name: "Crystal",
                resource_type: "special",
                opacity: 1.0,
                roughness: 0.5,
                emission: 0.0,
                hardness: 1,
                tile_top: TILE_PLACEHOLDER,
                tile_side: TILE_PLACEHOLDER,
                tile_bottom: TILE_PLACEHOLDER,
                only_on_top: false,
                single_item: true,
                cutout: false,
                cross: false,
            },
            Mud => BlockDef {
                display_name: "Mud",
                resource_type: "special",
                opacity: 1.0,
                roughness: 1.0,
                emission: 0.0,
                hardness: 1,
                tile_top: TILE_PLACEHOLDER,
                tile_side: TILE_PLACEHOLDER,
                tile_bottom: TILE_PLACEHOLDER,
                only_on_top: false,
                single_item: false,
                cutout: false,
                cross: false,
            },
            RedStone => BlockDef {
                display_name: "RedStone",
                resource_type: "special",
                opacity: 1.0,
                roughness: 0.85,
                emission: 0.0,
                hardness: 1,
                tile_top: TILE_PLACEHOLDER,
                tile_side: TILE_PLACEHOLDER,
                tile_bottom: TILE_PLACEHOLDER,
                only_on_top: false,
                single_item: false,
                cutout: false,
                cross: false,
            },
            // Handled by `block_defs::csv_def` above.
            Grass | Sand | SpruceWood | Stone | Water | OakLeaves | Soil | Bedrock
            | Cobblestone | Basalt | CherryWood | CherryLeaves | BirchWood | BirchLeaves
            | GoldOre | DiamondOre | EmeraldOre | CopperOre | OakWood | SpruceLeaves
            | Pumpkin | Bricks | ShortGrass => {
                unreachable!("block_defs::csv_def should have handled every CSV block")
            }
        }
    }

    /// Whether a physical object occupies this cell -- blocks player/entity
    /// movement. `only_on_top` decorations (short grass) are walk-through,
    /// like water and air.
    pub fn is_solid(self) -> bool {
        !matches!(self, BlockType::Air | BlockType::Water) && !self.def().only_on_top
    }

    /// How strongly a surface picks up sky reflections and a sun specular
    /// glint, derived from the block's `roughness` (less roughness = more
    /// glossy). 0 (fully matte) to 1 (mirror-like).
    pub fn reflectivity(self) -> f32 {
        (1.0 - self.def().roughness).clamp(0.0, 1.0)
    }

    /// Hits needed to break this block; 0 means unbreakable.
    pub fn hardness(self) -> u32 {
        self.def().hardness
    }

    pub fn is_unbreakable(self) -> bool {
        self.hardness() == 0
    }

    /// Whether a face against this block should be culled (i.e. this block
    /// fills its full cube and is not see-through).
    pub fn is_opaque(self) -> bool {
        let def = self.def();
        def.opacity >= 1.0 && !def.cutout && !def.cross
    }

    /// Whether this block can be aimed at / broken by the raycast, even if
    /// it's not physically solid (e.g. short grass).
    pub fn is_targetable(self) -> bool {
        !matches!(self, BlockType::Air | BlockType::Water)
    }

    /// Whether this is one of the tree wood species -- used by rules that
    /// search for "a tree" generically rather than one specific species.
    pub fn is_wood(self) -> bool {
        matches!(
            self,
            BlockType::OakWood | BlockType::SpruceWood | BlockType::CherryWood | BlockType::BirchWood
        )
    }

    pub fn from_hotbar_index(i: usize) -> Option<BlockType> {
        COLLECTIBLE_BLOCKS.get(i).copied()
    }

    pub fn name(self) -> &'static str {
        self.def().display_name
    }

    /// Case-insensitive lookup by snake_case id, for the Lua `replace_block`
    /// / `find_blocks` / `get_block` API. Matches `textures/blocks.csv`'s
    /// `id` column (normalized to snake_case, via the generated
    /// `block_defs::csv_from_name`) plus "air"/"mud"/"redstone"/"crystal"
    /// for the non-CSV extras.
    pub fn from_name(name: &str) -> Option<BlockType> {
        let lower = name.to_ascii_lowercase();
        if let Some(block) = block_defs::csv_from_name(&lower) {
            return Some(block);
        }
        match lower.as_str() {
            "air" => Some(BlockType::Air),
            "crystal" => Some(BlockType::Crystal),
            "mud" => Some(BlockType::Mud),
            "redstone" => Some(BlockType::RedStone),
            _ => None,
        }
    }
}
