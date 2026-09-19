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
    pub world_identity: crate::enchantment::WorldIdentity,
    pub creature_next_id: u32,
    pub enchantments: Vec<crate::enchantment::Saved>,
    pub spellbook: crate::spellbook::Spellbook,
    pub creature_statuses: std::collections::BTreeMap<u32, crate::creature::magic::Status>,
    pub starter_camp: Option<(i32, i32, i32)>,
    /// Complete scrollback, independent of the bounded on-screen log.
    pub chat_transcript: Vec<String>,
    pub npcs: Vec<crate::npc::Npc>,
    pub npc_distribution: crate::npc::Distribution,
    pub player: Option<PlayerSave>,
    pub weather: Option<crate::weather::WeatherState>,
    pub loot: Option<Vec<crate::loot::Drop>>,
    pub underground_discovered: std::collections::BTreeSet<(i32, i32)>,
    pub automation: crate::automation::State,
    pub machine_loot: Vec<crate::loot::Drop>,
    pub host: crate::crafting::Account,
    // Nicknames are the existing session identity; there are no accounts in the MVP.
    pub guests: std::collections::HashMap<String, crate::crafting::Account>,
    pub creatures: Option<Vec<crate::creature::SavedCreature>>,
    pub behaviors: std::collections::BTreeMap<u32, crate::creature::CreatureBehavior>,
    pub dragons: crate::creature::DragonSave,
    pub fish: crate::creature::FishSave,
    pub wildlife: std::collections::BTreeMap<u32, Option<(i32, i32)>>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlayerSave {
    pub health: f32,
    pub oxygen: f32,
    #[serde(default = "default_satiety")]
    pub satiety: f32,
    #[serde(default)]
    pub wetness: f32,
    pub poisoned: bool,
    pub speed: f32,
    pub jump: f32,
}
fn default_satiety() -> f32 {
    crate::player::MAX_SATIETY
}
impl PlayerSave {
    pub fn capture(player: &Player) -> Self {
        Self {
            health: player.health,
            oxygen: player.oxygen,
            satiety: player.satiety,
            wetness: player.wetness,
            poisoned: player.poisoned,
            speed: player.speed_multiplier,
            jump: player.jump_multiplier,
        }
    }
fn valid(&self) -> bool {
        self.health.is_finite()
            && (0.0..=crate::player::MAX_HEALTH).contains(&self.health)
            && self.oxygen.is_finite()
            && (0.0..=crate::player::MAX_OXYGEN).contains(&self.oxygen)
            && self.satiety.is_finite()
            && (0.0..=crate::player::MAX_SATIETY).contains(&self.satiety)
            && self.wetness.is_finite()
            && (0.0..=1.0).contains(&self.wetness)
            && self.speed.is_finite()
            && self.speed >= 0.0
            && self.jump.is_finite()
            && self.jump >= 0.0
    }
    pub fn restore(&self, player: &mut Player) {
        player.health = self.health;
        player.oxygen = self.oxygen;
        player.satiety = self.satiety.clamp(0.0, crate::player::MAX_SATIETY);
        player.wetness = self.wetness.clamp(0.0, 1.0);
        player.poisoned = self.poisoned;
        player.speed_multiplier = self.speed;
        player.jump_multiplier = self.jump;
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SaveV2 {
    #[serde(default = "legacy_generation")]
    generation: crate::worldgen::WorldGeneration,
    world: WorldSave,
    #[serde(default)]
    crafting: CraftingSave,
}
fn legacy_generation() -> crate::worldgen::WorldGeneration {
    crate::worldgen::WorldGeneration {
        tree_version: 0,
        underground: false,
        cave_version: 0,
        landscape_version: 0,
        ..Default::default()
    }
}
const MAGIC: &[u8] = b"VOXEL_SAVE_2\n";
/// Capture a development session without changing the named world's save file.
#[cfg(feature = "dev-playtest")]
pub fn playtest_snapshot(
    world: &World,
    player: &Player,
    camera: &Camera,
    time_of_day: f32,
    modules: Vec<ModuleSaveEntry>,
    crafting: &CraftingSave,
) -> Result<Vec<u8>, String> {
    encode_save(
        WorldSave {
            seed: world.seed,
            player_pos: player.position.to_array(),
            player_yaw: camera.yaw,
            player_pitch: camera.pitch,
            time_of_day,
            edits: world.edits.iter().map(|(k, v)| (*k, *v)).collect(),
            modules,
        },
        crafting,
        &world.generation,
    )
    .map_err(|e| e.to_string())
}
fn encode_save(
    world: WorldSave,
    crafting: &CraftingSave,
    generation: &crate::worldgen::WorldGeneration,
) -> Result<Vec<u8>, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct SaveRef<'a> {
        generation: &'a crate::worldgen::WorldGeneration,
        world: WorldSave,
        crafting: &'a CraftingSave,
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend(serde_json::to_vec(&SaveRef {
        world,
        crafting,
        generation,
    })?);
    Ok(bytes)
}
fn decode_save(
    bytes: &[u8],
) -> Option<(WorldSave, CraftingSave, crate::worldgen::WorldGeneration)> {
    if let Some(json) = bytes.strip_prefix(MAGIC) {
        let save: SaveV2 = serde_json::from_slice(json).ok()?;
        if !valid_world(&save.world)
            || save
                .crafting
                .starter_camp
                .is_some_and(|p| !crate::automation::valid_cell(p))
            || save.crafting.player.as_ref().is_some_and(|p| !p.valid())
            || save
                .crafting
                .weather
                .as_ref()
                .is_some_and(|w| !w.valid_save())
            || save
                .crafting
                .loot
                .as_ref()
                .is_some_and(|drops| drops.len() > 48 || drops.iter().any(|d| !d.valid_saved()))
        {
            return None;
        }
        save.generation.validate().ok()?;
        let recipes = crate::crafting::Registry::load().ok()?;
        save.crafting
            .automation
            .validate(crate::automation::balance(), &recipes)
            .ok()?;
        if save.crafting.machine_loot.len() > 48
            || save
                .crafting
                .machine_loot
                .iter()
                .any(|d| !d.valid_machine())
        {
            return None;
        }
        crate::automation::validate_account(
            &save.crafting.host,
            crate::automation::balance(),
            &recipes,
        )
        .ok()?;
        for account in save.crafting.guests.values() {
            crate::automation::validate_account(account, crate::automation::balance(), &recipes)
                .ok()?;
        }
        Some((save.world, save.crafting, save.generation))
    } else {
        let world = bincode::deserialize(bytes).ok()?;
        if !valid_world(&world) {
            return None;
        }
        Some((world, CraftingSave::default(), legacy_generation()))
    }
}
fn valid_world(world: &WorldSave) -> bool {
    world
        .player_pos
        .iter()
        .all(|p| p.is_finite() && p.abs() < 1_000_000.0)
        && world.player_yaw.is_finite()
        && world.player_pitch.is_finite()
        && world.time_of_day.is_finite()
        && (0.0..=1.0).contains(&world.time_of_day)
        && world.edits.iter().all(|((x, y, z), b)| {
            x.unsigned_abs() < 1_000_000
                && z.unsigned_abs() < 1_000_000
                && (0..crate::voxel::chunk::CHUNK_Y).contains(y)
                && *b != crate::voxel::BlockType::AutomationDevice
        })
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name == name.trim()
        && name.chars().count() <= 48
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || " -_".contains(c))
}
pub fn world_path(name: &str) -> Result<std::path::PathBuf, String> {
    if !valid_name(name) {
        return Err(
            "Use 1-48 letters, numbers, spaces, hyphens or underscores for the world name".into(),
        );
    }
    Ok(if name == "world" {
        "saves/world.bin".into()
    } else {
        std::path::PathBuf::from("saves").join(format!("world-{name}.bin"))
    })
}
pub fn saved_worlds() -> Vec<String> {
    saved_worlds_in(Path::new("saves"))
}
fn saved_worlds_in(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let file = entry.file_name();
            let Some(file) = file.to_str() else {
                continue;
            };
            let name = if file == "world.bin" {
                Some("world")
            } else {
                file.strip_prefix("world-")
                    .and_then(|s| s.strip_suffix(".bin"))
            };
            if let Some(name) = name.filter(|n| valid_name(n)) {
                names.push(name.to_owned());
            }
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    names
}

fn write_save(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    let temp = path.with_extension(format!("{extension}.tmp"));
    let backup = path.with_extension(format!("{extension}.bak"));
    let mut file = fs::File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // Retain the previous complete save; never delete it before replacement succeeds.
    if path.exists() {
        fs::copy(path, &backup)?;
    }
    fs::rename(&temp, path)
}

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
) -> Result<(), String> {
    let save = WorldSave {
        seed: world.seed,
        player_pos: player.position.to_array(),
        player_yaw: camera.yaw,
        player_pitch: camera.pitch,
        time_of_day,
        edits: world.edits.iter().map(|(k, v)| (*k, *v)).collect(),
        modules,
    };

    let path = world_path(&world.name)?;
    let bytes = encode_save(save, crafting, &world.generation).map_err(|e| e.to_string())?;
    write_save(&path, &bytes).map_err(|e| format!("Could not save {}: {e}", world.name))?;
    export_chat(
        &path.with_file_name(format!("{}_chat.log", world.name)),
        &crafting.chat_transcript,
    )
    .map_err(|e| {
        format!(
            "World saved, but could not export chat for {}: {e}",
            world.name
        )
    })
}

fn export_chat(path: &Path, transcript: &[String]) -> std::io::Result<()> {
    let mut text = transcript.join("\n");
    if !transcript.is_empty() {
        text.push('\n');
    }
    write_save(path, text.as_bytes())
}

pub fn save_exists() -> bool {
    !saved_worlds().is_empty()
}

pub fn load_world(name: &str) -> Option<LoadedWorld> {
    load_path(&world_path(name).ok()?, name)
}
fn load_path(path: &Path, name: &str) -> Option<LoadedWorld> {
    let bytes = fs::read(path).ok()?;
    let (save, crafting, generation) = decode_save(&bytes)?;
    let mut world = World::new(save.seed);
    world.name = name.to_owned();
    world.identity = crafting.world_identity.clone();
    world.underground_discovered = crafting.underground_discovered.clone();
    world.automation = crafting.automation.clone();
    world.starter_camp = crafting.starter_camp;
    world.generation = generation;
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

/// Read a development artifact without touching a named world save.
#[cfg(feature = "dev-playtest")]
pub fn load_playtest_snapshot(path: &Path) -> Option<LoadedWorld> {
    load_path(path, "Playtest replay")
}

#[cfg(test)]
mod tests {
    #[test]
    fn chat_export_retains_full_history_without_duplicates_across_saves() {
        let dir = std::path::PathBuf::from(format!("target/chat-save-test-{}", std::process::id()));
        let path = dir.join("Forest World_chat.log");
        let mut crafting = super::CraftingSave::default();
        crafting.chat_transcript = (0..600).map(|i| format!("Player: wiadomość {i}")).collect();
        let bytes = super::encode_save(world_save(), &crafting, &Default::default()).unwrap();
        let (_, mut restored, _) = super::decode_save(&bytes).unwrap();
        assert_eq!(restored.chat_transcript, crafting.chat_transcript);
        super::export_chat(&path, &restored.chat_transcript).unwrap();
        let first = std::fs::read(&path).unwrap();
        super::export_chat(&path, &restored.chat_transcript).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first);
        restored.chat_transcript.push("Guest: found iron!".into());
        super::export_chat(&path, &restored.chat_transcript).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 601);
        assert!(text.starts_with("Player: wiadomość 0\n"));
        assert!(text.ends_with("Guest: found iron!\n"));
        assert_eq!(
            std::fs::read(path.with_extension("log.bak")).unwrap(),
            first
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("log.bak")).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
    use super::*;
    #[cfg(feature = "dev-playtest")]
    #[test]
    fn playtest_snapshot_loads_through_the_real_save_reader() {
        let mut world = World::new(71);
        world.set_block(2, 127, 2, crate::voxel::BlockType::Crystal);
        let mut player = Player::new(Vec3::new(2.5, 60.0, 2.5));
        player.health = 37.0;
        player.crafting.mana = 17;
        let crafting = CraftingSave {
            starter_camp: Some((4, 30, 5)),
            player: Some(PlayerSave::capture(&player)),
            host: player.crafting.clone(),
            ..Default::default()
        };
        let bytes = playtest_snapshot(
            &world,
            &player,
            &Camera::new(player.position, 1.0),
            0.35,
            vec![],
            &crafting,
        )
        .unwrap();
        let path = std::path::PathBuf::from(format!(
            "target/playtest-save-test-{}.bin",
            std::process::id()
        ));
        fs::write(&path, bytes).unwrap();
        let loaded = load_path(&path, "diagnostic snapshot").unwrap();
        assert_eq!(loaded.player_pos, player.position);
        assert_eq!(loaded.world.starter_camp, Some((4, 30, 5)));
        assert_eq!(loaded.crafting.host.mana, 17);
        assert_eq!(loaded.crafting.player.unwrap().health, 37.0);
        assert_eq!(
            loaded.world.edits.get(&(2, 127, 2)),
            Some(&crate::voxel::BlockType::Crystal)
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn named_files_replace_safely_and_keep_previous_complete_save() {
        let dir = std::path::PathBuf::from(format!("target/save-test-{}", std::process::id()));
        let path = dir.join("world-First World.bin");
        let second = dir.join("world-Second World.bin");
        let old = encode_save(world_save(), &CraftingSave::default(), &Default::default()).unwrap();
        write_save(&path, &old).unwrap();
        write_save(&second, &old).unwrap();
        let mut crafting = CraftingSave::default();
        let p = (0, 30, 0);
        crafting.host.adventure = crate::adventure::Progress {
            stage: 2,
            home: Some((4, 30, 8)),
            explored_depths: true,
            recoveries: 3,
            ..Default::default()
        };
        crafting
            .guests
            .insert("expedition-guest".into(), crafting.host.clone());
        let mut chest = crate::automation::Device::new(crate::automation::Kind::Chest, p, 0);
        chest.items.insert("resource:stone".into(), 2_000_000);
        crafting.automation.devices.insert(p, chest);
        crafting.underground_discovered.insert((-1, 2));
        let mut weather = crate::weather::WeatherState::new(42);
        weather.set(crate::weather::Weather::Storm);
        crafting.weather = Some(weather);
        let mut player = Player::new(Vec3::ZERO);
        player.health = 31.0;
        player.poisoned = true;
        crafting.player = Some(PlayerSave::capture(&player));
        crafting.loot = Some(vec![crate::loot::Drop {
            pos: [1., 2., 3.],
            contents: vec![(crate::voxel::BlockType::Stone, 2)],
            age: 1.,
            cargo: Default::default(),
            machine: false,
            launch: None,
        }]);
        let mut saved = world_save();
        saved
            .edits
            .push(((4, 127, 4), crate::voxel::BlockType::Bricks));
        let bytes = encode_save(saved, &crafting, &Default::default()).unwrap();
        write_save(&path, &bytes).unwrap();
        assert_eq!(fs::read(path.with_extension("bin.bak")).unwrap(), old);
        assert_eq!(fs::read(&second).unwrap(), old);
        let mut loaded = load_path(&path, "First World").unwrap();
        assert_eq!(loaded.crafting.host.adventure, crafting.host.adventure);
        assert_eq!(
            loaded.crafting.guests["expedition-guest"].adventure,
            crafting.host.adventure
        );
        assert_eq!(loaded.world.name, "First World");
        assert_eq!(loaded.player_pos, Vec3::new(1., 2., 3.));
        loaded.world.ensure_chunk_loaded(0, 0);
        assert_eq!(
            loaded.world.get_block(4, 127, 4),
            crate::voxel::BlockType::Bricks
        );
        assert_eq!(
            loaded.world.get_block(1, 2, 3),
            crate::voxel::BlockType::Bricks
        );
        assert_eq!(
            loaded.world.get_block(0, 30, 0),
            crate::voxel::BlockType::AutomationDevice
        );
        assert!(loaded.world.underground_discovered.contains(&(-1, 2)));
        assert_eq!(saved_worlds_in(&dir), vec!["First World", "Second World"]);
        let restored = loaded.crafting;
        assert_eq!(restored.automation, crafting.automation);
        assert_eq!(
            restored.underground_discovered,
            crafting.underground_discovered
        );
        assert_eq!(restored.weather, crafting.weather);
        assert_eq!(restored.player, crafting.player);
        assert_eq!(
            restored.loot.unwrap()[0].contents,
            crafting.loot.unwrap()[0].contents
        );
        for name in ["../escape", "a/b", "a\\b", "", " world", "world."] {
            assert!(world_path(name).is_err());
        }
        assert_eq!(
            world_path("First World").unwrap(),
            Path::new("saves/world-First World.bin")
        );
        assert_eq!(world_path("world").unwrap(), Path::new("saves/world.bin"));
    }
    #[test]
    fn legacy_generation_stays_unchanged_and_bad_positions_are_rejected() {
        let mut config = serde_json::to_value(crate::worldgen::WorldGeneration::default()).unwrap();
        config.as_object_mut().unwrap().remove("underground");
        assert!(
            !serde_json::from_value::<crate::worldgen::WorldGeneration>(config)
                .unwrap()
                .underground
        );
        let mut world = world_save();
        world.player_pos[0] = f32::INFINITY;
        assert!(decode_save(&bincode::serialize(&world).unwrap()).is_none());
    }
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
        let (world, crafting, _) = decode_save(&bytes).unwrap();
        assert_eq!(world.seed, 42);
        assert_eq!(crafting.host, crate::crafting::Account::default());
        assert!(crafting.creatures.is_none());
        assert!(crafting.spellbook.spells.is_empty());
    }
    #[test]
    fn spellbook_survives_world_envelope_and_old_json_defaults_without_changing_modules() {
        let mut crafting = CraftingSave::default();
        let source = include_str!("../modules/target_heal.lua");
        let module =
            crate::scripting::Module::load("Heal".into(), "Heal my target".into(), source.into())
                .unwrap();
        let id = crafting.spellbook.remember(&module, "Host").unwrap();
        crafting
            .spellbook
            .set_flavor_quote(id, 1, "Even broken things remember the shape of hope.")
            .unwrap();
        let mut world = world_save();
        world.modules.push(module.to_save_entry());
        let bytes = encode_save(world, &crafting, &Default::default()).unwrap();
        let (world, mut restored, _) = decode_save(&bytes).unwrap();
        restored.spellbook.revalidate();
        let spell = restored.spellbook.get(id).unwrap();
        assert!(spell.ready());
        assert_eq!(spell.source, source);
        assert_eq!(
            spell.flavor_quote,
            "Even broken things remember the shape of hope."
        );
        assert_eq!(spell.artwork, crafting.spellbook.get(id).unwrap().artwork);
        assert_eq!(
            crate::spell_art::render(spell.artwork()),
            crate::spell_art::render(module.artwork)
        );
        assert_eq!(world.modules[0].source, source);
        let mut json: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(MAGIC).unwrap()).unwrap();
        // Existing worlds with remembered spells but no artwork migrate offline.
        json["crafting"]["spellbook"]["spells"][0]
            .as_object_mut()
            .unwrap()
            .remove("artwork");
        json["crafting"]["spellbook"]["spells"][0]
            .as_object_mut()
            .unwrap()
            .remove("flavor_quote");
        let mut legacy = MAGIC.to_vec();
        legacy.extend(serde_json::to_vec(&json).unwrap());
        let (_, mut migrated, _) = decode_save(&legacy).unwrap();
        migrated.spellbook.revalidate();
        assert!(migrated.spellbook.get(id).unwrap().flavor_quote.is_empty());
        assert_eq!(
            migrated.spellbook.get(id).unwrap().artwork(),
            spell.artwork()
        );
        json["crafting"]
            .as_object_mut()
            .unwrap()
            .remove("spellbook");
        let mut old = MAGIC.to_vec();
        old.extend(serde_json::to_vec(&json).unwrap());
        let (world, restored, _) = decode_save(&old).unwrap();
        assert!(restored.spellbook.spells.is_empty());
        assert_eq!(world.modules[0].source, source);
    }
    #[test]
    fn generated_rule_art_survives_world_save_and_attachment_save() {
        let artwork = crate::spell_art::Recipe::fallback("sheep hunt at night");
        let source = format!(
            "-- Intent plan: {}\nfunction on_tick(api) end",
            serde_json::json!({"artwork":artwork})
        );
        let module =
            crate::scripting::Module::load("Night hunt".into(), "A custom rule".into(), source)
                .unwrap();
        let mut world = world_save();
        world.modules.push(module.to_save_entry());
        let crafting = CraftingSave::default();
        let bytes = encode_save(world, &crafting, &Default::default()).unwrap();
        let (saved, _, _) = decode_save(&bytes).unwrap();
        let loaded = crate::scripting::ScriptHost::load_from_save(&saved.modules);
        assert_eq!(loaded.modules.last().unwrap().artwork, artwork);
        assert_eq!(
            crate::spell_art::render(loaded.modules.last().unwrap().artwork),
            crate::spell_art::render(artwork)
        );
        // Attachments use this same ModuleSaveEntry inside their Saved record.
        let entry: crate::scripting::ModuleSaveEntry =
            serde_json::from_str(&serde_json::to_string(&module.to_save_entry()).unwrap()).unwrap();
        let loaded = crate::scripting::ScriptHost::load_from_save(&[entry]);
        assert_eq!(loaded.modules[0].artwork, artwork);
    }
    #[test]
    fn automation_save_preserves_paid_batches_and_packed_devices() {
        use crate::automation::*;
        let mut crafting = CraftingSave::default();
        let p = (0, 30, 0);
        let mut d = Device::new(Kind::Condenser, p, 0);
        d.mana = 60;
        crafting.automation.devices.insert(p, d);
        crafting
            .automation
            .step(balance(), &crate::crafting::Registry::load().unwrap());
        crafting
            .host
            .packed_devices
            .push(crafting.automation.devices[&p].clone());
        crafting
            .host
            .production_goods
            .insert("creature:sheep".into(), 2);
        crafting.host.production_goods.insert("harvest:campfire_dish".into(), 1);
        crafting.host.production_goods.insert("harvest:campfire_herbal_dish".into(), 1);
        crafting.machine_loot.push(crate::loot::Drop {
            pos: [3.0, 30.3, 0.0],
            contents: vec![(crate::voxel::BlockType::Stone, 2)],
            age: 0.4,
            cargo: Default::default(),
            machine: true,
            launch: Some([0.5, 31.25, 0.5]),
        });
        let bytes = encode_save(world_save(), &crafting, &Default::default()).unwrap();
        let (_, loaded, _) = decode_save(&bytes).unwrap();
        assert_eq!(loaded.automation, crafting.automation);
        assert_eq!(loaded.host, crafting.host);
        assert_eq!(
            serde_json::to_string(&loaded.machine_loot).unwrap(),
            serde_json::to_string(&crafting.machine_loot).unwrap()
        );
        crafting.host.packed_devices[0].mana = u32::MAX;
        assert!(
            decode_save(&encode_save(world_save(), &crafting, &Default::default()).unwrap())
                .is_none()
        );
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
        let bytes = encode_save(world_save(), &crafting, &Default::default()).unwrap();
        let (world, loaded, _) = decode_save(&bytes).unwrap();
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
        assert_eq!(decode_save(&bytes).unwrap().2, legacy_generation());
        assert!(decode_save(b"not a save").is_none());
    }

    #[test]
    fn tree_generation_version_and_removed_wood_survive_save() {
        let mut saved = world_save();
        saved.edits.push(((15, 30, 16), crate::voxel::BlockType::Air));
        let generation = crate::worldgen::WorldGeneration::default();
        let bytes = encode_save(saved, &CraftingSave::default(), &generation).unwrap();
        let (loaded, _, settings) = decode_save(&bytes).unwrap();
        assert_eq!(settings.tree_version, 2);
        assert!(loaded.edits.contains(&((15, 30, 16), crate::voxel::BlockType::Air)));
        let mut json: serde_json::Value = serde_json::from_slice(&bytes[MAGIC.len()..]).unwrap();
        json["generation"].as_object_mut().unwrap().remove("tree_version");
        let mut old = MAGIC.to_vec();
        old.extend(serde_json::to_vec(&json).unwrap());
        assert_eq!(decode_save(&old).unwrap().2.tree_version, 0);
        json["generation"]["tree_version"] = 3.into();
        let mut invalid = MAGIC.to_vec();
        invalid.extend(serde_json::to_vec(&json).unwrap());
        assert!(decode_save(&invalid).is_none());
    }

    #[test]
    fn prompted_terrain_survives_save_and_invalid_settings_are_rejected() {
        let config = crate::worldgen::WorldGeneration {
            description: "Sandy islands".into(),
            creatures: [("sheep".into(), 1000), ("cow".into(), 0)]
                .into_iter()
                .collect(),
            shape: crate::worldgen::Shape::Islands,
            surface: crate::worldgen::Surface::Sand,
            trees: 0,
            ..Default::default()
        };
        let bytes = encode_save(world_save(), &CraftingSave::default(), &config).unwrap();
        let (_, _, loaded) = decode_save(&bytes).unwrap();
        assert_eq!(loaded, config);
        for x in -40..40 {
            assert_eq!(loaded.height(x, 19, 42), config.height(x, 19, 42));
        }
        let invalid = crate::worldgen::WorldGeneration {
            relief: 201,
            ..config
        };
        assert!(decode_save(
            &encode_save(world_save(), &CraftingSave::default(), &invalid).unwrap()
        )
        .is_none());
    }
}
