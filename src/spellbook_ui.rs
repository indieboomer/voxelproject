use crate::spellbook::{SpellId, Spellbook, TargetRequirement, Validation};

/// Original fantasy card frame. Clicking inspects; casting remains an explicit action.
fn card(
    ui: &mut egui::Ui,
    spell: &crate::spellbook::Spell,
    selected: bool,
    mana_free: bool,
    width: f32,
    is_host: bool,
) -> (egui::Response, bool) {
    use egui::{Color32, FontId, Pos2, Rect, Stroke, Vec2};
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 394.), egui::Sense::click());
    let tint = spell.artwork().tint();
    let gold = Color32::from_rgb(210, 177, 112);
    let ink = Color32::from_rgb(55, 42, 34);
    let ivory = Color32::from_rgb(243, 229, 195);
    let p = ui.painter();
    p.rect_filled(
        rect.translate(Vec2::new(2., 3.)),
        9.,
        Color32::from_black_alpha(85),
    );
    p.rect_filled(rect, 9., Color32::from_rgb(25, 28, 40));
    p.rect_stroke(
        rect.shrink(1.),
        9.,
        Stroke::new(
            if selected { 2.5_f32 } else { 1_f32 },
            if selected { tint } else { gold },
        ),
    );
    p.rect_stroke(
        rect.shrink(5.),
        6.,
        Stroke::new(1_f32, Color32::from_rgb(84, 73, 63)),
    );
    let title = Rect::from_min_max(
        rect.min + Vec2::new(12., 10.),
        rect.min + Vec2::new(width - 44., 47.),
    );
    card_text(ui, title, &spell.name, 16., ivory, 2);
    let cost = Pos2::new(rect.right() - 26., rect.top() + 28.);
    ui.painter()
        .circle_filled(cost, 14., Color32::from_rgb(44, 43, 66));
    ui.painter()
        .circle_stroke(cost, 14., Stroke::new(1_f32, tint));
    ui.painter().text(
        cost,
        egui::Align2::CENTER_CENTER,
        if mana_free { 0 } else { spell.mana_cost }.to_string(),
        FontId::proportional(16.),
        ivory,
    );
    let art = Rect::from_min_size(rect.min + Vec2::new(12., 52.), Vec2::new(width - 24., 128.));
    ui.painter()
        .rect_filled(art, 3., Color32::from_rgb(13, 17, 28));
    let image_rect = Rect::from_center_size(art.center(), Vec2::splat(128.));
    let mut child = ui.child_ui(image_rect, egui::Layout::top_down(egui::Align::Center));
    child.set_clip_rect(art.intersect(ui.clip_rect()));
    crate::spell_art::image(&mut child, spell.artwork(), Vec2::splat(128.));
    ui.painter().rect_stroke(art, 3., Stroke::new(1_f32, gold));
    card_text(
        ui,
        Rect::from_min_size(rect.min + Vec2::new(12., 187.), Vec2::new(width - 24., 18.)),
        &format!("INSTANT  /  {}", spell.target.label()),
        11.,
        tint,
        1,
    );
    if !spell.flavor_quote.is_empty() {
        let quote_rect =
            Rect::from_min_size(rect.min + Vec2::new(16., 211.), Vec2::new(width - 32., 48.));
        let mut job = egui::text::LayoutJob::simple(
            format!("“{}”", spell.flavor_quote),
            FontId::proportional(12.),
            ivory,
            quote_rect.width(),
        );
        job.wrap.max_rows = 3;
        for section in &mut job.sections {
            section.format.italics = true;
        }
        let galley = ui.fonts(|f| f.layout_job(job));
        ui.painter()
            .with_clip_rect(quote_rect.intersect(ui.clip_rect()))
            .galley(quote_rect.min, galley, ivory);
    }
    let text_rect =
        Rect::from_min_size(rect.min + Vec2::new(12., 265.), Vec2::new(width - 24., 76.));
    ui.painter()
        .rect_filled(text_rect, 3., Color32::from_rgb(221, 205, 174));
    card_text(ui, text_rect.shrink(7.), &spell.description, 13., ink, 4);
    card_text(
        ui,
        Rect::from_min_size(rect.min + Vec2::new(12., 348.), Vec2::new(width - 24., 18.)),
        &format!(
            "{} blocks   /   {}s cooldown",
            spell.range, spell.cooldown_seconds
        ),
        12.,
        ivory,
        1,
    );
    card_text(
        ui,
        Rect::from_min_size(
            rect.min + Vec2::new(12., 369.),
            Vec2::new(width - if is_host { 106. } else { 24. }, 16.),
        ),
        if !spell.ready() {
            "REVIEW NEEDED"
        } else if selected {
            "SELECTED"
        } else {
            "INSPECT SPELL"
        },
        10.,
        if spell.ready() {
            gold
        } else {
            Color32::LIGHT_RED
        },
        1,
    );
    let delete_clicked = if is_host {
        let button_rect =
            Rect::from_min_size(rect.min + Vec2::new(width - 86., 366.), Vec2::new(74., 21.));
        let button = ui.interact(
            button_rect,
            response.id.with("delete"),
            egui::Sense::click(),
        );
        ui.painter().rect_filled(
            button_rect,
            3.,
            if button.hovered() {
                Color32::from_rgb(94, 49, 48)
            } else {
                Color32::from_rgb(59, 38, 40)
            },
        );
        ui.painter().text(
            button_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Delete",
            FontId::proportional(12.),
            Color32::from_rgb(255, 172, 156),
        );
        button
            .on_hover_text(
                "Delete this spell and clear its hotbar bindings. Save with F5 to keep the change.",
            )
            .clicked()
    } else {
        false
    };
    (
        response.on_hover_text(format!(
            "{}\n{}\n{} mana · {}\nClick to inspect and manage.",
            spell.name,
            spell.description,
            if mana_free { 0 } else { spell.mana_cost },
            if spell.allow_guests {
                "Shared with guests"
            } else {
                "Host spell"
            }
        )),
        delete_clicked,
    )
}

fn card_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    size: f32,
    color: egui::Color32,
    rows: usize,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.into(),
        egui::FontId::proportional(size),
        color,
        rect.width(),
    );
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = false;
    let galley = ui.fonts(|f| f.layout_job(job));
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(rect.min, galley, color);
}

#[cfg(test)]
mod card_tests {
    #[test]
    fn card_delete_button_works_without_selecting_and_is_host_only() {
        let module = crate::scripting::Module::load(
            "Healing".into(),
            "Heal".into(),
            include_str!("../modules/target_heal.lua").into(),
        )
        .unwrap();
        let mut book = crate::spellbook::Spellbook::default();
        let id = book.remember(&module, "Host").unwrap();
        for host in [true, false] {
            let ctx = egui::Context::default();
            let mut position = egui::pos2(0., 0.);
            let mut deletes = 0;
            for frame in 0..4 {
                let mut events = Vec::new();
                if frame >= 2 {
                    events.push(egui::Event::PointerMoved(position));
                    events.push(egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: frame == 2,
                        modifiers: Default::default(),
                    });
                }
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640., 480.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let (response, deleted) =
                                super::card(ui, book.get(id).unwrap(), false, false, 230., host);
                            position = egui::pos2(
                                response.rect.right() - 40.,
                                response.rect.bottom() - 17.,
                            );
                            if deleted {
                                deletes += 1;
                            }
                        });
                    },
                );
            }
            assert_eq!(deletes, usize::from(host));
        }
    }
}

pub enum Action {
    GenerateQuote(SpellId),
    SetQuote(SpellId, u32, String),
    Update(SpellId, String, TargetRequirement),
    Duplicate(SpellId),
    Delete(SpellId),
    Cast(SpellId),
    AllowGuests(SpellId, bool),
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
    quote_job: Option<QuoteJob>,
}
struct QuoteJob {
    id: SpellId,
    revision: u32,
    receiver: std::sync::mpsc::Receiver<Result<String, String>>,
}
impl Panel {
    pub fn start_quote(&mut self, spell: &crate::spellbook::Spell, url: &str) {
        if self.quote_job.is_some() {
            self.feedback = "A quote is already being written.".into();
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let description = format!("{}: {}", spell.name, spell.description);
        let url = url.to_string();
        self.quote_job = Some(QuoteJob {
            id: spell.id,
            revision: spell.revision,
            receiver,
        });
        self.feedback = format!("Writing a quote for {}...", spell.name);
        std::thread::spawn(move || {
            let _ = sender.send(crate::llm::request_flavor_quote(&url, &description));
        });
    }
    pub fn poll_quote(&mut self) -> Option<Action> {
        let job = self.quote_job.as_ref()?;
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(_) => Err("Quote generation stopped; try again.".into()),
        };
        let job = self.quote_job.take().unwrap();
        match result {
            Ok(quote) => Some(Action::SetQuote(job.id, job.revision, quote)),
            Err(error) => {
                self.feedback = error;
                None
            }
        }
    }
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
        egui::Window::new("Spellbook").open(&mut open).default_width(980.0)
            .default_height(ctx.screen_rect().height()*0.84)
            .max_width((ctx.screen_rect().width()-32.).max(220.))
            .max_height(ctx.screen_rect().height()*0.88).vscroll(true)
            .show(ctx,|ui| {
                if is_host {ui.label("Remember a generated spell from the Rules panel, then manage it here.");}
                if !is_host {ui.label("Spells shared by the host. You can inspect, assign and cast them; the host manages their definitions.");}
                ui.horizontal_wrapped(|ui|{
                    ui.heading("Your collection");
                    ui.label(format!("{} / {} spells",book.spells.len(),crate::spellbook::MAX_SPELLS));
                    ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search spells...").desired_width(220.));
                });
                let query=self.search.to_lowercase();
                let visible:Vec<_>=book.spells.iter().filter(|spell|
                    format!("{} {}",spell.name,spell.description).to_lowercase().contains(&query)).collect();
                egui::ScrollArea::vertical().id_source("spell_cards")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_height((ctx.screen_rect().height()*0.48).max(405.)).show(ui,|ui| {
                    let columns=((ui.available_width()+12.)/230.).floor().max(1.) as usize;
                    let card_width=((ui.available_width()-12.*(columns-1) as f32)/columns as f32).min(250.);
                    for row in visible.chunks(columns) {
                        ui.horizontal(|ui|{
                            ui.spacing_mut().item_spacing.x=12.;
                            for spell in row {
                                let (response,delete)=card(ui,spell,self.selected==Some(spell.id),mana_free,card_width,is_host);
                                if delete {action=Some(Action::Delete(spell.id));}
                                else if response.clicked(){self.selected=Some(spell.id);}
                            }
                        });
                        ui.add_space(12.);
                    }
                    if book.spells.is_empty(){ui.label("Your collection awaits its first spell. Generate an instant spell and choose Remember in Rules.");}
                    else if visible.is_empty(){ui.label("No spells match your search.");}
                });
                if let Some(spell)=self.selected.and_then(|id|book.get(id)) {
                    if self.editing!=Some((spell.id,spell.revision)) {
                        self.editing=Some((spell.id,spell.revision));self.name=spell.name.clone();self.target=spell.target;
                    }
                    ui.separator();
                    ui.horizontal(|ui|{
                        crate::spell_art::image(ui,spell.artwork(),egui::vec2(40.,40.));
                        ui.heading(&spell.name);
                    });
                    if !spell.flavor_quote.is_empty(){ui.label(egui::RichText::new(format!("“{}”",spell.flavor_quote)).italics());}
                    ui.label(&spell.description);
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
                    if ui.add_enabled(self.quote_job.is_none(),egui::Button::new(if spell.flavor_quote.is_empty(){"Generate quote (local AI)"}else{"Rewrite quote (local AI)"})).clicked(){action=Some(Action::GenerateQuote(spell.id));}
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
