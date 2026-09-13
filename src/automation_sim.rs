use super::*;

impl State {
    /// Stable face/port order. Ports of different types never share a network.
    pub fn connections(&self, p: Cell, network: Network, b: &Balance) -> Vec<Cell> {
        let Some(source) = self.devices.get(&p) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for (face, net, direction) in source.ports(b) {
            if net != network || direction == Direction::In {
                continue;
            }
            let neighbor = face.neighbor(p);
            if self.devices.get(&neighbor).is_some_and(|d| {
                d.ports(b).iter().any(|&(f, n, dir)| {
                    f == face.opposite() && n == network && dir != Direction::Out
                })
            }) && !result.contains(&neighbor)
            {
                result.push(neighbor);
            }
        }
        result
    }

    pub fn step(&mut self, b: &Balance, recipes: &Registry) -> Vec<(Cell, Kind)> {
        let mut auras = Vec::new();
        let previous: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|(&p, d)| (p, (d.activity, d.signal)))
            .collect();
        self.tick = self.tick.wrapping_add(1);
        self.signals(b);
        self.collect(b);
        self.transfer_mana(b);
        self.transfer_matter(b);
        let blocked: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|(&p, d)| {
                let outputs = self.connections(p, Network::Matter, b);
                let blocked = matches!(d.kind, Kind::Condenser | Kind::Workshop | Kind::Smelter)
                    && !outputs.is_empty()
                    && outputs.iter().all(|to| {
                        !self.devices[to].open()
                            || self.devices[to].item_count()
                                >= b.def(self.devices[to].kind).item_capacity
                    });
                (p, blocked)
            })
            .collect();
        for (&p, device) in self.devices.iter_mut() {
            if device.kind.sustained() {
                if !device.config.enabled {
                    device.activity = Activity::Disabled;
                } else {
                    if device.powered_ticks == 0 {
                        let cost = recipes.mana_charge(1);
                        if device.mana >= cost {
                            device.mana -= cost;
                            device.powered_ticks = b.def(device.kind).duration_ticks;
                            if device.kind != Kind::Lantern {
                                auras.push((p, device.kind));
                                device.feedback(0);
                            }
                        }
                    }
                    device.activity = if device.powered_ticks > 0 {
                        device.powered_ticks -= 1;
                        Activity::Working
                    } else { Activity::InsufficientMana };
                }
            } else {
                produce(device, b, recipes, blocked[&p]);
            }
            if previous[&p] != (device.activity, device.signal) {
                device.feedback(2);
            }
        }
        auras
    }

    fn signals(&mut self, b: &Balance) {
        // Read all sensor values from one snapshot before propagating. No stale
        // signals survive removal; branches combine deterministically by max.
        let measurements: Vec<_> = self
            .devices
            .iter()
            .filter(|(_, d)| d.kind == Kind::Sensor)
            .map(|(&p, d)| {
                let target = self.devices.get(&d.config.sensor_target);
                let value = if center(p).distance(center(d.config.sensor_target)) <= 16.0 {
                    target.map_or(0, |t| match t.kind {
                        Kind::Vessel => t.mana,
                        Kind::Chest => {
                            if d.config.sensor_item.is_empty() {
                                t.item_count()
                            } else {
                                t.items.get(&d.config.sensor_item).copied().unwrap_or(0)
                            }
                        }
                        _ => 0,
                    })
                } else {
                    0
                };
                (p, value)
            })
            .collect();
        for d in self.devices.values_mut() {
            d.signal = 0;
        }
        for (p, value) in measurements {
            let d = self.devices.get_mut(&p).unwrap();
            let previous = d.latched;
            if value >= d.config.upper {
                d.latched = true;
            } else if value <= d.config.lower {
                d.latched = false;
            }
            if d.config.enabled {
                d.signal = match d.config.signal_mode {
                    SignalMode::State => u32::from(d.latched),
                    SignalMode::Pulse => u32::from(d.latched && !previous),
                    SignalMode::Numeric => value,
                };
            }
        }
        for _ in 0..self.devices.len() {
            let mut updates = Vec::new();
            for (&p, d) in &self.devices {
                if !d.config.enabled || d.signal == 0 {
                    continue;
                }
                for to in self.connections(p, Network::Signal, b) {
                    if self.devices[&to].config.enabled && self.devices[&to].signal < d.signal {
                        updates.push((to, d.signal));
                    }
                }
            }
            if updates.is_empty() {
                break;
            }
            for (p, value) in updates {
                let d = self.devices.get_mut(&p).unwrap();
                d.signal = d.signal.max(value);
            }
        }
    }

    fn collect(&mut self, b: &Balance) {
        if self.tick % b.collector_period_ticks != 0 {
            return;
        }
        let mut remaining: Vec<_> = self
            .devices
            .iter()
            .filter(|(_, d)| d.kind == Kind::Collector && d.config.enabled)
            .map(|(&p, _)| p)
            .collect();
        // A connected cluster shares one local ambient supply, including chains
        // across spatial-grid boundaries. Dense construction cannot multiply it.
        while let Some(first) = remaining.pop() {
            let mut cluster = vec![first];
            let mut at = 0;
            while at < cluster.len() {
                let p = cluster[at];
                let mut i = 0;
                while i < remaining.len() {
                    if center(p).distance(center(remaining[i])) <= b.collector_radius as f32 {
                        cluster.push(remaining.remove(i));
                    } else {
                        i += 1;
                    }
                }
                at += 1;
            }
            cluster.sort();
            let offset = (self.tick / b.collector_period_ticks) as usize % cluster.len();
            let mut supply = b.collector_supply;
            for i in 0..cluster.len() {
                let d = self
                    .devices
                    .get_mut(&cluster[(i + offset) % cluster.len()])
                    .unwrap();
                let amount = supply.min(b.def(d.kind).mana_capacity - d.mana);
                d.mana += amount;
                if amount > 0 {
                    d.feedback(1);
                }
                supply -= amount;
            }
        }
    }

    fn transfer_mana(&mut self, b: &Balance) {
        let mut sources: Vec<_> = self.devices.keys().copied().collect();
        if sources.is_empty() {
            return;
        }
        let length = sources.len();
        sources.rotate_left(self.tick as usize % length);
        let available: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|(&p, d)| {
                (
                    p,
                    if d.open() {
                        d.mana
                            .saturating_sub(d.config.reserve)
                            .min(b.def(d.kind).throughput)
                    } else {
                        0
                    },
                )
            })
            .collect();
        let mut inbound: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|(&p, d)| (p, b.def(d.kind).throughput))
            .collect();
        for p in sources {
            let mut budget = available[&p];
            let mut destinations = self.connections(p, Network::Mana, b);
            let length = destinations.len();
            if length > 0 {
                destinations.rotate_left(self.tick as usize % length);
            }
            for to in destinations {
                let dest = &self.devices[&to];
                if !dest.open() {
                    continue;
                }
                let amount = budget
                    .min(inbound[&to])
                    .min(b.def(dest.kind).mana_capacity - dest.mana);
                if amount == 0 {
                    continue;
                }
                self.devices.get_mut(&p).unwrap().mana -= amount;
                self.devices.get_mut(&to).unwrap().mana += amount;
                self.devices.get_mut(&p).unwrap().feedback(1);
                self.devices.get_mut(&to).unwrap().feedback(1);
                budget -= amount;
                *inbound.get_mut(&to).unwrap() -= amount;
            }
        }
    }

    fn transfer_matter(&mut self, b: &Balance) {
        let mut sources: Vec<_> = self.devices.keys().copied().collect();
        if sources.is_empty() {
            return;
        }
        let length = sources.len();
        sources.rotate_left(self.tick as usize % length);
        let stock: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|(&p, d)| (p, d.outgoing().clone()))
            .collect();
        let mut inbound: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|(&p, d)| (p, b.def(d.kind).throughput))
            .collect();
        for p in sources {
            if !self.devices[&p].open() {
                continue;
            }
            let mut budget = b.def(self.devices[&p].kind).throughput;
            for (item, &available) in &stock[&p] {
                let mut available = available;
                while available > 0 && budget > 0 {
                    let d = &self.devices[&p];
                    let mut destinations = self.connections(p, Network::Matter, b);
                    if d.kind == Kind::Splitter {
                        match d.config.routing {
                            Routing::RoundRobin => {
                                let len = destinations.len();
                                if len > 0 {
                                    destinations.rotate_left(d.cursor as usize % len);
                                }
                            }
                            Routing::Priority => (),
                            Routing::Filter => {
                                let primary = d.config.outputs[0].rotated(d.rotation).neighbor(p);
                                destinations.retain(|to| {
                                    if item == &d.config.filter {
                                        *to == primary
                                    } else {
                                        *to != primary
                                    }
                                });
                            }
                        }
                    }
                    let Some(to) = destinations.into_iter().find(|to| {
                        let d = &self.devices[to];
                        d.open()
                            && d.accepts(item)
                            && inbound[to] > 0
                            && d.item_count() < b.def(d.kind).item_capacity
                    }) else {
                        break;
                    };
                    let source = self.devices.get_mut(&p).unwrap();
                    take(source.outgoing_mut(), item, 1);
                    source.cursor = source.cursor.wrapping_add(1);
                    source.feedback(1);
                    add(&mut self.devices.get_mut(&to).unwrap().items, item, 1);
                    self.devices.get_mut(&to).unwrap().feedback(1);
                    *inbound.get_mut(&to).unwrap() -= 1;
                    available -= 1;
                    budget -= 1;
                }
            }
        }
    }
}

fn produce(d: &mut Device, b: &Balance, recipes: &Registry, blocked: bool) {
    if !d.config.enabled {
        d.activity = Activity::Disabled;
        return;
    }
    if d.kind == Kind::Valve && !d.open() {
        d.activity = Activity::Closed;
        return;
    }
    if blocked {
        d.activity = Activity::BlockedOutput;
        return;
    }
    if d.kind == Kind::Smelter {
        if d.config.eject_contents && d.activity == Activity::BlockedOutput && !d.output.is_empty()
        {
            return;
        }
        smelt(d, b, recipes);
        return;
    }
    if let Some(batch) = &mut d.batch {
        batch.elapsed = (batch.elapsed + 1).min(batch.duration);
        if batch.elapsed < batch.duration {
            d.activity = Activity::Working;
            return;
        }
        if count(&d.items) + count(&d.output) + count(&batch.output) > b.def(d.kind).item_capacity
            || d.mana + batch.mana_output > b.def(d.kind).mana_capacity
        {
            d.activity = Activity::BlockedOutput;
            return;
        }
        let batch = d.batch.take().unwrap();
        if d.kind==Kind::Dissipator && batch.ingredients.get("element:death").copied().unwrap_or(0)>0 {
            d.quest_production[1]=d.quest_production[1].saturating_add(1);
        }
        d.quest_production[2]=d.quest_production[2].saturating_add(u64::from(batch.output.get("creature:sheep").copied().unwrap_or(0)));
        d.feedback(0);
        for (id, n) in batch.output {
            add(&mut d.output, &id, n);
        }
        d.mana += batch.mana_output;
        d.activity = Activity::Idle;
        return;
    }
    let mut batch = Batch {
        ingredients: Inventory::new(),
        mana_cost: 0,
        output: Inventory::new(),
        mana_output: 0,
        elapsed: 0,
        duration: b.def(d.kind).duration_ticks,
    };
    match d.kind {
        Kind::Condenser => {
            batch.mana_cost = recipes.mana_charge(b.condenser_costs[d.config.element]);
            add(&mut batch.output, element_id(d.config.element), 1);
        }
        Kind::Dissipator => {
            add(&mut batch.ingredients, element_id(d.config.element), 1);
            batch.mana_output = b.dissipator_yields[d.config.element];
        }
        Kind::Workshop => {
            let Some(recipe) = recipes.recipes.iter().find(|r| r.id == d.config.recipe) else {
                d.activity = Activity::NoRecipe;
                return;
            };
            let mut formula = [None; 5];
            for (i, slot) in recipe.inputs.iter().enumerate() {
                formula[i] = Some(*slot);
                add(
                    &mut batch.ingredients,
                    element_id(slot.element.index()),
                    slot.amount as u32,
                );
            }
            batch.mana_cost = recipes.mana_cost(&formula);
            let mut account = Account::default();
            account.elements = crate::crafting::totals(&recipe.inputs).expect("validated recipe");
            account.mana = batch.mana_cost;
            let output = recipes
                .prepare(&mut account, &crate::crafting::Action::Craft(formula))
                .expect("registry recipe")
                .expect("craft output");
            let prefix = if output.kind == crate::crafting::ObjectKind::Creature {
                "creature"
            } else {
                "resource"
            };
            add(
                &mut batch.output,
                &format!("{prefix}:{}", output.id),
                output.quantity,
            );
        }
        Kind::Chest if d.config.eject_contents => {
            if d.items.is_empty() {
                d.activity = Activity::Idle;
            } else if d.activity != Activity::BlockedOutput {
                d.activity = Activity::Working;
            }
            return;
        }
        _ => {
            d.activity = Activity::Idle;
            return;
        }
    }
    if batch
        .ingredients
        .iter()
        .any(|(id, n)| d.items.get(id).copied().unwrap_or(0) < *n)
    {
        d.activity = Activity::MissingIngredients;
        return;
    }
    if d.mana < batch.mana_cost {
        d.activity = Activity::InsufficientMana;
        return;
    }
    // Reserve a full cycle only when its result could fit. During processing,
    // new inputs can fill the buffer; completion then holds the paid batch.
    if d.item_count() - count(&batch.ingredients) + count(&batch.output)
        > b.def(d.kind).item_capacity
        || d.mana - batch.mana_cost + batch.mana_output > b.def(d.kind).mana_capacity
    {
        d.activity = Activity::BlockedOutput;
        return;
    }
    for (id, n) in &batch.ingredients {
        take(&mut d.items, id, *n);
    }
    d.mana -= batch.mana_cost;
    d.batch = Some(batch);
    d.activity = Activity::Working;
}

/// Immediate, atomic ore conversion with one shared, persistent fuel reserve.
fn smelt(d: &mut Device, b: &Balance, recipes: &Registry) {
    if count(&d.output) >= b.def(d.kind).item_capacity {
        d.activity = Activity::BlockedOutput;
        return;
    }
    let mut made = false;
    for _ in 0..b.def(d.kind).throughput {
        let Some(ore) = d
            .items
            .keys()
            .find(|id| b.smelting.recipes.contains_key(*id))
            .cloned()
        else {
            d.activity = if made {
                Activity::Working
            } else {
                Activity::MissingIngredients
            };
            return;
        };
        let metal = &b.smelting.recipes[&ore];
        // Replacing one ore by one metal preserves capacity. No payment is
        // taken until the output is known to fit.
        if d.item_count() > b.def(d.kind).item_capacity {
            d.activity = Activity::BlockedOutput;
            return;
        }
        if recipes.mana_free {
            // Testing leaves both mana and already purchased fuel heat alone.
        } else if d.fuel_heat > 0 {
            d.fuel_heat -= 1;
        } else if d.mana >= b.smelting.mana_cost {
            d.mana -= b.smelting.mana_cost;
        } else {
            let fuel = if d.items.contains_key("resource:coal") {
                Some(("resource:coal".to_string(), b.smelting.coal_heat))
            } else {
                d.items
                    .keys()
                    .find(|id| {
                        id.strip_prefix("resource:")
                            .and_then(BlockType::from_name)
                            .is_some_and(BlockType::is_wood)
                    })
                    .map(|id| (id.clone(), b.smelting.wood_heat))
            };
            let Some((fuel, heat)) = fuel else {
                d.activity = if made {
                    Activity::Working
                } else {
                    Activity::MissingFuel
                };
                return;
            };
            take(&mut d.items, &fuel, 1);
            d.fuel_heat = heat - 1;
        }
        take(&mut d.items, &ore, 1);
        add(&mut d.output, metal, 1);
        d.quest_production[0]=d.quest_production[0].saturating_add(1);
        d.feedback(0);
        d.feedback(1);
        made = true;
    }
    d.activity = Activity::Working;
}

/// No wall-clock timestamps are stored: loading never earns offline production.
#[derive(Default)]
pub struct Clock {
    accumulator: f64,
}
impl Clock {
    #[cfg(test)]
    pub fn advance(
        &mut self,
        dt: f32,
        state: &mut State,
        b: &Balance,
        recipes: &Registry,
    ) -> usize {
        self.advance_with(dt, state, b, recipes, |_| {})
    }
    pub fn advance_with(
        &mut self, dt: f32, state: &mut State, b: &Balance, recipes: &Registry,
        mut apply: impl FnMut(&[(Cell, Kind)]),
    ) -> usize {
        if !dt.is_finite() || dt <= 0.0 {
            return 0;
        }
        self.accumulator += (dt as f64).min(1.0);
        let step = b.tick_ms as f64 / 1000.0;
        let mut ticks = 0;
        while self.accumulator + 1e-8 >= step {
            self.accumulator = (self.accumulator - step).max(0.0);
            apply(&state.step(b, recipes));
            ticks += 1;
        }
        ticks
    }
}
