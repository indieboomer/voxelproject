//! Model interpretation, typed capability compilation, and semantic review.
use super::{request_completion, request_json, ChatMessage, LlmClient, PromptKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub execution: String,
    pub summary: String,
    pub condition: String,
    pub actor_kind: String,
    pub effect: String,
    pub targets: String,
    pub api_groups: Vec<String>,
    pub requirements: Vec<String>,
    pub unsupported: Vec<String>,
}
const SPECIES: &[&str] = &[
    "sheep",
    "chicken",
    "cow",
    "wolf",
    "stone_golem",
    "goblin",
    "stinger",
    "sunscorch",
    "zombie",
    "skeleton",
    "dragon_green",
    "dragon_red",
    "fish",
];
const GROUPS: &[&str] = &[
    "creatures",
    "players",
    "inventory",
    "weather",
    "time",
    "blocks",
];
pub const SUMMARY_PREFIX: &str = "-- Interpretation: ";

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["execution","summary","condition","actor_kind","effect","targets","api_groups","requirements","unsupported"],"properties":{
        "execution":{"type":"string","enum":["rule","instant"]},
        "summary":{"type":"string"},
        "condition":{"type":"string","enum":["always","rain","sunny","storm","mist","night","day","custom"]},
        "actor_kind":{"type":"string","enum":["none","sheep","chicken","cow","wolf","stone_golem","goblin","stinger","sunscorch","zombie","skeleton","dragon_green","dragon_red","fish"]},
        "effect":{"type":"string","enum":["protect_players","suppress_attacks","custom"]},
        "targets":{"type":"string","enum":["players","all","custom"]},
        "api_groups":{"type":"array","items":{"type":"string","enum":GROUPS},"maxItems":6},
        "requirements":{"type":"array","items":{"type":"string"},"maxItems":8},
        "unsupported":{"type":"array","items":{"type":"string"},"maxItems":4}
    }})
}
impl Plan {
    pub fn validate(&self, kind: PromptKind) -> Result<(), String> {
        let expected = if kind == PromptKind::Rule {
            "rule"
        } else {
            "instant"
        };
        if self.execution != expected {
            return Err(format!(
                "Expected {expected} execution, got {}",
                self.execution
            ));
        }
        if !self.unsupported.is_empty() {
            return Err(format!(
                "Needs clarification or unsupported: {}",
                self.unsupported.join("; ")
            ));
        }
        if self.summary.trim().is_empty()
            || self.summary.len() > 500
            || self.summary.chars().any(char::is_control)
        {
            return Err("Interpretation must be one short nonempty line".into());
        }
        if self.requirements.is_empty()
            || self.requirements.len() > 8
            || self.requirements.iter().any(|r| r.len() > 600)
        {
            return Err("Expected 1-8 bounded behavior requirements".into());
        }
        if self.api_groups.is_empty()
            || self.api_groups.len() > 6
            || self
                .api_groups
                .iter()
                .any(|g| !GROUPS.contains(&g.as_str()))
        {
            return Err("Invalid API groups".into());
        }
        if ![
            "always", "rain", "sunny", "storm", "mist", "night", "day", "custom",
        ]
        .contains(&self.condition.as_str())
            || !(self.actor_kind == "none" || SPECIES.contains(&self.actor_kind.as_str()))
            || !["protect_players", "suppress_attacks", "custom"].contains(&self.effect.as_str())
        {
            return Err("Unknown condition, actor or effect".into());
        }
        if kind == PromptKind::Instant && self.condition != "always" {
            return Err("Instant actions must use condition=always; a desired time or weather is an outcome, not a condition".into());
        }
        let expected_targets = match self.effect.as_str() {
            "protect_players" => "players",
            "suppress_attacks" => "all",
            _ => "custom",
        };
        if !["players", "all", "custom"].contains(&self.targets.as_str())
            || (self.effect != "custom" && self.targets != expected_targets)
        {
            return Err(format!("Target scope {} is inconsistent with effect {}. Use protect_players for players/adventurers only; suppress_attacks for all targets; custom for richer requests.",self.targets,self.effect));
        }
        if self.effect != "custom"
            && (kind != PromptKind::Rule || self.condition == "custom" || self.actor_kind == "none")
        {
            return Err("Temporary protection requires a rule, one supported condition and a creature species. Use custom for richer requests.".into());
        }
        Ok(())
    }
    /// Generic compiler: no natural-language keywords or per-prompt Lua templates.
    pub fn compile(&self) -> Option<String> {
        if self.effect == "custom" {
            return None;
        }
        let predicate = match self.condition.as_str() {
            "always" => "true".into(),
            "night" => "api.is_night".into(),
            "day" => "not api.is_night".into(),
            weather => format!("api.weather == {weather:?}"),
        };
        let body = if self.effect == "protect_players" {
            format!(
                "for _, p in ipairs(api.players()) do api.protect_player(p.id, {:?}) end",
                self.actor_kind
            )
        } else {
            format!("for _, c in ipairs(api.creatures()) do if c.kind == {:?} then api.suppress_creature_attacks(c.id) end end",self.actor_kind)
        };
        Some(format!(
            "function on_tick(api)\n    if {predicate} then\n        {body}\n    end\nend"
        ))
    }
    fn annotate(&self, code: &str, verification: &str) -> String {
        let meaning = if self.effect == "custom" {
            self.summary.clone()
        } else {
            let condition = match self.condition.as_str() {
                "always" => "this rule is enabled",
                "rain" => "it rains",
                "night" => "it is night",
                "day" => "it is daytime",
                "storm" => "there is a storm",
                "mist" => "there is mist",
                _ => "the weather is sunny",
            };
            format!("While {condition}, {} cannot land attacks against {}. Protection ends with the condition or when this rule is disabled.", self.actor_kind.replace('_'," "),
                if self.targets=="players" {"players; other creatures are unaffected"} else {"any target"})
        };
        format!(
            "{SUMMARY_PREFIX}{meaning}\n-- Intent plan: {}\n-- Validation: {verification}\n{code}",
            serde_json::to_string(self).unwrap()
        )
    }
}
fn interpreted(
    url: &str,
    prompt: &str,
    kind: PromptKind,
    feedback: Option<&str>,
) -> Result<Plan, String> {
    let instructions = include_str!("../prompts/interpretation.txt");
    let mut messages = vec![
        ChatMessage::system(instructions.into()),
        ChatMessage::user(LlmClient::build_user_turn(prompt, kind)),
    ];
    if let Some(feedback) = feedback {
        messages.push(ChatMessage::user(format!("A previous interpretation failed review. Correct these issues while preserving original intent: {feedback}")));
    }
    // A complete example teaches the shape and actor/beneficiary distinction.
    messages.insert(
        1,
        ChatMessage::user(
            "Persistent rule request: mist keeps adventurers safe from goblin attacks".into(),
        ),
    );
    messages.insert(2,ChatMessage::assistant(serde_json::to_string(&Plan {
        execution:"rule".into(),summary:"During mist, players are protected from goblin attacks; protection ends with mist.".into(),
        condition:"mist".into(),actor_kind:"goblin".into(),effect:"protect_players".into(),targets:"players".into(),
        api_groups:vec!["weather".into(),"creatures".into(),"players".into()],
        requirements:vec!["Prevent goblins hurting players while mist lasts".into(),"Leave other targets and species unchanged".into()],unsupported:vec![]
    }).unwrap()));
    for attempt in 0..2 {
        let raw = request_json(
            url,
            messages
                .iter()
                .map(|m| ChatMessage {
                    role: m.role,
                    content: m.content.clone(),
                })
                .collect(),
            schema(),
        )?;
        let result = serde_json::from_value::<Plan>(raw.clone())
            .map_err(|e| e.to_string())
            .and_then(|p| {
                p.validate(kind)?;
                Ok(p)
            });
        match result {
            Ok(plan) => return Ok(plan),
            Err(error) if attempt == 0 => {
                messages.push(ChatMessage::assistant(raw.to_string()));
                messages.push(ChatMessage::user(format!(
                    "Correct this interpretation validation error: {error}"
                )));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}
fn review(
    url: &str,
    prompt: &str,
    plan: &Plan,
    code: Option<&str>,
) -> Result<Option<String>, String> {
    let schema = json!({"type":"object","additionalProperties":false,"required":["faithful","issues"],"properties":{"faithful":{"type":"boolean"},"issues":{"type":"array","items":{"type":"string"},"maxItems":6}}});
    let meaning = if plan.effect == "custom" {
        format!(
            "{}: {}. {}",
            if plan.execution == "instant" {
                "Execute once immediately on Run"
            } else {
                "Persistent rule"
            },
            plan.summary,
            plan.requirements.join("; ")
        )
    } else {
        let condition = match plan.condition.as_str() {
            "rain" => "it rains",
            "night" => "it is night",
            "day" => "it is daytime",
            "always" => "the rule is enabled",
            "storm" => "there is a storm",
            "mist" => "there is mist",
            _ => "the weather is sunny",
        };
        format!(
            "While {condition}, {} {}. Once that condition ends, this protection ends.",
            plan.actor_kind,
            if plan.effect == "protect_players" {
                "cannot attack or hurt players"
            } else {
                "cannot attack any target"
            }
        )
    };
    let mut messages=vec![ChatMessage::system(
        "Judge whether the proposed behavior satisfies the request. The game reviews every generated spell before the user clicks Run. One-time requests such as heal me mean heal the caster immediately on Run; on_cast is the correct implementation, not an unwanted delay. Return faithful=true when it does. Report ONLY concrete discrepancies, not hypothetical concerns. A temporary effect ends with its condition. Starting day means changing to daytime once. In this game creatures damage players through melee attacks; preventing those attacks protects players from them. Use the original requested scope; don't protect unrelated actors. Do not invent extra requirements.".into()),
        ChatMessage::user("Request: mist keeps adventurers safe from goblins. Proposed: While there is mist, goblins cannot attack or hurt players. Protection ends with mist.".into()),
        ChatMessage::assistant("{\"faithful\":true,\"issues\":[]}".into()),
        ChatMessage::user("Request: protect players from goblins during mist. Proposed: Goblins cannot attack players at night during mist.".into()),
        ChatMessage::assistant("{\"faithful\":false,\"issues\":[\"Nighttime was not requested.\"]}".into()),
        ChatMessage::user("Request: mist keeps adventurers safe from goblins. Proposed: During mist, goblins cannot attack any target, including other creatures.".into()),
        ChatMessage::assistant("{\"faithful\":false,\"issues\":[\"Protect players only (effect=protect_players). Suppressing attacks on other creatures adds unrequested protection.\"]}".into()),
        ChatMessage::user(format!("Request: {prompt}\nProposed: {meaning}"))];
    if let Some(code) = code {
        messages.push(ChatMessage::user(format!(
            "Also check this Lua implements it; report actual coding errors only.\n{code}"
        )));
    }
    let result = request_json(url, messages, schema)?;
    let faithful = result["faithful"]
        .as_bool()
        .ok_or("Invalid review response")?;
    let issues = result["issues"]
        .as_array()
        .ok_or("Invalid review issues")?
        .iter()
        .map(|s| s.as_str().unwrap_or("Invalid issue"))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(if faithful && issues.is_empty() {
        None
    } else {
        Some(if issues.is_empty() {
            "Interpretation/code did not pass fidelity review".into()
        } else {
            issues
        })
    })
}
fn focused_prompt(plan: &Plan) -> String {
    // Keep the complete contract but retrieve only the requested capability documentation.
    let full = LlmClient::system_prompt();
    let mut compact = String::new();
    for line in super::WORLD_API_COMPACT.lines() {
        let method = line.split('(').next().unwrap_or(line);
        let relevant = !line.starts_with("- api.")
            || method == "- api.broadcast"
            || method == "- api.players"
            || plan.api_groups.iter().any(|g| match g.as_str() {
                "creatures" => [
                    "creature",
                    "fish",
                    "chase",
                    "attack",
                    "aggress",
                    "behavior",
                    "protect",
                    "api.damage",
                    "api.destroy",
                    "api.die",
                ]
                .iter()
                .any(|k| method.contains(k)),
                "players" => method.contains("player") || method.contains("poison"),
                "inventory" => ["inventory", "resource", "item"]
                    .iter()
                    .any(|k| method.contains(k)),
                "weather" => ["weather", "rain"].iter().any(|k| method.contains(k)),
                "time" => ["time", "night", "dawn"].iter().any(|k| method.contains(k)),
                "blocks" => ["block", "terrain", "distance", "campfire", "water", "fish"]
                    .iter()
                    .any(|k| method.contains(k)),
                _ => false,
            });
        if relevant {
            compact.push_str(line);
            compact.push('\n');
        }
    }
    // Unrelated long examples encouraged accidental copying of their night/radius conditions.
    full.split("EXAMPLE MODULES")
        .next()
        .unwrap_or(&full)
        .replace(super::WORLD_API_COMPACT, &compact)
}
fn resolve_target_scope(url: &str, prompt: &str) -> Result<String, String> {
    let schema = json!({"type":"object","additionalProperties":false,"required":["targets"],"properties":{"targets":{"type":"string","enum":["players","all","custom"]}}});
    let result=request_json(url,vec![
        ChatMessage::system("Extract ONLY explicitly named victims in the request. Do not assume attacks target players when no victim is named. No named victims means targets=all. Return targets=players if the named victims/beneficiaries are players, adventurers or people. Return targets=all if ALL attacks by a creature are forbidden without specifying victims (anyone/everyone/no attacks). Return targets=custom for selected players, specific creature victims or more complex scope. Do not choose all merely because the attacker is named. 'Cannot hurt players' protects players, not other creatures. This is victim/beneficiary extraction, not code generation.".into()),
        ChatMessage::user("A stone golem cannot attack during a storm".into()),
        ChatMessage::assistant("{\"targets\":\"all\"}".into()),
        ChatMessage::user("A stone golem cannot hurt adventurers during a storm".into()),
        ChatMessage::assistant("{\"targets\":\"players\"}".into()),
        ChatMessage::user(prompt.into())],schema)?;
    let targets = result["targets"]
        .as_str()
        .ok_or("Invalid target-scope response")?;
    if !["players", "all", "custom"].contains(&targets) {
        return Err("Unknown target scope".into());
    }
    Ok(targets.into())
}

#[cfg(test)]
pub fn generate(
    url: &str,
    prompt: &str,
    kind: PromptKind,
    correction: Option<(&str, &str)>,
) -> Result<String, String> {
    generate_prepared(url, prompt, kind, correction, prepare(url, prompt, kind)?)
}

pub(super) fn prepare(url: &str, prompt: &str, kind: PromptKind) -> Result<Plan, String> {
    let mut plan = interpreted(url, prompt, kind, None)?;
    // A model-only critic of abstract plans produced false rejections in evaluation.
    // Validate structure here, then use executable checks and review actual custom Lua.
    if plan.effect != "custom" {
        // Separately extract beneficiaries: the small coder model tends to conflate
        // stopping harm to players with globally pacifying the attacking species.
        let resolved = resolve_target_scope(url, prompt)?;
        // Never broaden a player-protection interpretation into pacification of
        // unrelated creature combat. The narrower supported scope is reviewable.
        plan.targets = if resolved == "custom" {
            resolved
        } else if plan.targets == "players" || resolved == "players" {
            "players".into()
        } else {
            "all".into()
        };
        plan.effect = match plan.targets.as_str() {
            "players" => "protect_players",
            "all" => "suppress_attacks",
            _ => "custom",
        }
        .into();
        plan.validate(kind)?;
    }
    Ok(plan)
}

pub(super) fn generate_prepared(
    url: &str, prompt: &str, kind: PromptKind,
    correction: Option<(&str, &str)>, plan: Plan,
) -> Result<String, String> {
    if let Some(code) = plan.compile() {
        crate::world_api_validate::validate_source(&code)
            .first()
            .map_or(Ok(()), |i| Err(i.message.clone()))?;
        verify_policy(&plan, &code)?;
        return Ok(plan.annotate(&code, "typed capability; scenario checks passed"));
    }
    let mut messages=vec![ChatMessage::system(focused_prompt(&plan)),ChatMessage::user(format!("{}\nValidated interpretation: {}\nImplement all requirements. Do not invent constraints.",LlmClient::build_user_turn(prompt,kind),serde_json::to_string(&plan).unwrap()))];
    if let Some((code, error)) = correction {
        messages.push(ChatMessage::assistant(code.into()));
        messages.push(ChatMessage::user(format!(
            "Correct validation failure: {error}"
        )));
    }
    for attempt in 0..2 {
        let code = request_completion(
            url,
            messages
                .iter()
                .map(|m| ChatMessage {
                    role: m.role,
                    content: m.content.clone(),
                })
                .collect(),
        )?;
        let lint = crate::world_api_validate::validate_source(&code);
        let issue = if !lint.is_empty() {
            Some(
                lint.iter()
                    .map(|i| i.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        } else if let Err(error) = smoke_code(&code, kind) {
            Some(error)
        } else {
            review(url, prompt, &plan, Some(&code))?
        };
        match issue {
            None => {
                return Ok(plan.annotate(
                    &code,
                    "runtime-smoke-tested and model-reviewed Lua; review before enabling",
                ))
            }
            Some(error) if attempt == 0 => {
                messages.push(ChatMessage::assistant(code));
                messages.push(ChatMessage::user(format!(
                    "Repair these concrete failures: {error}"
                )));
            }
            Some(error) => return Err(format!("Behavior review failed: {error}")),
        }
    }
    unreachable!()
}

/// Exercise general Lua in isolated fixtures; catches runtime/API mistakes, not arbitrary semantic mismatches.
pub(crate) fn validate_candidate(code: &str, kind: PromptKind) -> Result<(), String> {
    let issues = crate::world_api_validate::validate_source(code);
    if !issues.is_empty() {
        return Err(issues.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
    }
    smoke_code(code, kind)
}

fn smoke_code(code: &str, kind: PromptKind) -> Result<(), String> {
    use crate::{
        creature::{CreatureKind, Creatures},
        scripting::{Module, PlayerSnapshot, ScriptHost},
        voxel::{World, COLLECTIBLE_BLOCKS},
        weather::{Weather, WeatherState},
    };
    use glam::Vec3;
    let module = Module::load("generation_smoke".into(), String::new(), code.into())?;
    if module.is_instant != (kind == PromptKind::Instant) {
        return Err(
            "Wrong callback: use on_cast for instant actions and on_tick for persistent rules"
                .into(),
        );
    }
    let mut world = World::new(1);
    // Placement APIs query loaded, edited blocks, not procedural height guesses.
    let mut chunk = crate::voxel::chunk::Chunk::new(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set_local(x, 24, z, crate::voxel::BlockType::Stone);
        }
    }
    world.chunks.insert((0, 0), chunk);
    let pos = Vec3::new(8.5, 25.0, 8.5);
    let mut creatures = Creatures::new();
    creatures.spawn_one(CreatureKind::Wolf, pos, 1);
    creatures.spawn_one(CreatureKind::Sheep, pos + Vec3::X, 2);
    let mut host = ScriptHost::new();
    host.modules.push(module);
    host.modules[0].enabled = true;
    for (weather_name, mut time) in [
        ("sunny", 0.25),
        ("rain", 0.25),
        ("rain", 0.75),
        ("sunny", 0.75),
    ] {
        let players = [0, 7].map(|id| PlayerSnapshot {
            id,
            pos,
            resources: [1; COLLECTIBLE_BLOCKS.len()],
            carrying_crystal: true,
            velocity: Vec3::ZERO,
            on_ground: true,
            sprinting: false,
            in_water: false,
            health: 10.0,
            poisoned: false,
            speed_multiplier: 1.0,
            jump_multiplier: 1.0,
            oxygen: 100.0,
        });
        let mut weather = WeatherState::new(1);
        weather.set(Weather::from_name(weather_name).unwrap());
        let out = if kind == PromptKind::Instant {
            host.run_cast(
                0,
                &world,
                &mut creatures,
                &players,
                &mut time,
                &mut weather,
                0,
                [1; COLLECTIBLE_BLOCKS.len()],
            )
        } else {
            host.run_tick(
                &world,
                &mut creatures,
                &players,
                &mut time,
                &mut weather,
                &[],
                &[],
                [1; COLLECTIBLE_BLOCKS.len()],
            )
        };
        if !out.crashes.is_empty() {
            return Err(format!(
                "Isolated {weather_name} scenario failed: {}",
                out.crashes.join("; ")
            ));
        }
        if kind == PromptKind::Instant {
            break;
        }
    }
    Ok(())
}

/// Run generated policies in an isolated authoritative simulation before review.
/// Each weather/day-night combination is evaluated, followed by condition exit.
pub fn verify_policy(plan: &Plan, code: &str) -> Result<(), String> {
    use crate::{
        creature::{AttackPolicy, CreatureKind, Creatures},
        scripting::{Module, PlayerSnapshot, ScriptHost},
        voxel::{World, COLLECTIBLE_BLOCKS},
        weather::{Weather, WeatherState},
    };
    use glam::Vec3;
    let species = match plan.actor_kind.as_str() {
        "sheep" => 0,
        "chicken" => 1,
        "stone_golem" => 2,
        "wolf" => 3,
        "stinger" => 4,
        "cow" => 5,
        "goblin" => 6,
        "sunscorch" => 7,
        "zombie" => 8,
        "skeleton" => 9,
        "dragon_green" => 10,
        "dragon_red" => 11,
        "fish" => 12,
        _ => return Err("Unknown policy species".into()),
    };
    let world = World::new(1);
    let pos = Vec3::new(0.5, world.terrain_height(0, 0) as f32 + 1.0, 0.5);
    let mut creatures = Creatures::new();
    let id = creatures.spawn_one(CreatureKind::from_u8(species), pos, 1);
    let players = [0, 7].map(|id| PlayerSnapshot {
        id,
        pos,
        resources: [0; COLLECTIBLE_BLOCKS.len()],
        carrying_crystal: false,
        velocity: Vec3::ZERO,
        on_ground: true,
        sprinting: false,
        in_water: false,
        health: 20.0,
        poisoned: false,
        speed_multiplier: 1.0,
        jump_multiplier: 1.0,
        oxygen: 100.0,
    });
    let mut host = ScriptHost::new();
    let mut module = Module::load("policy_scenario".into(), String::new(), code.into())?;
    module.enabled = true;
    host.modules.push(module);
    for weather_name in ["rain", "sunny", "rain", "storm", "mist"] {
        for mut time in [0.25, 0.75] {
            let mut weather = WeatherState::new(1);
            weather.set(Weather::from_name(weather_name).unwrap());
            let previous_time = time;
            let previous_creatures = creatures.snapshot_with_ids();
            let previous_behavior = serde_json::to_string(&creatures.behaviors).unwrap();
            let out = host.run_tick(
                &world,
                &mut creatures,
                &players,
                &mut time,
                &mut weather,
                &[],
                &[],
                [0; COLLECTIBLE_BLOCKS.len()],
            );
            if !out.crashes.is_empty() {
                return Err(format!("Scenario callback failed: {:?}", out.crashes));
            }
            if time != previous_time
                || weather.current.name() != weather_name
                || !out.block_edits.is_empty()
                || !out.player_effects.is_empty()
                || !out.broadcasts.is_empty()
                || creatures.snapshot_with_ids() != previous_creatures
                || serde_json::to_string(&creatures.behaviors).unwrap() != previous_behavior
            {
                return Err(
                    "Protection scenario changed unrelated world/player/creature state".into(),
                );
            }
            let active = match plan.condition.as_str() {
                "always" => true,
                "night" => crate::daynight::is_night(time),
                "day" => !crate::daynight::is_night(time),
                v => v == weather_name,
            };
            let policies = creatures
                .attack_policies
                .values()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            let expected = if !active {
                vec![]
            } else if plan.effect == "protect_players" {
                vec![
                    AttackPolicy::ProtectPlayer(0, species),
                    AttackPolicy::ProtectPlayer(7, species),
                ]
            } else {
                vec![AttackPolicy::SuppressCreature(id)]
            };
            if policies.len() != expected.len() || expected.iter().any(|p| !policies.contains(p)) {
                return Err(format!("Scenario mismatch at {weather_name}, time {time}"));
            }
        }
    }
    host.toggle_at(0);
    host.sync_attack_policies(&mut creatures);
    if !creatures.attack_policies.is_empty() {
        return Err("Policy survived disable".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focused_block_context_keeps_campfire_placement_and_water_capabilities() {
        let mut p=plan();p.api_groups=vec!["blocks".into()];
        let text=focused_prompt(&p);
        for method in ["place_campfire_near_player", "place_campfire", "get_campfire", "get_water", "get_waterfalls"] {
            assert!(text.contains(&format!("- api.{method}(")),"missing {method}");
        }
        assert!(text.contains("- api.broadcast("));
        p.api_groups=vec!["creatures".into()];
        assert!(focused_prompt(&p).contains("- api.spawn_fish("));
    }
    fn plan() -> Plan {
        Plan {
            execution: "rule".into(),
            summary: "Rain prevents wolf attacks on players; protection ends with rain.".into(),
            condition: "rain".into(),
            actor_kind: "wolf".into(),
            effect: "protect_players".into(),
            targets: "players".into(),
            api_groups: vec!["creatures".into(), "players".into(), "weather".into()],
            requirements: vec!["Protect players during rain only".into()],
            unsupported: vec![],
        }
    }
    #[test]
    fn typed_policy_compiles_and_checks_both_sides_of_conditions() {
        for condition in ["always", "rain", "sunny", "storm", "mist", "night", "day"] {
            for effect in ["protect_players", "suppress_attacks"] {
                let mut p = plan();
                p.condition = condition.into();
                p.effect = effect.into();
                p.targets = if effect == "protect_players" {
                    "players"
                } else {
                    "all"
                }
                .into();
                p.validate(PromptKind::Rule).unwrap();
                verify_policy(&p, &p.compile().unwrap()).unwrap();
            }
        }
    }
    #[test]
    fn validation_rejects_contradictory_or_unknown_plans_and_bad_lua() {
        let mut p = plan();
        p.execution = "instant".into();
        assert!(p.validate(PromptKind::Rule).is_err());
        p.execution = "rule".into();
        p.condition = "rainy".into();
        assert!(p.validate(PromptKind::Rule).is_err());
        p = plan();
        p.actor_kind = "wolf\"; api.die(1)".into();
        assert!(p.validate(PromptKind::Rule).is_err());
        assert!(verify_policy(&plan(), "function on_tick(api) end").is_err());
    }
    #[test]
    fn custom_smoke_checks_callback_and_runtime_contract() {
        assert!(smoke_code("function on_tick(api) end", PromptKind::Instant).is_err());
        assert!(smoke_code(
            "function on_cast(api,e) api.protect_player(e.player_id,'wolf') end",
            PromptKind::Instant
        )
        .is_err());
        assert!(smoke_code(
            "function on_cast(api,e) api.give_item(e.player_id,'iron',3) end",
            PromptKind::Instant
        )
        .is_ok());
    }

    #[test]
    fn campfire_generation_reports_coordinate_mistakes_and_checks_real_placement() {
        for arguments in ["event.player_id", "event.x, event.y, event.z", "{}"] {
            let code = format!("function on_cast(api,event) api.nearest_player({arguments}) end");
            let error = smoke_code(&code, PromptKind::Instant).unwrap_err();
            assert!(error.contains("nearest_player(x, y, z)"), "{error}");
            assert!(error.contains("api.place_campfire_near_player(event.player_id, 6)"), "{error}");
        }
        smoke_code(r#"
            function on_cast(api,event)
                local fire = api.place_campfire_near_player(event.player_id,6)
                assert(fire and fire.burning)
                assert(api.get_block(fire.x,fire.y,fire.z)=='campfire')
                local p = api.nearest_player(fire.x,fire.y,fire.z)
                assert(p and p.distance >= 0)
            end
        "#, PromptKind::Instant).unwrap();
        let mut p = plan();
        p.api_groups = vec!["blocks".into()];
        let context = focused_prompt(&p);
        assert!(context.contains("- api.players()"));
        assert!(context.contains("function on_cast(api, event) local fire = api.place_campfire_near_player"));
    }

    #[test]
    fn focused_docs_do_not_include_unrelated_inventory_actions() {
        let p = plan();
        let prompt = focused_prompt(&p);
        assert!(prompt.contains("- api.protect_player("));
        assert!(!prompt.contains("- api.give_item("));
    }
    #[test]
    #[ignore = "live preflight timing and semantic checks; requires local llama-server"]
    fn profile_preflight() {
        let cases = [
            ("players are protected during rain from wolf attack", "protect_players", "rain", "players"),
            ("if it rains then wolf doesnt attack", "suppress_attacks", "rain", "all"),
            ("start day", "custom", "always", "custom"),
            ("At night sheep hunt players carrying a crystal", "custom", "night", "custom"),
        ];
        for (prompt, effect, condition, targets) in cases {
            let start = std::time::Instant::now();
            let p = interpreted("http://127.0.0.1:8090/v1/chat/completions", prompt, super::super::classify_prompt(prompt), None).unwrap();
            let interpretation = start.elapsed().as_secs_f32();
            let scope = if p.effect != "custom" { Some(resolve_target_scope("http://127.0.0.1:8090/v1/chat/completions", prompt).unwrap()) } else { None };
            println!("{prompt}: interpretation={interpretation:.2}s total={:.2}s plan={}", start.elapsed().as_secs_f32(), serde_json::to_string(&p).unwrap());
            assert_eq!(p.effect, effect);
            if effect != "custom" {
                assert_eq!(p.condition, condition);
                assert_eq!(scope.as_deref(), Some(targets));
            }
        }
    }

    #[test]
    #[ignore = "32-case live model evaluation; writes results and reports failures without activating rules"]
    fn live_intent_evaluation_suite() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../data/intent_eval.json")).unwrap();
        let mut results = Vec::new();
        for case in cases {
            let prompt = case["prompt"].as_str().unwrap();
            let start = std::time::Instant::now();
            let kind = super::super::classify_prompt(prompt);
            let generated = generate(
                "http://127.0.0.1:8090/v1/chat/completions",
                prompt,
                kind,
                None,
            );
            let result = generated.and_then(|code| {
                crate::scripting::Module::load("evaluation".into(), prompt.into(), code.clone())?;
                if case["effect"] != "custom" {
                    let mut p = plan();
                    p.condition = case["condition"].as_str().unwrap().into();
                    p.actor_kind = case["actor_kind"].as_str().unwrap().into();
                    p.effect = case["effect"].as_str().unwrap().into();
                    verify_policy(&p, &code)
                        .map_err(|error| format!("{error}; candidate: {code}"))?;
                }
                Ok(code)
            });
            println!(
                "{} {} ({:.1}s)",
                if result.is_ok() { "PASS" } else { "FAIL" },
                prompt,
                start.elapsed().as_secs_f32()
            );
            results.push(json!({"prompt":prompt,"seconds":start.elapsed().as_secs_f32(),"code":result.as_ref().ok(),"error":result.as_ref().err(),"scenario_checked":case["effect"]!="custom"}));
            std::fs::write(
                "target/intent-evaluation.json",
                serde_json::to_vec_pretty(&results).unwrap(),
            )
            .unwrap();
        }
        println!(
            "Accepted/scenario-checked as applicable: {}/{}",
            results.iter().filter(|r| r["error"].is_null()).count(),
            results.len()
        );
    }

    #[test]
    #[ignore = "requires the existing local llama-server; saves live pipeline outputs"]
    fn live_natural_language_pipeline() {
        let prompts = [
            "if it rains then wolf doesnt attack",
            "players are protected during rain from wolf attack",
            "rain makes wolves leave adventurers alone",
            "During rain wolves cannot hurt players",
            "start day",
            "give me 3 iron in my inventory",
            "heal me",
        ];
        let mut results = Vec::new();
        for prompt in prompts {
            if std::env::var("VOXEL_INTENT_EVAL_FILTER").ok().as_deref() == Some("policies")
                && super::super::classify_prompt(prompt) != PromptKind::Rule
            {
                continue;
            }
            let start = std::time::Instant::now();
            let result = generate(
                "http://127.0.0.1:8090/v1/chat/completions",
                prompt,
                super::super::classify_prompt(prompt),
                None,
            );
            let result = result.and_then(|code| {
                let cases: Vec<Value> =
                    serde_json::from_str(include_str!("../data/intent_eval.json")).unwrap();
                if let Some(case) = cases
                    .iter()
                    .find(|c| c["prompt"].as_str().unwrap().eq_ignore_ascii_case(prompt))
                {
                    if case["effect"] != "custom" {
                        let mut p = plan();
                        p.effect = case["effect"].as_str().unwrap().into();
                        p.condition = case["condition"].as_str().unwrap().into();
                        p.actor_kind = case["actor_kind"].as_str().unwrap().into();
                        verify_policy(&p, &code)
                            .map_err(|error| format!("{error}; candidate: {code}"))?;
                    }
                }
                Ok(code)
            });
            println!(
                "{prompt}: {} ({:.1}s)",
                if result.is_ok() { "PASS" } else { "FAIL" },
                start.elapsed().as_secs_f32()
            );
            results.push(json!({"prompt":prompt,"seconds":start.elapsed().as_secs_f32(),"code":result.as_ref().ok(),"error":result.as_ref().err()}));
            std::fs::write(
                "target/intent-pipeline-live.json",
                serde_json::to_vec_pretty(&results).unwrap(),
            )
            .unwrap();
        }
        assert!(results.iter().all(|r| r["error"].is_null()), "{results:#?}");
    }
}
