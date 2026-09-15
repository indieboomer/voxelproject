//! Bounded recipient queues. Admission reserves a full invocation, never retries
//! an already executed callback to recover from shared-budget exhaustion.
use super::*;
use crate::world_api_gen::*;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) enum Event {
    Tick,
    Cast(PlayerId),
    TargetedCast(PlayerId, crate::spell_target::TargetContext),
    Death(DeathEvent),
    BlockBreak(BlockBreakEvent),
    Interact(InteractEvent),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paid_cast_tracks_its_own_success_and_busy_queue() {
        let mut host = ScriptHost::new();
        host.modules.push(rule("function on_cast(api,e) if e.player_id==7 then error('failed') end end"));
        let world = World::new(1);
        let mut creatures = Creatures::new();
        let mut weather = WeatherState::new(1);
        let mut time = 0.0;
        assert!(host.can_cast_immediately());
        let out = host.run_cast(0,&world,&mut creatures,&[],&mut time,&mut weather,0,[0;COLLECTIBLE_BLOCKS.len()]);
        assert!(out.crashes.is_empty());
        assert!(host.cast_succeeded(0,0));
        let out = host.run_cast(0,&world,&mut creatures,&[],&mut time,&mut weather,7,[0;COLLECTIBLE_BLOCKS.len()]);
        assert!(!out.crashes.is_empty());
        assert!(!host.cast_succeeded(0,7));
        assert!(!host.cast_succeeded(0,0));
        host.enqueue_cast(0,0);
        assert!(!host.can_cast_immediately());
    }

    fn rule(source: &str) -> Module {
        let mut m = Module::load("test".into(), String::new(), source.into()).unwrap();
        m.enabled = true;
        m
    }

    fn dispatch_one(host: &mut ScriptHost) -> TickOutcome {
        let mut out = TickOutcome::default();
        let mut creatures = Creatures::new();
        let world = World::new(1);
        let mut weather = WeatherState::new(1);
        let mut time = 0.0;
        let mut deaths = Vec::new();
        let mut budget = DispatchBudget::new();
        budget.callbacks = DISPATCH_CALLBACKS - 1;
        let mut input = TickInput {
            creatures: &mut creatures,
            world: &world,
            weather: &mut weather,
            time_of_day: &mut time,
            players: &[],
            host_resources: [0; COLLECTIBLE_BLOCKS.len()],
            block_edits: &mut out.block_edits,
            death_events: &mut deaths,
            broadcasts: &mut out.broadcasts,
            player_effects: &mut out.player_effects,
        };
        (out.crashes, out.warnings) = host.dispatch_with_budget(&mut input, budget);
        out
    }

    #[test]
    fn deferred_casts_keep_fifo_order_and_rotate_between_recipients() {
        let mut host = ScriptHost::new();
        for name in ["a", "b", "c"] {
            host.modules.push(rule(&format!(
                "function on_cast(api,e) api.broadcast('{name}'..e.player_id) end"
            )));
        }
        for index in 0..3 {
            for caster in 1..=3 {
                host.enqueue_cast(index, caster);
            }
        }
        let mut messages = Vec::new();
        while host.scheduler.pending > 0 {
            let out = dispatch_one(&mut host);
            assert!(out.crashes.is_empty());
            assert_eq!(out.broadcasts.len(), 1);
            messages.extend(out.broadcasts);
        }
        assert_eq!(
            messages,
            ["a1", "b1", "c1", "a2", "b2", "c2", "a3", "b3", "c3"]
        );
        assert!(dispatch_one(&mut host).broadcasts.is_empty());
    }

    #[test]
    fn tick_backlog_coalesces_and_new_rules_do_not_receive_old_events() {
        let mut host = ScriptHost::new();
        host.modules
            .push(rule("function on_tick(api) api.broadcast('tick') end"));
        for _ in 0..100 {
            host.enqueue_tick_events(&[], &[]);
        }
        assert_eq!(host.scheduler.pending, 1);
        host.modules
            .push(rule("function on_tick(api) api.broadcast('new') end"));
        assert_eq!(dispatch_one(&mut host).broadcasts, ["tick"]);
        assert!(dispatch_one(&mut host).broadcasts.is_empty());
    }

    #[test]
    fn removal_and_reactivation_cancel_old_recipient_work() {
        let mut host = ScriptHost::new();
        host.modules
            .push(rule("function on_tick(api) api.broadcast('removed') end"));
        host.modules
            .push(rule("function on_tick(api) api.broadcast('disabled') end"));
        host.enqueue_tick_events(&[], &[]);
        host.remove(0);
        host.toggle_at(0);
        host.toggle_at(0);
        assert_eq!(host.scheduler.pending, 0);
        assert!(dispatch_one(&mut host).broadcasts.is_empty());
    }

    #[test]
    fn queue_limits_drop_newest_and_warn_once_while_saturated() {
        let mut host = ScriptHost::new();
        for _ in 0..SCRIPT_MODULES_MAX {
            host.modules.push(rule(
                "function on_cast(api,e) api.broadcast(e.player_id) end",
            ));
        }
        for index in 0..SCRIPT_MODULES_MAX {
            for id in 0..(PENDING_PER_MODULE + 1) {
                host.enqueue_cast(index, id as PlayerId);
            }
        }
        assert_eq!(host.scheduler.pending, PENDING_TOTAL);
        assert!(host
            .scheduler
            .ready
            .iter()
            .all(|q| q.events.len() <= PENDING_PER_MODULE));
        assert_eq!(dispatch_one(&mut host).warnings.len(), 1);
        host.enqueue_cast(1, 999);
        assert!(dispatch_one(&mut host).warnings.is_empty());
    }

    #[test]
    fn admission_reserves_full_allowance_for_every_resource_and_reload() {
        let mut b = DispatchBudget::new();
        assert!(b.admits(true));
        b.work.instructions = DISPATCH_INSTRUCTIONS - SCRIPT_INSTRUCTIONS;
        assert!(b.admits(false));
        assert!(!b.admits(true));
        b.work.instructions += 1;
        assert!(!b.admits(false));
        b = DispatchBudget::new();
        b.work.api_calls = DISPATCH_API_CALLS - SCRIPT_API_CALLS + 1;
        assert!(!b.admits(false));
        b = DispatchBudget::new();
        b.work.native_work = DISPATCH_NATIVE_WORK - SCRIPT_NATIVE_WORK + 1;
        assert!(!b.admits(false));
        b = DispatchBudget::new();
        b.start -= Duration::from_millis(DISPATCH_TIME_MS);
        assert!(!b.admits(false));
    }

    #[test]
    fn saved_overflow_and_invalid_source_survive_roundtrip() {
        let entry = rule("function on_tick(api) end").to_save_entry();
        let mut entries = vec![entry; SCRIPT_MODULES_MAX + 2];
        entries[0].source = "this is invalid Lua".into();
        let host = ScriptHost::load_from_save(&entries);
        assert_eq!(host.modules.len(), SCRIPT_MODULES_MAX);
        assert_eq!(host.unloaded_entries.len(), 2);
        let saved = host.save_entries();
        assert_eq!(saved.len(), entries.len());
        assert!(saved.iter().any(|e| e.source == entries[0].source));
    }

    #[test]
    fn death_notification_defers_once_without_replaying_its_cause() {
        let mut host = ScriptHost::new();
        host.modules.push(rule("function on_cast(api) local id=api.spawn_creature('sheep',1,30,2); api.damage(id,999); api.broadcast('cause') end"));
        host.modules.push(rule("function on_tick(api) end; function on_death(api,e) api.broadcast(e.kind..':'..e.x..':'..e.z) end"));
        host.enqueue_cast(0, 0);
        let cause = dispatch_one(&mut host);
        assert!(cause.crashes.is_empty(), "{:?}", cause.crashes);
        assert_eq!(cause.broadcasts, ["cause"]);
        assert_eq!(host.scheduler.pending, 1);
        let effect = dispatch_one(&mut host);
        assert!(effect.crashes.is_empty());
        assert_eq!(effect.broadcasts, ["sheep:1.0:2.0"]);
        assert_eq!(host.scheduler.pending, 0);
        assert!(dispatch_one(&mut host).broadcasts.is_empty());
    }

    #[test]
    fn failed_cast_rebuilds_even_if_it_erased_its_entrypoint() {
        let mut host = ScriptHost::new();
        host.modules.push(rule("function on_cast(api,e) if e.player_id == 1 then on_cast=nil; error('broken') end; api.broadcast('retry') end"));
        host.enqueue_cast(0, 1);
        host.enqueue_cast(0, 2);
        assert_eq!(dispatch_one(&mut host).crashes.len(), 1);
        assert_eq!(host.scheduler.pending, 1);
        let retry = dispatch_one(&mut host);
        assert!(retry.crashes.is_empty());
        assert_eq!(retry.broadcasts, ["retry"]);
    }
}

impl Event {
    fn callback(&self) -> Callback<'_> {
        match self {
            Self::Tick => Callback::Tick,
            Self::Cast(id) => Callback::Cast(*id),
            Self::TargetedCast(id, context) => Callback::TargetedCast(*id, *context),
            Self::Death(e) => Callback::Death(e),
            Self::BlockBreak(e) => Callback::BlockBreak(e),
            Self::Interact(e) => Callback::Interact(e),
        }
    }
    fn accepts(self, m: &Module) -> bool {
        (if matches!(self, Self::Cast(_) | Self::TargetedCast(..)) {
            m.is_instant
        } else {
            m.enabled && !m.is_instant
        }) && (m.needs_reload
            || !matches!(
                m.lua.globals().raw_get::<_, Value>(self.callback().name()),
                Ok(Value::Nil)
            ))
    }
}

struct Queue {
    id: u64,
    epoch: u64,
    events: VecDeque<Event>,
}

#[derive(Default)]
pub(super) struct Scheduler {
    ready: VecDeque<Queue>,
    pending: usize,
    dropped: bool,
    warned: bool,
    reported_unloaded: bool,
}

impl Scheduler {
    pub fn cancel(&mut self, id: u64) {
        self.ready.retain(|q| q.id != id);
        self.pending = self.ready.iter().map(|q| q.events.len()).sum();
    }

    fn enqueue(&mut self, module: &Module, event: Event) {
        if !event.accepts(module) {
            return;
        }
        let existing = self.ready.iter().position(|q| q.id == module.runtime_id);
        if let Some(index) = existing {
            let q = &self.ready[index];
            if matches!(event, Event::Tick) && q.events.iter().any(|e| matches!(e, Event::Tick)) {
                return;
            }
            if q.events.len() >= PENDING_PER_MODULE {
                self.dropped = true;
                return;
            }
        }
        if self.pending >= PENDING_TOTAL {
            self.dropped = true;
            return;
        }
        let index = existing.unwrap_or_else(|| {
            self.ready.push_back(Queue {
                id: module.runtime_id,
                epoch: module.activation_epoch,
                events: VecDeque::new(),
            });
            self.ready.len() - 1
        });
        self.ready[index].events.push_back(event);
        self.pending += 1;
    }

    fn fanout(&mut self, modules: &[Module], event: Event) {
        for module in modules {
            self.enqueue(module, event);
        }
    }

    fn notice(&mut self, outcome: &mut TickOutcome) {
        if self.dropped && !self.warned {
            outcome.warnings.push("Rule event queue is full; newest events were dropped. Reduce active rules or event frequency.".into());
        }
        if self.dropped {
            self.warned = true;
        } else if self.pending == 0 {
            self.warned = false;
        }
        self.dropped = false;
    }
}

struct DispatchBudget {
    start: Instant,
    callbacks: usize,
    work: WorkUsage,
}

impl DispatchBudget {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            callbacks: 0,
            work: WorkUsage::default(),
        }
    }
    fn admits(&self, reload: bool) -> bool {
        let invocations = 1 + u32::from(reload);
        self.callbacks < DISPATCH_CALLBACKS
            && self.work.instructions + invocations * SCRIPT_INSTRUCTIONS <= DISPATCH_INSTRUCTIONS
            && self.work.api_calls + SCRIPT_API_CALLS <= DISPATCH_API_CALLS
            && self.work.native_work + SCRIPT_NATIVE_WORK <= DISPATCH_NATIVE_WORK
            && self.start.elapsed() + Duration::from_millis(u64::from(invocations) * SCRIPT_TIME_MS)
                <= Duration::from_millis(DISPATCH_TIME_MS)
    }
    fn record(&mut self, work: WorkUsage) {
        self.callbacks += 1;
        self.work.instructions += work.instructions;
        self.work.api_calls += work.api_calls;
        self.work.native_work += work.native_work;
    }
}

impl ScriptHost {
    pub(super) fn enqueue_combat_death(&mut self, death: DeathEvent) {
        self.scheduler.fanout(&self.modules, Event::Death(death));
    }

    pub(super) fn enqueue_tick_events(
        &mut self,
        breaks: &[BlockBreakEvent],
        interacts: &[InteractEvent],
    ) {
        // Deletions, disable/reactivation and replacement cannot redirect old events.
        self.scheduler.ready.retain(|q| {
            self.modules.iter().any(|m| {
                m.runtime_id == q.id && m.activation_epoch == q.epoch && (m.enabled || m.is_instant)
            })
        });
        self.scheduler.pending = self.scheduler.ready.iter().map(|q| q.events.len()).sum();
        self.scheduler.fanout(&self.modules, Event::Tick);
        for event in breaks.iter().take(PENDING_TOTAL) {
            self.scheduler
                .fanout(&self.modules, Event::BlockBreak(*event));
        }
        for event in interacts.iter().take(PENDING_TOTAL) {
            self.scheduler
                .fanout(&self.modules, Event::Interact(*event));
        }
        self.scheduler.dropped |= breaks.len() > PENDING_TOTAL || interacts.len() > PENDING_TOTAL;
    }

    pub(super) fn enqueue_cast(&mut self, index: usize, caster: PlayerId) {
        if let Some(module) = self.modules.get(index) {
            self.scheduler.enqueue(module, Event::Cast(caster));
        }
    }
    pub fn can_cast_immediately(&self) -> bool { self.scheduler.pending == 0 }
    pub(super) fn enqueue_targeted_cast(&mut self, index: usize, caster: PlayerId, context: crate::spell_target::TargetContext) {
        if let Some(module) = self.modules.get(index) {
            self.scheduler.enqueue(module, Event::TargetedCast(caster, context));
        }
    }
    pub fn cast_succeeded(&self, index: usize, caster: PlayerId) -> bool {
        self.modules.get(index).is_some_and(|m|self.last_cast_success==Some((m.runtime_id,caster)))
    }

    pub(super) fn dispatch(&mut self, input: &mut TickInput) -> (Vec<String>, Vec<String>) {
        self.dispatch_with_budget(input, DispatchBudget::new())
    }

    fn dispatch_with_budget(
        &mut self,
        input: &mut TickInput,
        mut budget: DispatchBudget,
    ) -> (Vec<String>, Vec<String>) {
        let mut outcome = TickOutcome::default();
        if !self.unloaded_entries.is_empty() && !self.scheduler.reported_unloaded {
            outcome.warnings.push(format!("{} saved rules could not be loaded (module limit or load error). Their source remains preserved in the save.", self.unloaded_entries.len()));
            self.scheduler.reported_unloaded = true;
        }
        while let Some(mut queue) = self.scheduler.ready.pop_front() {
            let Some(module) = self
                .modules
                .iter_mut()
                .find(|m| m.runtime_id == queue.id && m.activation_epoch == queue.epoch)
            else {
                self.scheduler.pending -= queue.events.len();
                continue;
            };
            let event = *queue.events.front().unwrap();
            if !event.accepts(module) {
                queue.events.pop_front();
                self.scheduler.pending -= 1;
                if !queue.events.is_empty() {
                    self.scheduler.ready.push_back(queue);
                }
                continue;
            }
            if !budget.admits(module.needs_reload) {
                self.scheduler.ready.push_front(queue);
                break;
            }
            queue.events.pop_front();
            self.scheduler.pending -= 1;
            if !queue.events.is_empty() {
                self.scheduler.ready.push_back(queue);
            }
            module.lua.set_app_data(self.inventory_registry.clone());
            let result = module.execute(input, event.callback());
            budget.record(module.last_work);
            match result {
                Ok(()) => {
                    if let Event::Cast(caster) | Event::TargetedCast(caster, _) = event {
                        self.last_cast_success = Some((module.runtime_id,caster));
                        module.error = None;
                    }
                }
                Err(error) => {
                    if matches!(event, Event::Cast(_) | Event::TargetedCast(..)) {
                        module.error = Some(error.to_string());
                        outcome
                            .crashes
                            .push(format!("Spell '{}' failed: {}", module.name, error));
                    } else {
                        module.disable(error);
                        outcome.crashes.push(crash_message(module));
                        self.scheduler.cancel(module.runtime_id);
                    }
                }
            }
            for death in input.death_events.drain(..) {
                self.scheduler.fanout(&self.modules, Event::Death(death));
            }
        }
        self.scheduler.notice(&mut outcome);
        (outcome.crashes, outcome.warnings)
    }
}
