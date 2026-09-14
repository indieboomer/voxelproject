use super::*;
use crate::adventure::{self, Action};

impl App {
    pub(super) fn adventure_actor_alive(&self, from: Option<Peer>) -> bool {
        if let Some(peer) = from {
            let NetRole::Host(host) = &self.net else {
                return false;
            };
            host.clients
                .get(&peer)
                .and_then(|id| host.remote_players.get(id))
                .is_some_and(|p| p.health > 0.)
        } else {
            self.player.health > 0.
        }
    }
    pub(super) fn submit_camp_action(&mut self, action: Action) {
        self.ui.journal.feedback = "Using the campfire…".into();
        if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(
                &client.socket,
                client.server_addr,
                ReliableMsg::CampAction(action),
            );
        } else {
            self.perform_camp_action(None, action);
        }
    }
    pub(super) fn perform_camp_action(&mut self, from: Option<Peer>, action: Action) {
        let (id, position, health, key) = if let Some(peer) = from {
            let NetRole::Host(host) = &self.net else {
                return;
            };
            let Some(&id) = host.clients.get(&peer) else {
                return;
            };
            let Some(rp) = host.remote_players.get(&id) else {
                return;
            };
            (id, rp.pos, rp.health, Some(peer.account_key(&rp.nickname)))
        } else {
            (
                HOST_PLAYER_ID,
                self.player.position,
                self.player.health,
                None,
            )
        };
        crate::crafting::load_interaction_area(&mut self.world, position);
        let threatened = self.creatures.snapshot_with_ids().iter().any(|c| {
            crate::creature::CreatureKind::from_u8(c.1).is_hostile()
                && Vec3::from_array(c.2).distance_squared(position) < 100.
        });
        let account = if let Some(key) = key {
            self.guest_accounts.entry(key).or_default()
        } else {
            &mut self.player.crafting
        };
        let result =
            adventure::transact(&self.world, account, position, health, threatened, action);
        let success = result.is_ok();
        let message = result.unwrap_or_else(|e| e);
        let updated = account.clone();
        if success && matches!(action, Action::Rest { .. }) {
            self.apply_player_effect(PlayerEffect::Health {
                player_id: id,
                delta: MAX_HEALTH,
            });
            self.apply_player_effect(PlayerEffect::Poisoned {
                player_id: id,
                poisoned: false,
            });
        }
        if let Some(peer) = from {
            if let NetRole::Host(host) = &mut self.net {
                host.reliable.send(
                    &host.socket,
                    peer,
                    ReliableMsg::CraftState {
                        account: updated,
                        feedback: None,
                    },
                );
                host.reliable
                    .send(&host.socket, peer, ReliableMsg::CampResult(message));
            }
        } else {
            self.ui.journal.feedback = message.clone();
            self.notify_important(message);
        }
    }

    pub(super) fn initialize_adventure(&mut self, fresh: bool) {
        if !matches!(self.net, NetRole::Host(_)) {
            return;
        }
        if !fresh && self.world.starter_camp.is_none() {
            adventure::restore_starter_camp(&mut self.world);
        }
        if self.player.crafting.adventure.home.is_none() {
            let home = adventure::respawn_position(
                &mut self.world,
                Some(adventure::cell(self.player.position)),
            );
            self.player.crafting.adventure.home = adventure::clear_feet(&self.world,adventure::cell(home)).then_some(adventure::cell(home));
            self.player.crafting.revision = self.player.crafting.revision.saturating_add(1);
        }
        if fresh {
            // A real, mineable campfire provides a readable first destination. Only
            // clear natural decorations; never excavate player construction or grants.
            if let Some((x, y, z)) = adventure::starter_camp(&mut self.world, self.player.position)
            {
                for a in -2..=2 {
                    for b in -2..=2 {
                        if a == 0 || b == 0 {
                            self.apply_block_edit(x + a, y, z + b, BlockType::Air);
                        }
                    }
                }
                self.apply_block_edit(x, y, z, BlockType::Campfire);
                self.world.starter_camp = Some((x, y, z));
                self.ui.map.waypoint = Some(adventure::feet((x, y, z)));
                self.notify_important("A campkeeper waits by the nearby fire. Press F to talk, or J for your field journal.".into());
            } else {
                self.notify_important("Open J for your field journal. Find a natural campfire, or create one with a world spell.".into());
            }
        }
    }

    pub(super) fn update_adventure(&mut self, dt: f32) {
        self.ui.journal.camp_has_keeper = self.ui.journal.camp
            .is_some_and(|camp| adventure::guide_position(&self.world, camp).is_some());
        self.ui.journal.tick(self.player.health, dt);
        if !matches!(self.net, NetRole::Host(_)) {
            self.ui.journal.recovery_seconds = if self.player.health <= 0. {
                Some((self.ui.journal.recovery_seconds.unwrap_or(3.) - dt).max(0.))
            } else {
                None
            };
            return;
        }
        let mut changed = adventure::observe(
            &self.world,
            &mut self.player.crafting,
            self.player.position,
            self.player.health,
        );
        let mut players = vec![(
            HOST_PLAYER_ID,
            self.player.health,
            self.player.crafting.adventure.home,
        )];
        if let NetRole::Host(host) = &self.net {
            for (&peer, &id) in &host.clients {
                if let Some(rp) = host.remote_players.get(&id) {
                    let account = self
                        .guest_accounts
                        .entry(peer.account_key(&rp.nickname))
                        .or_default();
                    if account.adventure.home.is_none() {
                        account.adventure.home = self.player.crafting.adventure.home;
                        account.revision = account.revision.saturating_add(1);
                        changed = true;
                    }
                    changed |= adventure::observe(&self.world, account, rp.pos, rp.health);
                    players.push((id, rp.health, account.adventure.home));
                }
            }
        }
        self.recovery_timers
            .retain(|id, _| players.iter().any(|p| p.0 == *id));
        self.recovery_arrivals
            .retain(|id, _| players.iter().any(|p| p.0 == *id));
        for (id, health, home) in players {
            if health > 0. {
                self.recovery_timers.remove(&id);
                continue;
            }
            let timer = self.recovery_timers.entry(id).or_insert(3.);
            *timer -= dt;
            if *timer > 0. {
                continue;
            }
            let destination = adventure::respawn_position(&mut self.world, home);
            self.apply_player_effect(PlayerEffect::Teleport {
                player_id: id,
                pos: destination,
            });
            self.apply_player_effect(PlayerEffect::Health {
                player_id: id,
                delta: MAX_HEALTH,
            });
            self.apply_player_effect(PlayerEffect::Poisoned {
                player_id: id,
                poisoned: false,
            });
            self.apply_player_effect(PlayerEffect::SpeedMultiplier {
                player_id: id,
                multiplier: 1.,
            });
            self.apply_player_effect(PlayerEffect::JumpMultiplier {
                player_id: id,
                multiplier: 1.,
            });
            self.creatures.protect_recovery(id);
            self.interaction_states.remove(&id);
            if id == HOST_PLAYER_ID {
                self.player.oxygen = MAX_OXYGEN;
                self.player.crafting.adventure.recoveries =
                    self.player.crafting.adventure.recoveries.saturating_add(1);
                self.player.crafting.revision = self.player.crafting.revision.saturating_add(1);
                self.ui.journal.open = false;
                self.sync_settings_input();
                self.notify_important(
                    "Recovered at camp. Inventory kept; 10 seconds of creature protection.".into(),
                );
            } else if let NetRole::Host(host) = &mut self.net {
                if let Some((&peer, _)) = host.clients.iter().find(|(_, pid)| **pid == id) {
                    if let Some(rp) = host.remote_players.get_mut(&id) {
                        rp.oxygen = MAX_OXYGEN;
                        let account = self
                            .guest_accounts
                            .entry(peer.account_key(&rp.nickname))
                            .or_default();
                        account.adventure.recoveries =
                            account.adventure.recoveries.saturating_add(1);
                        account.revision = account.revision.saturating_add(1);
                    }
                    host.reliable.send(
                        &host.socket,
                        peer,
                        ReliableMsg::Recovered {
                            pos: destination.to_array(),
                        },
                    );
                    self.recovery_arrivals.insert(id, destination);
                }
            }
            changed = true;
            self.recovery_timers.remove(&id);
        }
        self.ui.journal.recovery_seconds = self.recovery_timers.get(&HOST_PLAYER_ID).copied();
        if changed {
            self.sync_guest_mana();
        }
    }

    pub(super) fn nearby_guides(&self) -> Vec<(adventure::Cell, Vec3)> {
        let eye = self.camera.eye_position();
        let mut camps: Vec<_> = self
            .campfires
            .values()
            .flatten()
            .copied()
            .filter(|p| p.distance_squared(eye) < 32. * 32.)
            .collect();
        camps.sort_by(|a, b| a.distance_squared(eye).total_cmp(&b.distance_squared(eye)));
        camps
            .into_iter()
            .filter_map(|p| {
                let camp = adventure::cell(p);
                adventure::guide_position(&self.world, camp).map(|p| (camp, adventure::feet(p)))
            })
            .take(4)
            .collect()
    }
    pub(super) fn aimed_camp(&self) -> Option<adventure::Cell> {
        let eye = self.camera.eye_position();
        let direction = self.camera.forward();
        if let Some(hit) = raycast(&self.world, eye, direction, REACH) {
            if self
                .world
                .get_block(hit.target.0, hit.target.1, hit.target.2)
                == BlockType::Campfire
            {
                return Some(hit.target);
            }
        }
        self.nearby_guides().into_iter().find_map(|(camp, p)| {
            let aim = p + Vec3::Y;
            let distance = eye.distance(aim);
            (distance < 6.
                && direction.dot((aim - eye).normalize_or_zero()) > 0.965
                && raycast(&self.world, eye, aim - eye, (distance - 0.4).max(0.)).is_none())
            .then_some(camp)
        })
    }
    pub(super) fn update_adventure_hints(&mut self) {
        self.ui.journal.hint = None;
        self.ui.journal.target = None;
        if !self.cursor_grabbed || self.player.health <= 0. {
            return;
        }
        if let Some(book)=self.aimed_book() {
            self.ui.journal.hint=Some(format!("F - Read {} (4 recipes)",crate::gear_catalog::BOOK_NAMES[book.kind as usize]));return;
        }
        if let Some(npc)=self.aimed_npc() {
            self.ui.journal.hint=Some(format!("F · Talk to {}",crate::quests::NAMES[npc as usize]));return;
        }
        if let Some(camp) = self.aimed_camp() {
            self.ui.journal.hint = Some(if adventure::guide_position(&self.world, camp).is_some() { format!(
                "F · Talk to {} / rest at camp",
                adventure::guide_name(camp)
            ) } else { "F · Cook / rest at camp".into() });
            return;
        }
        let eye = self.camera.eye_position();
        let direction = self.camera.forward();
        let hit = raycast(&self.world, eye, direction, REACH);
        let mut creatures = match &self.net {
            NetRole::Host(_) => self.creatures.snapshot(),
            NetRole::Joined(c) => c.creature_snapshot.clone(),
        };
        creatures.sort_by(|a, b| {
            Vec3::from_array(a.0)
                .distance_squared(eye)
                .total_cmp(&Vec3::from_array(b.0).distance_squared(eye))
        });
        // Health and poses come from the same authoritative snapshot on clients.
        for mut c in creatures {
            c.1 &= 0x0f; // Render snapshots pack undead appearance in the high nibble.
            let p = Vec3::from_array(c.0) + Vec3::Y * 0.7;
            let distance = eye.distance(p);
            if distance > 8.
                || direction.dot((p - eye).normalize_or_zero()) < 0.97
                || raycast(&self.world, eye, p - eye, (distance - 0.4).max(0.)).is_some()
            {
                continue;
            }
            let kind = crate::creature::CreatureKind::from_u8(c.1);
            let name = crate::worldgen::CREATURE_SPECIES[kind.to_u8() as usize].replace('_', " ");
            let health = match &self.net {
                NetRole::Host(_) => self
                    .creatures
                    .snapshot_with_ids()
                    .into_iter()
                    .find(|v| v.1 == c.1 && v.2 == c.0)
                    .map(|v| v.3),
                NetRole::Joined(client) => client
                    .creature_vitals
                    .iter()
                    .find(|v| v.0 == c.0 && v.1 == c.1)
                    .map(|v| v.2),
            };
            if let Some(health) = health {
                self.ui.journal.target = Some((name, health, kind.max_health()));
            }
            self.ui.journal.hint = Some(
                if kind.is_hostile() {
                    "Hostile · sword attacks within 3 blocks"
                } else {
                    "Wildlife · non-hostile"
                }
                .into(),
            );
            return;
        }
        if let Some(hit) = hit {
            let block = self
                .world
                .get_block(hit.target.0, hit.target.1, hit.target.2);
            self.ui.journal.hint = Some(if self.world.automation.device_at(hit.target).is_some() {
                "F · Inspect device / storage".into()
            } else {
                format!(
                    "{} · {}",
                    block.name(),
                    if block.is_unbreakable() {
                        "Unbreakable"
                    } else if block.hand_pickable() {
                        "Mine with empty hand or tool"
                    } else {
                        block.required_tool().name()
                    }
                )
            });
        }
    }
}
