//! All world operations run on App's thread. Provider workers only return JSON.
use super::*;
use crate::coplay::{self, Panel, Run};
use serde_json::{json, Value};

impl App {
    fn coplay_snapshot(&self, panel: &Panel, args: &Value) -> Value {
        let mut center = self.player.position;
        if let Some(v) = args["center"].as_array().filter(|v| v.len() == 3) {
            let p = Vec3::new(
                v[0].as_f64().unwrap_or(0.) as f32,
                v[1].as_f64().unwrap_or(0.) as f32,
                v[2].as_f64().unwrap_or(0.) as f32,
            );
            if p.is_finite() && p.abs().max_element() < 1_000_000. {
                center = p;
            }
        }
        let radius = args["radius"].as_i64().unwrap_or(2).clamp(1, 8) as i32;
        let (x, y, z) = (
            center.x.floor() as i32,
            center.y.floor() as i32,
            center.z.floor() as i32,
        );
        let mut blocks = Vec::new();
        for bx in x - radius..=x + radius {
            for by in y - radius..=y + radius {
                for bz in z - radius..=z + radius {
                    let b = self.world.get_block(bx, by, bz);
                    if b != BlockType::Air && blocks.len() < 512 {
                        blocks.push(json!({"position":[bx,by,bz],"kind":b.id()}));
                    }
                }
            }
        }
        let creatures: Vec<_> = self
            .creatures
            .snapshot_with_ids()
            .into_iter()
            .filter(|c| Vec3::from_array(c.2).distance(center) <= 32.)
            .take(64)
            .collect();
        let properties: Vec<_> = creatures.iter().map(|c| json!({"id":c.0,
            "behavior":self.creatures.behaviors.get(&c.0),"statuses":self.creatures.magic_statuses.get(&c.0)})).collect();
        let devices: Vec<_> = self
            .world
            .automation
            .devices
            .values()
            .filter(|d| {
                Vec3::new(d.cell.0 as f32, d.cell.1 as f32, d.cell.2 as f32).distance(center) <= 32.
            })
            .take(32)
            .collect();
        let resources: std::collections::BTreeMap<_, _> = crate::voxel::block::COLLECTIBLE_BLOCKS
            .iter()
            .zip(self.player.resources_snapshot())
            .map(|(b, count)| (b.id(), count))
            .collect();
        json!({"world":self.world.identity.id,"name":self.world.name,"revision":self.world.identity.revision,
            "player":{"id":HOST_PLAYER_ID,"position":self.player.position.to_array(),"health":self.player.health,"resources":resources},
            "selection":format!("{:?}",self.player.crafting.hotbar.entry()),
            "time":self.time_of_day,"weather":self.weather,"target":panel.run.as_ref().and_then(|r| r.target.as_ref()),
            "creatures_id_kind_position_health_facing":creatures,"blocks":blocks,"blocks_truncated":blocks.len()==512,
            "creature_properties":properties,"devices":devices,
            "target_availability":panel.run.as_ref().and_then(|r|r.target.as_ref()).map(|t|t.available(&self.world,&self.creatures)),
            "rules":self.scripting.modules.iter().map(|m|json!({"id":m.name,"enabled":m.enabled,"error":m.error,"instant":m.is_instant,"api":m.api_version()})).collect::<Vec<_>>(),
            "recent_events":self.chat_transcript.iter().rev().take(12).collect::<Vec<_>>(),
            "backlog":panel.backlog.items.iter().map(|g|json!({"id":g.id,"key":g.key,"status":g.status,"behavior":g.behavior})).collect::<Vec<_>>()})
    }

    pub(super) fn update_coplay(&mut self) {
        let mut panel = std::mem::take(&mut self.ui.coplay);
        if !panel.loaded {
            panel.loaded = true;
            match coplay::Backlog::load(std::path::Path::new(coplay::BACKLOG_PATH)) {
                Ok(b) => panel.backlog = b,
                Err(e) => panel.backlog_error = Some(e),
            }
            for m in self
                .scripting
                .modules
                .iter()
                .filter(|m| m.name.starts_with("coplay-"))
            {
                if let Some(error) = &m.error {
                    panel
                        .session
                        .conversation
                        .push(format!("Rule {} inactive after load: {error}", m.name));
                }
            }
        }
        let host = matches!(self.net, NetRole::Host(_));
        if !host {
            if panel.run.is_some() {
                panel.stop("Failed: host authority required");
            }
            panel.submit = false;
            panel.undo = false;
            self.ui.coplay = panel;
            return;
        }
        if panel.open {
            panel.context = format!(
                "{} · revision {} · aimed {}",
                self.world.name,
                self.world.identity.revision,
                self.aimed_spell_context()
                    .map(|t| format!("{:?}", t.target))
                    .unwrap_or_else(|| "nothing".into())
            );
        }
        if panel.cancel || (!panel.enabled && panel.run.is_some()) {
            panel.cancel = false;
            panel.stop("Cancelled");
        }
        if panel.undo && panel.run.is_none() {
            panel.undo = false;
            panel.status = self
                .coplay_undo(&mut panel)
                .unwrap_or_else(|e| format!("Undo blocked: {e}"));
            panel.session.conversation.push(panel.status.clone());
        }
        if panel.submit && panel.run.is_none() && panel.enabled {
            panel.submit = false;
            let prompt = std::mem::take(&mut panel.input);
            let id = format!(
                "request-{}-{}",
                coplay::stamp(),
                panel.session.conversation.len()
            );
            let target = self
                .aimed_spell_context()
                .map(|t| {
                    crate::enchantment::Reference::capture(
                        &mut self.world,
                        &self.creatures,
                        t.target,
                    )
                })
                .transpose();
            match target {
                Err(e) => panel.status = format!("Failed to capture target: {e}"),
                Ok(target) => {
                    panel.session.last_target = target.clone();
                    panel.session.conversation.push(format!("You: {prompt}"));
                    let history = panel
                        .session
                        .conversation
                        .iter()
                        .rev()
                        .take(16)
                        .cloned()
                        .collect::<Vec<_>>();
                    let initial = json!({"request":prompt,"world":self.world.identity.id,"revision":self.world.identity.revision,"position":self.player.position.to_array(),"target":target,"history_newest_first":history,
                        "recent_rule_ids":panel.session.changes.iter().rev().take(12).map(|c| &c.id).collect::<Vec<_>>()});
                    panel.run = Some(Run {
                        id,
                        prompt,
                        started: Instant::now(),
                        steps: 0,
                        errors: 0,
                        tokens: 0,
                        input: vec![json!({"role":"user","content":initial.to_string()})],
                        pending: None,
                        limits: self.ui.settings.values.coplay.clone(),
                        evidence: Vec::new(),
                        discovered: false,
                        inspected: false,
                        mutated: false,
                        checked: false,
                        verified_checks: 0,
                        gap_ids: Vec::new(),
                        target,
                        world: self.world.identity.id.clone(),
                    });
                    panel.status = "Inspecting".into();
                }
            }
        }
        if panel.run.as_ref().is_some_and(Run::exhausted) {
            panel.stop("Stopped: request budget reached");
        }
        let response = panel
            .run
            .as_mut()
            .and_then(|r| r.pending.as_ref())
            .and_then(|rx| match rx.try_recv() {
                Ok(v) => Some(v),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Provider worker stopped".into()))
                }
                Err(_) => None,
            });
        if let Some(response) = response {
            if let Some(r) = &mut panel.run {
                r.pending = None;
            }
            match response {
                Err(e) => panel.stop(&format!("Failed: {e}")),
                Ok(reply) => {
                    let r = panel.run.as_mut().unwrap();
                    r.tokens = r.tokens.saturating_add(reply.tokens);
                    if r.tokens > r.limits.tokens {
                        panel.stop("Stopped: token budget reached; response actions discarded");
                    } else {
                        let calls = reply
                            .output
                            .iter()
                            .filter(|v| v["type"] == "function_call")
                            .cloned()
                            .collect::<Vec<_>>();
                        if calls.len() != 1 {
                            panel.stop("Failed: expected one tool call; no actions executed");
                        } else {
                            panel.run.as_mut().unwrap().input.extend(reply.output);
                            let call = &calls[0];
                            let decoded = serde_json::from_str::<Value>(
                                call["arguments"].as_str().unwrap_or(""),
                            );
                            let (op, args) = decoded
                                .ok()
                                .map(|v| {
                                    (
                                        v["operation"].as_str().unwrap_or("").to_owned(),
                                        serde_json::from_str::<Value>(
                                            v["arguments"].as_str().unwrap_or(""),
                                        )
                                        .unwrap_or(Value::Null),
                                    )
                                })
                                .unwrap_or_default();
                            let result = if call["name"] != "coplay" {
                                Err("Unknown tool".into())
                            } else {
                                self.coplay_tool(&mut panel, &op, &args)
                            };
                            if let Some(r) = &mut panel.run {
                                r.steps += 1;
                                let result = match result {
                                    Ok(v) => v,
                                    Err(e) => {
                                        r.errors += 1;
                                        panel.status = "Correcting".into();
                                        json!({"ok":false,"error":e,"committed":false})
                                    }
                                };
                                r.evidence
                                    .push(json!({"step":r.steps,"operation":op,"result":result}));
                                r.input.push(json!({"type":"function_call_output","call_id":call["call_id"],"output":result.to_string()}));
                            }
                        }
                    }
                }
            }
        }
        if panel.run.as_ref().is_some_and(Run::exhausted) {
            panel.stop("Stopped: request budget reached");
        }
        if panel.run.as_ref().is_some_and(|r| r.pending.is_none()) {
            let cfg = &self.ui.settings.values.aiapi;
            let key = if cfg.api_key.trim().is_empty() {
                std::env::var("OPENAI_API_KEY").unwrap_or_default()
            } else {
                cfg.api_key.clone()
            };
            let model = if cfg.model.trim().is_empty() {
                std::env::var("OPENAI_PLAYTEST_MODEL").unwrap_or_default()
            } else {
                cfg.model.clone()
            };
            if key.is_empty() || model.is_empty() {
                panel.stop("Failed: configure aiapi.OPENAI_API_KEY and OPENAI_PLAYTEST_MODEL in settings.json");
            } else {
                let r = panel.run.as_mut().unwrap();
                let body = coplay::request(&model, r);
                // Reserve a conservative upper bound for input plus output before making a paid call.
                if (body.to_string().len() as u64).saturating_add(r.limits.output_tokens as u64)
                    > r.limits.tokens.saturating_sub(r.tokens)
                {
                    panel.stop("Stopped: insufficient remaining model budget");
                } else {
                    let (tx, rx) = std::sync::mpsc::channel();
                    r.pending = Some(rx);
                    std::thread::spawn(move || {
                        let _ = tx.send(coplay::fetch(&key, body));
                    });
                }
            }
        }
        self.ui.coplay = panel;
    }

    fn coplay_tool(&mut self, p: &mut Panel, op: &str, a: &Value) -> Result<Value, String> {
        let r = p.run.as_ref().ok_or("Request cancelled")?;
        r.authorize(
            matches!(self.net, NetRole::Host(_)),
            p.enabled,
            &self.world.identity.id,
        )?;
        match op {
            "capabilities" => {
                let query = a["query"].as_str().unwrap_or("").to_lowercase();
                let docs = if query.is_empty() {
                    coplay::API_DOC
                        .lines()
                        .filter(|l| !l.starts_with("- api."))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    coplay::API_DOC
                        .lines()
                        .filter(|l| l.to_lowercase().contains(&query))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                p.run.as_mut().unwrap().discovered = true;
                Ok(
                    json!({"api_version":crate::world_api_gen::VERSION,"build":coplay::build(),"methods":crate::world_api_gen::METHOD_NAMES,"docs":docs,"query":query,"authoring":"execute bypasses spell fees only; Lua budgets unchanged; rule globals reset on reload"}),
                )
            }
            "inspect" => {
                let value = self.coplay_snapshot(p, a);
                let r = p.run.as_mut().unwrap();
                r.inspected = true;
                if r.mutated {
                    r.checked = true;
                }
                p.status = "Checking".into();
                Ok(value)
            }
            "verify" => {
                let result = coplay::verify(&self.coplay_snapshot(p, a), a)?;
                if result["passed"] == true {
                    // Metadata and a registered rule are not gameplay evidence.
                    let path = a["path"].as_str().unwrap_or("");
                    if [
                        "/blocks/",
                        "/creatures_id_kind_position_health_facing/",
                        "/player/",
                        "/weather/",
                    ]
                    .iter()
                    .any(|prefix| path.starts_with(prefix))
                    {
                        p.run.as_mut().unwrap().verified_checks += 1;
                    }
                }
                Ok(result)
            }
            "get_rule" => {
                let id = a["id"].as_str().ok_or("Missing rule id")?;
                let m = self
                    .scripting
                    .modules
                    .iter()
                    .find(|m| m.name == id)
                    .ok_or("Rule no longer exists")?;
                Ok(
                    json!({"id":m.name,"source":m.source,"enabled":m.enabled,"error":m.error,"api":m.api_version()}),
                )
            }
            "upsert_rule" | "execute" => {
                if !r.discovered || !r.inspected {
                    return Err("Discover capabilities and inspect first".into());
                }
                let action = a["action_id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() < 128)
                    .ok_or("Missing action_id")?;
                let receipt = format!("{}:{action}", r.id);
                if let Some(result) = p.session.receipt(&receipt, op, a)? {
                    return Ok(result);
                }
                if let Some(t) = &r.target {
                    if !t.available(&self.world, &self.creatures)? {
                        return Err("Target unloaded; wait or clarify".into());
                    }
                }
                let source = a["source"].as_str().ok_or("Missing Lua source")?;
                if source.len() > crate::world_api_gen::SCRIPT_SOURCE_BYTES {
                    return Err("Source limit exceeded".into());
                }
                let issues = world_api_validate::validate_source(source);
                if !issues.is_empty() {
                    return Err(issues
                        .iter()
                        .map(|i| i.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; "));
                }
                if world_api_validate::extract_api_version(source)
                    .is_some_and(|v| v != crate::world_api_gen::VERSION)
                {
                    return Err("API version mismatch".into());
                }
                let source = world_api_validate::tag_with_api_version(source);
                let mut module = Module::load("coplay-action".into(), r.prompt.clone(), source)?;
                p.status = "Applying".into();
                let result = if op == "execute" {
                    let players = self.host_player_positions();
                    let mut outcome = TickOutcome::default();
                    let mut deaths = Vec::new();
                    let before = self.creatures.snapshot_with_ids();
                    let mut input = crate::scripting::TickInput {
                        world: &self.world,
                        creatures: &mut self.creatures,
                        players: &players,
                        time_of_day: &mut self.time_of_day,
                        weather: &mut self.weather,
                        block_edits: &mut outcome.block_edits,
                        death_events: &mut deaths,
                        broadcasts: &mut outcome.broadcasts,
                        player_effects: &mut outcome.player_effects,
                        host_resources: self.player.resources_snapshot(),
                    };
                    self.scripting.author_execute(&mut module, &mut input)?;
                    let after = self.creatures.snapshot_with_ids();
                    let result = json!({"ok":true,"executed":true,"verified_behavior":false,"blocks":outcome.block_edits.iter().map(|(x,y,z,b)|json!([x,y,z,b.id()])).collect::<Vec<_>>(),"created_creatures":after.iter().filter(|c|!before.iter().any(|old|old.0==c.0)).collect::<Vec<_>>(),"removed_creatures":before.iter().filter(|c|!after.iter().any(|new|new.0==c.0)).map(|c|c.0).collect::<Vec<_>>(),"broadcasts":outcome.broadcasts,"player_effect_count":outcome.player_effects.len(),"undo":"Instant effects are not reversible through rule undo"});
                    self.creatures.combat_deaths.extend(deaths);
                    self.apply_tick_outcome(outcome);
                    result
                } else {
                    let id = coplay::install_rule(
                        &mut p.session,
                        &mut self.scripting,
                        &r.id,
                        module,
                        a,
                    )?;
                    self.scripting.sync_attack_policies(&mut self.creatures);
                    json!({"ok":true,"id":id,"registered":true,"verified_behavior":false,"note":"Observe relevant ticks/events before claiming behavior works"})
                };
                p.session.receipts.insert(
                    receipt,
                    json!({"operation":op,"arguments":a,"result":result}),
                );
                let r = p.run.as_mut().unwrap();
                r.mutated = true;
                r.checked = false;
                r.verified_checks = 0;
                p.session
                    .conversation
                    .push(format!("Applied {op}: {result}"));
                Ok(result)
            }
            "report_gap" => {
                if !r.discovered || !r.inspected {
                    return Err("Inspect current API and world before reporting a gap".into());
                }
                if let Some(e) = &p.backlog_error {
                    return Err(e.clone());
                }
                let evidence = json!({"request_id":r.id,"world":r.world,"original_request":r.prompt,"intended_outcome":r.prompt,"api_version":crate::world_api_gen::VERSION,"build":coplay::build(),"report":a,"tools":r.evidence});
                let mut draft = p.backlog.clone();
                let id = draft.report(a, evidence)?;
                coplay::write_json(std::path::Path::new(coplay::BACKLOG_PATH), &draft)?;
                p.backlog = draft;
                p.run.as_mut().unwrap().gap_ids.push(id.clone());
                Ok(json!({"ok":true,"id":id,"persisted":true}))
            }
            "resolve_gap" => {
                if r.verified_checks == 0 {
                    return Err(
                        "Recheck requires a passing verify call against actual gameplay state"
                            .into(),
                    );
                }
                let id = a["id"].as_str().ok_or("Missing id")?;
                let mut draft = p.backlog.clone();
                let item = draft
                    .items
                    .iter_mut()
                    .find(|g| g.id == id && g.status == "needs_recheck")
                    .ok_or("Human must explicitly request recheck using the backlog panel")?;
                let evidence = a["evidence"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("Missing acceptance evidence")?;
                item.status = "resolved".into();
                item.verified_build = Some(coplay::build());
                item.examples.push(json!({"recheck_request":r.id,"build":coplay::build(),"verification":evidence,"tools":r.evidence}));
                coplay::write_json(std::path::Path::new(coplay::BACKLOG_PATH), &draft)?;
                p.backlog = draft;
                Ok(json!({"ok":true,"id":id}))
            }
            "finish" => {
                let status = a["status"].as_str().ok_or("Missing status")?;
                if !["complete", "blocked", "partial", "clarification"].contains(&status) {
                    return Err("Invalid status".into());
                }
                if ["blocked", "partial"].contains(&status) && r.gap_ids.is_empty() {
                    return Err(
                        "Persist unavailable capability evidence with report_gap before finishing"
                            .into(),
                    );
                }
                if r.mutated && !r.checked {
                    return Err(
                        "Inspect the result before finishing; distinguish applied from verified"
                            .into(),
                    );
                }
                let summary = a["summary"].as_str().ok_or("Missing summary")?.to_owned();
                let verification = if r.mutated && r.verified_checks == 0 {
                    " [Applied; gameplay behavior remains unverified.]"
                } else {
                    ""
                };
                p.status = format!("{status}: {summary}{verification}");
                p.session.conversation.push(format!("Astra: {}", p.status));
                p.archive(&format!("{status}: {summary}"));
                Ok(json!({"ok":true}))
            }
            _ => Err("Unknown Co-Play operation".into()),
        }
    }

    fn coplay_undo(&mut self, p: &mut Panel) -> Result<String, String> {
        let message = coplay::undo_rule(&mut p.session, &mut self.scripting)?;
        self.scripting.sync_attack_policies(&mut self.creatures);
        Ok(message)
    }
}
