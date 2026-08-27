use std::fs;
use std::path::Path;

use glam::Vec3;

use crate::camera::Camera;
use crate::player::Player;
use crate::scripting::ModuleSaveEntry;
use crate::voxel::world::WorldSave;
use crate::voxel::World;

const SAVE_PATH: &str = "saves/world.bin";

pub struct LoadedWorld {
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

    match bincode::serialize(&save) {
        Ok(bytes) => {
            if let Err(e) = fs::write(SAVE_PATH, bytes) {
                log::error!("Failed to write save file: {e}");
            } else {
                log::info!("World saved ({} edited blocks).", save.edits.len());
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
    let save: WorldSave = bincode::deserialize(&bytes).ok()?;
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
        world,
        player_pos: Vec3::from_array(save.player_pos),
        yaw: save.player_yaw,
        pitch: save.player_pitch,
        time_of_day: save.time_of_day,
        modules: save.modules,
    })
}
