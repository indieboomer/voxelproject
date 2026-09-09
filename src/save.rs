use std::fs;
use std::path::Path;

use glam::Vec3;

use crate::camera::Camera;
use crate::player::Player;
use crate::scripting::ModuleSaveEntry;
use crate::voxel::world::WorldSave;
use crate::voxel::World;

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CraftingSave {
    pub host: crate::crafting::Account,
    // Nicknames are the existing session identity; there are no accounts in the MVP.
    pub guests: std::collections::HashMap<String, crate::crafting::Account>,
    pub creatures: Option<Vec<crate::creature::SavedCreature>>,
    pub behaviors: std::collections::BTreeMap<u32, crate::creature::CreatureBehavior>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SaveV2 {
    world: WorldSave,
    #[serde(default)]
    crafting: CraftingSave,
}
const MAGIC: &[u8] = b"VOXEL_SAVE_2\n";
fn encode_save(world: WorldSave, crafting: &CraftingSave) -> Result<Vec<u8>, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct SaveRef<'a> {
        world: WorldSave,
        crafting: &'a CraftingSave,
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend(serde_json::to_vec(&SaveRef { world, crafting })?);
    Ok(bytes)
}
fn decode_save(bytes: &[u8]) -> Option<(WorldSave, CraftingSave)> {
    if let Some(json) = bytes.strip_prefix(MAGIC) {
        let save: SaveV2 = serde_json::from_slice(json).ok()?;
        Some((save.world, save.crafting))
    } else {
        Some((bincode::deserialize(bytes).ok()?, CraftingSave::default()))
    }
}

const SAVE_PATH: &str = "saves/world.bin";

pub struct LoadedWorld {
    pub crafting: CraftingSave,
    pub world: World,
    pub player_pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub time_of_day: f32,
    pub modules: Vec<ModuleSaveEntry>,
}

pub fn save_world(
    world: &World,
    player: &Player,
    camera: &Camera,
    time_of_day: f32,
    modules: Vec<ModuleSaveEntry>,
    crafting: &CraftingSave,
) {
    let save = WorldSave {
        seed: world.seed,
        player_pos: player.position.to_array(),
        player_yaw: camera.yaw,
        player_pitch: camera.pitch,
        time_of_day,
        edits: world.edits.iter().map(|(k, v)| (*k, *v)).collect(),
        modules,
    };

    if let Some(parent) = Path::new(SAVE_PATH).parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            log::error!("Failed to create saves directory: {e}");
            return;
        }
    }

    let edit_count = save.edits.len();
    match encode_save(save, crafting) {
        Ok(bytes) => {
            if let Err(e) = fs::write("saves/world.bin.tmp", bytes)
                .and_then(|()| fs::rename("saves/world.bin.tmp", SAVE_PATH))
            {
                log::error!("Failed to write save file: {e}");
            } else {
                log::info!("World saved ({} edited blocks).", edit_count);
            }
        }
        Err(e) => log::error!("Failed to serialize save: {e}"),
    }
}

/// Whether a save file exists, for the main menu to decide whether "Load
/// World" should be selectable at all.
pub fn save_exists() -> bool {
    Path::new(SAVE_PATH).exists()
}

pub fn load_world() -> Option<LoadedWorld> {
    let bytes = fs::read(SAVE_PATH).ok()?;
    let (save, crafting) = decode_save(&bytes)?;
    let mut world = World::new(save.seed);
    for (pos, block) in save.edits {
        world.edits.insert(pos, block);
    }
    world.rebuild_redstone_positions();
    log::info!(
        "Loaded world with {} edited blocks and {} rule modules.",
        world.edits.len(),
        save.modules.len()
    );
    Some(LoadedWorld {
        crafting,
        world,
        player_pos: Vec3::from_array(save.player_pos),
        yaw: save.player_yaw,
        pitch: save.player_pitch,
        time_of_day: save.time_of_day,
        modules: save.modules,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world_save() -> WorldSave {
        WorldSave {
            seed: 42,
            player_pos: [1.0, 2.0, 3.0],
            player_yaw: 0.0,
            player_pitch: 0.0,
            time_of_day: 0.5,
            edits: vec![((1, 2, 3), crate::voxel::BlockType::Bricks)],
            modules: vec![],
        }
    }
    #[test]
    fn old_binary_saves_load_with_empty_elemental_inventory() {
        let bytes = bincode::serialize(&world_save()).unwrap();
        let (world, crafting) = decode_save(&bytes).unwrap();
        assert_eq!(world.seed, 42);
        assert_eq!(crafting.host, crate::crafting::Account::default());
        assert!(crafting.creatures.is_none());
    }
    #[test]
    fn new_saves_round_trip_host_guests_resources_and_creatures() {
        let mut crafting = CraftingSave::default();
        crafting.host.elements = [1, 2, 3, 4, 5];
        crafting.host.mana = 256;
        crafting.host.resources[0] = 17;
        crafting
            .guests
            .insert("guest".into(), crafting.host.clone());
        crafting.creatures = Some(vec![(73, 0, [1.0, 10.0, 3.0], 7.0, 12.0)]);
        let bytes = encode_save(world_save(), &crafting).unwrap();
        let (world, loaded) = decode_save(&bytes).unwrap();
        assert_eq!(loaded.host, crafting.host);
        assert_eq!(loaded.guests, crafting.guests);
        assert_eq!(loaded.creatures, crafting.creatures);
        assert_eq!(world.edits, world_save().edits);
        let mut creatures = crate::creature::Creatures::new();
        creatures.restore_saved(loaded.creatures.as_ref().unwrap(), 42);
        assert_eq!(creatures.snapshot_with_ids(), crafting.creatures.unwrap());
        assert!(creatures.spawn_one(crate::creature::CreatureKind::Sheep, Vec3::ZERO, 1) > 73);
    }
    #[test]
    fn missing_new_fields_default_and_corrupt_saves_fail_safely() {
        let account: crate::crafting::Account = serde_json::from_str("{}").unwrap();
        assert_eq!(account, crate::crafting::Account::default());
        let mut bytes = MAGIC.to_vec();
        bytes.extend(serde_json::to_vec(&serde_json::json!({"world":world_save()})).unwrap());
        assert_eq!(decode_save(&bytes).unwrap().1.host, account);
        assert!(decode_save(b"not a save").is_none());
    }
}
