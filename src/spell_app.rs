use super::*;
use crate::spell_network::{Request, Summary};
impl App {
    /// Host-authoritative physical card transfer. Only the spell ID crosses
    /// the network; source and metadata always come from the host spellbook.
    pub(super) fn transfer_spell_card(&mut self, from: Option<Peer>, spell_id: u64, target: String) {
        let NetRole::Host(host) = &self.net else { return; };
        let target = target.trim();
        if target.is_empty() || target.len() > 64 { self.notify_important("Invalid card recipient".into()); return; }
        let Some(spell) = self.scripting.spellbook.get(spell_id) else { self.notify_important("That spell no longer exists".into()); return; };
        if !spell.ready() { self.notify_important("Only validated spells can be traded".into()); return; }
        let sender_key = if let Some(peer) = from {
            let Some(&id) = host.clients.get(&peer) else { return; };
            let Some(player) = host.remote_players.get(&id) else { return; };
            peer.account_key(&player.nickname)
        } else { String::new() };
        let target_host = target.eq_ignore_ascii_case("host") || target.eq_ignore_ascii_case(&self.local_nickname);
        let target_id = host.remote_players.iter().find_map(|(id, p)|
            (p.nickname.eq_ignore_ascii_case(target) || p.display_name.eq_ignore_ascii_case(target)).then_some(*id));
        if !target_host && target_id.is_none() { self.notify_important(format!("Player '{target}' is not connected")); return; }
        if target_host && from.is_none() { self.notify_important("Choose another player".into()); return; }
        let target_key = target_id.and_then(|target_id| host.clients.iter().find_map(|(peer, id)| (*id == target_id).then(|| peer.account_key(&host.remote_players.get(id).unwrap().nickname))));
        let owns = if from.is_none() { self.player.crafting.has_spell_card(spell_id) } else { self.guest_accounts.get(&sender_key).map_or(false, |a| a.has_spell_card(spell_id)) };
        if !owns { self.notify_important("You do not own that spell card".into()); return; }
        if target_key.as_ref().is_some_and(|key| *key == sender_key) { self.notify_important("Choose another player".into()); return; }
        // Remove first, then add; rollback on a full recipient inventory.
        let removed = if from.is_none() { self.player.crafting.remove_spell_card(spell_id).is_ok() } else { self.guest_accounts.get_mut(&sender_key).map_or(false, |a| a.remove_spell_card(spell_id).is_ok()) };
        if !removed { return; }
        let recipient_result = if target_host { self.player.crafting.add_spell_card(spell_id) } else { self.guest_accounts.entry(target_key.clone().unwrap()).or_default().add_spell_card(spell_id) };
        if let Err(error) = recipient_result {
            if from.is_none() { let _ = self.player.crafting.add_spell_card(spell_id); } else if let Some(a) = self.guest_accounts.get_mut(&sender_key) { let _ = a.add_spell_card(spell_id); }
            self.notify_important(error);
            return;
        }
        let sender_snapshot = if from.is_none() { self.player.crafting.clone() } else { self.guest_accounts.get(&sender_key).cloned().unwrap_or_default() };
        let recipient_snapshot = if target_host { self.player.crafting.clone() } else { self.guest_accounts.get(target_key.as_ref().unwrap()).cloned().unwrap_or_default() };
        if let NetRole::Host(host) = &mut self.net {
            if let Some(peer) = from { host.reliable.send(&host.socket, peer, ReliableMsg::CraftState { account: sender_snapshot, feedback: Some("Spell card traded".into()) }); }
            if let Some(target_id) = target_id { if let Some((&peer, _)) = host.clients.iter().find(|(_, id)| **id == target_id) { host.reliable.send(&host.socket, peer, ReliableMsg::CraftState { account: recipient_snapshot, feedback: Some("You received a spell card".into()) }); } }
        }
        self.notify_all(format!("A spell card for '{}' was traded", spell.name));
    }

    pub(super) fn update_spell_hud(&mut self) {
        self.ui.spell_hud = None;
        let Some(crate::equipment::Entry::Spell(id)) = self.player.crafting.hotbar.entry() else {
            return;
        };
        let Some(spell) = self.scripting.spellbook.get(id) else {
            self.ui.spell_hud = Some(("Spell unavailable".into(), false));
            return;
        };
        let context = crate::spell_target::TargetContext::resolve(
            &self.world,
            &self.creatures,
            self.camera.eye_position(),
            self.camera.forward(),
        );
        let target = context.map_or_else(
            || "No target".to_string(),
            |c| match c.target {
                crate::spell_target::Target::Creature { id } => self
                    .creatures
                    .snapshot_with_ids()
                    .iter()
                    .find(|c| c.0 == id)
                    .map_or_else(
                        || "Creature".into(),
                        |c| format!("{:?}", crate::creature::CreatureKind::from_u8(c.1)),
                    ),
                crate::spell_target::Target::Block { material, .. } => material.name().into(),
            },
        );
        let cost = self.crafting_registry.mana_charge(spell.mana_cost);
        let remaining = if matches!(self.net, NetRole::Host(_)) {
            self.scripting.spell_cooldowns.get(&id).map_or(0., |t| {
                (spell.cooldown_seconds - t.elapsed().as_secs_f32()).max(0.)
            })
        } else {
            self.spell_network.ready_at.get(&id).map_or(0., |t| {
                t.saturating_duration_since(Instant::now()).as_secs_f32()
            })
        };
        let reason = if !spell.ready()
            || crate::equipment::Entry::Spell(id).count(&self.player.crafting) == 0
        {
            "Unavailable".into()
        } else if self.player.health <= 0. {
            "Defeated".into()
        } else if self.spell_network.pending.is_some() {
            "Waiting for host".into()
        } else if remaining > 0. {
            format!("Cooldown {remaining:.1}s")
        } else if self.player.crafting.mana < cost {
            "Not enough mana".into()
        } else if !spell.target.accepts(context.map(|c| c.target)) {
            format!("Aim at {} (18 blocks)", spell.target.label().to_lowercase())
        } else if matches!(self.net, NetRole::Host(_)) && !self.scripting.can_cast_immediately() {
            "Rules busy".into()
        } else {
            "Left-click to cast".into()
        };
        let ready = reason == "Left-click to cast";
        self.ui.spell_hud = Some((format!("{target} · {cost} mana · {reason}"), ready));
    }
    pub(super) fn publish_spell_catalog(&mut self) {
        self.scripting
            .spellbook
            .sync_hotbar(&mut self.player.crafting);
        let book = crate::spell_network::guest_book(&self.scripting.spellbook);
        for account in self.guest_accounts.values_mut() {
            book.sync_hotbar(account);
        }
        self.spell_network.catalog_revision = self.spell_network.catalog_revision.saturating_add(1);
        self.spell_network
            .guest_cooldowns
            .retain(|(_, id), _| self.scripting.spellbook.get(*id).is_some());
        if let NetRole::Host(host) = &mut self.net {
            self.spell_network
                .peers
                .retain(|peer, _| host.clients.contains_key(peer));
            let spells: Vec<_> = book.spells.iter().map(Summary::from_spell).collect();
            for (&peer, session) in &self.spell_network.peers {
                host.reliable.send(
                    &host.socket,
                    peer,
                    ReliableMsg::SpellCatalog {
                        session: session.token,
                        revision: self.spell_network.catalog_revision,
                        spells: spells.clone(),
                    },
                );
            }
        }
        self.sync_guest_mana();
    }
    pub(super) fn receive_spell_catalog(
        &mut self,
        session: u64,
        revision: u64,
        spells: Vec<Summary>,
    ) {
        if revision <= self.spell_network.received_revision
            || spells.len() > crate::spellbook::MAX_SPELLS
        {
            return;
        }
        let count = spells.len();
        let definitions: Vec<_> = spells.into_iter().filter_map(Summary::into_spell).collect();
        let ids: std::collections::HashSet<_> = definitions.iter().map(|s| s.id).collect();
        if definitions.len() != count || ids.len() != count {
            return;
        }
        if self.spell_network.session != Some(session) {
            self.spell_network.sequence = 0;
            self.spell_network.pending = None;
            self.spell_network.ready_at.clear();
        }
        self.spell_network.session = Some(session);
        self.spell_network.received_revision = revision;
        self.scripting.spellbook.spells = definitions;
        // Account packets remain authoritative. Catalogs only control display definitions.
    }
    pub(super) fn request_guest_spell(&mut self, id: u64) {
        let result = (|| -> Result<Request, String> {
            let session = self
                .spell_network
                .session
                .ok_or("Waiting for host spell list")?;
            if self.spell_network.pending.is_some() {
                return Err("Waiting for the host to finish your cast".into());
            }
            let spell = self
                .scripting
                .spellbook
                .get(id)
                .ok_or("Spell is no longer shared")?;
            if self.player.health <= 0. {
                return Err("Defeated players cannot cast".into());
            }
            if crate::equipment::Entry::Spell(id).count(&self.player.crafting) == 0 {
                return Err("Waiting for spell permission".into());
            }
            let remaining = self.spell_network.ready_at.get(&id).map_or(0., |t| {
                t.saturating_duration_since(Instant::now()).as_secs_f32()
            });
            if remaining > 0. {
                return Err(format!("Ready in {remaining:.1} seconds"));
            }
            let cost = self.crafting_registry.mana_charge(spell.mana_cost);
            if self.player.crafting.mana < cost {
                return Err(format!("This spell needs {cost} mana"));
            }
            self.spell_network.sequence = self
                .spell_network
                .sequence
                .checked_add(1)
                .ok_or("Reconnect to reset cast sequence")?;
            let facing = self.camera.forward();
            let context = crate::spell_target::TargetContext::resolve(
                &self.world,
                &self.creatures,
                self.camera.eye_position(),
                facing,
            );
            if !spell.target.accepts(context.map(|c| c.target)) {
                return Err(format!(
                    "Aim at a {} within 18 blocks",
                    spell.target.label().to_lowercase()
                ));
            }
            Ok(Request {
                session,
                sequence: self.spell_network.sequence,
                spell: id,
                revision: spell.revision,
                facing: facing.to_array(),
                target: context.map(|c| c.target),
            })
        })();
        match result {
            Ok(request) => {
                if let NetRole::Joined(client) = &mut self.net {
                    self.spell_network.pending = Some((request.sequence, id));
                    client.reliable.send(
                        &client.socket,
                        client.server_addr,
                        ReliableMsg::CastSpell(request),
                    );
                    self.ui.spellbook.feedback = "Casting… waiting for host".into();
                }
            }
            Err(error) => self.ui.spellbook.feedback = error,
        }
    }
    pub(super) fn handle_guest_spell(&mut self, peer: Peer, request: Request) {
        let NetRole::Host(host) = &self.net else {
            return;
        };
        let Some(&caster_id) = host.clients.get(&peer) else {
            return;
        };
        let Some(player) = host.remote_players.get(&caster_id) else {
            return;
        };
        let key = peer.account_key(&player.nickname);
        let eye = player.pos + Vec3::Y * 1.62;
        let alive = player.health > 0.;
        let Some(session) = self.spell_network.peers.get_mut(&peer) else {
            return;
        };
        if !session.fresh(&request) {
            return;
        }
        // Repeated/expired sequences never execute and cannot overwrite a newer result.
        let accepted = session.accept(&request, Instant::now());
        let token = session.token;
        let result = accepted.and_then(|()| {
            if !alive {
                return Err("Defeated players cannot cast".into());
            }
            if !self.scripting.can_cast_immediately() {
                return Err("Rules are busy; try again".into());
            }
            let spell = self
                .scripting
                .spellbook
                .get(request.spell)
                .ok_or("Spell no longer exists")?
                .clone();
            let context = crate::spell_network::resolve_request(
                &request,
                &spell,
                &self.world,
                &self.creatures,
                eye,
            )?;
            let cooldown_key = (key.clone(), spell.id);
            if let Some(last) = self.spell_network.guest_cooldowns.get(&cooldown_key) {
                let remaining = spell.cooldown_seconds - last.elapsed().as_secs_f32();
                if remaining > 0. {
                    return Err(format!("Ready in {remaining:.1} seconds"));
                }
            }
            let cost = self.crafting_registry.mana_charge(spell.mana_cost);
            let account = self
                .guest_accounts
                .get(&key)
                .ok_or("Player inventory unavailable")?;
            if !account.has_spell_card(spell.id) {
                return Err("You need the physical spell card to cast this spell".into());
            }
            if account.mana < cost {
                return Err(format!("This spell needs {cost} mana"));
            }
            let index = self.scripting.add_generated(spell.compiled()?)?;
            let account = self.guest_accounts.get_mut(&key).unwrap();
            account.mana -= cost;
            account.revision = account.revision.saturating_add(1);
            let resources = account.resources;
            let players = self.host_player_positions();
            let outcome = if let Some(context) = context {
                self.scripting.run_targeted_cast(
                    index,
                    &self.world,
                    &mut self.creatures,
                    &players,
                    &mut self.time_of_day,
                    &mut self.weather,
                    caster_id,
                    resources,
                    context,
                )
            } else {
                self.scripting.run_cast(
                    index,
                    &self.world,
                    &mut self.creatures,
                    &players,
                    &mut self.time_of_day,
                    &mut self.weather,
                    caster_id,
                    resources,
                )
            };
            self.apply_tick_outcome(outcome);
            let success = self.scripting.cast_succeeded(index, caster_id);
            let error = self.scripting.modules[index]
                .error
                .clone()
                .unwrap_or_else(|| "Cast failed".into());
            self.scripting.remove(index);
            if !success {
                let account = self.guest_accounts.get_mut(&key).unwrap();
                account.mana = account.mana.saturating_add(cost);
                account.revision = account.revision.saturating_add(1);
                return Err(format!("{error}. No mana spent."));
            }
            self.spell_network
                .guest_cooldowns
                .insert(cooldown_key, Instant::now());
            let facing = Vec3::from_array(request.facing);
            let origin = eye + facing * 0.6 - Vec3::Y * 0.3;
            let target = context.map_or(eye + facing * 3., |c| c.hit_position);
            self.spell_fx.cast(origin, target);
            if let NetRole::Host(host) = &mut self.net {
                for &peer in host.clients.keys() {
                    host.reliable.send(
                        &host.socket,
                        peer,
                        ReliableMsg::SpellCastFx {
                            origin: origin.to_array(),
                            target: target.to_array(),
                        },
                    );
                }
            }
            Ok(format!("Cast {}.", spell.name))
        });
        let remaining = self
            .spell_network
            .guest_cooldowns
            .get(&(key, request.spell))
            .map_or(0., |t| (1.5 - t.elapsed().as_secs_f32()).max(0.));
        if let NetRole::Host(host) = &mut self.net {
            host.reliable.send(
                &host.socket,
                peer,
                ReliableMsg::SpellResult {
                    session: token,
                    sequence: request.sequence,
                    spell: request.spell,
                    remaining,
                    message: result.unwrap_or_else(|e| e),
                },
            );
        }
        self.sync_guest_mana();
    }
}
