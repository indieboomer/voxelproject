//! Player-facing expedition journal and contextual feedback.
use crate::{
    adventure::{self, Action, Cell},
    crafting::Account,
    player::Player,
};
use egui::{Color32, RichText};

#[derive(Default)]
pub struct Journal {
    pub npc: Option<u8>,
    pub quest_request: Option<crate::quests::Action>,
    pub open: bool,
    pub hide_tracker: bool,
    pub camp: Option<Cell>,
    pub feedback: String,
    pub hint: Option<String>,
    pub target: Option<(String, f32, f32)>,
    pub damage_flash: f32,
    pub recovery_seconds: Option<f32>,
    pub previous_health: Option<f32>,
}
impl Journal {
    pub fn tick(&mut self, health: f32, dt: f32) {
        self.damage_flash = (self.damage_flash - dt * 2.).max(0.);
        if self
            .previous_health
            .replace(health)
            .is_some_and(|old| health < old)
        {
            self.damage_flash = 0.55;
        }
    }
    pub fn draw(&mut self, ctx: &egui::Context, player: &Player) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut request = None;
        let account = &player.crafting;
        egui::Window::new("Field journal [J]").open(&mut open)
            .default_width(540.).max_width((ctx.screen_rect().width()-50.).max(200.))
            .default_height((ctx.screen_rect().height()-140.).clamp(240.,620.))
            .max_height((ctx.screen_rect().height()-80.).max(200.)).vscroll(true).resizable(false).collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER,[0.,0.]).show(ctx,|ui| {
                self.quest_rows(ui,account);
                if self.npc.is_some() {if !self.feedback.is_empty() {ui.label(&self.feedback);}return;}
                if let Some(camp)=self.camp {
                    ui.heading(format!("{} · Campkeeper",adventure::guide_name(camp)));
                    ui.label("“A fire, a few supplies, and a story worth bringing back. That's all a traveler needs.”");
                    ui.separator();
                }
                ui.label(RichText::new(format!("{} / 3 contracts · {}",account.adventure.stage,account.adventure.title())).strong());
                ui.label(account.adventure.objective());
                ui.add_space(8.);
                progress_rows(ui,account);
                if account.adventure.stage==1 {ui.small(format!("Current height: Y {:.0}",player.position.y));}
                if account.adventure.stage < 3 {ui.colored_label(Color32::from_rgb(230,190,110),format!("Reward: {}",account.adventure.reward()));}
                ui.separator();
                if let Some(camp)=self.camp {
                    ui.label("Rest restores health, cures poison, and sets your recovery camp. Nearby hostiles prevent resting and trading.");
                    if ui.add_enabled(player.health>0.,egui::Button::new("Rest and set recovery camp")).clicked() {request=Some(Action::Rest{camp});}
                    if account.adventure.stage < 3 && ui.add_enabled(adventure::ready(account)&&player.health>0.,egui::Button::new(if account.adventure.stage==0 {"Deliver 6 oak wood + 4 stone"} else {"Complete contract"})).clicked() {request=Some(Action::Claim{camp});}
                } else {ui.label("Find a campfire and press F to speak with its keeper. Contracts are optional; your tools and world remain yours.");}
                if !self.feedback.is_empty() {ui.add_space(5.);ui.label(&self.feedback);}
                ui.separator();
                ui.collapsing("Expedition notes and controls",|ui| {
                ui.label("• Mine stone with the pickaxe and wood with the axe (I assigns your hotbar).\n• Hold a crystal to illuminate nearby terrain, including caves.\n• M opens the map. Right-click it to place a waypoint.\n• C crafts tools and materials; B builds devices.\n• The prompt console (`) lets you rewrite world rules.");
                });
                if let Some((x,y,z))=account.adventure.home {ui.small(format!("Recovery camp: {x}, {y}, {z} · Recoveries: {}",account.adventure.recoveries));}
                ui.small("If defeated, you return after 3 seconds and keep your inventory. Resting does not advance time.");
                let mut show_tracker=!self.hide_tracker;
                ui.checkbox(&mut show_tracker,"Show objective tracker while playing");
                self.hide_tracker=!show_tracker;
            });
        self.open = open;
        request
    }
    fn quest_rows(&mut self,ui:&mut egui::Ui,a:&Account) {
        ui.heading(format!("Travelers' contracts: {} / 20",a.adventure.quests.completed.count_ones()));
        ui.label("Objectives track your adventures automatically. Return to the named giver to claim 20 mana per quest. Deliveries consume supplies.");
        for npc in 0..6u8 {
            if self.npc.is_some_and(|n|n!=npc) {continue;}
            egui::CollapsingHeader::new(crate::quests::NAMES[npc as usize]).default_open(self.npc==Some(npc)).show(ui,|ui| {
                ui.label(crate::quests::GREETINGS[npc as usize]);
                for (id,q) in crate::quests::QUESTS.iter().enumerate().filter(|(_,q)|q.npc==npc) {
                    ui.separator();let done=a.adventure.quests.done(id);let n=crate::quests::progress(a,id);
                    ui.label(RichText::new(format!("{}{}",if done {"Completed: "}else{""},q.title)).strong());
                    ui.label(q.objective);
                    if !done {
                        ui.add(egui::ProgressBar::new(n as f32/q.target as f32).text(format!("{n}/{}",q.target)));
                        if ui.add_enabled(self.npc==Some(npc) && n>=q.target && self.quest_request.is_none(),egui::Button::new("Complete quest (+20 mana)")).clicked() {
                            self.quest_request=Some(crate::quests::Action::Claim{npc,quest:id as u8});
                        }
                        if id==15 && self.npc==Some(4) && ui.button("Light campfire (3 wood + 2 stone)").clicked() {self.quest_request=Some(crate::quests::Action::LightCampfire);}
                    }
                }
            });
        }
        if self.npc.is_none() {ui.small("The six travelers patrol near your first recovery camp. Look at one and press F.");}
        ui.separator();
    }
    pub fn hud(&self, ctx: &egui::Context, player: &Player, waypoint: Option<glam::Vec3>) {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("adventure_feedback"),
        ));
        if self.damage_flash > 0. {
            let r = ctx.screen_rect();
            let color =
                Color32::from_rgba_unmultiplied(180, 20, 25, (self.damage_flash * 150.) as u8);
            let edge = 10.;
            for rect in [
                egui::Rect::from_min_max(r.min, egui::pos2(r.right(), r.top() + edge)),
                egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - edge), r.max),
                egui::Rect::from_min_max(r.min, egui::pos2(r.left() + edge, r.bottom())),
                egui::Rect::from_min_max(egui::pos2(r.right() - edge, r.top()), r.max),
            ] {
                painter.rect_filled(rect, 0., color);
            }
        }
        if player.health <= 0. {
            egui::Area::new(egui::Id::new("recovery_overlay"))
                .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(20.)
                        .show(ui, |ui| {
                            ui.heading("You were overcome");
                            ui.label(format!(
                                "Returning to camp in {:.0}s…",
                                self.recovery_seconds.unwrap_or(3.).ceil()
                            ));
                            ui.label("Your inventory is safe.");
                        });
                });
            return;
        }
        if !self.hide_tracker {
            egui::Area::new(egui::Id::new("journal_tracker"))
                .anchor(
                    egui::Align2::LEFT_TOP,
                    [12., ctx.screen_rect().height() * 0.30 + 100.],
                )
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_black_alpha(145))
                        .rounding(5.)
                        .inner_margin(9.)
                        .show(ui, |ui| {
                            ui.set_max_width(260.);
                            ui.colored_label(
                                Color32::from_rgb(230, 190, 110),
                                format!("{}  [J]", player.crafting.adventure.title()),
                            );
                            if player.crafting.adventure.stage < 3 {
                                progress_rows(ui, &player.crafting);
                            }
                            if let Some(target) = waypoint {
                                let delta = target - player.position;
                                let ns = if delta.z < 0. { "N" } else { "S" };
                                let ew = if delta.x < 0. { "W" } else { "E" };
                                ui.label(format!(
                                    "Waypoint: {:.0}m {ns}{ew}",
                                    glam::Vec2::new(delta.x, delta.z).length()
                                ));
                            }
                        });
                });
        }
        if let Some((name, health, max)) = &self.target {
            egui::Area::new(egui::Id::new("target_health"))
                .anchor(egui::Align2::CENTER_CENTER, [0., 45.])
                .interactable(false)
                .show(ctx, |ui| {
                    ui.set_width(190.);
                    ui.add(
                        egui::ProgressBar::new((health / max).clamp(0., 1.))
                            .fill(Color32::from_rgb(170, 65, 60))
                            .text(format!("{name}  {health:.0}/{max:.0}")),
                    );
                });
        }
        if let Some(hint) = &self.hint {
            let pos = ctx.screen_rect().center() + egui::vec2(0., 78.);
            painter.text(
                pos + egui::vec2(1., 1.),
                egui::Align2::CENTER_TOP,
                hint,
                egui::FontId::proportional(14.),
                Color32::BLACK,
            );
            painter.text(
                pos,
                egui::Align2::CENTER_TOP,
                hint,
                egui::FontId::proportional(14.),
                Color32::LIGHT_GRAY,
            );
        }
    }
}
fn progress_rows(ui: &mut egui::Ui, a: &Account) {
    match a.adventure.stage {
        0 => {
            ui.label(format!(
                "Oak wood {}/6 · Stone {}/4",
                adventure::count(a, crate::voxel::BlockType::OakWood).min(6),
                adventure::count(a, crate::voxel::BlockType::Stone).min(4)
            ));
        }
        1 => {
            ui.label(if a.adventure.explored_depths {
                "Depths explored — return to a campkeeper"
            } else {
                "Explore a cave at Y 12 or below"
            });
        }
        2 => {
            ui.label(if a.adventure.crafted_tool {
                "Sword crafted — return to a campkeeper"
            } else {
                "Craft a new sword [C]"
            });
        }
        _ => {
            ui.label("Wayfinder · all contracts complete");
        }
    }
}
