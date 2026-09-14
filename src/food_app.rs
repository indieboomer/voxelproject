use super::*;
impl App {
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
        let account = if let Some(key) = key {
            self.guest_accounts.entry(key).or_default()
        } else {
            &mut self.player.crafting
        };
        let result = crate::food::eat(account, health, block, revision);
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
