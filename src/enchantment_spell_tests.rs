use super::*;
use crate::{
    crafting::{Account, Registry},
    equipment::Entry,
    scripting::{Module, ScriptHost},
    spell_target::TargetContext,
    spellbook::{SpellType, Spellbook, TargetRequirement},
};
use glam::Vec3;

const SOURCE: &str = "-- spell_type: enchantment\n-- spell_target: block\nfunction on_tick(api) local t=api.get_rule_target(); if not t then return end; api.broadcast(t.kind .. ':' .. t.creator_id) end";

struct Fixture {
    host: ScriptHost,
    world: World,
    creatures: Creatures,
    account: Account,
    registry: Registry,
    id: u64,
    eye: Vec3,
}
impl Fixture {
    fn new() -> Self {
        let mut host = ScriptHost::new();
        let module = Module::load("Ward".into(), "Guard an object".into(), SOURCE.into()).unwrap();
        let id = host.spellbook.remember(&module, "Host").unwrap();
        let mut world = World::new(1);
        world
            .chunks
            .insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        world.set_block(8, 25, 12, BlockType::Stone);
        world.set_block(10, 25, 12, BlockType::Stone);
        let mut account = Account::default();
        account.mana = 100;
        account.add_spell_card(id).unwrap();
        host.spellbook.sync_hotbar(&mut account);
        account.hotbar.assign(Some(Entry::Spell(id)));
        Self {
            host,
            world,
            creatures: Creatures::new(),
            account,
            registry: Registry::load().unwrap(),
            id,
            eye: Vec3::new(8.5, 25.5, 8.5),
        }
    }
    fn cast_request(&self) -> Cast {
        Cast {
            spell_id: self.id,
            caster_id: 0,
            owner: "host".into(),
            eye: self.eye,
            alive: true,
            guest: false,
            context: TargetContext::resolve(&self.world, &self.creatures, self.eye, Vec3::Z),
        }
    }
    fn cast(&mut self, cast: Cast) -> Result<Reference, String> {
        self.host.cast_enchantment(
            &mut self.world,
            &self.creatures,
            &mut self.account,
            &self.registry,
            cast,
        )
    }
    fn tick(&mut self) -> crate::scripting::TickOutcome {
        self.host.run_tick(
            &self.world,
            &mut self.creatures,
            &[],
            &mut 0.5,
            &mut crate::weather::WeatherState::new(1),
            &[],
            &[],
            self.account.resources,
        )
    }
}

#[test]
fn remember_enchantment_without_target_is_inert_and_survives_reload() {
    let (kind, prompt) = crate::spell_workshop::Kind::Enchantment.prepare("Guard an object");
    assert!(matches!(kind, crate::llm::PromptKind::Rule));
    assert!(prompt.contains("[BOUND_OBJECT_RULE]"));
    assert!(!prompt.contains("[BOUND_TARGET:"));
    let mut f = Fixture::new();
    assert_eq!(
        f.host.spellbook.get(f.id).unwrap().spell_type,
        SpellType::Enchantment
    );
    assert!(!f.host.spellbook.get(f.id).unwrap().can_run_directly());
    assert!(f.host.save_enchantments().is_empty());
    let module = f.host.spellbook.get(f.id).unwrap().compiled().unwrap();
    let index = f.host.add_generated(module).unwrap();
    assert_eq!(f.host.toggle_at(index), None);
    assert!(f.tick().broadcasts.is_empty());
    let bytes = serde_json::to_vec(&f.host.spellbook).unwrap();
    let mut book: Spellbook = serde_json::from_slice(&bytes).unwrap();
    book.revalidate();
    assert!(book.get(f.id).unwrap().ready());
    assert!(!String::from_utf8(bytes).unwrap().contains("world_id"));
    // Even an accidentally enabled draft save cannot become a world rule.
    let mut entries = f.host.save_entries();
    entries[0].enabled = true;
    assert!(!ScriptHost::load_from_save(&entries).modules[0].enabled);
}

#[test]
fn enchantment_failed_casts_are_free_and_do_not_create_instances() {
    for case in 0..9 {
        let mut f = Fixture::new();
        let mut cast = f.cast_request();
        match case {
            0 => cast.context = None,
            1 => cast.eye = Vec3::new(8.5, 25.5, -30.),
            2 => f.host.spellbook.spells[0].target = TargetRequirement::Creature,
            3 => f.account.mana = 0,
            4 => {
                f.account.remove_spell_card(f.id).unwrap();
            }
            5 => cast.guest = true,
            6 => {
                f.world.set_block(8, 25, 12, BlockType::Air);
            }
            7 => cast.alive = false,
            _ => f.account.hotbar.select(8),
        }
        let before = f.account.clone();
        let next = f.world.identity.next_creation;
        assert!(f.cast(cast).is_err(), "case {case}");
        assert_eq!(f.account, before, "case {case} consumed resources");
        assert_eq!(f.world.identity.next_creation, next);
        assert!(f.host.save_enchantments().is_empty());
    }
}

#[test]
fn enchantment_template_creates_independent_instances_and_edits_do_not_rewrite_them() {
    let mut f = Fixture::new();
    f.cast(f.cast_request()).unwrap();
    f.eye.x = 10.5;
    f.cast(f.cast_request()).unwrap();
    assert_eq!(f.account.mana, 100 - 2 * crate::crafting::RULE_MANA);
    let saved = f.host.save_enchantments();
    assert_eq!(saved.len(), 2);
    assert_ne!(saved[0].binding.id, saved[1].binding.id);
    assert_ne!(saved[0].binding.target, saved[1].binding.target);
    assert_eq!(saved[0].binding.source_spell, Some((f.id, 1)));
    f.host
        .spellbook
        .update(f.id, "Revised ward".into(), TargetRequirement::Creature)
        .unwrap();
    assert_eq!(f.host.modules[0].name, "Ward");
    assert_eq!(
        f.host.modules[0].attachment.as_ref().unwrap().source_spell,
        Some((f.id, 1))
    );
    assert_eq!(f.tick().broadcasts, ["block:0", "block:0"]);
    f.host.delete_rule(0);
    assert!(
        f.host.spellbook.get(f.id).is_some(),
        "deleting an instance keeps its template"
    );
    f.host.spellbook.delete(f.id).unwrap();
    assert_eq!(
        f.tick().broadcasts,
        ["block:0"],
        "deleting template keeps existing instance"
    );
}

#[test]
fn enchantment_guest_reconnect_and_save_restore_keep_cards_targets_and_ownership() {
    let mut f = Fixture::new();
    f.host.spellbook.spells[0].allow_guests = true;
    let mut cast = f.cast_request();
    cast.guest = true;
    cast.caster_id = 3;
    cast.owner = "direct:Guest".into();
    f.cast(cast).unwrap();
    let save = crate::save::CraftingSave {
        spellbook: f.host.spellbook.clone(),
        enchantments: f.host.save_enchantments(),
        world_identity: f.world.identity.clone(),
        guests: [("direct:Guest".into(), f.account.clone())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let save: crate::save::CraftingSave =
        serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
    f.world.identity = save.world_identity;
    f.host = ScriptHost::new();
    f.host.spellbook = save.spellbook;
    f.host.spellbook.revalidate();
    f.host.load_enchantments(save.enchantments);
    f.account = save.guests["direct:Guest"].clone();
    crate::spell_network::guest_book(&f.host.spellbook).sync_hotbar(&mut f.account);
    assert_eq!(f.account.hotbar.entry(), Some(Entry::Spell(f.id)));
    assert!(f.account.has_spell_card(f.id));
    f.host.refresh_enchantment_owners(|_| None);
    assert_eq!(
        f.host.modules[0].attachment.as_ref().unwrap().creator,
        u32::MAX
    );
    f.host
        .refresh_enchantment_owners(|key| (key == "direct:Guest").then_some(7));
    assert_eq!(f.tick().broadcasts, ["block:7"]);
    let binding = f.host.modules[0].attachment.as_ref().unwrap();
    assert_eq!(binding.owner, "direct:Guest");
    assert_eq!(binding.source_spell, Some((f.id, 1)));
    assert!(binding.target.available(&f.world, &f.creatures).unwrap());
    f.world.set_block(8, 25, 12, BlockType::Air);
    f.world.set_block(8, 25, 12, BlockType::Stone);
    // Save and load after removal must not attach to the replacement.
    let saved = f.host.save_enchantments();
    f.host = ScriptHost::new();
    f.host.load_enchantments(saved);
    assert!(f.tick().broadcasts.is_empty());
    assert!(!f.host.modules[0].enabled);
    assert!(f.host.modules[0]
        .attachment
        .as_ref()
        .unwrap()
        .lost
        .is_some());
}

#[test]
fn enchantment_hotbar_selection_replacement_and_deletion_use_existing_rules() {
    let mut f = Fixture::new();
    let second = f.host.spellbook.duplicate(f.id).unwrap();
    f.account.add_spell_card(second).unwrap();
    f.host.spellbook.sync_hotbar(&mut f.account);
    f.account.hotbar.select(8);
    assert_eq!(f.account.hotbar.entry(), None);
    f.account.hotbar.assign(Some(Entry::Spell(f.id)));
    f.account.hotbar.assign(Some(Entry::Spell(second)));
    let mut account: Account =
        serde_json::from_slice(&serde_json::to_vec(&f.account).unwrap()).unwrap();
    assert_eq!(account.hotbar.entry(), Some(Entry::Spell(second)));
    f.host.spellbook.delete(second).unwrap();
    f.host.spellbook.sync_hotbar(&mut account);
    assert_eq!(account.hotbar.entry(), None);
    account.hotbar.select(0);
    assert_eq!(account.hotbar.entry(), Some(Entry::Spell(f.id)));
    account.hotbar.assign(None);
    assert_eq!(account.hotbar.entry(), None);
}

#[test]
fn enchantment_network_resolves_host_definition_and_rejects_forgery() {
    let mut f = Fixture::new();
    f.host.spellbook.spells[0].allow_guests = true;
    let spell = f.host.spellbook.get(f.id).unwrap();
    let summary = crate::spell_network::Summary::from_spell(spell);
    let summary: crate::spell_network::Summary =
        bincode::deserialize(&bincode::serialize(&summary).unwrap()).unwrap();
    let display = summary.into_spell().unwrap();
    assert_eq!(display.spell_type, SpellType::Enchantment);
    assert!(display.source.is_empty());
    assert!(!display.can_run_directly());
    let request = crate::spell_network::Request {
        device_id: None,
        world_revision: f.world.identity.revision,
        session: 1,
        sequence: 1,
        spell: f.id,
        revision: 1,
        facing: Vec3::Z.to_array(),
        target: f.cast_request().context.map(|c| c.target),
    };
    let resolve = |r: &crate::spell_network::Request, eye| {
        crate::spell_network::resolve_request(r, spell, &f.world, &f.creatures, eye)
    };
    assert!(resolve(&request, f.eye).is_ok());
    for case in 0..6 {
        let mut forged = request.clone();
        match case {
            0 => forged.spell += 1,
            1 => forged.revision += 1,
            2 => forged.target = None,
            3 => forged.facing = [f32::NAN, 0., 1.],
            4 => forged.facing = Vec3::NEG_Z.to_array(),
            _ => forged.world_revision += 1,
        }
        assert!(resolve(&forged, f.eye).is_err());
    }
    assert!(resolve(&request, Vec3::new(8.5, 25.5, -30.)).is_err());
}

#[test]
fn enchantment_guest_picks_authoritative_creature_ids_and_host_rechecks_them() {
    let mut f = Fixture::new();
    f.host.spellbook.spells[0].target = TargetRequirement::Creature;
    f.host.spellbook.spells[0].allow_guests = true;
    let id = f.creatures.spawn_one(
        crate::creature::CreatureKind::Sheep,
        Vec3::new(8.5, 24.85, 10.5),
        1,
    );
    let bodies = f.creatures.targeting_snapshot();
    let bodies: Vec<crate::spell_target::CreatureBody> =
        bincode::deserialize(&bincode::serialize(&bodies).unwrap()).unwrap();
    let preview = TargetContext::resolve_replica(&f.world, &bodies, f.eye, Vec3::Z).unwrap();
    assert_eq!(preview.target, Target::Creature { id });
    let host_pick = TargetContext::resolve(&f.world, &f.creatures, f.eye, Vec3::Z).unwrap();
    assert_eq!(preview.target, host_pick.target);
    let mut cast = f.cast_request();
    cast.guest = true;
    cast.context = Some(preview);
    cast.owner = "direct:Guest".into();
    cast.caster_id = 2;
    f.cast(cast).unwrap();
    assert_eq!(f.tick().broadcasts, ["creature:2"]);
    f.world.set_block(8, 25, 9, BlockType::Stone);
    assert!(preview.validate(&f.world, &f.creatures, f.eye).is_err());
    assert!(matches!(
        TargetContext::resolve_replica(&f.world, &bodies, f.eye, Vec3::Z)
            .unwrap()
            .target,
        Target::Block { .. }
    ));
}

#[test]
fn enchantment_devices_require_matching_category_and_placement_id() {
    let mut f = Fixture::new();
    let cell = (8, 25, 10);
    f.world.automation.devices.insert(
        cell,
        crate::automation::Device::new(crate::automation::Kind::Chest, cell, 0),
    );
    f.world.automation.ensure_device_ids();
    let mut cast = f.cast_request();
    let balance = f.account.mana;
    assert!(
        f.cast(cast).is_err(),
        "block-only spell must reject a device"
    );
    assert_eq!(f.account.mana, balance);
    f.host.spellbook.spells[0].target = TargetRequirement::Device;
    f.host.spellbook.spells[0].allow_guests = true;
    cast = f.cast_request();
    let request = crate::spell_network::Request {
        device_id: Some(f.world.automation.devices[&cell].persistent_id),
        world_revision: f.world.identity.revision,
        session: 1,
        sequence: 1,
        spell: f.id,
        revision: 1,
        facing: Vec3::Z.to_array(),
        target: cast.context.map(|c| c.target),
    };
    let spell = f.host.spellbook.get(f.id).unwrap();
    assert!(
        crate::spell_network::resolve_request(&request, spell, &f.world, &f.creatures, f.eye)
            .is_ok()
    );
    let reference = f.cast(cast).unwrap();
    assert!(matches!(reference.object, Object::Device { .. }));
    assert_eq!(f.tick().broadcasts, ["device:0"]);
    let mut request = request;
    request.world_revision = f.world.identity.revision;
    f.world
        .automation
        .devices
        .get_mut(&cell)
        .unwrap()
        .persistent_id += 1;
    assert!(crate::spell_network::resolve_request(
        &request,
        f.host.spellbook.get(f.id).unwrap(),
        &f.world,
        &f.creatures,
        f.eye
    )
    .is_err());
    assert!(reference.available(&f.world, &f.creatures).is_err());
}

#[test]
fn enchantment_ui_generates_without_target_and_offers_remember_without_run() {
    fn texts(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(t) => out.push(t.galley.job.text.clone()),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut f = Fixture::new();
    f.host
        .add_generated(f.host.spellbook.get(f.id).unwrap().compiled().unwrap())
        .unwrap();
    let ctx = egui::Context::default();
    let mut prompt = "Guard an object".to_string();
    let mut kind = crate::spell_workshop::Kind::Enchantment;
    let mut requests = crate::ui::UiRequests::default();
    let mut rendered = Vec::new();
    for frame in 0..3 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600., 1000.),
                )),
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
                events: if frame == 2 {
                    vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            ctrl: true,
                            command: true,
                            ..Default::default()
                        },
                    }]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            |ctx| {
                crate::spell_workshop::draw(
                    ctx,
                    &mut prompt,
                    &mut kind,
                    true,
                    true,
                    &f.host,
                    Some(0),
                    None,
                    &f.registry,
                    &mut None,
                    &mut None,
                    &mut requests,
                )
            },
        );
        rendered.clear();
        for shape in output.shapes {
            texts(&shape.shape, &mut rendered);
        }
    }
    assert_eq!(requests.submit_prompt.as_deref(), Some("Guard an object"));
    assert!(rendered.iter().any(|t| t == "Remember"), "{rendered:?}");
    assert!(!rendered
        .iter()
        .any(|t| t == "Run" || t.contains("Enchant + enable")));
    let mut panel = crate::spellbook_ui::Panel::default();
    panel.open = true;
    panel.selected = Some(f.id);
    for _ in 0..3 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600., 1000.),
                )),
                ..Default::default()
            },
            |ctx| {
                panel.draw(ctx, &f.host.spellbook, true, false);
            },
        );
        rendered.clear();
        for shape in output.shapes {
            texts(&shape.shape, &mut rendered);
        }
    }
    assert!(rendered.iter().any(|t| t == "ENCHANTMENT"));
    assert!(rendered.iter().any(|t| t.contains("Targets: Block")));
    assert!(!rendered
        .iter()
        .any(|t| t.contains("Run spell") || t.contains("▶  Run")));
}

#[test]
fn enchantment_legacy_binding_and_instant_spell_types_still_load() {
    let mut f = Fixture::new();
    f.cast(f.cast_request()).unwrap();
    let mut saved = serde_json::to_value(f.host.save_enchantments()).unwrap();
    saved[0]["binding"]
        .as_object_mut()
        .unwrap()
        .remove("source_spell");
    saved[0]["binding"].as_object_mut().unwrap().remove("owner");
    let entries = serde_json::from_value(saved).unwrap();
    f.host = ScriptHost::new();
    f.host.load_enchantments(entries);
    assert_eq!(f.tick().broadcasts, ["block:0"]);
    let instant = Module::load(
        "Flash".into(),
        "Flash".into(),
        "function on_cast(api,event) end".into(),
    )
    .unwrap();
    let id = f.host.spellbook.remember(&instant, "Host").unwrap();
    let mut saved = serde_json::to_value(&f.host.spellbook).unwrap();
    saved["spells"][0]
        .as_object_mut()
        .unwrap()
        .remove("spell_type");
    let mut book: Spellbook = serde_json::from_value(saved).unwrap();
    book.revalidate();
    assert!(book.get(id).unwrap().can_run_directly());
    assert!(book.get(id).unwrap().ready());
}
