//! Device controls; commands are requests, never local inventory mutations.
use crate::automation::*;
use crate::crafting::{Account, Registry};
#[path = "chest_ui.rs"]
mod chest_ui;

pub struct Panel {
    pub open: bool,
    pub selected: Option<Cell>,
    pub build: Option<(Kind, u8, Option<usize>)>,
    config: Config,
    revision: u64,
    item: String,
    amount: u32,
    pub feedback: String,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            selected: None,
            build: None,
            config: Config::default(),
            revision: 0,
            item: element_id(0).into(),
            amount: 1,
            feedback: String::new(),
        }
    }
}
impl Panel {
    /// Preserve the inventory selection while machine controls own interaction.
    pub fn tools_suspended(&self) -> bool {
        self.open || self.build.is_some()
    }
    pub fn inspect(&mut self, d: &Device) {
        self.open = true;
        self.selected = Some(d.cell);
        self.config = d.config.clone();
        self.revision = d.config_revision;
        self.feedback.clear();
        if d.kind==Kind::Smelter {self.item="resource:copper_ore".into();}
    }
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        state: &State,
        account: &Account,
        registry: &Registry,
    ) -> Option<Action> {
        let mut command = None;
        if let Some((kind, rotation, _)) = self.build {
            egui::Window::new("Placement").anchor(egui::Align2::CENTER_BOTTOM,[0.0,-110.0]).title_bar(false).show(ctx,|ui| {
                ui.label(format!("{} · turn {} · Left/right click: place · R: rotate · B/Esc: exit",balance().def(kind).name,rotation));
                if kind!=Kind::Chest {ui.small("Ports: blue single mark = mana · amber pair = matter · violet bars = signal");}
                if !self.feedback.is_empty() {ui.label(&self.feedback);}
            });
        }
        if !self.open {
            return None;
        }
        if let Some(chest)=self.selected.and_then(|p|state.devices.get(&p)).filter(|d|d.kind==Kind::Chest) {
            return chest_ui::draw(ctx,chest,account,&mut self.open,&mut self.item,&mut self.amount,&self.feedback);
        }
        let mut open = self.open;
        let mut close = false;
        egui::Window::new("Mage's workshop").open(&mut open).default_pos(egui::pos2(16.,56.)).default_width(410.0).vscroll(true).show(ctx,|ui| {
            ui.label(&self.feedback);
            if let Some(p)=self.selected {
                let Some(d)=state.devices.get(&p)else{ui.label("Device removed. Open B to build.");return;};
                if self.config==d.config {self.revision=d.config_revision;}
                ui.heading(&balance().def(d.kind).name);ui.small(format!("Cell {p:?} · rotation {}",d.rotation));
                ui.label(format!("Activity: {:?} · Network mana: {}/{}",d.activity,d.mana,balance().def(d.kind).mana_capacity));
                if let Some(batch)=&d.batch {ui.add(egui::ProgressBar::new(batch.elapsed as f32/batch.duration as f32).show_percentage());ui.small("This paid cycle keeps its recipe until complete.");}
                ui.checkbox(&mut self.config.enabled,"Enabled");
                if matches!(d.kind,Kind::Chest|Kind::Smelter) {
                    ui.checkbox(&mut self.config.eject_contents,if d.kind==Kind::Smelter {"Eject finished metal nearby"}else{"Eject contents nearby (bags / living creatures)"});
                    ui.small("Turn off to store stock or feed another machine through the output port.");
                }
                if d.kind==Kind::Smelter {
                    let s=&balance().smelting;
                    ui.label(format!("Instant: 1 ore → 1 metal. {} mana per ore, or wood/coal.",registry.mana_charge(s.mana_cost)));
                    ui.small(format!("One wood: {} ores. One coal: {} ores. Stored heat: {} ores.",s.wood_heat,s.coal_heat,d.fuel_heat));
                    ui.small("Automatic energy: saved heat, then mana, then coal, then wood. No fuel is spent in mana-free testing mode.");
                    for (ore,metal) in &s.recipes {ui.small(format!("{} → {}",ore.trim_start_matches("resource:"),metal.trim_start_matches("resource:")));}
                    if d.activity==Activity::MissingFuel {ui.label("Needs mana, coal, or wood in the input buffer.");}
                }
                if matches!(d.kind,Kind::Condenser|Kind::Dissipator) {
                    egui::ComboBox::from_label("Element").selected_text(element_id(self.config.element)).show_ui(ui,|ui|{for i in 0..5 {ui.selectable_value(&mut self.config.element,i,element_id(i));}});
                    ui.small(format!("Condenser cost: {} · Dissipator return: {} mana",registry.mana_charge(balance().condenser_costs[self.config.element]),balance().dissipator_yields[self.config.element]));
                }
                if d.kind==Kind::Workshop {
                    egui::ComboBox::from_label("Ordered recipe").selected_text(if self.config.recipe.is_empty(){"Select recipe"}else{&self.config.recipe}).show_ui(ui,|ui|{
                        for r in &registry.recipes {ui.selectable_value(&mut self.config.recipe,r.id.clone(),&r.id);}
                    });
                    if let Some(r)=registry.recipes.iter().find(|r|r.id==self.config.recipe) {
                        let mut slots=[None;5];for (i,s) in r.inputs.iter().enumerate(){slots[i]=Some(*s);}
                        let ingredients=r.inputs.iter().map(|s|format!("{} {}",s.amount,element_id(s.element.index()).trim_start_matches("element:"))).collect::<Vec<_>>().join(" → ");
                        ui.label(format!("{ingredients} → {} ×{}",r.output.id,r.output.quantity));
                        ui.small(format!("{} network mana per cycle; arrival order is ignored",registry.mana_cost(&slots)));
                    }
                }
                if balance().def(d.kind).mana_capacity>0 {ui.horizontal(|ui|{ui.label("Reserve");ui.add(egui::DragValue::new(&mut self.config.reserve).clamp_range(0..=balance().def(d.kind).mana_capacity));});}
                if matches!(d.kind,Kind::Conduit|Kind::Channel|Kind::Splitter|Kind::Valve) {
                    egui::ComboBox::from_label("Local input face").selected_text(format!("{:?}",self.config.input)).show_ui(ui,|ui|{for f in Face::ALL {ui.selectable_value(&mut self.config.input,f,format!("{f:?}"));}});
                    ui.label("Local output faces (selection order sets priority):");
                    ui.horizontal_wrapped(|ui|{for f in Face::ALL {if f==self.config.input {continue;}let mut selected=self.config.outputs.contains(&f);
                        if ui.checkbox(&mut selected,format!("{f:?}")).changed(){if selected{self.config.outputs.push(f);}else{self.config.outputs.retain(|v|*v!=f);}}
                    }});
                    self.config.outputs.retain(|f|*f!=self.config.input);
                }
                if d.kind==Kind::Splitter {
                    egui::ComboBox::from_label("Routing").selected_text(format!("{:?}",self.config.routing)).show_ui(ui,|ui|{for mode in [Routing::RoundRobin,Routing::Priority,Routing::Filter]{ui.selectable_value(&mut self.config.routing,mode,format!("{mode:?}"));}});
                    ui.label("Filter item ID (matching → first output, others → remaining):");ui.text_edit_singleline(&mut self.config.filter);
                }
                if d.kind==Kind::Sensor {
                    ui.label("Target chest/vessel cell (within 16 blocks):");
                    ui.horizontal(|ui|{ui.add(egui::DragValue::new(&mut self.config.sensor_target.0));ui.add(egui::DragValue::new(&mut self.config.sensor_target.1));ui.add(egui::DragValue::new(&mut self.config.sensor_target.2));});
                    ui.label("Item ID, or empty for total chest inventory:");ui.text_edit_singleline(&mut self.config.sensor_item);
                    ui.horizontal(|ui|{ui.label("Off below/equal");ui.add(egui::DragValue::new(&mut self.config.lower));ui.label("On above/equal");ui.add(egui::DragValue::new(&mut self.config.upper));});
                    egui::ComboBox::from_label("Signal mode").selected_text(format!("{:?}",self.config.signal_mode)).show_ui(ui,|ui|{for m in [SignalMode::State,SignalMode::Pulse,SignalMode::Numeric]{ui.selectable_value(&mut self.config.signal_mode,m,format!("{m:?}"));}});
                }
                if d.kind==Kind::Valve {ui.checkbox(&mut self.config.valve_matter,"Matter valve (off = mana)");ui.checkbox(&mut self.config.invert,"Open when signal is OFF (stock-limit control)");}
                ui.label(format!("Signal value: {}",d.signal));
                ui.horizontal(|ui|{
                    if ui.button("Apply settings").clicked(){command=Some(Action::Configure{cell:p,config:self.config.clone(),expected:self.revision});}
                    if ui.button("Rotate").clicked(){command=Some(Action::Rotate{cell:p});}
                    if ui.button("Pack intact").clicked(){command=Some(Action::Pack{cell:p});}
                });
                if balance().def(d.kind).item_capacity>0 {
                ui.separator();
                if d.kind==Kind::Chest {ui.label(format!("Stored: {} (no slot limit)",d.item_count()));} else {ui.label(format!("Stored matter: {}/{}",d.item_count(),balance().def(d.kind).item_capacity));}
                for (id,n) in &d.items {ui.small(format!("Input/storage: {id} ×{n}"));}
                for (id,n) in &d.output {ui.small(format!("Output: {id} ×{n}"));}
                let mut choices:Vec<String>=(0..5).map(|i|element_id(i).into()).collect();
                choices.extend(crate::voxel::COLLECTIBLE_BLOCKS.iter().map(|b|format!("resource:{}",b.id())));
                choices.extend(["item:axe","item:pickaxe","item:sword","item:bow"].map(str::to_owned));
                choices.extend(account.production_goods.keys().cloned());choices.extend(d.items.keys().cloned());choices.extend(d.output.keys().cloned());
                choices.sort();choices.dedup();
                egui::ComboBox::from_label("Transfer matter").selected_text(&self.item).show_ui(ui,|ui|{for item in choices {ui.selectable_value(&mut self.item,item.clone(),item);}});
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.amount).clamp_range(1..=256));
                    if ui.button("Deposit").clicked(){command=Some(Action::Deposit{cell:p,item:self.item.clone(),amount:self.amount});}
                    if ui.button("Withdraw").clicked(){command=Some(Action::Withdraw{cell:p,item:self.item.clone(),amount:self.amount});}
                });
                ui.small(format!("You carry {} of this matter",account_count(account,&self.item)));
                ui.horizontal(|ui| {
                    let carried=account_count(account,&self.item);
                    let stored=d.items.get(&self.item).copied().unwrap_or(0).saturating_add(d.output.get(&self.item).copied().unwrap_or(0));
                    if ui.add_enabled(carried>0,egui::Button::new("Deposit all selected")).clicked(){command=Some(Action::Deposit{cell:p,item:self.item.clone(),amount:carried});}
                    if ui.add_enabled(stored>0,egui::Button::new("Take all selected")).clicked(){command=Some(Action::Withdraw{cell:p,item:self.item.clone(),amount:stored});}
                });
                if self.item.starts_with("creature:") && account_count(account,&self.item)>0 && ui.button("Release one carried bound creature nearby").clicked() {
                    command=Some(Action::Release{cell:p,item:self.item.clone()});
                }
                }
                if d.kind.chargeable() {
                    ui.horizontal(|ui|{ui.label("Mana to add");ui.add(egui::DragValue::new(&mut self.amount).clamp_range(1..=balance().def(d.kind).mana_capacity));});
                    if ui.button(format!("Add {} network mana ({} personal mana)",self.amount,registry.mana_charge(self.amount))).clicked(){command=Some(Action::Charge{cell:p,amount:self.amount});}
                }
                if d.kind.sustained() {
                    let seconds=balance().def(d.kind).duration_ticks as f32*balance().tick_ms as f32/1000.0;
                    ui.label(format!("1 mana powers {seconds:.0} seconds. Refill here or connect mana at the base."));
                    ui.small(format!("Stored runtime: {:.1} seconds",(d.mana as f32*seconds)+(d.powered_ticks as f32*balance().tick_ms as f32/1000.0)));
                    if d.kind!=Kind::Lantern {ui.label(if d.kind==Kind::DarkAltar {"Damages all creatures within 6 blocks by 3 health each second."} else {"Heals all creatures within 6 blocks by 3 health each second, up to full health."});}
                }
                ui.separator();
                for (f,n,dir) in d.ports(balance()) {ui.small(format!("{f:?}: {n:?} {dir:?}"));}
            } else {
                ui.label("Build devices with carried resources. F configures; left click packs intact.");
                for def in &balance().devices {
                    let affordable=def.cost.iter().all(|(id,n)|account_count(account,id)>=*n);
                    if ui.add_enabled(affordable,egui::Button::new(&def.name)).clicked(){self.build=Some((def.kind,0,None));close=true;}
                    ui.small(def.cost.iter().map(|(id,n)|format!("{n} {id}")).collect::<Vec<_>>().join(", "));
                }
                ui.separator();ui.label("Packed installations (all cargo and progress preserved):");
                for (i,d) in account.packed_devices.iter().enumerate() {
                    if ui.button(format!("Place {} · {} mana · {} matter",balance().def(d.kind).name,d.mana,d.item_count())).clicked(){self.build=Some((d.kind,d.rotation,Some(i)));close=true;}
                }
                for (item,n) in &account.production_goods {ui.small(format!("Bound creature: {item} ×{n}"));}
            }
        });
        self.open = open && !close;
        command
    }
}
