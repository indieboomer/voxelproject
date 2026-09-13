//! Bounded land tasks driven only by player observations and normal movement inputs.
use super::{Action, Cell, Observation, Status};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "snake_case", deny_unknown_fields)]
pub enum Task {
    MoveTo {
        position: [f32; 3],
    },
    Mine {
        target: Cell,
    },
    Interact {
        target: Cell,
    },
    FollowAttack {
        creature: u32,
    },
    Place {
        target: Cell,
        support: Cell,
        block: crate::voxel::BlockType,
    },
}
impl Task {
    pub fn activity(&self) -> &'static str {
        match self {
            Self::MoveTo { .. } => "Moving to destination",
            Self::Mine { .. } => "Approaching / mining a block",
            Self::Interact { .. } => "Approaching / interacting",
            Self::FollowAttack { .. } => "Following / attacking a creature",
            Self::Place { .. } => "Approaching / placing a block",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Completed,
    Cancelled,
    Blocked,
    TargetLost,
    Stalled,
    Timeout,
    Invalid,
    ActionRejected,
}

pub struct Controller {
    pub task: Task,
    pub result: Option<(End, String)>,
    started: u64,
    last_progress: u64,
    last_position: Vec3,
    last_action: Action,
    path: VecDeque<Cell>,
}
fn cell(p: Vec3) -> Cell {
    (
        p.x.floor() as i32,
        (p.y + 0.02).floor() as i32,
        p.z.floor() as i32,
    )
}
fn feet(c: Cell) -> Vec3 {
    Vec3::new(c.0 as f32 + 0.5, c.1 as f32, c.2 as f32 + 0.5)
}

impl Controller {
    pub fn new(task: Task, observation: &Observation) -> Self {
        Self {
            task,
            result: None,
            started: observation.tick,
            last_progress: observation.tick,
            last_position: Vec3::from_array(observation.position),
            last_action: Action::Wait,
            path: VecDeque::new(),
        }
    }
    pub fn cancel(&mut self) {
        self.result = Some((End::Cancelled, "Cancelled by caller".into()));
        self.path.clear();
    }
    fn finish(&mut self, end: End, reason: &str) -> Action {
        self.result = Some((end, reason.into()));
        self.path.clear();
        Action::Wait
    }
    pub fn next(&mut self, o: &Observation) -> Action {
        if self.result.is_some() {
            return Action::Wait;
        }
        if o.tick.saturating_sub(self.started) >= 600 {
            return self.finish(End::Timeout, "Task exceeded 30 seconds");
        }
        let position = Vec3::from_array(o.position);
        if position.distance(self.last_position) > 0.15 {
            self.last_position = position;
            self.last_progress = o.tick;
        }
        if o.health <= 0.0 {
            return self.finish(End::ActionRejected, "Player is dead");
        }
        if o.oxygen < 90.0 {
            return self.finish(End::Blocked, "Land controller stopped after submersion");
        }
        if let Some(previous) = o.previous_action.as_ref().filter(|_| o.tick > self.started) {
            if previous.status == Status::Rejected && previous.reason != "Cooldown active" {
                return self.finish(End::ActionRejected, &previous.reason);
            }
            if previous.status == Status::Completed
                && matches!(
                    self.last_action,
                    Action::Mine { .. } | Action::Interact { .. } | Action::Place { .. }
                )
            {
                return self.finish(End::Completed, "Target action completed");
            }
        }
        let (destination, reach) = match self.task {
            Task::MoveTo { position } => {
                let p = Vec3::from_array(position);
                if !p.is_finite() || p.abs().max_element() > 1_000_000.0 {
                    return self.finish(End::Invalid, "Invalid destination");
                }
                if p.distance(Vec3::from_array(o.position)) < 0.25 {
                    return self.finish(End::Completed, "Destination reached");
                }
                (p, 0.0)
            }
            Task::Mine { target } | Task::Interact { target } => {
                if !o.terrain.iter().any(|t| t.cell == target) {
                    return self.finish(End::TargetLost, "Block is no longer visible");
                }
                (super::center(target), 3.5)
            }
            Task::FollowAttack { creature } => {
                let Some(c) = o.creatures.iter().find(|c| c.0 == creature) else {
                    return self.finish(End::TargetLost, "Creature disappeared or left sight");
                };
                (Vec3::from_array(c.2) + Vec3::Y * 0.6, 2.5)
            }
            Task::Place {
                target, support, ..
            } => {
                if [target, support].iter().any(|p| {
                    !(-1_000_000..=1_000_000).contains(&p.0)
                        || !(-1_000_000..=1_000_000).contains(&p.2)
                        || !(0..crate::voxel::chunk::CHUNK_Y).contains(&p.1)
                }) {
                    return self.finish(End::Invalid, "Invalid placement coordinates");
                }
                if (target.0 - support.0).abs()
                    + (target.1 - support.1).abs()
                    + (target.2 - support.2).abs()
                    != 1
                {
                    return self
                        .finish(End::Invalid, "Placement requires an adjacent support face");
                }
                if !o.terrain.iter().any(|t| t.cell == support) {
                    return self.finish(End::TargetLost, "Placement support is no longer visible");
                }
                (
                    super::center(support)
                        + (super::center(target) - super::center(support)) * 0.499,
                    4.5,
                )
            }
        };
        let eye = position + Vec3::Y * 1.62;
        if let Task::Place {
            target, support, ..
        } = self.task
        {
            if target.1 == support.1 + 1
                && eye.y < destination.y + 0.2
                && eye.distance(destination) < 4.5
            {
                self.last_action = Action::Move {
                    direction: [0.0; 3],
                    jump: true,
                };
                return self.last_action.clone();
            }
        }
        let action = if reach > 0.0 && eye.distance(destination) < reach {
            self.last_progress = o.tick;
            let direction = (destination - eye).normalize_or_zero();
            if Vec3::from_array(o.aim).dot(direction)
                < if matches!(self.task, Task::Place { .. }) {
                    0.995
                } else {
                    0.9999
                }
            {
                Action::Look {
                    direction: direction.to_array(),
                }
            } else {
                match self.task {
                    Task::Mine { target } => {
                        let block = o.terrain.iter().find(|t| t.cell == target).unwrap().block;
                        let item = Some(crate::equipment::Entry::Gear(block.required_tool()));
                        if o.equipment != item {
                            Action::Equip { item }
                        } else if o.tick % 4 == 0 {
                            Action::Mine { target }
                        } else {
                            Action::Wait
                        }
                    }
                    Task::Interact { target } => Action::Interact { target },
                    Task::Place {
                        target,
                        support,
                        block,
                    } => {
                        let item = Some(crate::equipment::Entry::Resource(block));
                        if o.equipment != item {
                            Action::Equip { item }
                        } else if o.tick % 4 == 0 {
                            Action::Place {
                                target: support,
                                destination: Some(target),
                            }
                        } else {
                            Action::Wait
                        }
                    }
                    Task::FollowAttack { .. } => {
                        let item =
                            Some(crate::equipment::Entry::Gear(crate::equipment::Gear::Sword));
                        if o.equipment != item {
                            Action::Equip { item }
                        } else if o.tick % 7 == 0 {
                            Action::Attack
                        } else {
                            Action::Wait
                        }
                    }
                    Task::MoveTo { .. } => unreachable!(),
                }
            }
        } else {
            if o.tick.saturating_sub(self.last_progress) > 40 {
                return self.finish(End::Stalled, "No movement progress for two seconds");
            }
            let known: BTreeSet<_> = o.walkable.iter().copied().collect();
            if self
                .path
                .front()
                .is_some_and(|p| position.distance(feet(*p)) < 0.25)
            {
                self.path.pop_front();
            }
            if self.path.is_empty() || self.path.iter().any(|p| !known.contains(p)) {
                self.path = match route(cell(position), destination, reach, &known) {
                    Some(path) => path,
                    None => {
                        return self.finish(
                            End::Blocked,
                            "No observed dry land path within the 512-node budget",
                        )
                    }
                };
            }
            let next = if reach == 0.0 && cell(position) == cell(destination) {
                destination
            } else if let Some(next) = self.path.front().copied() {
                feet(next)
            } else {
                return self.finish(End::Blocked, "No approach cell in reach");
            };
            let delta = next - position;
            let horizontal = Vec3::new(delta.x, 0.0, delta.z).normalize_or_zero();
            Action::Move {
                direction: horizontal.to_array(),
                jump: delta.y > 0.2,
            }
        };
        self.last_action = action.clone();
        action
    }
}

fn route(
    start: Cell,
    destination: Vec3,
    reach: f32,
    known: &BTreeSet<Cell>,
) -> Option<VecDeque<Cell>> {
    let mut parents = BTreeMap::new();
    parents.insert(start, start);
    let mut queue = VecDeque::from([start]);
    while let Some(at) = queue.pop_front() {
        let reached = if reach == 0.0 {
            at == cell(destination)
        } else {
            (feet(at) + Vec3::Y * 1.62).distance(destination) < reach
        };
        if reached {
            let mut path = VecDeque::new();
            let mut cursor = at;
            while cursor != start {
                path.push_front(cursor);
                cursor = parents[&cursor];
            }
            return Some(path);
        }
        if parents.len() >= 512 {
            return None;
        }
        for (dx, dz) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
            for dy in [0, 1, -1] {
                let next = (at.0 + dx, at.1 + dy, at.2 + dz);
                if known.contains(&next) && !parents.contains_key(&next) {
                    parents.insert(next, at);
                    queue.push_back(next);
                }
            }
        }
    }
    // Long journeys are executed as observed, reachable segments. A nearby
    // obstructed destination still fails instead of silently becoming another goal.
    if reach == 0.0 && feet(start).distance(destination) > 10.0 {
        let closest = parents
            .keys()
            .copied()
            .min_by_key(|p| (feet(*p).distance_squared(destination) * 100.0) as u64)?;
        if feet(closest).distance(destination) + 1.0 < feet(start).distance(destination) {
            let mut path = VecDeque::new();
            let mut cursor = closest;
            while cursor != start {
                path.push_front(cursor);
                cursor = parents[&cursor];
            }
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_around_walls_and_rejects_disconnected_targets() {
        let mut known: BTreeSet<_> = (0..7)
            .flat_map(|x| (0..7).map(move |z| (x, 60, z)))
            .collect();
        for z in 0..6 {
            known.remove(&(3, 60, z));
        }
        let path = route((1, 60, 1), feet((5, 60, 1)), 0.0, &known).unwrap();
        assert!(path.contains(&(3, 60, 6)));
        known.remove(&(3, 60, 6));
        assert!(route((1, 60, 1), feet((5, 60, 1)), 0.0, &known).is_none());
    }
}
