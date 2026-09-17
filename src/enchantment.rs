//! Persistent single-object bindings. Object replacement never silently retargets a rule.
use crate::{
    creature::Creatures,
    spell_target::Target,
    voxel::{BlockType, World},
};
use serde::{Deserialize, Serialize};
type Cell = (i32, i32, i32);
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldIdentity {
    pub id: String,
    pub revision: u64,
    pub next_creation: u64,
    pub blocks: Vec<(Cell, u64)>,
}
impl Default for WorldIdentity {
    fn default() -> Self {
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            id: format!(
                "{time:x}-{:x}",
                SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            revision: 0,
            next_creation: 1,
            blocks: Vec::new(),
        }
    }
}
impl WorldIdentity {
    pub fn block_changed(&mut self, cell: Cell) {
        self.revision = self.revision.saturating_add(1);
        if let Some((_, epoch)) = self.blocks.iter_mut().find(|b| b.0 == cell) {
            *epoch = epoch.saturating_add(1);
        }
    }
    fn track(&mut self, cell: Cell) -> Result<u64, String> {
        if let Some((_, epoch)) = self.blocks.iter().find(|b| b.0 == cell) {
            return Ok(*epoch);
        }
        if self.blocks.len() >= 1024 {
            return Err("Tracked block limit reached".into());
        }
        self.blocks.push((cell, 0));
        Ok(0)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Object {
    Creature {
        id: u32,
        species: u8,
    },
    Block {
        cell: Cell,
        material: BlockType,
        generation: u64,
    },
    Device {
        id: u64,
        cell: Cell,
        kind: crate::automation::Kind,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub world_id: String,
    pub world_revision: u64,
    pub object: Object,
}
impl Reference {
    pub fn capture(
        world: &mut World,
        creatures: &Creatures,
        target: Target,
    ) -> Result<Self, String> {
        world.automation.ensure_device_ids();
        let object = match target {
            Target::Creature { id } => {
                let species = creatures
                    .snapshot_with_ids()
                    .iter()
                    .find(|c| c.0 == id)
                    .map(|c| c.1)
                    .ok_or("Creature disappeared")?;
                Object::Creature { id, species }
            }
            Target::Block {
                position: cell,
                material,
            } => {
                if let Some(d) = world.automation.device_at(cell) {
                    Object::Device {
                        id: d.persistent_id,
                        cell: d.cell,
                        kind: d.kind,
                    }
                } else {
                    if world.get_block(cell.0, cell.1, cell.2) != material {
                        return Err("Block changed".into());
                    }
                    Object::Block {
                        cell,
                        material,
                        generation: world.identity.track(cell)?,
                    }
                }
            }
        };
        Ok(Self {
            world_id: world.identity.id.clone(),
            world_revision: world.identity.revision,
            object,
        })
    }
    /// False means temporarily unloaded: skip callbacks without losing the binding.
    pub fn available(&self, world: &World, creatures: &Creatures) -> Result<bool, String> {
        if self.world_id != world.identity.id {
            return Err("Attachment belongs to another world".into());
        }
        match self.object {
            Object::Creature { id, species } => {
                if creatures
                    .snapshot_with_ids()
                    .iter()
                    .any(|c| c.0 == id && c.1 == species)
                {
                    Ok(true)
                } else {
                    Err("Creature died or was removed".into())
                }
            }
            Object::Block {
                cell,
                material,
                generation,
            } => {
                if generation == u64::MAX
                    || world
                        .identity
                        .blocks
                        .iter()
                        .find(|b| b.0 == cell)
                        .map(|b| b.1)
                        != Some(generation)
                {
                    return Err("Block was replaced".into());
                }
                if !loaded(world, cell) {
                    return Ok(false);
                }
                if world.get_block(cell.0, cell.1, cell.2) == material {
                    Ok(true)
                } else {
                    Err("Block was replaced".into())
                }
            }
            Object::Device { id, cell, kind } => {
                if id == 0
                    || id == u64::MAX
                    || !world
                        .automation
                        .devices
                        .get(&cell)
                        .is_some_and(|d| d.persistent_id == id && d.kind == kind)
                {
                    return Err("Device was packed, removed or replaced".into());
                }
                Ok(loaded(world, cell))
            }
        }
    }
    pub fn matches_aim(&self, world: &World, target: Target) -> bool {
        match (&self.object, target) {
            (Object::Creature { id, .. }, Target::Creature { id: aim }) => *id == aim,
            (Object::Block { cell, .. }, Target::Block { position, .. }) => *cell == position,
            (Object::Device { id, .. }, Target::Block { position, .. }) => world
                .automation
                .device_at(position)
                .is_some_and(|d| d.persistent_id == *id),
            _ => false,
        }
    }
    pub fn label(&self) -> String {
        match self.object {
            Object::Creature { id, species } => format!(
                "{:?} #{id}",
                crate::creature::CreatureKind::from_u8(species)
            ),
            Object::Block { cell, material, .. } => {
                format!("{} at {}, {}, {}", material.name(), cell.0, cell.1, cell.2)
            }
            Object::Device { id, kind, .. } => format!("{} #{id}", kind.id()),
        }
    }
}
fn loaded(world: &World, cell: Cell) -> bool {
    world
        .chunks
        .contains_key(&crate::voxel::chunk::world_to_chunk(cell.0, cell.2))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub id: u64,
    pub creator: u32,
    pub target: Reference,
    pub lost: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Saved {
    pub binding: Binding,
    pub module: crate::scripting::ModuleSaveEntry,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub target: Reference,
    pub name: String,
    pub status: String,
}

pub fn hud(ctx: &egui::Context, target: &str, rules: &[String]) {
    if rules.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("object_enchantments"))
        .anchor(egui::Align2::CENTER_CENTER, [0., 65.])
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(egui::Color32::from_black_alpha(185))
                .inner_margin(6.)
                .show(ui, |ui| {
                    ui.colored_label(egui::Color32::from_rgb(210, 170, 255), target);
                    for rule in rules.iter().take(3) {
                        ui.small(rule);
                    }
                    if rules.len() > 3 {
                        ui.small(format!("+{} more in Rules", rules.len() - 3));
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::{Module, ScriptHost};
    use glam::Vec3;
    fn world() -> World {
        let mut w = World::new(1);
        let mut c = crate::voxel::chunk::Chunk::new(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                c.set_local(x, 24, z, BlockType::Stone);
            }
        }
        w.chunks.insert((0, 0), c);
        w
    }
    fn bind(w: &mut World, c: &Creatures, target: Target, source: &str) -> ScriptHost {
        let reference = Reference::capture(w, c, target).unwrap();
        let mut host = ScriptHost::new();
        host.add_generated(
            Module::load("attached".into(), "original request".into(), source.into()).unwrap(),
        )
        .unwrap();
        host.attach_at(
            0,
            Binding {
                id: 1,
                creator: 0,
                target: reference,
                lost: None,
            },
        )
        .unwrap();
        host
    }
    fn tick(host: &mut ScriptHost, w: &World, c: &mut Creatures) -> crate::scripting::TickOutcome {
        host.run_tick(
            w,
            c,
            &[],
            &mut 0.5,
            &mut crate::weather::WeatherState::new(1),
            &[],
            &[],
            [0; crate::voxel::COLLECTIBLE_BLOCKS.len()],
        )
    }
    const REPORT: &str =
        "function on_tick(api) local t=assert(api.get_rule_target()); api.broadcast(t.kind) end";
    #[test]
    fn block_binding_pauses_unloaded_and_never_follows_same_material_replacement() {
        let mut w = world();
        let mut c = Creatures::new();
        w.set_block(8, 25, 8, BlockType::Stone);
        let mut host = bind(
            &mut w,
            &c,
            Target::Block {
                position: (8, 25, 8),
                material: BlockType::Stone,
            },
            REPORT,
        );
        assert_eq!(tick(&mut host, &w, &mut c).broadcasts, ["block"]);
        w.set_block(9, 25, 9, BlockType::Soil);
        assert_eq!(tick(&mut host, &w, &mut c).broadcasts, ["block"]);
        let chunk = w.chunks.remove(&(0, 0)).unwrap();
        assert!(tick(&mut host, &w, &mut c).broadcasts.is_empty());
        assert!(host.modules[0].enabled);
        w.chunks.insert((0, 0), chunk);
        assert_eq!(tick(&mut host, &w, &mut c).broadcasts, ["block"]);
        w.set_block(8, 25, 8, BlockType::Air);
        w.set_block(8, 25, 8, BlockType::Stone);
        assert!(tick(&mut host, &w, &mut c).broadcasts.is_empty());
        assert!(!host.modules[0].enabled);
        assert!(host.modules[0].attachment.as_ref().unwrap().lost.is_some());
        host.toggle_at(0);
        assert!(tick(&mut host, &w, &mut c).broadcasts.is_empty());
    }
    #[test]
    fn attachment_save_is_separate_and_survives_module_reordering_and_world_reload() {
        let mut w = world();
        let mut c = Creatures::new();
        let id = c.spawn_one(
            crate::creature::CreatureKind::Sheep,
            Vec3::new(5., 25., 5.),
            1,
        );
        let mut host = bind(&mut w, &c, Target::Creature { id }, REPORT);
        host.add_generated(
            Module::load(
                "other".into(),
                "other".into(),
                "function on_tick(api) end".into(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(host.save_entries().len(), 1);
        let state = crate::save::CraftingSave {
            world_identity: w.identity.clone(),
            enchantments: host.save_enchantments(),
            creature_next_id: c.next_identity(),
            creatures: Some(c.snapshot_with_ids()),
            ..Default::default()
        };
        let saved: crate::save::CraftingSave =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        let mut restored = world();
        restored.identity = saved.world_identity;
        let mut restored_c = Creatures::new();
        restored_c.restore_saved(&saved.creatures.unwrap(), 1);
        restored_c.reserve_identities(saved.creature_next_id);
        let mut host = ScriptHost::load_from_save(&host.save_entries());
        host.load_enchantments(saved.enchantments);
        host.remove(0);
        assert_eq!(
            tick(&mut host, &restored, &mut restored_c).broadcasts,
            ["creature"]
        );
        assert_eq!(host.modules[0].prompt, "original request");
        host.detach_at(0);
        assert!(!host.modules[0].enabled);
        assert!(host.save_enchantments().is_empty());
        assert_eq!(host.save_entries().len(), 1);
    }
    #[test]
    fn dead_creature_ids_are_not_reused_on_reload_and_bindings_do_not_cross_worlds() {
        let mut w = world();
        let mut c = Creatures::new();
        let id = c.spawn_one(
            crate::creature::CreatureKind::Sheep,
            Vec3::new(5., 25., 5.),
            1,
        );
        let mut host = bind(&mut w, &c, Target::Creature { id }, REPORT);
        let next = c.next_identity();
        c.destroy(id);
        let mut restored = Creatures::new();
        restored.restore_saved(&c.snapshot_with_ids(), 1);
        restored.reserve_identities(next);
        let replacement = restored.spawn_one(
            crate::creature::CreatureKind::Sheep,
            Vec3::new(5., 25., 5.),
            1,
        );
        assert!(replacement > id);
        assert!(tick(&mut host, &w, &mut restored).broadcasts.is_empty());
        assert!(!host.modules[0].enabled);
        let reference =
            Reference::capture(&mut w, &restored, Target::Creature { id: replacement }).unwrap();
        assert!(reference.available(&world(), &restored).is_err());
    }
    #[test]
    fn packed_device_replacement_gets_new_id_but_configuration_keeps_attachment() {
        use crate::automation::{Action, Device, Kind};
        let mut w = world();
        let mut c = Creatures::new();
        let cell = (8, 25, 8);
        w.automation
            .devices
            .insert(cell, Device::new(Kind::Chest, cell, 0));
        let mut host = bind(
            &mut w,
            &c,
            Target::Block {
                position: cell,
                material: BlockType::AutomationDevice,
            },
            REPORT,
        );
        let old = w.automation.devices[&cell].persistent_id;
        w.automation.devices.get_mut(&cell).unwrap().config.enabled = false;
        assert_eq!(tick(&mut host, &w, &mut c).broadcasts, ["device"]);
        let mut state = std::mem::take(&mut w.automation);
        let mut account = crate::crafting::Account::default();
        let feet = Vec3::new(10.5, 25., 8.5);
        crate::automation::apply(
            &w,
            &mut state,
            &mut account,
            feet,
            &[],
            &Action::Pack { cell },
            crate::automation::balance(),
            &crate::crafting::Registry::load().unwrap(),
        )
        .unwrap();
        crate::automation::apply(
            &w,
            &mut state,
            &mut account,
            feet,
            &[],
            &Action::Place {
                kind: Kind::Chest,
                cell,
                rotation: 0,
                packed: Some(0),
            },
            crate::automation::balance(),
            &crate::crafting::Registry::load().unwrap(),
        )
        .unwrap();
        w.automation = state;
        assert_ne!(w.automation.devices[&cell].persistent_id, old);
        assert!(tick(&mut host, &w, &mut c).broadcasts.is_empty());
        assert!(!host.modules[0].enabled);
    }
    #[test]
    fn queued_callbacks_recheck_staged_target_replacement() {
        let mut w = world();
        let mut c = Creatures::new();
        w.set_block(8, 25, 8, BlockType::Stone);
        let reference = Reference::capture(
            &mut w,
            &c,
            Target::Block {
                position: (8, 25, 8),
                material: BlockType::Stone,
            },
        )
        .unwrap();
        let mut host = ScriptHost::new();
        let mut edit = Module::load(
            "edit".into(),
            "".into(),
            "function on_tick(api) api.replace_block(8,25,8,'soil') end".into(),
        )
        .unwrap();
        edit.enabled = true;
        host.modules.push(edit);
        host.add_generated(Module::load("bound".into(), "".into(), REPORT.into()).unwrap())
            .unwrap();
        host.attach_at(
            1,
            Binding {
                id: 1,
                creator: 0,
                target: reference,
                lost: None,
            },
        )
        .unwrap();
        let out = tick(&mut host, &w, &mut c);
        assert_eq!(out.block_edits.len(), 1);
        assert!(out.broadcasts.is_empty());
        assert!(!host.modules[1].enabled);
    }
    #[test]
    fn starter_wards_heal_only_near_the_bound_block_or_device() {
        for device in [false, true] {
            let mut w = world();
            let mut c = Creatures::new();
            let cell = (6, 25, 5);
            if device {
                w.automation.devices.insert(
                    cell,
                    crate::automation::Device::new(crate::automation::Kind::Chest, cell, 0),
                );
            } else {
                w.set_block(cell.0, cell.1, cell.2, BlockType::Stone);
            }
            let near = c.spawn_one(
                crate::creature::CreatureKind::Sheep,
                Vec3::new(4., 25., 5.),
                1,
            );
            let far = c.spawn_one(
                crate::creature::CreatureKind::Sheep,
                Vec3::new(14., 25., 5.),
                2,
            );
            c.damage(near, 2.);
            c.damage(far, 2.);
            let source = if device {
                include_str!("../modules/attached_device_ward.lua")
            } else {
                include_str!("../modules/attached_block_ward.lua")
            };
            let material = if device {
                BlockType::AutomationDevice
            } else {
                BlockType::Stone
            };
            let mut host = bind(
                &mut w,
                &c,
                Target::Block {
                    position: cell,
                    material,
                },
                source,
            );
            let before = c.snapshot_with_ids();
            let mut weather = crate::weather::WeatherState::new(1);
            weather.set(crate::weather::Weather::Rain);
            let out = host.run_tick(
                &w,
                &mut c,
                &[],
                &mut 0.5,
                &mut weather,
                &[],
                &[],
                [0; crate::voxel::COLLECTIBLE_BLOCKS.len()],
            );
            assert!(out.crashes.is_empty(), "{:?}", out.crashes);
            let after = c.snapshot_with_ids();
            assert!(
                after.iter().find(|c| c.0 == near).unwrap().3
                    > before.iter().find(|c| c.0 == near).unwrap().3
            );
            assert_eq!(
                after.iter().find(|c| c.0 == far).unwrap().3,
                before.iter().find(|c| c.0 == far).unwrap().3
            );
        }
    }
    #[test]
    fn failed_callback_rolls_back_and_keeps_binding_when_vm_is_rebuilt() {
        let mut w = world();
        let mut c = Creatures::new();
        let id = c.spawn_one(
            crate::creature::CreatureKind::Sheep,
            Vec3::new(5., 25., 5.),
            1,
        );
        c.damage(id, 2.);
        let mut host=bind(&mut w,&c,Target::Creature{id},"function on_tick(api) local t=assert(api.get_rule_target()); if api.is_night then api.heal_creature(t.id,1); error('rollback') end api.broadcast(t.kind) end");
        let before = c.snapshot_with_ids();
        tick(&mut host, &w, &mut c);
        assert_eq!(c.snapshot_with_ids(), before);
        assert!(!host.modules[0].enabled);
        host.toggle_at(0);
        let out = host.run_tick(
            &w,
            &mut c,
            &[],
            &mut 0.1,
            &mut crate::weather::WeatherState::new(1),
            &[],
            &[],
            [0; crate::voxel::COLLECTIBLE_BLOCKS.len()],
        );
        assert_eq!(out.broadcasts, ["creature"]);
        assert!(host.modules[0].attachment.is_some());
    }
}
