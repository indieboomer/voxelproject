//! Isolated action replay, not a replay of the live server's simulation loop.
use super::{Action, Actor, Context, Outcome};
use glam::Vec3;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Deserialize)]
struct Header {
    schema: u32,
    agent_position: [f32; 3],
    agent_inventory: crate::crafting::Account,
    agent_state: crate::save::PlayerSave,
    registry: crate::crafting::Registry,
    #[serde(default)]
    replay_players: Vec<[f32; 3]>,
}

#[derive(Deserialize)]
struct Record {
    action: Action,
    outcome: Outcome,
    position_after: [f32; 3],
    inventory_after: Value,
    edits: Value,
    interacts: Value,
    #[serde(default)]
    players: Option<Vec<[f32; 3]>>,
    #[serde(default)]
    technical_player_after: Option<Value>,
    #[serde(default)]
    findings: Vec<super::evidence::Finding>,
}

/// Re-execute recorded actions without a model, window, network, or named-world writes.
pub fn replay(directory: &Path) -> Result<Value, String> {
    let header: Header = serde_json::from_reader(
        File::open(directory.join("session.json")).map_err(|e| format!("Read session: {e}"))?,
    )
    .map_err(|e| format!("Invalid session header: {e}"))?;
    if !matches!(header.schema, 1 | 2) {
        return Err(format!("Unsupported session schema {}", header.schema));
    }
    if !Vec3::from_array(header.agent_position).is_finite()
        || header
            .replay_players
            .iter()
            .any(|p| !Vec3::from_array(*p).is_finite())
    {
        return Err("Invalid replay player position".into());
    }
    let loaded = crate::save::load_playtest_snapshot(&directory.join("initial.bin"))
        .ok_or("Cannot load initial.bin")?;
    let mut world = loaded.world;
    let mut creatures = crate::creature::Creatures::new();
    if let Some(saved) = loaded.crafting.creatures {
        creatures.restore_saved(&saved, world.seed as u64);
        for (id, state) in loaded.crafting.behaviors {
            if creatures.behaviors.contains_key(&id) {
                creatures.behaviors.insert(id, state);
            }
        }
    }
    creatures.restore_dragons(loaded.crafting.dragons);
    creatures.restore_fish(loaded.crafting.fish);
    creatures.wildlife.extend(loaded.crafting.wildlife);
    let mut loot = crate::loot::Effects::restore_machine(
        loaded.crafting.loot.unwrap_or(loaded.crafting.machine_loot),
    );
    let mut actor = Actor::new(Vec3::from_array(header.agent_position));
    actor.player.crafting = header.agent_inventory;
    header.agent_state.restore(&mut actor.player);
    let players: Vec<_> = header
        .replay_players
        .into_iter()
        .map(Vec3::from_array)
        .collect();
    let log = BufReader::new(
        File::open(directory.join("events.jsonl")).map_err(|e| format!("Read events: {e}"))?,
    );
    let mut matched_ticks = 0u64;
    let mut attempted_ticks = 0u64;
    let mut first_divergence = Value::Null;
    let mut findings_reproduced = Vec::new();
    for (index, line) in log.lines().enumerate() {
        let line = line.map_err(|e| format!("Read event {}: {e}", index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(&line).map_err(|e| format!("Invalid event {}: {e}", index + 1))?;
        if value.get("authoritative_event").is_some() || value.get("external_event").is_some() {
            first_divergence = json!({"line":index + 1, "tick":value["tick"],
                "classification":"uncertain", "event":value,
                "reason":"External authoritative event cannot be reconstructed by isolated action replay."});
            break;
        }
        let record: Record = serde_json::from_value(value)
            .map_err(|e| format!("Invalid action event {}: {e}", index + 1))?;
        let tick_players: Option<Vec<Vec3>> = record
            .players
            .as_ref()
            .map(|entries| entries.iter().copied().map(Vec3::from_array).collect());
        if tick_players
            .as_ref()
            .is_some_and(|entries| entries.iter().any(|p| !p.is_finite()))
        {
            return Err(format!("Invalid collision player in event {}", index + 1));
        }
        // Saves persist edits and generation, not the transient loaded chunk set.
        let p = actor.player.position;
        let cx = (p.x.floor() as i32).div_euclid(16);
        let cz = (p.z.floor() as i32).div_euclid(16);
        for x in cx - 1..=cx + 1 {
            for z in cz - 1..=cz + 1 {
                world.ensure_chunk_loaded(x, z);
            }
        }
        let mut context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &header.registry,
            players: tick_players.as_deref().unwrap_or(&players),
        };
        let (outcome, effects) = actor.act(&mut context, &record.action);
        attempted_ticks += 1;
        let mut actual = json!({"outcome":outcome, "position_after":actor.player.position.to_array(),
            "inventory_after":actor.player.crafting, "edits":effects.edits, "interacts":effects.interacts});
        let mut expected = json!({"outcome":record.outcome, "position_after":record.position_after,
            "inventory_after":record.inventory_after, "edits":record.edits, "interacts":record.interacts});
        let mut differences = Vec::new();
        for field in ["outcome", "inventory_after", "edits", "interacts"] {
            if expected[field] != actual[field] {
                differences.push(field);
            }
        }
        if let Some(state) = record.technical_player_after {
            expected["technical_player_after"] = state;
            actual["technical_player_after"] =
                json!(crate::save::PlayerSave::capture(&actor.player));
            if !approximately_equal(
                &expected["technical_player_after"],
                &actual["technical_player_after"],
            ) {
                differences.push("technical_player_after");
            }
        }
        if record
            .position_after
            .iter()
            .zip(actor.player.position.to_array())
            .any(|(a, b)| !a.is_finite() || !b.is_finite() || (a - b).abs() > 0.0001)
        {
            differences.push("position_after");
        }
        if !differences.is_empty() {
            first_divergence = json!({"line":index + 1, "tick":outcome.tick,
                "fields":differences, "expected":expected, "actual":actual,
                "classification":"uncertain",
                "reason":"Isolated replay diverged; external world evolution or uncaptured transient state may explain the difference. This is not evidence of a game bug."});
            break;
        }
        for (cell, block, _) in effects.edits {
            context.world.set_block(cell.0, cell.1, cell.2, block);
        }
        for mut finding in effects.findings {
            if record.findings.iter().any(|original| {
                original.tick == finding.tick && original.invariant == finding.invariant
            }) {
                finding.reproduced = true;
                findings_reproduced.push(finding);
            }
        }
        matched_ticks += 1;
    }
    Ok(
        json!({"schema":1, "mode":"isolated_action_replay", "matched_ticks":matched_ticks,
        "attempted_ticks":attempted_ticks, "first_divergence":first_divergence,
        "findings_reproduced":findings_reproduced,
        "status":if first_divergence.is_null() && matched_ticks > 0 {"matched"} else {"uncertain"},
        "scope":"Recorded Actor actions and direct effects against initial save; no LLM calls or multiplayer coverage.",
        "limitations":["Creature AI, loot aging, automation ticks, weather, Lua/event callbacks, and other players' live actions are not replayed.",
            "Actor transient state starts at defaults; loaded chunks are regenerated around the agent. Full live-world determinism is not claimed.",
            "A matched action trace does not establish scenario success or absence of game bugs."]}),
    )
}

fn approximately_equal(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() <= 0.0001,
            _ => a == b,
        },
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.get(key)
                        .is_some_and(|other| approximately_equal(value, other))
                })
        }
        _ => expected == actual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playtest::{Scenario, Session, STEP};

    #[test]
    fn replays_saved_action_trace_and_localizes_tampered_outcome() {
        let (mut world, mut creatures, mut loot, registry, actor) =
            crate::playtest::tests::fixture();
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
            Scenario::GatherCraft,
        )
        .unwrap();
        let mut context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        for _ in 0..400 {
            let effects = session.update(STEP, &mut context).unwrap();
            for (cell, block, _) in effects.edits {
                context.world.set_block(cell.0, cell.1, cell.2, block);
            }
            if session.script.finished.is_some() {
                break;
            }
        }
        assert!(session.script.crafted);
        let report = replay(&session.directory).unwrap();
        assert_eq!(report["status"], "matched", "{report}");
        assert!(report["matched_ticks"].as_u64().unwrap() > 1);
        assert_eq!(report["findings_reproduced"], json!([]));
        let path = session.directory.join("events.jsonl");
        let contents = std::fs::read_to_string(&path).unwrap();
        let mut rows: Vec<Value> = contents
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        rows[1]["outcome"]["reason"] = json!("tampered reason");
        std::fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let report = replay(&session.directory).unwrap();
        assert_eq!(report["status"], "uncertain");
        assert_eq!(report["matched_ticks"], 1);
        assert_eq!(report["first_divergence"]["tick"], 2);
        assert_eq!(report["first_divergence"]["classification"], "uncertain");
        let mut rows: Vec<Value> = contents
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        rows[0]["technical_player_after"]["health"] = json!(1.0);
        std::fs::write(
            &path,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let report = replay(&session.directory).unwrap();
        assert_eq!(report["matched_ticks"], 0);
        assert_eq!(
            report["first_divergence"]["fields"],
            json!(["technical_player_after"])
        );
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n",
                contents.lines().next().unwrap(),
                json!({"tick":1,"authoritative_event":"Hostile creature hit Agent1","health":90})
            ),
        )
        .unwrap();
        let report = replay(&session.directory).unwrap();
        assert_eq!(report["matched_ticks"], 1);
        assert_eq!(report["status"], "uncertain");
        assert!(report["first_divergence"]["reason"]
            .as_str()
            .unwrap()
            .contains("External"));
    }
}
