//! Local preferences, separate from world saves and multiplayer state.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiTheme {
    #[default]
    Generic,
    Fantasy,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub player_name: String,
    /// Stable direct-session account key; display-name edits do not move inventory.
    pub connection_name: String,
    pub gameplay: Gameplay,
    pub appearance: Appearance,
    pub multiplayer: Multiplayer,
    pub aiapi: AiApi,
}

/// Local-only credentials. Retained in normal builds so saving preferences does
/// not erase a development configuration. Never copy into world/session artifacts.
#[derive(Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiApi {
    #[serde(rename = "OPENAI_API_KEY")]
    pub api_key: String,
    #[serde(rename = "OPENAI_PLAYTEST_MODEL")]
    pub model: String,
}
impl std::fmt::Debug for AiApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiApi").field("api_key", &"[redacted]")
            .field("model", &self.model).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gameplay {
    pub show_block_target: bool,
    pub mana_free: bool,
}
impl Default for Gameplay {
    fn default() -> Self {
        Self {
            show_block_target: true,
            mana_free: false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub ui_theme: UiTheme,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiplayerMode {
    #[default]
    Direct,
    Steam,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Multiplayer {
    pub mode: MultiplayerMode,
    pub steam_app_id: u32,
    pub allow_guest_prompting: bool,
}
impl Default for Multiplayer {
    fn default() -> Self {
        Self {
            mode: MultiplayerMode::Direct,
            steam_app_id: 480,
            allow_guest_prompting: false,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => {
                // Serde type errors can contain the offending value, including a secret.
                serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read settings: invalid JSON or field type at line {}, column {}", e.line(), e.column()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read settings: {e}")),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .and_then(|()| std::fs::rename(&temporary, path))
            .map_err(|e| format!("Settings apply now, but could not be saved: {e}"))
    }
}

pub struct SettingsPanel {
    #[cfg(feature = "dev-playtest")]
    pub playtest_request: bool,
    #[cfg(feature = "dev-playtest")]
    pub playtest_status: String,
    #[cfg(feature = "dev-playtest")]
    pub playtest_scenario: crate::playtest::Scenario,
    #[cfg(feature = "dev-playtest")]
    pub playtest_in_game: bool,
    name_job: Option<std::sync::mpsc::Receiver<Result<String,String>>>,
    pub name_status: String,
    pub open: bool,
    pub values: Settings,
    message: String,
}

impl SettingsPanel {
    pub fn poll_name(&mut self,url:&str) {
        if let Some(job)=&self.name_job {
            if let Ok(result)=job.try_recv() {
                self.name_job=None;
                if self.values.player_name.trim().is_empty() {
                    self.values.player_name=match result {Ok(name)=>{self.name_status="AI name suggestion ready".into();name},Err(_)=>{self.name_status="Local AI unavailable; using a fantasy fallback".into();crate::fantasy_name::fallback()}};
                    let _=self.values.save(Path::new("settings.json"));
                }
            }
        }
        if self.values.player_name.trim().is_empty() && self.name_job.is_none() {
            let (tx,rx)=std::sync::mpsc::channel();let url=url.to_string();
            std::thread::spawn(move || {let _=tx.send(crate::fantasy_name::request(&url));});
            self.name_job=Some(rx);self.name_status="Asking local AI for a fantasy name… You can also type your own.".into();
        }
    }
    pub fn new(ctx: &egui::Context) -> Self {
        let (values, message) = match Settings::load(Path::new("settings.json")) {
            Ok(s) => (s, String::new()),
            Err(e) => (Settings::default(), e),
        };
        crate::ui_theme::apply(ctx, values.appearance.ui_theme);
        Self {
            name_job:None,
            #[cfg(feature = "dev-playtest")]
            playtest_request: false,
            #[cfg(feature = "dev-playtest")]
            playtest_status: String::new(),
            #[cfg(feature = "dev-playtest")]
            playtest_scenario: Default::default(),
            #[cfg(feature = "dev-playtest")]
            playtest_in_game: false,
            name_status:String::new(),
            open: false,
            values,
            message,
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context) {
        let previous = self.values.clone();
        egui::Window::new("Settings")
            .id(egui::Id::new("settings_panel"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .default_height(560.0)
            .max_height((ctx.screen_rect().height() - 64.0).max(180.0))
            .vscroll(true)
            .show(ctx, |ui| {
                #[cfg(feature = "dev-playtest")]
                {
                    ui.group(|ui| {
                        ui.label("Development playtesting");
                        egui::ComboBox::from_id_source("playtest_scenario").selected_text(self.playtest_scenario.name()).show_ui(ui,|ui| {
                            for scenario in [crate::playtest::Scenario::GatherCraft,crate::playtest::Scenario::WalkReturn,crate::playtest::Scenario::MineBlock,crate::playtest::Scenario::AiGatherTool,crate::playtest::Scenario::AiShelter,crate::playtest::Scenario::AiEncounterReturn] {
                                ui.selectable_value(&mut self.playtest_scenario,scenario,scenario.name());
                            }
                        });
                        ui.small("Actions change this world. Session logs include a starting snapshot.");
                        if self.playtest_scenario.is_ai() {
                            ui.small("Sends limited gameplay observations to OpenAI. Reads OPENAI_API_KEY and OPENAI_PLAYTEST_MODEL from settings.json > aiapi, with environment fallback. Limits: 10 minutes, 96 decisions, 160,000 tokens.");
                        }
                        if ui.add_enabled(self.playtest_in_game,egui::Button::new("Start / stop Agent1")).clicked() { self.playtest_request = true; }
                        if !self.playtest_in_game {ui.small("Enter a world to start a session.");}
                        ui.label(&self.playtest_status);
                    });
                }
                ui.heading("Player name");
                ui.add(egui::TextEdit::singleline(&mut self.values.player_name).char_limit(crate::net::MAX_NICKNAME_LEN).hint_text("Leave empty for an AI fantasy name"));
                if ui.button("Suggest funny fantasy name (AI)").clicked() {self.values.player_name.clear();}
                if !self.name_status.is_empty() {ui.small(&self.name_status);}
                ui.heading("Multiplayer");
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.values.multiplayer.mode,
                        MultiplayerMode::Direct,
                        "Direct / LAN",
                    );
                    ui.add_enabled_ui(cfg!(feature = "steam"), |ui| {
                        ui.selectable_value(
                            &mut self.values.multiplayer.mode,
                            MultiplayerMode::Steam,
                            "Steam friends",
                        );
                    });
                });
                if !cfg!(feature = "steam") {
                    ui.small("Steam support is not included in this build. Build with build.bat steam, then restart the game.");
                }
                ui.label("Applies to your next session. One host and up to three guests.");
                ui.checkbox(&mut self.values.multiplayer.allow_guest_prompting, "When hosting: allow guests to prompt");
                ui.small("Applies immediately while hosting. Guests use their local AI; you review and activate their proposals.");
                ui.separator();
                ui.heading("Gameplay");
                ui.checkbox(&mut self.values.gameplay.mana_free, "Mana-free actions (testing)");
                ui.small("Applies to everyone when you host. Joined games use the host's setting. Materials are still required.");
                ui.checkbox(&mut self.values.gameplay.show_block_target, "Show block targeting outlines");
                ui.small("Gold: compatible tool. Green: build. Red: cannot harvest with the active item.");
                ui.small("Build preview appears for an active resource when placement is valid.");
                ui.separator();
                ui.heading("Appearance");
                ui.label("UI style");
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.values.appearance.ui_theme,
                        UiTheme::Generic,
                        "Generic",
                    );
                    ui.selectable_value(
                        &mut self.values.appearance.ui_theme,
                        UiTheme::Fantasy,
                        "Fantasy",
                    );
                });
                ui.add_space(8.0);
                ui.label(match self.values.appearance.ui_theme {
                    UiTheme::Generic => "Classic neutral panels with larger, easy-to-read text.",
                    UiTheme::Fantasy => "Pixel lettering, bronze borders and dark stone panels.",
                });
                ui.separator();
                ui.label("Preview");
                ui.group(|ui| {
                    ui.heading("Traveler's inventory");
                    ui.label("Earth 12   Life 8   Mana 24");
                    ui.horizontal_wrapped(|ui| {
                        let _ = ui.button("Craft");
                        ui.add_enabled(false, egui::Button::new("Unavailable"));
                    });
                });
                ui.add_space(12.0);
                ui.label("Appearance applies immediately. Preferences are saved on this device.");
                if !self.message.is_empty() {
                    ui.colored_label(ui.visuals().warn_fg_color, &self.message);
                    if ui.button("Retry saving").clicked() {
                        self.persist();
                    }
                }
                ui.separator();
                if ui.button("Done").clicked() {
                    self.open = false;
                }
                ui.small("Esc / F10 to close");
            });
        if previous != self.values {
            if previous.appearance.ui_theme != self.values.appearance.ui_theme {
                crate::ui_theme::apply(ctx, self.values.appearance.ui_theme);
            }
            self.persist();
            ctx.request_repaint();
        }
    }

    fn persist(&mut self) {
        self.message = self
            .values
            .save(Path::new("settings.json"))
            .err()
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ai_credentials_survive_preferences_save_and_are_redacted_from_debug_and_errors() {
        let mut settings: Settings = serde_json::from_str(r#"{"aiapi":{"OPENAI_API_KEY":"local-test-secret","OPENAI_PLAYTEST_MODEL":"test-model"}}"#).unwrap();
        settings.player_name = "Changed preference".into();
        let path = std::env::temp_dir().join(format!("voxel-ai-settings-test-{}.json", std::process::id()));
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.aiapi, settings.aiapi);
        assert!(!format!("{loaded:?}").contains("local-test-secret"));
        std::fs::write(&path, br#"{"aiapi":{"OPENAI_API_KEY":{"secret":"local-test-secret"}}}"#).unwrap();
        assert!(!Settings::load(&path).unwrap_err().contains("local-test-secret"));
        std::fs::remove_file(path).unwrap();
    }
    use super::*;
    #[test]
    fn missing_fields_and_future_sections_are_compatible() {
        let values: Settings = serde_json::from_str(r#"{"audio":{"volume":42}}"#).unwrap();
        assert_eq!(values.appearance.ui_theme, UiTheme::Generic);
        assert!(values.gameplay.show_block_target);
        assert!(!values.gameplay.mana_free);
        let testing:Settings=serde_json::from_str(r#"{"gameplay":{"mana_free":true}}"#).unwrap();
        assert!(testing.gameplay.mana_free);
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&testing).unwrap()).unwrap(),testing);
        assert!(!values.multiplayer.allow_guest_prompting);
        let shared: Settings = serde_json::from_str(r#"{"multiplayer":{"allow_guest_prompting":true}}"#).unwrap();
        assert!(shared.multiplayer.allow_guest_prompting);
        assert_eq!(serde_json::from_str::<Settings>(&serde_json::to_string(&shared).unwrap()).unwrap(), shared);
        let disabled: Settings =
            serde_json::from_str(r#"{"gameplay":{"show_block_target":false}}"#).unwrap();
        assert!(!disabled.gameplay.show_block_target);
        assert_eq!(
            serde_json::from_str::<Settings>(&serde_json::to_string(&disabled).unwrap()).unwrap(),
            disabled
        );
        assert!(
            serde_json::from_str::<Settings>(r#"{"appearance":{"ui_theme":"invalid"}}"#).is_err()
        );
        let fantasy = Settings {
            appearance: Appearance {
                ui_theme: UiTheme::Fantasy,
            },
            ..Settings::default()
        };
        assert_eq!(
            serde_json::from_str::<Settings>(&serde_json::to_string(&fantasy).unwrap()).unwrap(),
            fantasy
        );
    }
    #[test]
    fn preferences_survive_replacing_an_existing_file() {
        let path =
            std::env::temp_dir().join(format!("voxel-settings-test-{}.json", std::process::id()));
        let generic = Settings::default();
        generic.save(&path).unwrap();
        let fantasy = Settings {
            appearance: Appearance {
                ui_theme: UiTheme::Fantasy,
            },
            ..Settings::default()
        };
        fantasy.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), fantasy);
        std::fs::write(&path, b"broken").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), generic);
    }
}
