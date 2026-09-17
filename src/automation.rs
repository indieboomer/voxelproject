//! Host-owned magical devices. Integer fixed ticks; rendering and Lua are callers.
use crate::crafting::{Account, Registry};
use crate::voxel::{BlockType, World, COLLECTIBLE_BLOCKS};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[path = "automation_sim.rs"]
mod sim;
pub use sim::Clock;
#[path = "automation_ejection.rs"]
mod ejection;
pub use ejection::eject_chests;

/// Each paid aura cycle affects creatures once, on the host, including normal death events.
pub fn apply_auras(creatures: &mut crate::creature::Creatures, auras: &[(Cell, Kind)]) {
    for &(cell, kind) in auras {
        let origin = center(cell);
        if kind == Kind::Shrine {
            creatures.heal_near(origin, 6.0, 3.0);
        } else if kind == Kind::DarkAltar {
            for (id, _, pos, _, _) in creatures.snapshot_with_ids() {
                if Vec3::from_array(pos).distance_squared(origin) <= 36.0 {
                    if let Some(death) = creatures.damage(id, 3.0) {
                        creatures.combat_deaths.push(death);
                    }
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "automation_tests.rs"]
mod tests;

pub type Cell = (i32, i32, i32);
pub type Inventory = BTreeMap<String, u32>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Collector,
    Vessel,
    Condenser,
    Dissipator,
    Conduit,
    Channel,
    Splitter,
    Chest,
    Workshop,
    Sensor,
    Valve,
    Signal,
    Smelter,
    Lantern,
    DarkAltar,
    Shrine,
}
impl Kind {
    pub const ALL: [Self; 16] = [
        Self::Collector,
        Self::Vessel,
        Self::Condenser,
        Self::Dissipator,
        Self::Conduit,
        Self::Channel,
        Self::Splitter,
        Self::Chest,
        Self::Workshop,
        Self::Sensor,
        Self::Valve,
        Self::Signal,
        Self::Smelter,
        Self::Lantern,
        Self::DarkAltar,
        Self::Shrine,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Collector => "collector",
            Self::Vessel => "vessel",
            Self::Condenser => "condenser",
            Self::Dissipator => "dissipator",
            Self::Conduit => "conduit",
            Self::Channel => "channel",
            Self::Splitter => "splitter",
            Self::Chest => "chest",
            Self::Workshop => "workshop",
            Self::Sensor => "sensor",
            Self::Valve => "valve",
            Self::Signal => "signal",
            Self::Smelter => "smelter",
            Self::Lantern => "lantern",
            Self::DarkAltar => "dark_altar",
            Self::Shrine => "shrine",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }
    pub fn height(self) -> i32 {
        match self {
            Self::Lantern => 3,
            Self::DarkAltar | Self::Shrine => 2,
            _ => 1,
        }
    }
    pub fn sustained(self) -> bool {
        matches!(self, Self::Lantern | Self::DarkAltar | Self::Shrine)
    }
    pub fn chargeable(self) -> bool {
        self == Self::Vessel || self.sustained()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    West,
    East,
    Down,
    Up,
    North,
    South,
}
impl Face {
    pub const ALL: [Self; 6] = [
        Self::West,
        Self::East,
        Self::Down,
        Self::Up,
        Self::North,
        Self::South,
    ];
    pub fn delta(self) -> Cell {
        match self {
            Self::West => (-1, 0, 0),
            Self::East => (1, 0, 0),
            Self::Down => (0, -1, 0),
            Self::Up => (0, 1, 0),
            Self::North => (0, 0, -1),
            Self::South => (0, 0, 1),
        }
    }
    pub fn opposite(self) -> Self {
        match self {
            Self::West => Self::East,
            Self::East => Self::West,
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
        }
    }
    pub fn rotated(mut self, turns: u8) -> Self {
        for _ in 0..turns % 4 {
            self = match self {
                Self::East => Self::South,
                Self::South => Self::West,
                Self::West => Self::North,
                Self::North => Self::East,
                x => x,
            };
        }
        self
    }
    pub fn neighbor(self, p: Cell) -> Cell {
        let d = self.delta();
        (p.0 + d.0, p.1 + d.1, p.2 + d.2)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Mana,
    Matter,
    Signal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    In,
    Out,
    Both,
}
pub type Port = (Face, Network, Direction);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definition {
    pub kind: Kind,
    pub name: String,
    pub cost: Inventory,
    pub mana_capacity: u32,
    pub item_capacity: u32,
    pub throughput: u32,
    pub duration_ticks: u32,
    pub ports: Vec<Port>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub smelting: Smelting,
    pub tick_ms: u32,
    pub max_devices: usize,
    pub collector_radius: i32,
    pub collector_supply: u32,
    pub collector_period_ticks: u64,
    pub condenser_costs: [u32; 5],
    pub dissipator_yields: [u32; 5],
    pub devices: Vec<Definition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Smelting {
    pub recipes: BTreeMap<String, String>,
    pub mana_cost: u32,
    pub wood_heat: u32,
    pub coal_heat: u32,
}
impl Balance {
    pub fn validate(&self) -> Result<(), String> {
        if self.smelting.recipes.is_empty()
            || self.smelting.recipes.len() > 32
            || self.smelting.mana_cost == 0
            || self.smelting.mana_cost > 10000
            || self.smelting.wood_heat == 0
            || self.smelting.coal_heat <= self.smelting.wood_heat
            || self.smelting.coal_heat > 10000
            || self
                .smelting
                .recipes
                .iter()
                .any(|(ore, metal)| !valid_item(ore) || !valid_item(metal) || ore == metal)
        {
            return Err("Invalid smelting balance".into());
        }
        if !(20..=1000).contains(&self.tick_ms)
            || !(1..=128).contains(&self.max_devices)
            || !(1..=32).contains(&self.collector_radius)
            || self.collector_period_ticks == 0
            || self.collector_supply == 0
            || self.collector_supply > 1000
        {
            return Err("Invalid automation timing or supply".into());
        }
        for i in 0..5 {
            if self.condenser_costs[i] == 0
                || self.condenser_costs[i] > 10000
                || self.dissipator_yields[i] >= self.condenser_costs[i]
            {
                return Err("Element dissipation must lose mana".into());
            }
        }
        for kind in Kind::ALL {
            let defs: Vec<_> = self.devices.iter().filter(|d| d.kind == kind).collect();
            if defs.len() != 1 {
                return Err("Every device needs exactly one definition".into());
            }
            let d = defs[0];
            if d.mana_capacity > 10000
                || (d.kind != Kind::Chest && d.item_capacity > 256)
                || d.throughput == 0
                || d.throughput > 256
                || d.duration_ticks == 0
                || d.duration_ticks > 36000
                || d.ports.len() > 18
                || !valid_inventory(&d.cost)
            {
                return Err("Invalid device definition".into());
            }
        }
        Ok(())
    }
    pub fn def(&self, kind: Kind) -> &Definition {
        self.devices
            .iter()
            .find(|d| d.kind == kind)
            .expect("validated device definition")
    }
}
pub fn balance() -> &'static Balance {
    static BALANCE: std::sync::OnceLock<Balance> = std::sync::OnceLock::new();
    BALANCE.get_or_init(|| {
        let b: Balance =
            serde_json::from_str(include_str!("../data/automation.json")).expect("automation JSON");
        b.validate().expect("automation balance");
        b
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Routing {
    #[default]
    RoundRobin,
    Priority,
    Filter,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalMode {
    #[default]
    State,
    Pulse,
    Numeric,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub element: usize,
    pub recipe: String,
    pub reserve: u32,
    pub input: Face,
    pub outputs: Vec<Face>,
    pub filter: String,
    pub routing: Routing,
    pub sensor_target: Cell,
    pub sensor_item: String,
    pub lower: u32,
    pub upper: u32,
    pub signal_mode: SignalMode,
    pub invert: bool,
    pub valve_matter: bool,
    pub eject_contents: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            element: 0,
            recipe: String::new(),
            reserve: 0,
            input: Face::West,
            outputs: vec![Face::East],
            filter: String::new(),
            routing: Routing::RoundRobin,
            sensor_target: (0, 0, 0),
            sensor_item: String::new(),
            lower: 5,
            upper: 10,
            signal_mode: SignalMode::State,
            invert: false,
            valve_matter: false,
            eject_contents: true,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Activity {
    #[default]
    Idle,
    Working,
    Disabled,
    MissingIngredients,
    InsufficientMana,
    BlockedOutput,
    NoRecipe,
    Closed,
    MissingFuel,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    pub ingredients: Inventory,
    pub mana_cost: u32,
    pub output: Inventory,
    pub mana_output: u32,
    pub elapsed: u32,
    pub duration: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    #[serde(default)]
    pub persistent_id: u64,
    /// Completed metal ingots, Death dissipation, and bound sheep production.
    #[serde(default)]
    pub quest_production: [u64; 3],
    #[serde(default)]
    pub powered_ticks: u32,
    #[serde(default)]
    pub fuel_heat: u32,
    #[serde(default)]
    pub last_ejection_tick: u64,
    /// Monotonic presentation counters: completed cycles, transfers, state edges.
    /// Snapshots retain events even when buffers fill and drain between packets.
    #[serde(default)]
    pub feedback_events: [u64; 3],
    pub cell: Cell,
    pub kind: Kind,
    pub rotation: u8,
    pub config: Config,
    pub items: Inventory,
    pub output: Inventory,
    pub mana: u32,
    pub batch: Option<Batch>,
    pub signal: u32,
    pub latched: bool,
    pub cursor: u32,
    pub activity: Activity,
    pub config_revision: u64,
}
impl Device {
    pub(crate) fn feedback(&mut self, category: usize) {
        self.feedback_events[category] = self.feedback_events[category].wrapping_add(1);
    }
    pub fn new(kind: Kind, cell: Cell, rotation: u8) -> Self {
        let mut config = Config::default();
        if kind == Kind::Chest {
            config.eject_contents = false;
        }
        if kind == Kind::Splitter {
            config.outputs = vec![Face::East, Face::North, Face::South];
        }
        Self {
            quest_production: [0; 3],
            persistent_id: 0,
            powered_ticks: 0,
            feedback_events: [0; 3],
            fuel_heat: 0,
            last_ejection_tick: 0,
            cell,
            kind,
            rotation,
            config,
            items: Inventory::new(),
            output: Inventory::new(),
            mana: 0,
            batch: None,
            signal: 0,
            latched: false,
            cursor: 0,
            activity: Activity::Idle,
            config_revision: 0,
        }
    }
    pub fn ports(&self, b: &Balance) -> Vec<Port> {
        let mut ports = b.def(self.kind).ports.clone();
        if matches!(self.kind, Kind::Chest | Kind::Smelter) && self.config.eject_contents {
            ports.retain(|p| p.1 != Network::Matter || p.2 == Direction::In);
        }
        if matches!(
            self.kind,
            Kind::Conduit | Kind::Channel | Kind::Splitter | Kind::Valve
        ) {
            let net = if matches!(self.kind, Kind::Channel | Kind::Splitter)
                || (self.kind == Kind::Valve && self.config.valve_matter)
            {
                Network::Matter
            } else {
                Network::Mana
            };
            ports.retain(|p| p.1 == Network::Signal);
            ports.push((self.config.input, net, Direction::In));
            ports.extend(
                self.config
                    .outputs
                    .iter()
                    .map(|&face| (face, net, Direction::Out)),
            );
        }
        ports
            .into_iter()
            .map(|(f, n, d)| (f.rotated(self.rotation), n, d))
            .collect()
    }
    pub fn item_count(&self) -> u32 {
        count(&self.items).saturating_add(count(&self.output))
    }
    pub fn accepts(&self, item: &str) -> bool {
        match self.kind {
            Kind::Smelter => {
                balance().smelting.recipes.contains_key(item)
                    || item == "resource:coal"
                    || item
                        .strip_prefix("resource:")
                        .and_then(BlockType::from_name)
                        .is_some_and(BlockType::is_wood)
            }
            Kind::Workshop => item.starts_with("element:"),
            Kind::Dissipator => item == element_id(self.config.element),
            Kind::Chest | Kind::Channel | Kind::Splitter => true,
            Kind::Valve => self.config.valve_matter,
            _ => false,
        }
    }
    pub fn outgoing(&self) -> &Inventory {
        if matches!(self.kind, Kind::Condenser | Kind::Workshop | Kind::Smelter) {
            &self.output
        } else {
            &self.items
        }
    }
    fn outgoing_mut(&mut self) -> &mut Inventory {
        if matches!(self.kind, Kind::Condenser | Kind::Workshop | Kind::Smelter) {
            &mut self.output
        } else {
            &mut self.items
        }
    }
    pub fn open(&self) -> bool {
        self.config.enabled
            && (self.kind != Kind::Valve || ((self.signal > 0) ^ self.config.invert))
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub next_device_id: u64,
    pub tick: u64,
    #[serde(with = "device_map")]
    pub devices: BTreeMap<Cell, Device>,
}
mod device_map {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        m: &BTreeMap<Cell, Device>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        m.values().collect::<Vec<_>>().serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<Cell, Device>, D::Error> {
        let values = Vec::<Device>::deserialize(d)?;
        if values.len() > 128 {
            return Err(serde::de::Error::custom("Too many devices"));
        }
        let mut result = BTreeMap::new();
        for v in values {
            if result.insert(v.cell, v).is_some() {
                return Err(serde::de::Error::custom("Duplicate device cell"));
            }
        }
        Ok(result)
    }
}
pub fn element_id(index: usize) -> &'static str {
    [
        "element:earth",
        "element:fire",
        "element:water",
        "element:life",
        "element:death",
    ][index.min(4)]
}
pub fn valid_item(id: &str) -> bool {
    if id.strip_prefix("harvest:").is_some_and(|v| matches!(v, "wool" | "egg" | "milk" | "honey" | "cooked_egg" | "cooked_milk" | "cooked_honey" | "cooked_pumpkin")) { return true; }
    if id.strip_prefix("spell:").is_some_and(|v| v.parse::<u64>().is_ok_and(|n| n > 0)) { return true; }
    if (0..5).any(|i| element_id(i) == id) {
        return true;
    }
    if let Some(name) = id.strip_prefix("resource:") {
        return BlockType::from_name(name).is_some_and(|b| COLLECTIBLE_BLOCKS.contains(&b));
    }
    if let Some(name) = id.strip_prefix("item:") {
        return crate::equipment::Gear::ALL.iter().any(|g| g.id() == name);
    }
    id.strip_prefix("creature:")
        .is_some_and(|name| crate::crafting::creature_kind(name).is_some())
}
fn valid_inventory(items: &Inventory) -> bool {
    items.len() <= 512 && items.iter().all(|(id, n)| valid_item(id) && *n > 0)
}
pub fn count(items: &Inventory) -> u32 {
    items.values().fold(0u32, |a, b| a.saturating_add(*b))
}
fn add(items: &mut Inventory, id: &str, n: u32) {
    if n > 0 {
        *items.entry(id.into()).or_default() += n;
    }
}
fn take(items: &mut Inventory, id: &str, n: u32) {
    let entry = items.get_mut(id).expect("reserved matter");
    *entry -= n;
    if *entry == 0 {
        items.remove(id);
    }
}
pub fn center(p: Cell) -> Vec3 {
    Vec3::new(p.0 as f32 + 0.5, p.1 as f32 + 0.5, p.2 as f32 + 0.5)
}
pub fn valid_cell(p: Cell) -> bool {
    p.0.unsigned_abs() <= 999_000
        && p.2.unsigned_abs() <= 999_000
        && (1..crate::voxel::chunk::CHUNK_Y - 1).contains(&p.1)
}

pub fn validate_account(account: &Account, b: &Balance, recipes: &Registry) -> Result<(), String> {
    if !account.adventure.valid() {
        return Err("Invalid adventure progress".into());
    }
    if account.packed_devices.len() > 8
        || account.production_goods.len() > 13
        || account
            .production_goods
            .iter()
            .any(|(id, n)| !id.starts_with("creature:") || !valid_item(id) || *n > 1_000_000)
    {
        return Err("Invalid packed inventory".into());
    }
    for d in &account.packed_devices {
        let mut s = State::default();
        s.devices.insert(d.cell, d.clone());
        s.validate(b, recipes)?;
    }
    Ok(())
}

impl State {
    pub fn ensure_device_ids(&mut self) {
        self.next_device_id = self.next_device_id.max(1);
        for d in self.devices.values() {
            self.next_device_id = self.next_device_id.max(d.persistent_id.saturating_add(1));
        }
        let mut seen = std::collections::HashSet::new();
        for d in self.devices.values_mut() {
            if d.persistent_id == 0 || !seen.insert(d.persistent_id) {
                d.persistent_id = self.next_device_id;
                self.next_device_id = self.next_device_id.saturating_add(1);
                seen.insert(d.persistent_id);
            }
        }
    }
    /// Multi-block props are stored once, at their base. Resolve any occupied voxel.
    pub fn device_at(&self, p: Cell) -> Option<&Device> {
        if !valid_cell(p) {
            return None;
        }
        (0..3).find_map(|dy| {
            self.devices
                .get(&(p.0, p.1 - dy, p.2))
                .filter(|d| dy < d.kind.height())
        })
    }
    pub fn validate(&self, b: &Balance, recipes: &Registry) -> Result<(), String> {
        if self.devices.len() > b.max_devices {
            return Err("Device limit reached".into());
        }
        for (p, d) in &self.devices {
            if *p != d.cell
                || !valid_cell(*p)
                || d.rotation > 3
                || !valid_inventory(&d.items)
                || !valid_inventory(&d.output)
                || d.mana > b.def(d.kind).mana_capacity
                || d.item_count() > b.def(d.kind).item_capacity
                || !valid_cell((p.0, p.1 + d.kind.height() - 1, p.2))
                || d.powered_ticks > b.def(d.kind).duration_ticks
                || (!d.kind.sustained() && d.powered_ticks != 0)
                || (1..d.kind.height()).any(|dy| self.devices.contains_key(&(p.0, p.1 + dy, p.2)))
            {
                return Err("Invalid stored device".into());
            }
            validate_config(d.kind, &d.config, b, recipes)?;
            if d.fuel_heat > b.smelting.coal_heat
                || (d.kind != Kind::Smelter && d.fuel_heat != 0)
                || (d.kind == Kind::Smelter && d.batch.is_some())
            {
                return Err("Invalid smelter heat or batch".into());
            }
            if let Some(batch) = &d.batch {
                if !valid_inventory(&batch.ingredients)
                    || !valid_inventory(&batch.output)
                    || batch.duration == 0
                    || batch.duration > 36000
                    || batch.elapsed > batch.duration
                    || batch.mana_output > b.def(d.kind).mana_capacity
                    || batch.mana_cost > 10000
                    || count(&batch.output) > b.def(d.kind).item_capacity
                {
                    return Err("Invalid production progress".into());
                }
            }
        }
        Ok(())
    }
    pub fn configure(
        &mut self,
        p: Cell,
        config: Config,
        expected: u64,
        b: &Balance,
        recipes: &Registry,
    ) -> Result<(), String> {
        let d = self.devices.get_mut(&p).ok_or("No device at that cell")?;
        if d.config_revision != expected {
            return Err("Device configuration changed; reopen the panel".into());
        }
        validate_config(d.kind, &config, b, recipes)?;
        if d.kind == Kind::Valve
            && d.config.valve_matter != config.valve_matter
            && (d.mana > 0 || d.item_count() > 0)
        {
            return Err("Empty valve before changing its connection type".into());
        }
        let revision = d.config_revision.checked_add(1).ok_or("Revision limit")?;
        d.config = config;
        d.config_revision = revision;
        Ok(())
    }
}
fn validate_config(kind: Kind, c: &Config, b: &Balance, recipes: &Registry) -> Result<(), String> {
    if c.element > 4
        || c.reserve > b.def(kind).mana_capacity
        || c.outputs.is_empty()
        || c.outputs.len() > 5
        || c.outputs.contains(&c.input)
        || c.outputs
            .iter()
            .enumerate()
            .any(|(i, f)| c.outputs[..i].contains(f))
        || c.lower >= c.upper
        || c.upper > 1_000_000
        || (!c.filter.is_empty() && !valid_item(&c.filter))
        || (!c.sensor_item.is_empty() && !valid_item(&c.sensor_item))
        || (!c.recipe.is_empty() && !recipes.recipes.iter().any(|r| r.id == c.recipe))
    {
        return Err("Invalid device settings".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    Place {
        kind: Kind,
        cell: Cell,
        rotation: u8,
        packed: Option<usize>,
    },
    Pack {
        cell: Cell,
    },
    Rotate {
        cell: Cell,
    },
    Configure {
        cell: Cell,
        config: Config,
        expected: u64,
    },
    Charge {
        cell: Cell,
        amount: u32,
    },
    Deposit {
        cell: Cell,
        item: String,
        amount: u32,
    },
    Withdraw {
        cell: Cell,
        item: String,
        amount: u32,
    },
    Release {
        cell: Cell,
        item: String,
    },
}
impl Action {
    pub fn cell(&self) -> Cell {
        match self {
            Self::Place { cell, .. }
            | Self::Pack { cell }
            | Self::Rotate { cell }
            | Self::Configure { cell, .. }
            | Self::Charge { cell, .. }
            | Self::Deposit { cell, .. }
            | Self::Withdraw { cell, .. }
            | Self::Release { cell, .. } => *cell,
        }
    }
}

/// Stage both accounts and installation; failures leave every count unchanged.
pub fn apply(
    world: &World,
    state: &mut State,
    account: &mut Account,
    feet: Vec3,
    players: &[Vec3],
    action: &Action,
    b: &Balance,
    recipes: &Registry,
) -> Result<(), String> {
    let p = action.cell();
    if !valid_cell(p) || !feet.is_finite() || center(p).distance(feet + Vec3::Y) > 7.0 {
        return Err("Device is out of reach".into());
    }
    let eye = feet + Vec3::Y * 1.62;
    let delta = center(p) - eye;
    if crate::raycast::raycast(world, eye, delta, delta.length()).is_some_and(|hit| {
        hit.target != p && state.device_at(hit.target).is_none_or(|d| d.cell != p)
    }) {
        return Err("Device is obstructed".into());
    }
    let mut next = state.clone();
    let mut funds = account.clone();
    match action {
        Action::Release { .. } => {
            return Err("Creature release requires the authoritative creature transaction".into())
        }
        Action::Place {
            kind,
            rotation,
            packed,
            ..
        } => {
            if *rotation > 3
                || next.devices.len() >= b.max_devices
                || (0..kind.height()).any(|dy| {
                    let q = (p.0, p.1 + dy, p.2);
                    !valid_cell(q)
                        || next.device_at(q).is_some()
                        || world.get_block(q.0, q.1, q.2) != BlockType::Air
                })
            {
                return Err("Occupied cell or device limit".into());
            }
            if players.iter().any(|v| {
                v.x + 0.3 > p.0 as f32
                    && v.x - 0.3 < (p.0 + 1) as f32
                    && v.z + 0.3 > p.2 as f32
                    && v.z - 0.3 < (p.2 + 1) as f32
                    && v.y + 1.8 > p.1 as f32
                    && v.y < (p.1 + kind.height()) as f32
            }) {
                return Err("A player occupies that cell".into());
            }
            let mut device = if let Some(index) = packed {
                if *index >= funds.packed_devices.len() {
                    return Err("Packed device unavailable".into());
                }
                let d = funds.packed_devices.remove(*index);
                if d.kind != *kind {
                    return Err("Packed device changed".into());
                }
                d
            } else {
                for (item, n) in &b.def(*kind).cost {
                    account_take(&mut funds, item, *n)?;
                }
                {
                    let mut d = Device::new(*kind, p, *rotation);
                    if kind.sustained() {
                        d.mana = b.def(*kind).mana_capacity.min(30);
                    }
                    d
                }
            };
            device.cell = p;
            next.ensure_device_ids();
            device.persistent_id = next.next_device_id;
            next.next_device_id = next
                .next_device_id
                .checked_add(1)
                .ok_or("Device identity limit")?;
            device.rotation = *rotation;
            if matches!(device.kind, Kind::Chest | Kind::Smelter) {
                device.last_ejection_tick = next.tick;
            }
            next.devices.insert(p, device);
        }
        Action::Pack { .. } => {
            if funds.packed_devices.len() >= 8 {
                return Err("Packed device inventory full (8); place a packed device first".into());
            }
            funds
                .packed_devices
                .push(next.devices.remove(&p).ok_or("No device")?);
        }
        Action::Rotate { .. } => {
            let d = next.devices.get_mut(&p).ok_or("No device")?;
            d.rotation = (d.rotation + 1) % 4;
            d.config_revision = d.config_revision.checked_add(1).ok_or("Revision limit")?;
        }
        Action::Configure {
            config, expected, ..
        } => next.configure(p, config.clone(), *expected, b, recipes)?,
        Action::Charge { amount, .. } => {
            let d = next.devices.get_mut(&p).ok_or("No device")?;
            if !d.kind.chargeable()
                || *amount == 0
                || *amount > b.def(d.kind).mana_capacity - d.mana
            {
                return Err("Charge requires space in a vessel, lantern, altar, or shrine".into());
            }
            funds.mana = funds
                .mana
                .checked_sub(recipes.mana_charge(*amount))
                .ok_or("Insufficient personal mana")?;
            d.mana += amount;
        }
        Action::Deposit { item, amount, .. } => {
            let d = next.devices.get_mut(&p).ok_or("No device")?;
            if !valid_item(item)
                || *amount == 0
                || !d.accepts(item)
                || *amount > b.def(d.kind).item_capacity - d.item_count()
            {
                return Err("Incompatible input or full buffer".into());
            }
            account_take(&mut funds, item, *amount)?;
            add(&mut d.items, item, *amount);
        }
        Action::Withdraw { item, amount, .. } => {
            let d = next.devices.get_mut(&p).ok_or("No device")?;
            if !valid_item(item) || *amount == 0 {
                return Err("Invalid matter".into());
            }
            let output = d.output.get(item).copied().unwrap_or(0).min(*amount);
            let input = amount - output;
            if d.items.get(item).copied().unwrap_or(0) < input {
                return Err("Not enough stored matter".into());
            }
            if output > 0 {
                take(&mut d.output, item, output);
            }
            if input > 0 {
                take(&mut d.items, item, input);
            }
            account_add(&mut funds, item, *amount)?;
        }
    }
    funds.revision = funds
        .revision
        .checked_add(1)
        .ok_or("Inventory revision limit")?;
    if matches!(
        action,
        Action::Charge { .. } | Action::Deposit { .. } | Action::Withdraw { .. }
    ) {
        next.devices.get_mut(&p).unwrap().feedback(1);
    }
    next.validate(b, recipes)?;
    validate_account(&funds, b, recipes)?;
    *state = next;
    *account = funds;
    Ok(())
}
fn account_slot<'a>(account: &'a mut Account, id: &str) -> Result<&'a mut u32, String> {
    if let Some(i) = (0..5).find(|&i| element_id(i) == id) {
        return Ok(&mut account.elements[i]);
    }
    if let Some(name) = id.strip_prefix("resource:") {
        let i = COLLECTIBLE_BLOCKS
            .iter()
            .position(|b| b.id() == name)
            .ok_or("Unknown resource")?;
        return Ok(&mut account.resources[i]);
    }
    if let Some(name) = id.strip_prefix("item:") {
        let i = crate::equipment::Gear::ALL
            .iter()
            .find(|g| g.id() == name)
            .ok_or("Unknown item")?
            .to_owned() as usize;
        return Ok(&mut account.gear[i]);
    }
    if valid_item(id) {
        return Ok(account.production_goods.entry(id.into()).or_default());
    }
    Err("Unknown matter".into())
}
pub fn account_count(account: &Account, id: &str) -> u32 {
    if let Some(id) = id.strip_prefix("spell:").and_then(|v| v.parse::<u64>().ok()) { return u32::from(account.has_spell_card(id)); }
    let mut copy = account.clone();
    account_slot(&mut copy, id).map(|v| *v).unwrap_or(0)
}
fn account_take(account: &mut Account, id: &str, n: u32) -> Result<(), String> {
    if let Some(id) = id.strip_prefix("spell:").and_then(|v| v.parse::<u64>().ok()) {
        if n != 1 { return Err("Spell cards transfer one at a time".into()); }
        return account.remove_spell_card(id);
    }
    let v = account_slot(account, id)?;
    *v = v
        .checked_sub(n)
        .ok_or_else(|| format!("Requires {n} {id}"))?;
    Ok(())
}
pub(crate) fn account_add(account: &mut Account, id: &str, n: u32) -> Result<(), String> {
    if let Some(id) = id.strip_prefix("spell:").and_then(|v| v.parse::<u64>().ok()) {
        if n != 1 { return Err("Spell cards transfer one at a time".into()); }
        return account.add_spell_card(id);
    }
    let v = account_slot(account, id)?;
    *v = v.checked_add(n).ok_or("Inventory full")?;
    Ok(())
}

/// Creature recipes make transportable bound figurines, released explicitly in
/// safe space using the same placement and population checks as manual crafting.
pub fn release(
    world: &World,
    creatures: &mut crate::creature::Creatures,
    account: &mut Account,
    feet: Vec3,
    players: &[Vec3],
    cell: Cell,
    item: &str,
) -> Result<(), String> {
    if !feet.is_finite() || center(cell).distance(feet) > 7.0 {
        return Err("Out of reach".into());
    }
    let kind = item
        .strip_prefix("creature:")
        .and_then(crate::crafting::creature_kind)
        .ok_or("Not a bound creature")?;
    let mut next = account.clone();
    account_take(&mut next, item, 1)?;
    let position = if kind == crate::creature::CreatureKind::Fish {
        crate::creature::Creatures::fish_spawn_near(world, feet, 8.0, account.revision)
    } else {
        crate::crafting::spawn_position(world, creatures, feet, players)
    }
    .ok_or("No safe space to release creature")?;
    let mut draft = crate::creature::CreatureDraft::new(creatures);
    draft
        .spawn_in_world(world, kind, position, account.revision)
        .ok_or("Creature clearance or population limit")?;
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("Inventory revision limit")?;
    draft.commit(creatures);
    *account = next;
    Ok(())
}
