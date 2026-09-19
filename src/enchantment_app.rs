use super::*;
use crate::enchantment::Summary;
impl App {
    pub(super) fn detach_rule(&mut self, index: usize) {
        if !matches!(self.net, NetRole::Host(_)) {
            return;
        }
        if self
            .scripting
            .modules
            .get(index)
            .is_some_and(|m| m.attachment.is_some())
        {
            self.scripting.detach_at(index);
            self.notify_important(
                "Enchantment removed and spell disabled. Previous world changes remain. Save with F5.".into(),
            );
            self.send_enchantment_summaries(None);
        }
    }
    pub(super) fn send_enchantment_summaries(&mut self, peer: Option<Peer>) {
        let NetRole::Host(host) = &mut self.net else {
            return;
        };
        let summaries: Vec<_> = self
            .scripting
            .modules
            .iter()
            .filter_map(|m| {
                m.attachment.as_ref().map(|binding| {
                    let status = if binding.lost.is_some() {
                        "Target lost"
                    } else if m.error.is_some() {
                        "Error"
                    } else if !m.enabled {
                        "Disabled"
                    } else {
                        match binding.target.available(&self.world, &self.creatures) {
                            Ok(true) => "Active",
                            Ok(false) => "Paused: unloaded",
                            Err(_) => "Target lost",
                        }
                    };
                    Summary {
                        target: binding.target.clone(),
                        name: m.name.chars().take(80).collect(),
                        status: status.into(),
                    }
                })
            })
            .collect();
        if summaries != self.enchantment_summaries || peer.is_some() {
            self.enchantment_summaries = summaries.clone();
            self.enchantment_revision = self.enchantment_revision.saturating_add(1);
            for &client in host.clients.keys() {
                if peer.is_none() || peer == Some(client) {
                    host.reliable.send(
                        &host.socket,
                        client,
                        ReliableMsg::Enchantments {
                            revision: self.enchantment_revision,
                            summaries: summaries.clone(),
                        },
                    );
                }
            }
        }
    }
    pub(super) fn update_enchantment_view(&mut self) {
        self.send_enchantment_summaries(None);
        self.ui.aimed_object.clear();
        self.ui.aimed_enchantments.clear();
        if let Some(context) = self.aimed_spell_context() {
            self.ui.aimed_object = match context.target {
                crate::spell_target::Target::Creature { id } => self
                    .creatures
                    .snapshot_with_ids()
                    .iter()
                    .find(|c| c.0 == id)
                    .map_or_else(
                        || format!("Creature #{id}"),
                        |c| {
                            format!(
                                "{:?} #{id} · {:.0}/{:.0} health",
                                crate::creature::CreatureKind::from_u8(c.1),
                                c.3,
                                c.4
                            )
                        },
                    ),
                crate::spell_target::Target::Block {
                    position: p,
                    material,
                } => self.world.automation.device_at(p).map_or_else(
                    || format!("{} at {}, {}, {}", material.name(), p.0, p.1, p.2),
                    |d| format!("{} device", d.kind.id()),
                ),
            };
            self.ui.aimed_enchantments = self
                .enchantment_summaries
                .iter()
                .filter(|s| s.target.matches_aim(&self.world, context.target))
                .map(|s| format!("{} · {}", s.name, s.status))
                .collect();
        }
    }
}
