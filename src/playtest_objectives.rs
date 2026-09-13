//! Scenario success is determined from game state and accepted actions, never model prose.
use super::{Action, Cell, Effects, Outcome, Scenario, Status};
use crate::{
    creature::Creatures,
    player::Player,
    voxel::{BlockType, World},
};
use glam::Vec3;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Serialize)]
pub struct Objective {
    pub scenario: Scenario,
    pub home: [f32; 3],
    pub shelter: Vec<Cell>,
    pub placed: BTreeSet<Cell>,
    pub crafted_tool: bool,
    pub engaged_hostile: bool,
    pub left_home: bool,
    safe_since: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encounter_requires_real_engagement_return_and_safety() {
        let (mut world, mut creatures, mut loot, registry, mut actor) =
            super::super::tests::fixture();
        let home = actor.player.position;
        let mut objective = Objective::new(Scenario::AiEncounterReturn, home);
        assert!(!objective.complete(&world, &actor.player, &creatures, 100));
        let enemy = creatures.spawn_one(
            crate::creature::CreatureKind::Skeleton,
            home + Vec3::X * 4.0,
            123,
        );
        let mut context = super::super::Context {
            world: &mut world,
            creatures: &mut creatures,
            loot: &mut loot,
            registry: &registry,
            players: &[],
        };
        let mut departure = super::super::controller::Controller::new(
            super::super::controller::Task::MoveTo {
                position: (home + Vec3::X * 2.5).to_array(),
            },
            &actor.observe(&context),
        );
        for _ in 0..50 {
            let action = departure.next(&actor.observe(&context));
            let (outcome, effects) = actor.act(&mut context, &action);
            objective.record(&action, &outcome, &effects, false, &actor.player);
            if departure.result.is_some() {
                break;
            }
        }
        let mut task = super::super::controller::Controller::new(
            super::super::controller::Task::FollowAttack { creature: enemy },
            &actor.observe(&context),
        );
        for _ in 0..200 {
            let action = task.next(&actor.observe(&context));
            let target = context.creatures.weapon_target(
                context.world,
                actor.player.position + Vec3::Y * 1.62,
                actor.aim,
                3.2,
            ) == Some(enemy);
            let (outcome, effects) = actor.act(&mut context, &action);
            objective.record(&action, &outcome, &effects, target, &actor.player);
            for (_, damage) in context.creatures.update(
                context.world,
                super::super::STEP,
                &[(super::super::AGENT_ID, actor.player.position)],
            ) {
                actor.player.damage(damage);
            }
            if !context
                .creatures
                .snapshot_with_ids()
                .iter()
                .any(|c| c.0 == enemy)
            {
                break;
            }
        }
        assert!(
            objective.engaged_hostile && objective.left_home,
            "engaged {}, departed {}, controller {:?}, position {:?}",
            objective.engaged_hostile,
            objective.left_home,
            task.result,
            actor.player.position
        );
        let mut task = super::super::controller::Controller::new(
            super::super::controller::Task::MoveTo {
                position: home.to_array(),
            },
            &actor.observe(&context),
        );
        for _ in 0..200 {
            let action = task.next(&actor.observe(&context));
            let (outcome, effects) = actor.act(&mut context, &action);
            objective.record(&action, &outcome, &effects, false, &actor.player);
            if objective.complete(context.world, &actor.player, context.creatures, actor.tick) {
                return;
            }
        }
        panic!("Encounter did not complete: health {}, position {:?}, objective engaged {}, departed {}",actor.player.health,actor.player.position,objective.engaged_hostile,objective.left_home);
    }
}
impl Objective {
    pub fn new(scenario: Scenario, home: Vec3) -> Self {
        let (x, y, z) = (
            home.x.floor() as i32,
            home.y.floor() as i32,
            home.z.floor() as i32,
        );
        let mut shelter = vec![];
        if scenario == Scenario::AiShelter {
            for dy in 0..2 {
                for dx in -1i32..=1 {
                    for dz in -1i32..=1 {
                        if (dx.abs() == 1 || dz.abs() == 1) && (dx, dz) != (0, 1) {
                            shelter.push((x + dx, y + dy, z + dz));
                        }
                    }
                }
            }
            for dx in -1..=1 {
                for dz in -1..=1 {
                    if (dx, dz) != (0, 0) {
                        shelter.push((x + dx, y + 2, z + dz));
                    }
                }
            }
            shelter.push((x, y + 2, z));
            shelter.sort_by_key(|p| {
                (
                    p.1,
                    if (p.0 - x).abs() == 1 && (p.2 - z).abs() == 1 {
                        0
                    } else if (p.0, p.2) == (x, z) {
                        2
                    } else {
                        1
                    },
                )
            });
        }
        Self {
            scenario,
            home: home.to_array(),
            shelter,
            placed: BTreeSet::new(),
            crafted_tool: false,
            engaged_hostile: false,
            left_home: false,
            safe_since: None,
        }
    }
    pub fn validate_site(&self, world: &World) -> Result<(), String> {
        let h = Vec3::from_array(self.home);
        let (x, y, z) = (h.x.floor() as i32, h.y.floor() as i32, h.z.floor() as i32);
        for dx in -1..=1 {
            for dz in -1..=1 {
                if !world.get_block(x + dx, y - 1, z + dz).is_solid()
                    || (0..3).any(|dy| world.get_block(x + dx, y + dy, z + dz) != BlockType::Air)
                {
                    return Err("Shelter needs a flat, clear 3 x 3 patch around Agent1's spawn, with three blocks of headroom".into());
                }
            }
        }
        Ok(())
    }
    pub fn protected(&self, p: Cell) -> bool {
        if self.scenario != Scenario::AiShelter {
            return false;
        }
        let h = Vec3::from_array(self.home);
        (p.0 - h.x.floor() as i32).abs() <= 1
            && (p.2 - h.z.floor() as i32).abs() <= 1
            && (h.y.floor() as i32 - 1..=h.y.floor() as i32 + 2).contains(&p.1)
    }
    pub fn record(
        &mut self,
        action: &Action,
        result: &Outcome,
        effects: &Effects,
        hostile_target: bool,
        player: &Player,
    ) {
        self.left_home |= player.position.distance(Vec3::from_array(self.home)) >= 2.0;
        if result.status != Status::Completed {
            return;
        }
        self.crafted_tool |= matches!(
            action,
            Action::Craft {
                recipe: crate::crafting::Action::CraftGear(_)
            }
        );
        self.engaged_hostile |= matches!(action, Action::Attack) && hostile_target;
        if matches!(action, Action::Place { .. }) {
            for (p, b, _) in &effects.edits {
                if b.is_solid() {
                    self.placed.insert(*p);
                }
            }
        }
    }
    pub fn complete(
        &mut self,
        world: &World,
        player: &Player,
        creatures: &Creatures,
        tick: u64,
    ) -> bool {
        if player.health <= 0.0 {
            return false;
        }
        match self.scenario {
            Scenario::AiGatherTool => self.crafted_tool,
            Scenario::AiShelter => {
                let h = Vec3::from_array(self.home);
                let (x, y, z) = (h.x.floor() as i32, h.y.floor() as i32, h.z.floor() as i32);
                self.shelter
                    .iter()
                    .all(|p| self.placed.contains(p) && world.get_block(p.0, p.1, p.2).is_solid())
                    && (0..2).all(|dy| {
                        world.get_block(x, y + dy, z) == BlockType::Air
                            && world.get_block(x, y + dy, z + 1) == BlockType::Air
                    })
                    && (-1..=1).all(|dx| {
                        (-1..=1).all(|dz| world.get_block(x + dx, y - 1, z + dz).is_solid())
                    })
            }
            Scenario::AiEncounterReturn => {
                let safe = self.left_home
                    && self.engaged_hostile
                    && player.position.distance(Vec3::from_array(self.home)) < 0.75
                    && !creatures.snapshot_with_ids().iter().any(|c| {
                        crate::creature::CreatureKind::from_u8(c.1).is_hostile()
                            && Vec3::from_array(c.2).distance(player.position) < 4.0
                    });
                if !safe {
                    self.safe_since = None;
                    return false;
                }
                tick.saturating_sub(*self.safe_since.get_or_insert(tick)) >= 40
            }
            _ => false,
        }
    }
}
