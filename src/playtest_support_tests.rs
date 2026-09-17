//! A controlled discovery fixture: normal actions must preserve plant support.
use super::*;
use serde_json::{json, Value};

fn recorded_action(
    actor: &mut Actor,
    context: &mut Context<'_>,
    action: Action,
    rows: &mut Vec<Value>,
) -> Outcome {
    let (outcome, effects) = actor.act(context, &action);
    rows.push(json!({"action":action,"outcome":outcome,
        "position_after":actor.player.position.to_array(),
        "inventory_after":actor.player.crafting,
        "edits":effects.edits,"interacts":effects.interacts,"findings":effects.findings}));
    for (p, block, _) in effects.edits {
        context.world.set_block(p.0, p.1, p.2, block);
    }
    outcome
}

#[test]
fn reproduces_unsupported_decoration_after_legal_support_mining() {
    let (mut world, mut creatures, mut loot, registry, mut actor) = tests::fixture();
    // Explicit initial setup, persisted before any action. The agent does not
    // receive resources or teleport during the trace.
    actor.player.position = Vec3::new(8.5, 61.0, 8.5);
    world.set_block(8, 60, 8, BlockType::Stone);
    let support = (10, 61, 8);
    let decoration = (10, 62, 8);
    world.set_block(10, 60, 8, BlockType::Stone);
    world.set_block(support.0, support.1, support.2, BlockType::Stone);
    let grass_index = crate::voxel::COLLECTIBLE_BLOCKS
        .iter()
        .position(|b| *b == BlockType::ShortGrass)
        .unwrap();
    actor.player.crafting.resources[grass_index] = 1;
    let snapshot = crate::save::playtest_snapshot(
        &world,
        &actor.player,
        &crate::camera::Camera::new(actor.player.position, 1.0),
        0.25,
        vec![],
        &crate::save::CraftingSave::default(),
    )
    .unwrap();
    let mut session = Session::create(
        actor.player.position,
        &world,
        &snapshot,
        &registry,
        Scenario::MineBlock,
    )
    .unwrap();
    let header_path = session.directory.join("session.json");
    let mut header: Value = serde_json::from_slice(&std::fs::read(&header_path).unwrap()).unwrap();
    header["agent_inventory"] = json!(actor.player.crafting);
    header["agent_state"] = json!(crate::save::PlayerSave::capture(&actor.player));
    header["fixture"] = json!({"name":"plant_support_removal",
        "setup":"Flat dry ground, elevated standing block, two-block stone pillar, one ShortGrass and default tools. No inventory grants during actions.",
        "support":support,"decoration":decoration});
    std::fs::write(&header_path, serde_json::to_vec_pretty(&header).unwrap()).unwrap();
    let mut context = Context {
        world: &mut world,
        creatures: &mut creatures,
        loot: &mut loot,
        registry: &registry,
        players: &[],
    };
    let mut rows = Vec::new();
    assert_eq!(
        recorded_action(
            &mut actor,
            &mut context,
            Action::Equip {
                item: Some(equipment::Entry::Resource(BlockType::ShortGrass))
            },
            &mut rows
        )
        .status,
        Status::Completed
    );
    let direction = (Vec3::new(10.5, 61.99, 8.5) - (actor.player.position + Vec3::Y * 1.62))
        .normalize()
        .to_array();
    recorded_action(
        &mut actor,
        &mut context,
        Action::Look { direction },
        &mut rows,
    );
    let placed = recorded_action(
        &mut actor,
        &mut context,
        Action::Place {
            target: support,
            destination: Some(decoration),
        },
        &mut rows,
    );
    assert_eq!(placed.status, Status::Completed, "{}", placed.reason);
    assert_eq!(context.world.get_block(10, 62, 8), BlockType::ShortGrass);
    assert_eq!(actor.player.crafting.resources[grass_index], 0);
    assert!(context.world.get_block(10, 61, 8).is_solid());
    recorded_action(
        &mut actor,
        &mut context,
        Action::Equip {
            item: Some(equipment::Entry::Gear(equipment::Gear::Pickaxe)),
        },
        &mut rows,
    );
    let direction = (center(support) - (actor.player.position + Vec3::Y * 1.62))
        .normalize()
        .to_array();
    recorded_action(
        &mut actor,
        &mut context,
        Action::Look { direction },
        &mut rows,
    );
    for _ in 0..8 {
        for _ in 0..4 {
            recorded_action(&mut actor, &mut context, Action::Wait, &mut rows);
        }
        let outcome = recorded_action(
            &mut actor,
            &mut context,
            Action::Mine { target: support },
            &mut rows,
        );
        assert_ne!(outcome.status, Status::Rejected, "{}", outcome.reason);
        if context.world.get_block(10, 61, 8) == BlockType::Air {
            break;
        }
    }
    assert_eq!(context.world.get_block(10, 61, 8), BlockType::Air);
    let block = context
        .world
        .get_block(decoration.0, decoration.1, decoration.2);
    let unsupported = block.def().only_on_top
        && !context
            .world
            .get_block(decoration.0, decoration.1 - 1, decoration.2)
            .is_solid();
    assert!(
        unsupported,
        "Candidate was not reproduced; revisit the issue report"
    );
    let log = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(session.directory.join("events.jsonl"), log).unwrap();
    let reproduction = replay::replay(&session.directory).unwrap();
    assert_eq!(reproduction["status"], "matched", "{reproduction}");
    assert_eq!(reproduction["matched_ticks"], json!(rows.len()));
    assert!(
        reproduction["findings_reproduced"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["invariant"] == "decoration_support"
                && finding["reproduced"] == true
                && finding["tick"] == actor.tick),
        "{reproduction}"
    );
    let issue = json!({"schema":1,"id":"terrain.unsupported_decoration",
        "title":"Mining a plant's supporting block leaves the plant floating",
        "intended_goal":"Place ShortGrass legally, then mine its supporting stone",
        "observed_result":"The supporting stone becomes Air while ShortGrass remains above it",
        "expected_behavior":"only_on_top blocks must retain solid support below",
        "basis":{"kind":"specification","reference":"src/voxel/block.rs:117-119",
            "text":"only_on_top blocks can only ever sit on a solid block below"},
        "classification":"game_bug","severity":"low","confidence":"high",
        "independent_check":{"block":block,"only_on_top":block.def().only_on_top,
            "support_block":context.world.get_block(10,61,8),"violated":unsupported},
        "event_references":{"path":"events.jsonl","placement_tick":placed.tick,
            "support_removal_tick":actor.tick,"first_line":1,"last_line":rows.len()},
        "reproduction":reproduction,"reproduced":true,
        "scope":"Controlled scripted Actor fixture; not autonomous AI discovery or multiplayer coverage",
        "initial_save":"initial.bin","session":"session.json"});
    std::fs::write(
        session.directory.join("support-issue.json"),
        serde_json::to_vec_pretty(&issue).unwrap(),
    )
    .unwrap();
    session.script.finished = Some("Controlled support issue discovery finished".into());
    eprintln!(
        "Reproduced support issue artifacts: {}",
        session.directory.display()
    );
}
