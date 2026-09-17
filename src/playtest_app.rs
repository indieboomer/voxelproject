use super::*;

impl App {
    pub(super) fn playtest_player_effect(&mut self, effect: &PlayerEffect) -> bool {
        let Some(session) = &mut self.playtest else {
            return false;
        };
        if session.script.finished.is_some() {
            return false;
        }
        let player = &mut session.actor.player;
        let handled = match effect {
            PlayerEffect::Health { player_id, delta }
                if *player_id == crate::playtest::AGENT_ID =>
            {
                if *delta >= 0.0 {
                    player.heal(*delta);
                } else {
                    player.damage(-*delta);
                }
                true
            }
            PlayerEffect::Poisoned {
                player_id,
                poisoned,
            } if *player_id == crate::playtest::AGENT_ID => {
                player.poisoned = *poisoned;
                true
            }
            PlayerEffect::SpeedMultiplier {
                player_id,
                multiplier,
            } if *player_id == crate::playtest::AGENT_ID => {
                player.set_speed_multiplier(*multiplier);
                true
            }
            PlayerEffect::JumpMultiplier {
                player_id,
                multiplier,
            } if *player_id == crate::playtest::AGENT_ID => {
                player.set_jump_multiplier(*multiplier);
                true
            }
            PlayerEffect::Teleport { player_id, pos }
                if *player_id == crate::playtest::AGENT_ID =>
            {
                player.position = *pos;
                player.velocity = Vec3::ZERO;
                true
            }
            PlayerEffect::GiveItem {
                player_id,
                block,
                amount,
            } if *player_id == crate::playtest::AGENT_ID => {
                player.add_resources(*block, *amount);
                true
            }
            PlayerEffect::TakeItem {
                player_id,
                block,
                amount,
            } if *player_id == crate::playtest::AGENT_ID => {
                player.take_resources(*block, *amount);
                true
            }
            PlayerEffect::Inventory {
                player_id,
                balances,
                resources,
            } if *player_id == crate::playtest::AGENT_ID => {
                player.crafting.resources = *resources;
                player.crafting.elements = balances.elements;
                player.crafting.mana = balances.mana;
                player.crafting.gear = balances.items;
                player.crafting.revision = player.crafting.revision.saturating_add(1);
                true
            }
            _ => false,
        };
        if handled {
            session.external_event(&format!("{effect:?}"));
        }
        handled
    }

    pub(super) fn playtest_player_snapshot(&self) -> Option<PlayerSnapshot> {
        let session = self.playtest.as_ref()?;
        if session.script.finished.is_some() {
            return None;
        }
        let p = &session.actor.player;
        Some(PlayerSnapshot {
            id: crate::playtest::AGENT_ID,
            pos: p.position,
            finances: crate::scripting::InventoryBalances::from_account(&p.crafting),
            resources: p.crafting.resources,
            carrying_crystal: p.carrying_crystal,
            velocity: p.velocity,
            on_ground: p.on_ground,
            sprinting: p.sprinting,
            in_water: is_in_water(&self.world, p.position),
            health: p.health,
            poisoned: p.poisoned,
            speed_multiplier: p.speed_multiplier,
            jump_multiplier: p.jump_multiplier,
            oxygen: p.oxygen,
        })
    }

    pub(super) fn update_playtest(&mut self, dt: f32) {
        self.ui.settings.playtest_in_game = true;
        if matches!(&self.net,NetRole::Host(host) if !host.clients.is_empty()) {
            if let Some(mut session) = self.playtest.take() {
                let result = session.cancel();
                self.ui.settings.playtest_status = format!(
                    "Stopped because a guest joined. Logs: {}{}",
                    session.directory.display(),
                    result
                        .err()
                        .map_or(String::new(), |e| format!("; report failed: {e}"))
                );
            }
        }
        if std::mem::take(&mut self.ui.settings.playtest_request) {
            if let Some(mut session) = self.playtest.take() {
                self.ui.settings.playtest_status = match session.cancel() {
                    Ok(()) => format!("Stopped. Logs: {}", session.directory.display()),
                    Err(e) => format!("Could not write report: {e}"),
                };
            } else {
                let start = (|| {
                    let NetRole::Host(host) = &self.net else {
                        return Err("Start on the host; separate clients are phase 6".to_owned());
                    };
                    if !host.clients.is_empty() {
                        return Err("Phase-one sessions require a solo host".into());
                    }
                    crate::crafting::load_interaction_area(&mut self.world, self.player.position);
                    let position =
                        crate::playtest::spawn_position(&self.world, self.player.position)
                            .ok_or("No clear dry ground near you")?;
                    let snapshot = crate::save::playtest_snapshot(
                        &self.world,
                        &self.player,
                        &self.camera,
                        self.time_of_day,
                        self.scripting.save_entries(),
                        &self.crafting_save(),
                    )?;
                    crate::playtest::Session::create(
                        position,
                        &self.world,
                        &snapshot,
                        &self.crafting_registry,
                        self.ui.settings.playtest_scenario,
                    )
                })();
                match start {
                    Ok(session) => {
                        self.ui.settings.playtest_status =
                            format!("Agent1 running. Logs: {}", session.directory.display());
                        self.playtest = Some(session);
                    }
                    Err(e) => self.ui.settings.playtest_status = e,
                }
            }
        }
        let Some(mut session) = self.playtest.take() else {
            return;
        };
        crate::crafting::load_interaction_area(&mut self.world, session.actor.player.position);
        let positions = vec![self.player.position];
        let mut context = crate::playtest::Context {
            world: &mut self.world,
            creatures: &mut self.creatures,
            loot: &mut self.loot,
            registry: &self.crafting_registry,
            players: &positions,
        };
        match session.update(dt, &mut context) {
            Ok(effects) => {
                for (p, block, old) in effects.edits {
                    self.apply_block_edit(p.0, p.1, p.2, block);
                    if block == BlockType::Air {
                        self.pending_block_breaks.push(BlockBreakEvent {
                            x: p.0,
                            y: p.1,
                            z: p.2,
                            block: old,
                            player_id: crate::playtest::AGENT_ID,
                        });
                        for p in self.world.flood_from(p) {
                            self.apply_block_edit(p.0, p.1, p.2, BlockType::Water);
                        }
                    }
                }
                for (p, block) in effects.interacts {
                    self.pending_interacts.push(InteractEvent {
                        x: p.0,
                        y: p.1,
                        z: p.2,
                        block,
                        player_id: crate::playtest::AGENT_ID,
                    });
                }
            }
            Err(e) => {
                session.script.finished = Some(format!("Logging failed; stopped: {e}"));
            }
        }
        if let Some(reason) = &session.script.finished {
            self.ui.settings.playtest_status =
                format!("{reason}. Logs: {}", session.directory.display());
        } else {
            self.ui.settings.playtest_status = format!(
                "{}. Logs: {}",
                session.status(),
                session.directory.display()
            );
        }
        self.playtest = Some(session);
    }

    pub(super) fn playtest_nameplate(&mut self) {
        self.ui.agent_home_marker = None;
        let Some(session) = &self.playtest else {
            return;
        };
        let target = session.actor.player.position + Vec3::Y * 2.15;
        let eye = self.camera.eye_position();
        let (home, label) = session.marker();
        let marker = home + Vec3::Y * 0.1;
        let distance = eye.distance(marker);
        if distance < 48.0
            && crate::raycast::raycast(&self.world, eye, marker - eye, (distance - 0.1).max(0.0))
                .is_none()
        {
            let clip = self.camera.view_proj() * marker.extend(1.0);
            if clip.w > 0.0 {
                let ndc = clip.truncate() / clip.w;
                if ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z) {
                    let screen = self.window.inner_size();
                    let scale = self.window.scale_factor() as f32;
                    self.ui.agent_home_marker = Some((
                        egui::pos2(
                            (ndc.x + 1.0) * 0.5 * screen.width as f32 / scale,
                            (1.0 - ndc.y) * 0.5 * screen.height as f32 / scale,
                        ),
                        label,
                    ));
                }
            }
        }
        let distance = eye.distance(target);
        if distance > 48.0
            || crate::raycast::raycast(&self.world, eye, target - eye, (distance - 0.3).max(0.0))
                .is_some()
        {
            return;
        }
        let clip = self.camera.view_proj() * target.extend(1.0);
        if clip.w <= 0.0 {
            return;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 || !(0.0..=1.0).contains(&ndc.z) {
            return;
        }
        let screen = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        self.ui.agent_nameplate = Some((
            egui::pos2(
                (ndc.x + 1.0) * 0.5 * screen.width as f32 / scale,
                (1.0 - ndc.y) * 0.5 * screen.height as f32 / scale,
            ),
            session.overhead_status(),
        ));
    }
}
