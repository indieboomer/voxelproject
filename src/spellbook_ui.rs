use crate::spellbook::{SpellId, Spellbook, TargetRequirement, Validation};
use egui::{Color32, RichText, Vec2};

const GOLD: Color32 = Color32::from_rgb(236, 192, 99);
const PANEL: Color32 = Color32::from_rgb(17, 23, 32);

fn panel() -> egui::Frame {
    egui::Frame::none().fill(PANEL).stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(126, 104, 65))).inner_margin(8.0).rounding(3.0)
}

fn card(ui: &mut egui::Ui, spell: &crate::spellbook::Spell, selected: bool, mana_free: bool, width: f32, _is_host: bool) -> (egui::Response, bool) {
    let height = 286.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let tint = spell.artwork().tint();
    let gold = Color32::from_rgb(210, 177, 112);
    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(2.0, 3.0)), 6.0, Color32::from_black_alpha(90));
    p.rect_filled(rect, 6.0, Color32::from_rgb(24, 29, 39));
    p.rect_stroke(rect.shrink(1.0), 6.0, egui::Stroke::new(if selected { 2.5_f32 } else { 1.0_f32 }, if selected { tint } else { gold }));
    p.text(rect.min + egui::vec2(9.0, 9.0), egui::Align2::LEFT_TOP, &spell.name, egui::FontId::proportional(15.0), Color32::from_rgb(247, 219, 126));
    p.circle_filled(egui::pos2(rect.right() - 23.0, rect.top() + 20.0), 13.0, Color32::from_rgb(35, 45, 70));
    let mana_text = if mana_free { "0".to_string() } else { spell.mana_cost.to_string() };
    p.text(egui::pos2(rect.right() - 23.0, rect.top() + 20.0), egui::Align2::CENTER_CENTER, mana_text, egui::FontId::proportional(14.0), Color32::WHITE);
    let art = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 34.0), egui::vec2(width - 16.0, 104.0));
    p.rect_filled(art, 3.0, Color32::from_rgb(10, 15, 25));
    let image_rect = egui::Rect::from_center_size(art.center(), Vec2::splat(100.0));
    let _ = p;
    let mut child = ui.child_ui(image_rect, egui::Layout::top_down(egui::Align::Center));
    child.set_clip_rect(art.intersect(ui.clip_rect()));
    crate::spell_art::image(&mut child, spell.artwork(), Vec2::splat(100.0));
    let p = ui.painter();
    p.rect_stroke(art, 3.0, egui::Stroke::new(1.0_f32, gold));
    let kind = if spell.target == TargetRequirement::Optional { "INSTANT" } else { "ENCHANTMENT" };
    p.text(rect.min + egui::vec2(9.0, 145.0), egui::Align2::LEFT_TOP, kind, egui::FontId::proportional(10.0), tint);
    let mut job = egui::text::LayoutJob::simple(spell.description.clone(), egui::FontId::proportional(12.0), Color32::from_rgb(235, 225, 201), width - 18.0);
    job.wrap.max_rows = 3;
    let desc = ui.fonts(|f| f.layout_job(job));
    p.galley(rect.min + egui::vec2(9.0, 163.0), desc, Color32::from_rgb(235, 225, 201));
    let button_rect = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, height - 32.0), egui::vec2(width - 16.0, 24.0));
    let button = ui.interact(button_rect, response.id.with("cast"), egui::Sense::click());
    p.rect_filled(button_rect, 3.0, if button.hovered() { Color32::from_rgb(41, 76, 106) } else { Color32::from_rgb(25, 46, 69) });
    p.text(button_rect.center(), egui::Align2::CENTER_CENTER, if spell.target == TargetRequirement::Optional { "▶  Run" } else { "Target" }, egui::FontId::proportional(13.0), Color32::WHITE);
    (response.on_hover_text(format!("{}\n{} mana", spell.name, if mana_free { 0 } else { spell.mana_cost })), false)
}

pub enum Action { GenerateQuote(SpellId), SetQuote(SpellId, u32, String), Update(SpellId, String, TargetRequirement), Duplicate(SpellId), Delete(SpellId), Cast(SpellId), AllowGuests(SpellId, bool), Transfer(SpellId, String) }

#[derive(Default)]
pub struct Panel {
    pub open: bool, pub selected: Option<SpellId>, search: String, filter: String,
    name: String, target: TargetRequirement,
    pub feedback: String, trade_target: String, quote_job: Option<QuoteJob>, view_code: Option<SpellId>,
}
struct QuoteJob { id: SpellId, revision: u32, receiver: std::sync::mpsc::Receiver<Result<String, String>> }

impl Panel {
    pub fn start_quote(&mut self, spell: &crate::spellbook::Spell, url: &str) {
        if self.quote_job.is_some() { self.feedback = "A quote is already being written.".into(); return; }
        let (sender, receiver) = std::sync::mpsc::channel();
        let description = format!("{}: {}", spell.name, spell.description);
        self.quote_job = Some(QuoteJob { id: spell.id, revision: spell.revision, receiver });
        self.feedback = format!("Writing a quote for {}...", spell.name);
        let url = url.to_string();
        std::thread::spawn(move || { let _ = sender.send(crate::llm::request_flavor_quote(&url, &description)); });
    }
    pub fn poll_quote(&mut self) -> Option<Action> {
        let job = self.quote_job.as_ref()?;
        let result = match job.receiver.try_recv() { Ok(result) => result, Err(std::sync::mpsc::TryRecvError::Empty) => return None, Err(_) => Err("Quote generation stopped; try again.".into()) };
        let job = self.quote_job.take().unwrap();
        match result { Ok(quote) => Some(Action::SetQuote(job.id, job.revision, quote)), Err(error) => { self.feedback = error; None } }
    }
    pub fn draw(&mut self, ctx: &egui::Context, book: &Spellbook, is_host: bool, mana_free: bool) -> Option<Action> {
        if !self.open { return None; }
        let mut open = true; let mut close_requested = false; let mut action = None; let screen = ctx.screen_rect();
        egui::Window::new("Spellbook").open(&mut open).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(egui::vec2((screen.width() - 36.0).min(1320.0), (screen.height() - 36.0).min(860.0))).resizable(false).collapsible(false).show(ctx, |ui| {
            ui.visuals_mut().override_text_color = Some(Color32::from_rgb(235, 226, 207));
            ui.horizontal(|ui| { ui.label(RichText::new("SPELLBOOK").size(27.0).strong().color(GOLD)); ui.label("Your collection of spells. Remember, organize and use them to shape the world."); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if ui.small_button("×").clicked() { close_requested = true; } ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search spells...").desired_width(220.0)); }); });
            ui.separator();
            let count = |kind: &str| book.spells.iter().filter(|s| kind == "All" || (kind == "Instant" && s.target == TargetRequirement::Optional) || (kind != "Instant" && s.target != TargetRequirement::Optional)).count();
            ui.horizontal(|ui| { for kind in ["All", "Instant", "Persistent", "Attached"] { if ui.selectable_label(self.filter == kind || (self.filter.is_empty() && kind == "All"), format!("{} ({})", kind, count(kind))).clicked() { self.filter = kind.into(); } } });
            let query = self.search.to_lowercase(); let filter = if self.filter.is_empty() { "All" } else { self.filter.as_str() };
            let visible: Vec<_> = book.spells.iter().filter(|spell| { let text_match = query.is_empty() || format!("{} {} {}", spell.name, spell.description, spell.original_prompt).to_lowercase().contains(&query); let type_match = filter == "All" || (filter == "Instant" && spell.target == TargetRequirement::Optional) || (filter != "Instant" && filter != "All" && spell.target != TargetRequirement::Optional); text_match && type_match }).collect();
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical().id_source("spell_card_grid").auto_shrink([false, false]).show(&mut columns[0], |ui| { let n = ((ui.available_width() + 10.0) / 190.0).floor().max(1.0) as usize; let w = ((ui.available_width() - 10.0 * (n - 1) as f32) / n as f32).max(150.0); for row in visible.chunks(n) { ui.horizontal(|ui| { ui.spacing_mut().item_spacing.x = 10.0; for spell in row { let (response, delete) = card(ui, spell, self.selected == Some(spell.id), mana_free, w, is_host); if delete { action = Some(Action::Delete(spell.id)); } else if response.clicked() { self.selected = Some(spell.id); } } }); ui.add_space(10.0); } if visible.is_empty() { ui.label(if book.spells.is_empty() { "Your collection awaits its first spell. Generate one in Spell Workshop." } else { "No spells match this search or filter." }); } });
                egui::ScrollArea::vertical().id_source("spell_detail").auto_shrink([false, false]).show(&mut columns[1], |ui| {
                    if let Some(spell) = self.selected.and_then(|id| book.get(id)) { panel().show(ui, |ui| {
                        ui.horizontal(|ui| { crate::spell_art::image(ui, spell.artwork(), Vec2::splat(150.0)); ui.vertical(|ui| { ui.heading(RichText::new(&spell.name).color(GOLD)); ui.colored_label(if spell.target == TargetRequirement::Optional { Color32::from_rgb(164, 239, 170) } else { Color32::LIGHT_BLUE }, if spell.target == TargetRequirement::Optional { "INSTANT" } else { "ENCHANTMENT" }); ui.label(format!("Mana cost  {}", if mana_free { 0 } else { spell.mana_cost })); if spell.ready() && ui.button("▶  Run spell").clicked() { action = Some(Action::Cast(spell.id)); } if spell.target != TargetRequirement::Optional { let _ = ui.button("Target"); } }); });
                        if !spell.flavor_quote.is_empty() { ui.label(RichText::new(format!("“{}”", spell.flavor_quote)).italics()); } ui.separator(); ui.label(&spell.description);
                        ui.collapsing("Original Prompt", |ui| { ui.label(&spell.original_prompt); });
                        ui.collapsing("Advanced Info", |ui| { ui.label(format!("Spell ID: {}  ·  Revision {}  ·  Author: {}  ·  API {}", spell.id, spell.revision, spell.author, spell.api_version)); match &spell.validation { Validation::Ready { checked_api } => { ui.colored_label(Color32::LIGHT_GREEN, format!("Validated against API {checked_api}")); }, Validation::Review { reason } => { ui.colored_label(Color32::LIGHT_RED, reason); } } });
                        ui.horizontal_wrapped(|ui| { if ui.button("View Code").clicked() { self.view_code = Some(spell.id); } if is_host && ui.button("Duplicate").clicked() { action = Some(Action::Duplicate(spell.id)); } if is_host && ui.button(RichText::new("Delete").color(Color32::LIGHT_RED)).clicked() { action = Some(Action::Delete(spell.id)); } });
                        if is_host { ui.collapsing("Manage spell", |ui| { ui.horizontal(|ui| { ui.label("Name"); ui.text_edit_singleline(&mut self.name); }); egui::ComboBox::from_id_source("spell_target_requirement").selected_text(self.target.label()).show_ui(ui, |ui| { for target in [TargetRequirement::Optional, TargetRequirement::Creature, TargetRequirement::Block] { ui.selectable_value(&mut self.target, target, target.label()); } }); if ui.button("Save changes").clicked() { action = Some(Action::Update(spell.id, self.name.clone(), self.target)); } if ui.add_enabled(self.quote_job.is_none(), egui::Button::new("Generate quote (local AI)")).clicked() { action = Some(Action::GenerateQuote(spell.id)); } let mut allowed = spell.allow_guests; if ui.checkbox(&mut allowed, "Allow guests to cast").changed() { action = Some(Action::AllowGuests(spell.id, allowed)); } ui.horizontal(|ui| { ui.label("Give card to"); ui.text_edit_singleline(&mut self.trade_target); if ui.add_enabled(!self.trade_target.trim().is_empty(), egui::Button::new("Give card")).clicked() { action = Some(Action::Transfer(spell.id, self.trade_target.trim().to_owned())); } }); }); }
                    }); } else { ui.centered_and_justified(|ui| { ui.label("Select a spell to inspect its details."); }); }
                });
            });
            ui.separator(); ui.horizontal(|ui| { ui.label("📖  Tip: Create and manage spells in Spell Workshop."); if ui.button("Open Spell Workshop [~]").clicked() { close_requested = true; } });
        });
        if close_requested { open = false; }
        self.open = open;
        if let Some(id) = self.view_code { if let Some(spell) = book.get(id) { let mut code_open = true; let mut code = spell.source.clone(); egui::Window::new(format!("Spell Code: {}", spell.name)).open(&mut code_open).resizable(true).default_size(Vec2::new(620.0, 440.0)).show(ctx, |ui| { ui.label(format!("Original prompt: {}", spell.original_prompt)); ui.add(egui::TextEdit::multiline(&mut code).code_editor().desired_width(f32::INFINITY)); }); if !code_open { self.view_code = None; } } else { self.view_code = None; } }
        action
    }
}
