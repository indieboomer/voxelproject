//! Explicit development-only paid check in the real winit/App update loop.
use super::*;
use serde_json::json;

impl App {
    pub(crate) fn start_coplay_live_check(&mut self) {
        self.weather.set(Weather::Sunny);
        self.ui.coplay.enabled = true;
        self.ui.coplay.open = true;
        self.ui.coplay.input="Create a persistent world rule that keeps the weather rainy. Discover the relevant API, inspect, register the rule, then inspect and verify actual rain before finishing. Do not change anything else.".into();
        self.ui.coplay.submit = true;
        self.sync_settings_input();
    }
    pub(crate) fn finish_coplay_live_check(&mut self) -> bool {
        if self.ui.coplay.live_phase == u8::MAX {
            return true;
        }
        if self.ui.coplay.run.is_some() || self.ui.coplay.submit {
            return false;
        }
        let phase = self.ui.coplay.live_phase;
        let count = self
            .scripting
            .modules
            .iter()
            .filter(|m| m.name.starts_with("coplay-"))
            .count();
        let expected = if phase == 1 || phase == 2 {
            Weather::Mist
        } else {
            Weather::Rain
        };
        if phase >= 2
            && self.weather.current != expected
            && self
                .ui
                .coplay
                .live_wait
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return false;
        }
        let mut pass = self.weather.current == expected
            && count == 1
            && (phase >= 2 || self.ui.coplay.status.starts_with("complete:"));
        if phase >= 4 {
            pass &= self.ui.coplay.status.starts_with("blocked:")
                && self
                    .ui
                    .coplay
                    .session
                    .requests
                    .last()
                    .is_some_and(|r| r["gap_ids"].as_array().is_some_and(|a| !a.is_empty()));
        }
        if phase == 5 {
            let requests = &self.ui.coplay.session.requests;
            pass &= requests.len() >= 2
                && requests[requests.len() - 1]["gap_ids"]
                    == requests[requests.len() - 2]["gap_ids"];
        }
        self.ui.coplay.live_evidence.push(json!({"phase":phase,"passed":pass,"weather":self.weather.current.name(),"coplay_rules":count,"status":self.ui.coplay.status}));
        if pass && phase == 0 {
            self.ui.coplay.live_phase = 1;
            self.ui.coplay.input="Modify that same weather rule to keep mist instead of rain. Reuse its stable rule ID, read its current definition, then inspect and verify the changed weather. Do not create another rule.".into();
            self.ui.coplay.submit = true;
            return false;
        }
        if pass && phase == 1 {
            let saved = save_world(
                &self.world,
                &self.player,
                &self.camera,
                self.time_of_day,
                self.scripting.save_entries(),
                &self.crafting_save(),
            );
            if saved.is_ok() {
                if let Some(loaded) = load_world(&self.world.name) {
                    self.scripting = ScriptHost::load_from_save(&loaded.modules);
                    self.ui.coplay.live_phase = 2;
                    self.ui.coplay.live_wait = Some(Instant::now());
                    self.weather.set(Weather::Sunny);
                    return false;
                }
            }
            self.ui
                .coplay
                .live_evidence
                .push(json!({"save_reload_failed":true}));
        }
        if pass && phase == 2 {
            let mut panel = std::mem::take(&mut self.ui.coplay);
            let result = crate::coplay::undo_rule(&mut panel.session, &mut self.scripting);
            panel.live_evidence.push(json!({"undo":result}));
            panel.live_phase = 3;
            panel.live_wait = Some(Instant::now());
            self.ui.coplay = panel;
            self.weather.set(Weather::Sunny);
            return false;
        }
        if pass && (phase == 3 || phase == 4) {
            self.ui.coplay.live_phase = phase + 1;
            self.ui.coplay.input="Add a persistent custom on-screen button labelled Weather that invokes my weather rule when clicked. Inspect the actual World API for UI/button support and possible compositions. If unavailable, persist the precise missing capability and finish blocked. Do not substitute chat commands or another behavior. Reuse any existing backlog entry for this underlying capability.".into();
            self.ui.coplay.submit = true;
            return false;
        }
        let evidence = json!({"success":pass && phase==5,"world":self.world.name,"api":crate::world_api_gen::VERSION,"phases":self.ui.coplay.live_evidence,"requests":self.ui.coplay.session.requests,"conversation":self.ui.coplay.session.conversation});
        let _ =
            crate::coplay::write_json(std::path::Path::new("target/coplay-live.json"), &evidence);
        let _ = self
            .ui
            .coplay
            .backlog
            .export(std::path::Path::new(crate::coplay::EXPORT_PATH));
        self.ui.coplay.live_phase = u8::MAX;
        true
    }
}
