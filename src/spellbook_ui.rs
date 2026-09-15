use crate::spellbook::{SpellId, Spellbook, TargetRequirement, Validation};

pub enum Action {
    Update(SpellId, String, TargetRequirement),
    Duplicate(SpellId),
    Delete(SpellId),
    Cast(SpellId),
    AllowGuests(SpellId,bool),
}
#[derive(Default)]
pub struct Panel {
    pub open: bool,
    pub selected: Option<SpellId>,
    search: String,
    editing: Option<(SpellId, u32)>,
    name: String,
    target: TargetRequirement,
    pub feedback: String,
}
impl Panel {
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        book: &Spellbook,
        is_host: bool,
        mana_free: bool,
    ) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = self.open;
        let mut action = None;
        egui::Window::new("Spellbook").open(&mut open).default_width(720.0)
            .max_height(ctx.screen_rect().height()*0.8).vscroll(true)
            .show(ctx,|ui| {
                if is_host {ui.label("Remember a generated spell from the Rules panel, then manage it here.");}
                if !is_host {ui.label("Spells shared by the host. You can inspect, assign and cast them; the host manages their definitions.");}
                ui.horizontal(|ui|{ui.label("Search");ui.text_edit_singleline(&mut self.search);});
                let query=self.search.to_lowercase();
                egui::ScrollArea::vertical().id_source("spell_list").max_height(190.0).show(ui,|ui| {
                    for spell in &book.spells {
                        if !format!("{} {}",spell.name,spell.description).to_lowercase().contains(&query) {continue;}
                        let status=if spell.ready(){"Ready"}else{"Review needed"};
                        if ui.selectable_label(self.selected==Some(spell.id),format!("{}  ·  {status}",spell.name)).clicked() {self.selected=Some(spell.id);}
                    }
                    if book.spells.is_empty(){ui.label("No remembered spells yet.");}
                });
                if let Some(spell)=self.selected.and_then(|id|book.get(id)) {
                    if self.editing!=Some((spell.id,spell.revision)) {
                        self.editing=Some((spell.id,spell.revision));self.name=spell.name.clone();self.target=spell.target;
                    }
                    ui.separator();
                    ui.horizontal(|ui|{
                        let (rect,_)=ui.allocate_exact_size(egui::vec2(32.0,32.0),egui::Sense::hover());
                        let color=match spell.icon {1=>egui::Color32::LIGHT_GREEN,2=>egui::Color32::LIGHT_BLUE,_=>egui::Color32::from_rgb(190,145,245)};
                        let center=rect.center();
                        ui.painter().circle_stroke(center,13.0,egui::Stroke::new(2.0_f32,color));
                        for (a,b) in [(egui::vec2(-7.0,0.0),egui::vec2(7.0,0.0)),(egui::vec2(0.0,-9.0),egui::vec2(0.0,9.0))] {
                            ui.painter().line_segment([center+a,center+b],egui::Stroke::new(2.0_f32,color));
                        }
                        ui.heading(&spell.name);
                    });ui.label(&spell.description);
                    ui.horizontal_wrapped(|ui|{
                        if ui.add_enabled(spell.ready(),egui::Button::new("Cast at current aim")).clicked(){action=Some(Action::Cast(spell.id));}
                        if ui.add_enabled(is_host,egui::Button::new("Duplicate")).clicked(){action=Some(Action::Duplicate(spell.id));}
                        if ui.add_enabled(is_host,egui::Button::new("Delete")).clicked(){action=Some(Action::Delete(spell.id));}
                    });
                    ui.small("Aim before opening the book. Casting spends mana and affects the world.");
                    ui.label(format!("Target: {}  |  Range: {} blocks  |  Cost: {} mana  |  Cooldown: {} s",
                        spell.target.label(),spell.range,if mana_free{0}else{spell.mana_cost},spell.cooldown_seconds));
                    ui.label(format!("Revision {}  ·  Author: {}",spell.revision,spell.author));
                    match &spell.validation {
                        Validation::Ready{..}=>{ui.colored_label(egui::Color32::LIGHT_GREEN,"Compatible with this game. Review the effect before casting.");}
                        Validation::Review{reason}=>{ui.colored_label(egui::Color32::LIGHT_RED,reason);}
                    }
                    if is_host {
                    let mut allowed=spell.allow_guests;
                    if ui.checkbox(&mut allowed,"Allow guests to cast this spell").on_hover_text("Guests can execute this saved code with its normal World API capabilities. The host validates each cast and charges the caster's mana.").changed() {action=Some(Action::AllowGuests(spell.id,allowed));}
                    ui.horizontal(|ui|{ui.label("Name");ui.text_edit_singleline(&mut self.name);});
                    egui::ComboBox::from_id_source("spell_target_requirement").selected_text(self.target.label()).show_ui(ui,|ui|{
                        for target in [TargetRequirement::Optional,TargetRequirement::Creature,TargetRequirement::Block] {
                            ui.selectable_value(&mut self.target,target,target.label());
                        }
                    });
                    ui.small("Creature or Block requires that aim. Spell-defined keeps the spell's own targeting.");
                    ui.horizontal_wrapped(|ui|{
                        if ui.button("Save changes").clicked(){action=Some(Action::Update(spell.id,self.name.clone(),self.target));}
                    });
                    ui.collapsing("Original prompt",|ui|{ui.label(&spell.original_prompt);});
                    ui.collapsing("Advanced: source and validation",|ui|{
                        ui.label(format!("Spell ID: {} · Source API: {}",spell.id,spell.api_version));
                        if let Validation::Ready{checked_api}=&spell.validation {ui.label(format!("Checked against API {checked_api}: syntax, API names, sandbox loading and metadata; not proof of intended behavior."));}
                        ui.add(egui::Label::new(egui::RichText::new(&spell.source).monospace()).wrap(true));
                        if let Some(plan)=&spell.interpretation {ui.label(plan.to_string());}
                    });
                    }
                }
                if !self.feedback.is_empty(){ui.separator();ui.label(&self.feedback);}
                ui.small(if is_host {"Save your world with F5 to keep changes."} else {"The host saves shared spells and your hotbar assignments."});
            });
        self.open = open;
        action
    }
}
