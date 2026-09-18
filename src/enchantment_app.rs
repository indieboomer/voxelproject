use super::*;
use crate::enchantment::{Binding, Reference, Summary};
impl App {
    pub(super) fn selected_enchantment_target(&mut self) -> Result<Reference, String> {
        let target = if self.console_open {
            self.console_attachment.clone().map_err(|_|"No target was selected when the console opened. Close it, aim at an object, and reopen it.".to_string())?
        } else {
            self.capture_enchantment_target()?
        };
        if !target.available(&self.world, &self.creatures)? {
            return Err("Selected target is unloaded. Close the console and return to it.".into());
        }
        Ok(target)
    }
    pub(super) fn capture_enchantment_target(&mut self) -> Result<Reference, String> {
        let target = crate::spell_target::TargetContext::resolve(
            &self.world,
            &self.creatures,
            self.camera.eye_position(),
            self.camera.forward(),
        )
        .ok_or("Aim at a creature, block or device within 18 blocks")?
        .target;
        Reference::capture(&mut self.world, &self.creatures, target)
    }
    pub(super) fn attach_rule(&mut self, index: usize) {
        if !matches!(self.net, NetRole::Host(_)) {
            return;
        }
        let result = (|| -> Result<String, String> {
            let module = self
                .scripting
                .modules
                .get(index)
                .ok_or("Rule no longer exists")?;
            if module.is_instant || module.attachment.is_some() {
                return Err("Choose a permanent spell without an enchantment target".into());
            }
            let reference = if let Some(candidate) = &module.attachment_candidate {
                candidate.clone()
            } else {
                self.selected_enchantment_target()?
            };
            if !reference.available(&self.world, &self.creatures)? {
                return Err("Target is unloaded; return to it before enchanting".into());
            }
            let id = self.world.identity.next_creation.max(1);
            self.world.identity.next_creation =
                id.checked_add(1).ok_or("Creation ID limit reached")?;
            self.world.identity.revision = self.world.identity.revision.saturating_add(1);
            self.scripting.attach_at(
                index,
                Binding {
                    id,
                    creator: HOST_PLAYER_ID,
                    target: reference.clone(),
                    lost: None,
                },
            )?;
            Ok(format!(
                "Enchantment activated on {}. Save with F5. It stops if the target disappears.",
                reference.label()
            ))
        })();
        self.notify_important(result.unwrap_or_else(|e| e));
        self.send_enchantment_summaries(None);
    }
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
        if self.console_open {
            if let Ok(target) = &self.console_attachment {
                self.ui.aimed_object = target.label();
                match target.available(&self.world, &self.creatures) {
                    Ok(true) => {}
                    Ok(false) => self.ui.aimed_object.push_str(" (unloaded)"),
                    Err(_) => self.ui.aimed_object.push_str(" (target lost)"),
                }
                self.ui.aimed_enchantments = self
                    .enchantment_summaries
                    .iter()
                    .filter(|s| s.target == *target)
                    .map(|s| format!("{} · {}", s.name, s.status))
                    .collect();
            } else {
                self.ui.aimed_object.clear();
                self.ui.aimed_enchantments.clear();
            }
            return;
        }
        self.ui.aimed_object.clear();
        self.ui.aimed_enchantments.clear();
        if let Some(context) = crate::spell_target::TargetContext::resolve(
            &self.world,
            &self.creatures,
            self.camera.eye_position(),
            self.camera.forward(),
        ) {
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
