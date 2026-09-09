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
    pub appearance: Appearance,
    pub multiplayer: Multiplayer,
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
}
impl Default for Multiplayer {
    fn default() -> Self {
        Self {
            mode: MultiplayerMode::Direct,
            steam_app_id: 480,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read settings: {e}"))
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
    pub open: bool,
    pub values: Settings,
    message: String,
}

impl SettingsPanel {
    pub fn new(ctx: &egui::Context) -> Self {
        let (values, message) = match Settings::load(Path::new("settings.json")) {
            Ok(s) => (s, String::new()),
            Err(e) => (Settings::default(), e),
        };
        crate::ui_theme::apply(ctx, values.appearance.ui_theme);
        Self {
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
    use super::*;
    #[test]
    fn missing_fields_and_future_sections_are_compatible() {
        let values: Settings = serde_json::from_str(r#"{"audio":{"volume":42}}"#).unwrap();
        assert_eq!(values.appearance.ui_theme, UiTheme::Generic);
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
