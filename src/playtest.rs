//! Development-only player control. No model, privileged inventory grants or teleports.
use crate::{
    crafting, equipment,
    input::Input,
    player::Player,
    voxel::{BlockType, World},
};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::File,
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

pub type Cell = (i32, i32, i32);
pub const STEP: f32 = 0.05;
pub const AGENT_ID: u32 = u32::MAX - 1;
#[path = "playtest_ai.rs"]
pub mod ai;
#[path = "playtest_controller.rs"]
pub mod controller;
#[path = "playtest_objectives.rs"]
pub mod objectives;
#[path = "playtest_evidence.rs"]
pub mod evidence;
#[path = "playtest_replay.rs"]
pub mod replay;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Move {
        direction: [f32; 3],
        jump: bool,
    },
    Look {
        direction: [f32; 3],
    },
    Interact {
        target: Cell,
    },
    Collect,
    Equip {
        item: Option<equipment::Entry>,
    },
    Mine {
        target: Cell,
    },
    Place {
        target: Cell,
        #[serde(default)]
        destination: Option<Cell>,
    },
    Craft {
        recipe: crafting::Action,
    },
    Attack,
    Device {
        operation: crate::automation::Action,
    },
    Wait,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Completed,
    Progress,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub tick: u64,
    pub status: Status,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Terrain {
    pub cell: Cell,
    pub block: BlockType,
}
#[derive(Clone, Serialize)]
pub struct DeviceView {
    pub cell: Cell,
    pub kind: crate::automation::Kind,
    pub mana: u32,
    pub items: std::collections::BTreeMap<String, u32>,
    pub output: std::collections::BTreeMap<String, u32>,
    pub config: crate::automation::Config,
    pub config_revision: u64,
}

#[derive(Clone, Serialize)]
pub struct Observation {
    pub tick: u64,
    pub position: [f32; 3],
    pub aim: [f32; 3],
    pub health: f32,
    pub mana: u32,
    pub oxygen: f32,
    pub equipment: Option<equipment::Entry>,
    pub inventory: crafting::Account,
    pub terrain: Vec<Terrain>,
    pub walkable: Vec<Cell>,
    pub creatures: Vec<(u32, u8, [f32; 3])>,
    pub drops: Vec<crate::loot::Drop>,
    pub devices: Vec<(Cell, crate::automation::Kind)>,
    pub inspected_device: Option<DeviceView>,
    pub recipes: Vec<crafting::Recipe>,
    pub gear_recipes: Vec<(equipment::Gear, (u32, u32, u32))>,
    pub gear_materials: Vec<(equipment::Gear, Vec<(BlockType,u32)>)>,
    pub compositions: Vec<crafting::ObjectComposition>,
    pub mana_costs: [u32; 5],
    pub mana_free: bool,
    pub interactions: Vec<&'static str>,
    pub recent_events: Vec<Outcome>,
    pub recent_world_events: Vec<String>,
    pub previous_action: Option<Outcome>,
}

pub struct Context<'a> {
    pub world: &'a mut World,
    pub creatures: &'a mut crate::creature::Creatures,
    pub loot: &'a mut crate::loot::Effects,
    pub registry: &'a crafting::Registry,
    pub players: &'a [Vec3],
}

#[derive(Default)]
pub struct Effects {
    pub edits: Vec<(Cell, BlockType, BlockType)>,
    pub interacts: Vec<(Cell, BlockType)>,
    pub findings: Vec<evidence::Finding>,
}

pub struct Actor {
    pub player: Player,
    pub aim: Vec3,
    pub tick: u64,
    clock: Instant,
    mining: equipment::Mining,
    recent: VecDeque<Outcome>,
    world_events: VecDeque<String>,
    inspected: Option<Cell>,
}

fn center(p: Cell) -> Vec3 {
    Vec3::new(p.0 as f32, p.1 as f32, p.2 as f32) + Vec3::splat(0.5)
}

fn visible(world: &World, eye: Vec3, target: Vec3) -> bool {
    let distance = eye.distance(target);
    distance <= 12.0
        && crate::raycast::raycast(world, eye, target - eye, (distance - 0.2).max(0.0)).is_none()
}

impl Actor {
    pub fn new(position: Vec3) -> Self {
        Self {
            player: Player::new(position),
            aim: Vec3::Z,
            tick: 0,
            clock: Instant::now(),
            mining: Default::default(),
            recent: VecDeque::new(),
            world_events: VecDeque::new(),
            inspected: None,
        }
    }

    pub fn observe(&self, context: &Context<'_>) -> Observation {
        let world = &*context.world;
        let eye = self.player.position + Vec3::Y * 1.62;
        let mut terrain = Vec::new();
        // Bounded visibility sampling, independent of world size. Hidden ore is never exposed.
        for yaw in 0..64 {
            for pitch in -12..=12 {
                let y = yaw as f32 * std::f32::consts::TAU / 64.0;
                let p = pitch as f32 * std::f32::consts::FRAC_PI_2 / 13.0;
                let dir = Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos());
                if let Some(hit) = crate::raycast::raycast(world, eye, dir, 12.0) {
                    terrain.push(Terrain {
                        cell: hit.target,
                        block: world.get_block(hit.target.0, hit.target.1, hit.target.2),
                    });
                }
            }
        }
        terrain.sort_by_key(|t| t.cell);
        terrain.dedup_by_key(|t| t.cell);
        let walkable = terrain
            .iter()
            .filter_map(|t| {
                let (x, y, z) = t.cell;
                (t.block.is_solid()
                    && world.get_block(x, y + 1, z) == BlockType::Air
                    && world.get_block(x, y + 2, z) == BlockType::Air
                    && visible(world, eye, center((x, y + 1, z))))
                .then_some((x, y + 1, z))
            })
            .collect();
        Observation {
            inspected_device: self
                .inspected
                .and_then(|p| world.automation.devices.get(&p))
                .filter(|d| {
                    crate::raycast::raycast(world, eye, center(d.cell) - eye, 7.0)
                        .is_some_and(|h| h.target == d.cell)
                })
                .map(|d| DeviceView {
                    cell: d.cell,
                    kind: d.kind,
                    mana: d.mana,
                    items: d.items.clone(),
                    output: d.output.clone(),
                    config: d.config.clone(),
                    config_revision: d.config_revision,
                }),
            tick: self.tick,
            position: self.player.position.to_array(),
            aim: self.aim.to_array(),
            health: self.player.health,
            mana: self.player.crafting.mana,
            oxygen: self.player.oxygen,
            equipment: self.player.crafting.hotbar.entry(),
            inventory: self.player.crafting.clone(),
            terrain,
            walkable,
            creatures: context
                .creatures
                .snapshot_with_ids()
                .into_iter()
                .filter(|c| visible(world, eye, Vec3::from_array(c.2) + Vec3::Y * 0.6))
                .map(|c| (c.0, c.1, c.2))
                .collect(),
            drops: context
                .loot
                .drops
                .iter()
                .filter(|d| visible(world, eye, d.display_position()))
                .cloned()
                .collect(),
            devices: world
                .automation
                .devices
                .values()
                .filter(|d| {
                    crate::raycast::raycast(world, eye, center(d.cell) - eye, 7.0)
                        .is_some_and(|h| h.target == d.cell)
                })
                .map(|d| (d.cell, d.kind))
                .collect(),
            recipes: context.registry.recipes.clone(),
            gear_recipes: equipment::Gear::ALL
                .into_iter()
                .filter_map(|g| crafting::gear_formula(g, false).ok().map(|r| (g, r)))
                .collect(),
            gear_materials: equipment::Gear::available().map(|g|(g,g.ingredients(false))).collect(),
            compositions: context.registry.compositions.clone(),
            mana_costs: context.registry.mana_costs,
            mana_free: context.registry.mana_free,
            interactions: vec![
                "move", "look", "interact", "collect", "equip", "mine", "place", "craft", "attack", "device",
                "wait",
            ],
            recent_events: self.recent.iter().cloned().collect(),
            previous_action: self.recent.back().cloned(),
            recent_world_events: self.world_events.iter().cloned().collect(),
        }
    }

    pub fn act(&mut self, context: &mut Context<'_>, action: &Action) -> (Outcome, Effects) {
        let before = self.player.crafting.clone();
        let position_before = self.player.position;
        let embedded_before = evidence::embedded(context.world, position_before);
        let mut effects = Effects::default();
        let mut input = Input::new();
        let mut forward = Vec3::new(self.aim.x, 0.0, self.aim.z).normalize_or_zero();
        if forward == Vec3::ZERO {
            forward = Vec3::Z;
        }
        let now = self.clock + Duration::from_millis(self.tick * 50);
        let result: Result<(Status, String), String> = (|| {
            if self.player.health <= 0.0 {
                return Err("Player is dead".into());
            }
            match action {
                Action::Move { direction, jump } => {
                    let d = Vec3::from_array(*direction);
                    if !d.is_finite() || d.y != 0.0 || d.length_squared() > 1.001 {
                        return Err("Move requires a finite horizontal unit vector".into());
                    }
                    if d.length_squared() > 0.0 {
                        forward = d.normalize();
                        self.aim = forward;
                        input.key_event(
                            winit::keyboard::KeyCode::KeyW,
                            winit::event::ElementState::Pressed,
                        );
                    }
                    if *jump {
                        input.key_event(
                            winit::keyboard::KeyCode::Space,
                            winit::event::ElementState::Pressed,
                        );
                    }
                }
                Action::Look { direction } => {
                    let d = Vec3::from_array(*direction);
                    if !d.is_finite() || d.length_squared() < 0.01 || d.length_squared() > 1.001 {
                        return Err("Look requires a finite unit vector".into());
                    }
                    self.aim = d.normalize();
                }
                Action::Equip { item } => {
                    if item.is_some_and(|e| e.count(&self.player.crafting) == 0) {
                        return Err("Item not owned".into());
                    }
                    let mut hotbar = self.player.crafting.hotbar.clone();
                    hotbar.assign(*item);
                    equipment::accept_hotbar(&mut self.player.crafting, &hotbar)?;
                }
                Action::Craft { recipe } => {
                    let revision = self.player.crafting.revision;
                    let text = context.registry.execute(
                        &mut self.player.crafting,
                        revision,
                        recipe,
                        context.world,
                        context.creatures,
                        self.player.position,
                        context.players,
                    )?;
                    return Ok((Status::Completed, text));
                }
                Action::Collect => {
                    let before = self.player.crafting.revision;
                    context.loot.collect(
                        context.world,
                        self.player.position,
                        &mut self.player.crafting,
                    );
                    if before == self.player.crafting.revision {
                        return Err(
                            "No collectible drop in reach (delay, obstruction or capacity)".into(),
                        );
                    }
                }
                Action::Mine { target } | Action::Place { target, .. } => {
                    if let Action::Place {
                        destination: Some(destination),
                        ..
                    } = action
                    {
                        let hit = crate::raycast::raycast(
                            context.world,
                            self.player.position + Vec3::Y * 1.62,
                            self.aim,
                            6.0,
                        )
                        .ok_or("No block in reach")?;
                        if hit.place != *destination {
                            return Err(
                                "Placement face changed; refusing a different destination".into()
                            );
                        }
                    }
                    let intent = equipment::Intent {
                        hotbar: self.player.crafting.hotbar.clone(),
                        item: self.player.crafting.hotbar.entry(),
                        target: Some(*target),
                        direction: self.aim.to_array(),
                        action: if matches!(action, Action::Mine { .. }) {
                            equipment::Action::Mine
                        } else {
                            equipment::Action::Place
                        },
                    };
                    let edit = equipment::block_action_at(
                        context.world,
                        &mut self.player.crafting,
                        &mut self.mining,
                        self.player.position,
                        context.players,
                        &intent,
                        now,
                    )?;
                    if let Some((p, b, old)) = edit {
                        effects.edits.push((p, b, old));
                    } else {
                        return Ok((Status::Progress, "Mining hit accepted".into()));
                    }
                }
                Action::Attack => {
                    let intent = equipment::Intent {
                        hotbar: self.player.crafting.hotbar.clone(),
                        item: self.player.crafting.hotbar.entry(),
                        target: None,
                        direction: self.aim.to_array(),
                        action: equipment::Action::Attack,
                    };
                    equipment::attack_at(
                        context.world,
                        context.creatures,
                        &mut self.player.crafting,
                        &mut self.mining,
                        self.player.position,
                        &intent,
                        now,
                    )?;
                    if intent.item==Some(equipment::Entry::Gear(equipment::Gear::LifeStaff)){self.player.heal(15.);}
                }
                Action::Interact { target } => {
                    let hit = crate::raycast::raycast(
                        context.world,
                        self.player.position + Vec3::Y * 1.62,
                        self.aim,
                        6.0,
                    )
                    .ok_or("No block in reach")?;
                    if hit.target != *target {
                        return Err("Target changed or is obstructed".into());
                    }
                    self.inspected = Some(*target);
                    effects.interacts.push((
                        *target,
                        context.world.get_block(target.0, target.1, target.2),
                    ));
                }
                Action::Device { operation } => {
                    let mut state = context.world.automation.clone();
                    let mut players = context.players.to_vec();
                    players.push(self.player.position);
                    crate::automation::apply(
                        context.world,
                        &mut state,
                        &mut self.player.crafting,
                        self.player.position,
                        &players,
                        operation,
                        crate::automation::balance(),
                        context.registry,
                    )?;
                    context.world.automation = state;
                    self.inspected = Some(operation.cell());
                }
                Action::Wait => {}
            }
            Ok((Status::Completed, "Action accepted".into()))
        })();
        // Observe the transaction before physics and periodic mana regeneration.
        effects.findings = evidence::transaction(
            self.tick + 1, action, &before, &self.player.crafting,
            result.is_err(), &effects, context.registry,
        );
        effects.findings.extend(evidence::support(self.tick + 1, context.world, &effects));
        let right = forward.cross(Vec3::Y);
        self.player
            .update(context.world, &input, forward, right, STEP);
        if !self.player.position.is_finite() || !self.player.velocity.is_finite() {
            effects.findings.push(evidence::Finding::new(self.tick + 1,
                "finite_movement", "Player position and velocity must remain finite",
                format!("Position {:?} -> {:?}, velocity {:?}", position_before,
                    self.player.position, self.player.velocity)));
        } else if !embedded_before && evidence::embedded(context.world, self.player.position) {
            effects.findings.push(evidence::Finding::new(self.tick + 1,
                "terrain_collision", "Movement from clear space must not enter solid terrain",
                format!("Position {:?} -> {:?}", position_before, self.player.position)));
        }
        let feet = self.player.position;
        if context.world.get_block(
            feet.x.floor() as i32,
            feet.y.floor() as i32,
            feet.z.floor() as i32,
        ) == BlockType::Water
        {
            self.player
                .drain_oxygen(crate::player::OXYGEN_DRAIN_PER_SEC * STEP * crate::gear_catalog::oxygen_factor(&self.player.crafting));
            if self.player.oxygen <= 0.0 {
                self.player
                    .damage(crate::player::DROWNING_DAMAGE_PER_SEC * STEP);
            }
        } else {
            self.player
                .regenerate_oxygen(crate::player::OXYGEN_REGEN_PER_SEC * STEP);
        }
        if (self.tick + 1) % 100 == 0 {
            self.player.crafting.regenerate_mana();
        }
        self.player.carrying_crystal = self.player.resource_count(BlockType::Crystal) > 0;
        self.tick += 1;
        let (status, reason) = result.unwrap_or_else(|e| {
            (
                Status::Rejected,
                if e.is_empty() {
                    "Cooldown active".into()
                } else {
                    e
                },
            )
        });
        let outcome = Outcome {
            tick: self.tick,
            status,
            reason,
        };
        if self.recent.len() == 16 {
            self.recent.pop_front();
        }
        self.recent.push_back(outcome.clone());
        (outcome, effects)
    }
}

/// A deliberately short phase-one script. It only chooses targets from observations.
/// Mining awards materials through block_action; decomposition/crafting use Registry::execute.
pub struct GatherCraft {
    target: Option<Terrain>,
    pub finished: Option<String>,
    pub crafted: bool,
    collected: u32,
}
impl Default for GatherCraft {
    fn default() -> Self {
        Self {
            target: None,
            finished: None,
            crafted: false,
            collected: 0,
        }
    }
}
impl GatherCraft {
    pub fn next(&mut self, observation: &Observation) -> Action {
        if self.finished.is_some() {
            return Action::Wait;
        }
        if observation.tick >= 1200 {
            self.finished =
                Some("Controller timeout: gather/craft did not finish in 60 seconds".into());
            return Action::Wait;
        }
        if let Some(previous) = &observation.previous_action {
            if previous.reason.starts_with("Created ") {
                self.crafted = true;
                self.finished = Some(previous.reason.clone());
                return Action::Wait;
            }
        }
        // Prefer a craftable resource; this never supplies missing ingredients.
        for recipe in &observation.recipes {
            if recipe.output.kind == crafting::ObjectKind::Creature {
                continue;
            }
            if crafting::totals(&recipe.inputs)
                .is_ok_and(|cost| observation.inventory.can_afford_elements(cost))
            {
                let mut slots = [None; 5];
                for (slot, input) in slots.iter_mut().zip(&recipe.inputs) {
                    *slot = Some(input.clone());
                }
                let cost = if observation.mana_free {
                    0
                } else {
                    observation.mana_costs[recipe.inputs.len() - 1]
                };
                if observation.mana < cost {
                    return Action::Wait;
                }
                return Action::Craft {
                    recipe: crafting::Action::Craft(slots),
                };
            }
        }
        for (i, &block) in crate::voxel::COLLECTIBLE_BLOCKS.iter().enumerate() {
            if observation.inventory.resources[i] > 0 {
                if !observation.mana_free && observation.mana < 1 {
                    return Action::Wait;
                }
                self.collected += 1;
                return Action::Craft {
                    recipe: crafting::Action::Extract { block, amount: 1 },
                };
            }
        }
        if self.collected > 16 {
            self.finished = Some(
                "Scenario unavailable: gathered elements do not match a resource recipe".into(),
            );
            return Action::Wait;
        }
        let eye = Vec3::from_array(observation.position) + Vec3::Y * 1.62;
        self.target = observation
            .terrain
            .iter()
            .filter(|t| {
                let d = center(t.cell) - eye;
                d.length() < 5.0
                    && d.x.abs() + d.z.abs() > 1.5
                    && !t.block.is_unbreakable()
                    && observation
                        .compositions
                        .iter()
                        .any(|c| c.id == t.block.id() && c.elements != [0; 5])
            })
            .min_by_key(|t| ((center(t.cell) - eye).length_squared() * 100.0) as u32)
            .cloned();
        let Some(target) = &self.target else {
            self.finished = Some("Scenario unavailable: no visible harvestable material in reach; move to exposed terrain and restart".into());
            return Action::Wait;
        };
        let item = Some(equipment::Entry::Gear(target.block.required_tool()));
        if observation.equipment != item {
            return Action::Equip { item };
        }
        // Alternate aim and strikes: cooldown is still enforced by the shared validator.
        let direction = (center(target.cell) - eye).normalize();
        if Vec3::from_array(observation.aim).dot(direction) < 0.9999 || observation.tick % 5 != 0 {
            return Action::Look {
                direction: direction.to_array(),
            };
        }
        Action::Mine {
            target: target.cell,
        }
    }
}

pub struct Session {
    pub actor: Actor,
    pub script: GatherCraft,
    pub directory: PathBuf,
    log: File,
    accumulator: f32,
    scenario: Scenario,
    task: Option<controller::Controller>,
    returning: bool,
    home: Vec3,
    observation: Option<Observation>,
    failures: u32,
    ai: Option<ai::Ai>,
    objective: objectives::Objective,
    findings: Vec<evidence::Finding>,
    last_activity: Option<(u64, &'static str)>,
    pub visual: std::collections::HashMap<u32, crate::remote_player::RemotePlayer>,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenario {
    #[default]
    GatherCraft,
    WalkReturn,
    MineBlock,
    AiGatherTool,
    AiShelter,
    AiEncounterReturn,
}
impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Self::GatherCraft => "Gather and craft",
            Self::WalkReturn => "Walk and return",
            Self::MineBlock => "Approach and mine",
            Self::AiGatherTool => "OpenAI: gather and craft a tool",
            Self::AiShelter => "OpenAI: build a small shelter",
            Self::AiEncounterReturn => "OpenAI: survive and return",
        }
    }
    pub fn is_ai(self) -> bool {
        matches!(
            self,
            Self::AiGatherTool | Self::AiShelter | Self::AiEncounterReturn
        )
    }
}
impl Session {
    pub fn create(
        position: Vec3,
        world: &World,
        snapshot: &[u8],
        registry: &crafting::Registry,
        scenario: Scenario,
    ) -> Result<Self, String> {
        let objective = objectives::Objective::new(scenario, position);
        if scenario == Scenario::AiShelter {
            objective.validate_site(world)?;
        }
        let ai = if scenario.is_ai() {
            let mut ai = ai::Ai::from_configuration()?;
            ai.configure(scenario, position);
            Some(ai)
        } else {
            None
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        static SESSION_SEQUENCE: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let sequence = SESSION_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = PathBuf::from("target/playtests");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let directory = root.join(format!("{stamp}-{}-{sequence}", std::process::id()));
        std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
        let actor = Actor::new(position);
        let header = serde_json::json!({"schema":2,"build_version":env!("CARGO_PKG_VERSION"),
            "executable_fingerprint":build_fingerprint()?,
            "seed":world.seed,"world_name":world.name,"generation":world.generation,
            "controller":"scripted-v2","scenario":scenario,"objective":objective,"ai":ai.as_ref().map(ai::Ai::config),"tick_seconds":STEP,"max_ticks":if ai.is_some(){12000}else{1200},
            "initial_save":"initial.bin","agent_position":position.to_array(),"agent_inventory":actor.player.crafting,
            "agent_state":crate::save::PlayerSave::capture(&actor.player),
            "registry":registry,"scope":"host-local development session; not multiplayer replication coverage"});
        std::fs::write(directory.join("initial.bin"), snapshot).map_err(|e| e.to_string())?;
        std::fs::write(
            directory.join("session.json"),
            serde_json::to_vec_pretty(&header).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let log = File::create(directory.join("events.jsonl")).map_err(|e| e.to_string())?;
        let visual = [(
            AGENT_ID,
            crate::remote_player::RemotePlayer::new(position, 0.0, false, "Agent1".into()),
        )]
        .into_iter()
        .collect();
        Ok(Self {
            actor,
            objective,
            findings: Vec::new(),
            last_activity: None,
            script: Default::default(),
            directory,
            log,
            accumulator: 0.0,
            visual,
            scenario,
            task: None,
            returning: false,
            home: position,
            observation: None,
            failures: 0,
            ai,
        })
    }

    pub fn update(&mut self, dt: f32, context: &mut Context<'_>) -> Result<Effects, String> {
        self.accumulator += dt.clamp(0.0, 0.1);
        if self.accumulator < STEP || self.script.finished.is_some() {
            return Ok(Effects::default());
        }
        self.accumulator -= STEP;
        // Decisions/sensing at 4 Hz; movement and cooldown clocks remain 20 Hz.
        let sensed = self.actor.tick % 5 == 0 || self.observation.is_none();
        if sensed {
            self.observation = Some(self.actor.observe(context));
        }
        let mut observation = self.observation.clone().unwrap();
        observation.tick = self.actor.tick;
        observation.position = self.actor.player.position.to_array();
        observation.aim = self.actor.aim.to_array();
        observation.inventory = self.actor.player.crafting.clone();
        observation.equipment = self.actor.player.crafting.hotbar.entry();
        observation.health = self.actor.player.health;
        observation.oxygen = self.actor.player.oxygen;
        observation.mana = self.actor.player.crafting.mana;
        observation.previous_action = self.actor.recent.back().cloned();
        observation.recent_events = self.actor.recent.iter().cloned().collect();
        observation.recent_world_events = self.actor.world_events.iter().cloned().collect();
        if self.scenario.is_ai()
            && self.objective.complete(
                context.world,
                &self.actor.player,
                context.creatures,
                self.actor.tick,
            )
        {
            self.script.crafted = true;
            self.script.finished =
                Some("Scenario completed by authoritative objective checks".into());
            self.report(self.script.finished.as_ref().unwrap())?;
            return Ok(Effects::default());
        }
        let action = if let Some(ai) = &mut self.ai {
            ai.progress = self.objective.clone();
            let action = ai.next(&observation);
            if let Some(reason) = &ai.stopped {
                self.script.finished = Some(reason.clone());
            }
            action
        } else if self.scenario == Scenario::GatherCraft {
            if sensed {
                self.script.next(&observation)
            } else {
                Action::Wait
            }
        } else {
            self.task_action(&observation)
        };
        let hostile_target = if matches!(action, Action::Attack) {
            context
                .creatures
                .weapon_target(
                    context.world,
                    self.actor.player.position + Vec3::Y * 1.62,
                    self.actor.aim,
                    3.2,
                )
                .is_some_and(|id| {
                    context.creatures.snapshot_with_ids().iter().any(|c| {
                        c.0 == id && crate::creature::CreatureKind::from_u8(c.1).is_hostile()
                    })
                })
        } else {
            false
        };
        let (outcome, effects) = self.actor.act(context, &action);
        let activity = match &action {
            Action::Move { .. } => "Moving",
            Action::Look { .. } => "Looking / aiming",
            Action::Interact { .. } => "Interacting",
            Action::Collect => "Collecting drops",
            Action::Equip { .. } => "Equipping an item",
            Action::Mine { .. } => "Mining a block",
            Action::Place { .. } => "Placing a block",
            Action::Craft { .. } => "Crafting / converting materials",
            Action::Attack => "Attacking",
            Action::Device { .. } => "Operating a device",
            Action::Wait => "Waiting",
        };
        if !matches!(action, Action::Wait) {
            self.last_activity = Some((self.actor.tick, if outcome.status == Status::Rejected {
                "Action rejected / reconsidering"
            } else { activity }));
        }
        self.findings.extend(effects.findings.iter().cloned());
        self.objective.record(
            &action,
            &outcome,
            &effects,
            hostile_target,
            &self.actor.player,
        );
        if outcome.status == Status::Rejected && outcome.reason != "Cooldown active" {
            self.failures += 1;
        }
        if self.failures >= 8 {
            self.script.finished = Some(format!(
                "Controller stopped after eight rejected actions: {}",
                outcome.reason
            ));
        }
        if matches!(
            action,
            Action::Craft {
                recipe: crafting::Action::Craft(_)
            }
        ) && outcome.status == Status::Completed
        {
            if self.scenario == Scenario::GatherCraft {
                self.script.crafted = true;
                self.script.finished = Some(outcome.reason.clone());
            }
        }
        if self.scenario == Scenario::AiGatherTool
            && matches!(
                action,
                Action::Craft {
                    recipe: crafting::Action::CraftGear(_)
                }
            )
            && outcome.status == Status::Completed
        {
            self.script.crafted = true;
            self.script.finished =
                Some("Crafted an additional tool through the validated crafting action".into());
        }
        let decisions = self.ai.as_mut().map(|ai| std::mem::take(&mut ai.events));
        let record = serde_json::json!({"observation":if sensed {Some(&observation)} else {None},"action":action,"outcome":outcome,
            "ai_events":decisions,
            "objective":self.objective,
            "position_after":self.actor.player.position.to_array(),"inventory_after":self.actor.player.crafting,
            "edits":effects.edits,"interacts":effects.interacts,"findings":effects.findings,
            "technical_player_after":crate::save::PlayerSave::capture(&self.actor.player),
            "players":context.players});
        let written = serde_json::to_writer(&mut self.log, &record)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                self.log
                    .write_all(b"\n")
                    .and_then(|()| self.log.flush())
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = written {
            self.script.finished = Some(format!("Logging failed; stopped: {e}"));
        }
        if let Some(p) = self.visual.get_mut(&AGENT_ID) {
            p.pos = self.actor.player.position;
            p.held = self.actor.player.crafting.hotbar.entry();
            p.yaw = self.actor.aim.z.atan2(self.actor.aim.x);
            if outcome.status != Status::Rejected {
                if matches!(
                    action,
                    Action::Mine { .. } | Action::Place { .. } | Action::Craft { .. }
                ) {
                    p.animation.start(crate::player_animation::Clip::Work);
                } else if matches!(action, Action::Attack) {
                    p.animation.start(crate::player_animation::Clip::Attack);
                }
            }
            p.animation_received = Instant::now();
            p.animation.advance(
                STEP,
                self.actor.player.velocity.length(),
                self.actor.player.on_ground,
            );
        }
        if let Some(reason) = &self.script.finished {
            if let Some(p) = self.visual.get_mut(&AGENT_ID) {
                p.animation.start(crate::player_animation::Clip::Idle);
            }
            if let Err(e) = self.report(reason) {
                self.script.finished = Some(format!("Report write failed; stopped: {e}"));
            }
        }
        Ok(effects)
    }

    pub fn report(&self, reason: &str) -> Result<(), String> {
        let reproduction = if self.findings.is_empty() { serde_json::json!({"status":"not_attempted","reason":"No invariant finding"}) }
            else { replay::replay(&self.directory).unwrap_or_else(|error| serde_json::json!({"status":"uncertain","error":error})) };
        let reproduced = reproduction.get("findings_reproduced").and_then(serde_json::Value::as_array)
            .is_some_and(|findings| !findings.is_empty());
        let report = serde_json::json!({"schema":2,"scenario":self.scenario,"objective":self.objective,"completed":self.script.crafted,"reason":reason,
            "ai_usage":self.ai.as_ref().map(ai::Ai::usage),
            "ticks":self.actor.tick,"events":"events.jsonl","initial_save":"initial.bin",
            "classification":if self.script.crafted {"scenario_completed"} else {"controller_or_scenario_failure"},
            "findings":self.findings,"reproduction":reproduction,
            "reproduced":reproduced,"model_cost":if self.ai.is_some(){None}else{Some(0)},"note":"Reproduced means an invariant finding recurred in the matching isolated replay prefix, not that its cause is established. Isolated replay does not simulate concurrent host, rule, creature or device updates."});
        std::fs::write(
            self.directory.join("report.json"),
            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }

    fn task_action(&mut self, o: &Observation) -> Action {
        if o.tick >= 1200 {
            self.script.finished = Some("Session timeout".into());
            return Action::Wait;
        }
        if self.task.is_none() {
            let task = match self.scenario {
                Scenario::WalkReturn => {
                    let target = o
                        .walkable
                        .iter()
                        .copied()
                        .filter(|p| {
                            let distance =
                                Vec3::new(p.0 as f32 + 0.5, p.1 as f32, p.2 as f32 + 0.5)
                                    .distance(self.home);
                            (3.0..=5.0).contains(&distance)
                        })
                        .min();
                    let Some((x, y, z)) = target else {
                        self.script.finished =
                            Some("Scenario unavailable: no observed land destination".into());
                        return Action::Wait;
                    };
                    controller::Task::MoveTo {
                        position: [x as f32 + 0.5, y as f32, z as f32 + 0.5],
                    }
                }
                Scenario::MineBlock => {
                    let eye = Vec3::from_array(o.position) + Vec3::Y * 1.62;
                    let target = o
                        .terrain
                        .iter()
                        .filter(|t| {
                            !t.block.is_unbreakable()
                                && super_distance(t.cell, eye) > 4.0
                                && super_distance(t.cell, eye) < 7.0
                        })
                        .min_by_key(|t| (super_distance(t.cell, eye) * 100.0) as u32);
                    let Some(t) = target else {
                        self.script.finished =
                            Some("Scenario unavailable: no observed mining target".into());
                        return Action::Wait;
                    };
                    controller::Task::Mine { target: t.cell }
                }
                Scenario::GatherCraft
                | Scenario::AiGatherTool
                | Scenario::AiShelter
                | Scenario::AiEncounterReturn => unreachable!(),
            };
            self.task = Some(controller::Controller::new(task, o));
        }
        let task = self.task.as_mut().unwrap();
        let action = task.next(o);
        if let Some((end, reason)) = &task.result {
            if *end == controller::End::Completed
                && self.scenario == Scenario::WalkReturn
                && !self.returning
            {
                self.returning = true;
                self.task = Some(controller::Controller::new(
                    controller::Task::MoveTo {
                        position: self.home.to_array(),
                    },
                    o,
                ));
            } else {
                self.script.crafted = *end == controller::End::Completed;
                self.script.finished = Some(format!("{end:?}: {reason}"));
            }
        }
        action
    }

    pub fn cancel(&mut self) -> Result<(), String> {
        if self.script.finished.is_some() {
            return Ok(());
        }
        if let Some(task) = &mut self.task {
            task.cancel();
        }
        self.script.finished = Some("Cancelled by user".into());
        self.report("Cancelled by user")
    }
    pub fn marker(&self) -> (Vec3, &'static str) {
        (
            Vec3::from_array(self.objective.home),
            if self.scenario == Scenario::AiShelter {
                "Agent1 shelter site"
            } else {
                "Agent1 start"
            },
        )
    }
    pub fn status(&self) -> String {
        self.ai.as_ref().map_or_else(
            || format!("{}: tick {}", self.scenario.name(), self.actor.tick),
            ai::Ai::status,
        )
    }

    pub fn overhead_status(&self) -> String {
        if let Some(reason) = &self.script.finished {
            if self.script.crafted { return "Task completed".into(); }
            let text: String = reason.chars().take(64).collect();
            return format!("Stopped: {text}{}", if reason.chars().count() > 64 { "…" } else { "" });
        }
        if let Some(status) = self.ai.as_ref().and_then(ai::Ai::overhead_status) {
            return status;
        }
        if let Some(task) = self.task.as_ref().filter(|task| task.result.is_none()) {
            return task.task.activity().into();
        }
        // Keep brief actions readable between control ticks, without delaying AI state changes.
        if let Some((tick, activity)) = self.last_activity {
            if self.actor.tick.saturating_sub(tick) <= 10 { return activity.into(); }
        }
        if self.actor.tick == 0 { "Starting playtest" } else { "Waiting / observing" }.into()
    }

    pub fn external_event(&mut self, event: &str) {
        if self.actor.world_events.len() == 16 {
            self.actor.world_events.pop_front();
        }
        self.actor.world_events.push_back(event.into());
        let record = serde_json::json!({"tick":self.actor.tick,"authoritative_event":event,
            "health":self.actor.player.health,"inventory":self.actor.player.crafting});
        let written = serde_json::to_writer(&mut self.log, &record)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                self.log
                    .write_all(b"\n")
                    .and_then(|()| self.log.flush())
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = written {
            self.script.finished = Some(format!("Logging failed: {e}"));
        }
    }
    pub fn hostile_hit(&mut self, damage: f32) {
        if damage.is_finite() && damage > 0.0 && self.script.finished.is_none() {
            self.objective.engaged_hostile = true;
            self.external_event("Hostile creature hit Agent1");
        }
    }
}
fn super_distance(p: Cell, eye: Vec3) -> f32 {
    center(p).distance(eye)
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.script.finished.is_none() {
            if let Err(error) = self.report("Session ended before scenario completion") {
                log::warn!("Playtest report: {error}");
            }
        }
    }
}

fn build_fingerprint() -> Result<String, String> {
    static FINGERPRINT: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    FINGERPRINT
        .get_or_init(|| {
            use std::io::Read;
            let mut file = File::open(std::env::current_exe().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let mut hash = 0xcbf29ce484222325u64;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                for byte in &buffer[..n] {
                    hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
                }
            }
            Ok(format!("fnv1a64:{hash:016x}"))
        })
        .clone()
}

/// Spawn setup only: choose clear, dry ground near the host; never teleport during actions.
pub fn spawn_position(world: &World, host: Vec3) -> Option<Vec3> {
    for (dx, dz) in [(3, 0), (-3, 0), (0, 3), (0, -3), (3, 3), (-3, -3)] {
        let x = host.x.floor() as i32 + dx;
        let z = host.z.floor() as i32 + dz;
        for y in (host.y.floor() as i32 - 3..=host.y.floor() as i32 + 3).rev() {
            if world.get_block(x, y - 1, z).is_solid()
                && world.get_block(x, y, z) == BlockType::Air
                && world.get_block(x, y + 1, z) == BlockType::Air
            {
                return Some(Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture() -> (
        World,
        crate::creature::Creatures,
        crate::loot::Effects,
        crafting::Registry,
        Actor,
    ) {
        let mut world = World::new(71);
        world.ensure_chunk_loaded(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                for y in 59..65 {
                    world.set_block(
                        x,
                        y,
                        z,
                        if y == 59 {
                            BlockType::Soil
                        } else {
                            BlockType::Air
                        },
                    );
                }
            }
        }
        (
            world,
            crate::creature::Creatures::new(),
            Default::default(),
            crafting::Registry::load().unwrap(),
            Actor::new(Vec3::new(8.5, 60.0, 8.5)),
        )
    }
    fn commit(context: &mut Context<'_>, effects: Effects) {
        assert!(effects.findings.is_empty(), "{:?}", effects.findings);
        for (p, b, _) in effects.edits {
            context.world.set_block(p.0, p.1, p.2, b);
        }
    }
    #[test]
    fn gathers_decomposes_and_crafts_without_inventory_grants() {
        let (mut world, mut creatures, mut loot, registry, mut actor) = fixture();
        assert!(actor.player.crafting.resources.iter().all(|n| *n == 0));
        let mut context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        let mut script = GatherCraft::default();
        let mut mined = false;
        for _ in 0..400 {
            let observation = actor.observe(&context);
            let action = script.next(&observation);
            let (outcome, effects) = actor.act(&mut context, &action);
            mined |= !effects.edits.is_empty();
            commit(&mut context, effects);
            assert_ne!(outcome.reason, "Inventory full");
            if script.finished.is_some() {
                break;
            }
        }
        assert!(mined);
        assert!(
            script.crafted,
            "{:?}, last: {:?}",
            script.finished, actor.recent
        );
        assert!(actor.player.crafting.resources.iter().any(|n| *n > 0));
    }
    #[test]
    fn actions_enforce_costs_tools_reach_collision_and_cooldowns() {
        let (mut world, mut creatures, mut loot, registry, mut actor) = fixture();
        world.set_block(10, 61, 8, BlockType::Stone);
        let mut context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        let before = serde_json::to_value(&actor.player.crafting).unwrap();
        let (result, _) = actor.act(
            &mut context,
            &Action::Craft {
                recipe: crafting::Action::CraftGear(equipment::Gear::Sword),
            },
        );
        assert_eq!(result.status, Status::Rejected);
        assert_eq!(
            serde_json::to_value(&actor.player.crafting).unwrap(),
            before
        );
        actor.aim = Vec3::X;
        let target = (10, 61, 8);
        let (result, _) = actor.act(&mut context, &Action::Mine { target });
        assert!(result.reason.contains("pickaxe"));
        actor.act(
            &mut context,
            &Action::Equip {
                item: Some(equipment::Entry::Gear(equipment::Gear::Pickaxe)),
            },
        );
        let (result, _) = actor.act(&mut context, &Action::Mine { target });
        assert_eq!(result.status, Status::Progress);
        let (result, _) = actor.act(&mut context, &Action::Mine { target });
        assert_eq!(result.reason, "Cooldown active");
        let (result, _) = actor.act(
            &mut context,
            &Action::Mine {
                target: (15, 61, 8),
            },
        );
        assert_eq!(result.status, Status::Rejected);
        let (result, _) = actor.act(
            &mut context,
            &Action::Move {
                direction: [100.0, 0.0, 0.0],
                jump: false,
            },
        );
        assert_eq!(result.status, Status::Rejected);
        for y in 60..64 {
            context.world.set_block(10, y, 8, BlockType::Stone);
        }
        for _ in 0..30 {
            actor.act(
                &mut context,
                &Action::Move {
                    direction: [1.0, 0.0, 0.0],
                    jump: false,
                },
            );
        }
        assert!(
            actor.player.position.x < 9.71,
            "{:?}",
            actor.player.position
        );
    }
    #[test]
    fn observations_do_not_reveal_buried_resources() {
        let (mut world, mut creatures, mut loot, registry, actor) = fixture();
        world.set_block(8, 58, 8, BlockType::IronOre);
        let context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        let observation = actor.observe(&context);
        assert!(!observation.terrain.iter().any(|t| t.cell == (8, 58, 8)));
        assert!(!observation.terrain.is_empty());
        let json = serde_json::to_value(observation).unwrap();
        assert!(json.get("seed").is_none());
        assert!(json.get("world").is_none());
    }
    #[test]
    fn session_writes_inspectable_actions_outcomes_and_report() {
        let (mut world, mut creatures, mut loot, registry, actor) = fixture();
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
            commit(&mut context, effects);
            if session.script.finished.is_some() {
                break;
            }
        }
        assert!(session.script.crafted, "{:?}", session.script.finished);
        let lines = std::fs::read_to_string(session.directory.join("events.jsonl")).unwrap();
        for line in lines.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(record.get("action").is_some());
            assert!(record.get("outcome").is_some());
        }
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(session.directory.join("report.json")).unwrap())
                .unwrap();
        assert_eq!(report["completed"], true);
        assert_eq!(report["reproduced"], false);
        assert_eq!(
            std::fs::read(session.directory.join("initial.bin")).unwrap(),
            snapshot
        );
        println!("Gather/craft artifacts: {}", session.directory.display());
    }

    #[test]
    fn movement_and_mining_scenarios_finish_through_player_physics() {
        for scenario in [Scenario::WalkReturn, Scenario::MineBlock] {
            let (mut world, mut creatures, mut loot, registry, actor) = fixture();
            let snapshot = crate::save::playtest_snapshot(
                &world,
                &actor.player,
                &crate::camera::Camera::new(actor.player.position, 1.0),
                0.25,
                vec![],
                &Default::default(),
            )
            .unwrap();
            let mut session = Session::create(
                actor.player.position,
                &world,
                &snapshot,
                &registry,
                scenario,
            )
            .unwrap();
            let mut context = Context {
                world: &mut world,
                creatures: &mut creatures,
                loot: &mut loot,
                registry: &registry,
                players: &[],
            };
            for _ in 0..650 {
                let effects = session.update(STEP, &mut context).unwrap();
                commit(&mut context, effects);
                if session.script.finished.is_some() {
                    break;
                }
            }
            assert!(
                session.script.crafted,
                "{scenario:?}: {:?}, position {:?}",
                session.script.finished, session.actor.player.position
            );
            if scenario == Scenario::WalkReturn {
                assert!(
                    session
                        .actor
                        .player
                        .position
                        .distance(actor.player.position)
                        < 0.25
                );
            } else {
                assert!(
                    session
                        .actor
                        .player
                        .position
                        .distance(actor.player.position)
                        > 0.5,
                    "Mining scenario must approach before striking"
                );
            }
            println!(
                "{} artifacts: {}",
                scenario.name(),
                session.directory.display()
            );
        }
    }

    #[test]
    fn controller_detects_cancel_target_loss_stall_timeout_and_invalid_arguments() {
        let (mut world, mut creatures, mut loot, registry, actor) = fixture();
        let context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        let mut observation = actor.observe(&context);
        let mut task = controller::Controller::new(
            controller::Task::MoveTo {
                position: [10.5, 60.0, 8.5],
            },
            &observation,
        );
        task.cancel();
        assert!(matches!(task.next(&observation), Action::Wait));
        assert_eq!(task.result.unwrap().0, controller::End::Cancelled);
        let mut task = controller::Controller::new(
            controller::Task::FollowAttack { creature: 123456 },
            &observation,
        );
        task.next(&observation);
        assert_eq!(task.result.unwrap().0, controller::End::TargetLost);
        let mut task = controller::Controller::new(
            controller::Task::MoveTo {
                position: [10.5, 60.0, 8.5],
            },
            &observation,
        );
        observation.tick = 41;
        task.next(&observation);
        assert_eq!(task.result.unwrap().0, controller::End::Stalled);
        let mut task = controller::Controller::new(
            controller::Task::MoveTo {
                position: [10.5, 60.0, 8.5],
            },
            &observation,
        );
        observation.tick += 600;
        task.next(&observation);
        assert_eq!(task.result.unwrap().0, controller::End::Timeout);
        let mut task = controller::Controller::new(
            controller::Task::MoveTo {
                position: [f32::NAN, 0.0, 0.0],
            },
            &observation,
        );
        task.next(&observation);
        assert_eq!(task.result.unwrap().0, controller::End::Invalid);
        assert!(serde_json::from_str::<Action>(
            r#"{"action":"move","direction":[100,0,0],"jump":false,"teleport":true}"#
        )
        .is_err());
    }

    #[test]
    fn controller_aims_equips_and_attacks_a_visible_creature() {
        let (mut world, mut creatures, mut loot, registry, mut actor) = fixture();
        let id = creatures.spawn_one(
            crate::creature::CreatureKind::Sheep,
            Vec3::new(10.5, 60.0, 8.5),
            77,
        );
        let health = creatures
            .snapshot_with_ids()
            .into_iter()
            .find(|c| c.0 == id)
            .unwrap()
            .3;
        let mut context = Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        let observation = actor.observe(&context);
        let mut controller = controller::Controller::new(
            controller::Task::FollowAttack { creature: id },
            &observation,
        );
        for _ in 0..15 {
            let action = controller.next(&actor.observe(&context));
            let (_, effects) = actor.act(&mut context, &action);
            commit(&mut context, effects);
        }
        assert!(context
            .creatures
            .snapshot_with_ids()
            .into_iter()
            .find(|c| c.0 == id)
            .is_none_or(|c| c.3 < health));
        assert_eq!(
            actor.player.crafting.hotbar.entry(),
            Some(equipment::Entry::Gear(equipment::Gear::Sword))
        );
    }
}

#[cfg(test)]
#[path = "playtest_support_tests.rs"]
mod support_tests;
