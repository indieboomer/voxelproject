use super::*;
impl App {
    pub(super) fn submit_eat_harvest(&mut self, item: String) {
        let revision = self.player.crafting.revision;
        if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(&client.socket, client.server_addr, ReliableMsg::EatHarvest { item, revision });
        } else { self.perform_eat_harvest(None, item, revision); }
    }
    pub(super) fn perform_eat_harvest(&mut self, from: Option<Peer>, item: String, revision: u64) {
        let (id, health, key) = if let Some(peer) = from {
            let NetRole::Host(host) = &self.net else { return; };
            let Some(&id) = host.clients.get(&peer) else { return; };
            let Some(rp) = host.remote_players.get(&id) else { return; };
            (id, rp.health, Some(peer.account_key(&rp.nickname)))
        } else { (self.local_player_id, self.player.health, None) };
        let Some(heal) = crate::food::harvest_healing(&item).filter(|v| *v > 0. || matches!(item.as_str(), "harvest:glowcap" | "harvest:cooked_glowcap" | "harvest:fired_glowcap")) else { self.notify_important("Raw eggs are not nutritious; cook them first".into()); return; };
        let account = if let Some(key) = key { self.guest_accounts.entry(key).or_default() } else { &mut self.player.crafting };
        let result: Result<f32, String> = if account.revision != revision { Err("Inventory changed; try again".into()) } else if account.production_goods.get(&item).copied().unwrap_or(0) == 0 { Err("You do not have that food".into()) } else { account.production_goods.get_mut(&item).map(|n| *n -= 1); if account.production_goods.get(&item) == Some(&0) { account.production_goods.remove(&item); } account.revision = account.revision.saturating_add(1); Ok(heal.min((MAX_HEALTH - health).max(0.))) };
        let message = result.as_ref().map_or_else(|e| e.clone(), |n| if matches!(item.as_str(), "harvest:glowcap" | "harvest:cooked_glowcap" | "harvest:fired_glowcap") { "Ate a poisonous glowing mushroom".into() } else { format!("Ate food: restored {n:.0} health") });
        if let Ok(delta) = result { self.apply_player_effect(PlayerEffect::Health { player_id: id, delta }); if matches!(item.as_str(), "harvest:glowcap" | "harvest:cooked_glowcap" | "harvest:fired_glowcap") { self.apply_player_effect(PlayerEffect::Poisoned { player_id: id, poisoned: true }); } let satiety = crate::food::harvest_satiety(&item); if from.is_none() { self.player.restore_satiety(satiety); } else if let NetRole::Host(host) = &mut self.net { if let Some(p) = host.remote_players.get_mut(&id) { p.satiety = (p.satiety + satiety).min(crate::player::MAX_SATIETY); } } }
        if let Some(peer) = from { if let NetRole::Host(host) = &mut self.net { let account = self.guest_accounts.get(&peer.account_key(&host.remote_players.get(&host.clients[&peer]).unwrap().nickname)).cloned().unwrap_or_default(); host.reliable.send(&host.socket, peer, ReliableMsg::CraftState { account, feedback: None }); host.reliable.send(&host.socket, peer, ReliableMsg::FoodResult(message)); } } else { self.crafting_ui.feedback = message; }
    }

    pub(super) fn submit_eat(&mut self, block: BlockType) {
        let revision = self.player.crafting.revision;
        if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(
                &client.socket,
                client.server_addr,
                ReliableMsg::EatFood { block, revision },
            );
            self.crafting_ui.feedback = "Waiting for the host...".into();
        } else {
            self.perform_eat(None, block, revision);
        }
    }
    pub(super) fn perform_eat(&mut self, from: Option<Peer>, block: BlockType, revision: u64) {
        let (id, health, key) = if let Some(peer) = from {
            let NetRole::Host(host) = &self.net else {
                return;
            };
            let Some(&id) = host.clients.get(&peer) else {
                return;
            };
            let Some(rp) = host.remote_players.get(&id) else {
                return;
            };
            (id, rp.health, Some(peer.account_key(&rp.nickname)))
        } else {
            (self.local_player_id, self.player.health, None)
        };
        let satiety_before = if from.is_some() {
            match &self.net {
                NetRole::Host(host) => host.remote_players.get(&id).map_or(0.0, |p| p.satiety),
                _ => 0.0,
            }
        } else {
            self.player.satiety
        };
        let effective_health = if health >= MAX_HEALTH
            && satiety_before < crate::player::MAX_SATIETY
        {
            MAX_HEALTH - 0.001
        } else {
            health
        };
        let account = if let Some(key) = key {
            self.guest_accounts.entry(key).or_default()
        } else {
            &mut self.player.crafting
        };
        let result = crate::food::eat(account, effective_health, block, revision);
        let message = match &result {
            Ok(n) => format!("Ate {}: restored {n:.0} health", block.name()),
            Err(e) => e.clone(),
        };
        let updated = account.clone();
        if let Ok(delta) = result {
            self.apply_player_effect(PlayerEffect::Health {
                player_id: id,
                delta,
            });
            if block == BlockType::Glowcap { self.apply_player_effect(PlayerEffect::Poisoned { player_id: id, poisoned: true }); }
            let satiety = crate::food::satiety(block);
            if let Some(peer) = from {
                if let NetRole::Host(host) = &mut self.net {
                    if let Some(player) = host.remote_players.get_mut(&id) {
                        player.satiety = (player.satiety + satiety).min(crate::player::MAX_SATIETY);
                    }
                }
                let _ = peer;
            } else {
                self.player.restore_satiety(satiety);
            }
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
                    .send(&host.socket, peer, ReliableMsg::FoodResult(message));
            }
        } else {
            self.crafting_ui.feedback = message;
        }
    }
}
