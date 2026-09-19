//! A one-time language-to-settings step; chunk generation never calls the model.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Mainland,
    Islands,
    Flat,
    Mountains,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    #[default]
    Natural,
    Sand,
    Snow,
    Stone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldGeneration {
    /// Missing in old saves: preserve the original branchless trees.
    #[serde(default)]
    pub tree_version: u8,
    /// Zero preserves worlds saved before geological detail and flower patches.
    #[serde(default)]
    pub landscape_version: u8,
    #[serde(default)]
    pub underground: bool,
    #[serde(default)]
    pub cave_version: u8,
    pub description: String,
    /// Natural population percentages; omitted species retain their defaults.
    #[serde(default)]
    pub creatures: std::collections::BTreeMap<String, u16>,
    pub shape: Shape,
    pub surface: Surface,
    /// Percentage of the normal tree density (0..=300).
    pub trees: u16,
    /// Percentage of normal relief (0..=200).
    pub relief: u16,
    /// Approximate island spacing in blocks (64..=512).
    pub island_size: u16,
}
impl Default for WorldGeneration {
    fn default() -> Self {
        Self {
            tree_version: 2,
            landscape_version: 3,
            underground: true,
            cave_version: 2,
            description: String::new(),
            creatures: Default::default(),
            shape: Shape::Mainland,
            surface: Surface::Natural,
            trees: 100,
            relief: 100,
            island_size: 192,
        }
    }
}
// Keep aligned with CreatureKind's stable wire IDs.
pub const CREATURE_SPECIES: &[&str] = &[
    "sheep",
    "chicken",
    "stone_golem",
    "wolf",
    "stinger",
    "cow",
    "goblin",
    "sunscorch",
    "zombie",
    "skeleton",
    "dragon_green",
    "dragon_red",
    "fish",
    "skeleton_sorcerer",
];

impl WorldGeneration {
    pub fn abundance(&self, species: &str) -> u16 {
        self.creatures.get(species).copied().unwrap_or(100)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self
            .creatures
            .iter()
            .any(|(key, value)| !CREATURE_SPECIES.contains(&key.as_str()) || *value > 1000)
            || self.cave_version > 2
            || self.tree_version > 2
            || self.landscape_version > 3
            || self.description.len() > 2048
            || self.trees > 300
            || self.relief > 200
            || !(64..=512).contains(&self.island_size)
        {
            return Err("Invalid world generation settings".into());
        }
        Ok(())
    }
    pub fn height(&self, x: i32, z: i32, seed: u32) -> i32 {
        let h = self.base_height(x, z, seed);
        if self.landscape_version == 0
            || self.landscape_version >= 3
            || self.shape == Shape::Flat
            || self.relief == 0
        {
            return h;
        }
        if self.shape == Shape::Mainland && crate::voxel::terrain::tributary(x, z, seed).is_some() {
            return h;
        }
        crate::voxel::terrain::geological_height(
            x,
            z,
            seed,
            h,
            self.relief as f32 / 100.,
            self.landscape_version,
        )
    }
    fn base_height(&self, x: i32, z: i32, seed: u32) -> i32 {
        use crate::voxel::{chunk::TERRAIN_HEIGHT as CHUNK_Y, noise::fbm, world::SEA_LEVEL};
        let original =
            || crate::voxel::terrain::height_with_lakes(x, z, seed, self.landscape_version >= 3);
        if self.shape == Shape::Mainland && self.relief == 100 {
            return crate::voxel::terrain::tributary(x, z, seed)
                .map(|p| p.0)
                .unwrap_or_else(original);
        }
        let detail = fbm(
            x as f32 * 0.025,
            z as f32 * 0.025,
            seed ^ 0x715A,
            3,
            2.,
            0.5,
        );
        let scale = self.relief as f32 / 100.;
        let h = match self.shape {
            Shape::Mainland => 24. + (original() as f32 - 24.) * scale,
            Shape::Flat => 25. + (detail - 0.5) * 4. * scale,
            Shape::Mountains => 26. + (original() as f32 - 18.).max(0.) * (1. + scale),
            Shape::Islands => {
                let size = self.island_size as f32;
                let land = fbm(x as f32 / size, z as f32 / size, seed ^ 0x15A1, 3, 2., 0.5);
                // A small guaranteed starting island avoids spawning in deep water.
                let radius = ((x as f32).powi(2) + (z as f32).powi(2)).sqrt();
                let start = (1. - radius / 32.).max(0.);
                let ocean = SEA_LEVEL as f32 - 5.
                    + ((land - 0.48) * 65. + start * 16.).max(-10.) * (0.5 + scale * 0.5)
                    + (detail - 0.5) * 3.;
                ocean.max(SEA_LEVEL as f32 + 6. - radius * 0.5)
            }
        };
        let h = (h.round() as i32).clamp(5, CHUNK_Y - 12);
        if self.shape == Shape::Mainland {
            crate::voxel::terrain::tributary(x, z, seed)
                .map(|p| p.0)
                .unwrap_or(h)
        } else {
            h
        }
    }
}

pub fn resolve(description: &str, base_url: &str) -> Result<WorldGeneration, String> {
    let description = description.trim();
    if description.is_empty() {
        return Ok(WorldGeneration::default());
    }
    if description.len() > 2048 {
        return Err("World description must fit in 2048 UTF-8 bytes".into());
    }
    crate::llm_server::ensure_ready(base_url)?;
    let mut config = crate::llm::describe_world(description, base_url)?;
    config.landscape_version = 3;
    config.tree_version = 2;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creature_settings_are_bounded_and_old_worlds_keep_defaults() {
        let mut config = WorldGeneration::default();
        config.creatures.insert("sheep".into(), 1000);
        config.creatures.insert("cow".into(), 0);
        assert!(config.validate().is_ok());
        let restored: WorldGeneration =
            serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored, config);
        let mut old = serde_json::to_value(&config).unwrap();
        old.as_object_mut().unwrap().remove("creatures");
        old.as_object_mut().unwrap().remove("cave_version");
        old.as_object_mut().unwrap().remove("landscape_version");
        old.as_object_mut().unwrap().remove("tree_version");
        let old: WorldGeneration = serde_json::from_value(old).unwrap();
        assert_eq!(old.abundance("cow"), 100);
        assert_eq!(old.cave_version, 0);
        assert_eq!(old.landscape_version, 0);
        assert_eq!(old.tree_version, 0);
        assert_eq!(config.cave_version, 2);
        config.creatures.insert("cow".into(), 1001);
        assert!(config.validate().is_err());
        config.creatures.remove("cow");
        config.creatures.insert("unicorn".into(), 100);
        assert!(config.validate().is_err());
    }
    #[test]
    fn blank_description_needs_no_model_and_legacy_preserves_terrain() {
        let mut config = resolve("  \n ", "invalid://no-model").unwrap();
        assert_eq!(config.landscape_version, 3);
        config.landscape_version = 0;
        for x in -100..100 {
            assert_eq!(
                config.height(x, x * 3, 42),
                crate::voxel::terrain::height(x, x * 3, 42)
            );
        }
    }
    #[test]
    fn islands_have_land_water_and_dry_origin() {
        for seed in [1, 42, 9001] {
            let config = WorldGeneration {
                shape: Shape::Islands,
                ..Default::default()
            };
            assert!(config.height(0, 0, seed) > crate::voxel::world::SEA_LEVEL);
            let mut water = 0;
            let mut land = 0;
            for x in (-512..512).step_by(16) {
                for z in (-512..512).step_by(16) {
                    if config.height(x, z, seed) <= crate::voxel::world::SEA_LEVEL {
                        water += 1;
                    } else {
                        land += 1;
                    }
                }
            }
            assert!(
                water > 500 && land > 100,
                "seed {seed}: water={water}, land={land}"
            );
        }
    }
    #[test]
    fn settings_are_bounded() {
        assert!(WorldGeneration {
            trees: 301,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(WorldGeneration {
            island_size: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    #[ignore = "requires the local llama server and bundled model"]
    fn live_creature_description() {
        let url = std::env::var("WORLDGEN_TEST_URL")
            .unwrap_or_else(|_| crate::net::DEFAULT_LLM_URL.into());
        let config = resolve("This world is full of sheep but no cows", &url).unwrap();
        assert_eq!(config.abundance("cow"), 0, "{config:?}");
        assert!(config.abundance("sheep") >= 500, "{config:?}");
        assert_eq!(config.abundance("chicken"), 100, "{config:?}");
    }

    #[test]
    #[ignore = "requires the local llama server and bundled model"]
    fn live_world_descriptions() {
        for (prompt, shape, surface) in [
            ("A sandy desert with no trees", None, Some(Surface::Sand)),
            (
                "Small tropical islands in a vast ocean",
                Some(Shape::Islands),
                None,
            ),
            (
                "A flat snow-covered world",
                Some(Shape::Flat),
                Some(Surface::Snow),
            ),
        ] {
            let started = std::time::Instant::now();
            let url = std::env::var("WORLDGEN_TEST_URL")
                .unwrap_or_else(|_| crate::net::DEFAULT_LLM_URL.into());
            let config = resolve(prompt, &url).unwrap();
            println!(
                "{prompt}: {config:?} ({:.2}s)",
                started.elapsed().as_secs_f32()
            );
            if let Some(shape) = shape {
                assert_eq!(config.shape, shape);
            }
            if let Some(surface) = surface {
                assert_eq!(config.surface, surface);
            }
            if prompt.contains("no trees") {
                assert_eq!(config.trees, 0);
            }
        }
    }
}
