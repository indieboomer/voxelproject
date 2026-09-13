use super::*;
impl App {
    pub(super) fn initialize_npcs(&mut self) {
        if !matches!(self.net, NetRole::Host(_)) {
            self.npcs.clear();
            return;
        }
        let mut seen = [false; 6];
        self.npcs.retain(|n| {
            let keep = !seen[n.kind as usize];
            seen[n.kind as usize] = true;
            keep
        });
        if self.npcs.len() < 6 {
            let center = self
                .player
                .crafting
                .adventure
                .home
                .map(crate::adventure::feet)
                .unwrap_or(self.player.position);
            for npc in crate::npc::populate(&mut self.world, center) {
                if !seen[npc.kind as usize] {
                    self.npcs.push(npc);
                }
            }
        }
    }
    pub(super) fn update_npcs(&mut self, dt: f32) {
        let players = self.all_player_positions();
        for npc in &mut self.npcs {
            npc.tick(&self.world, &players, dt);
        }
        // Clients predict only these harmless patrol poses; all dialogue is host-validated.
        self.npc_timer += dt;
        if self.npc_timer >= 0.5 {
            self.npc_timer = 0.;
            if let NetRole::Host(host) = &mut self.net {
                for &peer in host.clients.keys() {
                    host.reliable
                        .send(&host.socket, peer, ReliableMsg::Npcs(self.npcs.clone()));
                }
            }
        }
    }
    pub(super) fn aimed_npc(&self) -> Option<u8> {
        let eye = self.camera.eye_position();
        let dir = self.camera.forward();
        self.npcs
            .iter()
            .filter(|n| crate::npc::can_talk(&self.world, n, self.player.position))
            .filter_map(|n| {
                let delta = Vec3::from_array(n.position) + Vec3::Y - eye;
                (dir.dot(delta.normalize_or_zero()) > 0.965)
                    .then_some((delta.length_squared(), n.kind))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|v| v.1)
    }
    pub(super) fn submit_quest_action(&mut self, action: crate::quests::Action) {
        if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(
                &client.socket,
                client.server_addr,
                ReliableMsg::QuestAction(action),
            );
            self.ui.journal.feedback = "Waiting for the host...".into();
        } else {
            self.perform_quest_action(None, action);
        }
    }
    pub(super) fn perform_quest_action(
        &mut self,
        from: Option<Peer>,
        action: crate::quests::Action,
    ) {
        if !self.adventure_actor_alive(from) {
            return;
        }
        let (feet, key) = if let Some(peer) = from {
            let NetRole::Host(host) = &self.net else {
                return;
            };
            let Some(rp) = host
                .clients
                .get(&peer)
                .and_then(|id| host.remote_players.get(id))
            else {
                return;
            };
            (rp.pos, Some(peer.account_key(&rp.nickname)))
        } else {
            (self.player.position, None)
        };
        crate::crafting::load_interaction_area(&mut self.world, feet);
        let npc = match action {
            crate::quests::Action::Claim { npc, .. } => npc,
            crate::quests::Action::LightCampfire => 4,
        };
        let in_reach = self
            .npcs
            .iter()
            .any(|n| n.kind == npc && crate::npc::can_talk(&self.world, n, feet));
        let players = self.all_player_positions();
        let account = if let Some(key) = key {
            self.guest_accounts.entry(key).or_default()
        } else {
            &mut self.player.crafting
        };
        let mut edit = None;
        let result = if !in_reach {
            Err("Move closer to the quest giver with a clear view".into())
        } else {
            match action {
                crate::quests::Action::Claim { npc, quest } => {
                    crate::quests::claim(account, npc, quest as usize)
                }
                crate::quests::Action::LightCampfire => {
                    let p = crate::adventure::cell(feet);
                    let candidate = [(2, 0), (-2, 0), (0, 2), (0, -2), (2, 2), (-2, -2)]
                        .into_iter()
                        .map(|(x, z)| (p.0 + x, p.1, p.2 + z))
                        .find(|p| {
                            crate::adventure::clear_feet(&self.world, *p)
                                && self.world.automation.device_at(*p).is_none()
                                && players
                                    .iter()
                                    .all(|v| v.distance(crate::adventure::feet(*p)) > 1.5)
                                && self.npcs.iter().all(|n| {
                                    Vec3::from_array(n.position)
                                        .distance(crate::adventure::feet(*p))
                                        > 1.5
                                })
                                && {
                                    let delta = crate::adventure::feet(*p) + Vec3::Y * 0.5
                                        - (feet + Vec3::Y * 1.62);
                                    raycast(
                                        &self.world,
                                        feet + Vec3::Y * 1.62,
                                        delta,
                                        delta.length(),
                                    )
                                    .is_none()
                                }
                        });
                    if let Some(p) = candidate {
                        let mut next = account.clone();
                        crate::quests::pay_supplies(&mut next, 3, 2).map(|()| {
                            next.adventure.quests.record(15, 1);
                            next.revision = next.revision.saturating_add(1);
                            *account = next;
                            edit = Some(p);
                            "Campfire lit. Come warm your hands.".to_string()
                        })
                    } else {
                        Err("Clear a nearby level space for the campfire".into())
                    }
                }
            }
        };
        let message = result.unwrap_or_else(|e| e);
        let updated = account.clone();
        if let Some(p) = edit {
            self.apply_block_edit(p.0, p.1, p.2, BlockType::Campfire);
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
}
