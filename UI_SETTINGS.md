# UI styles and settings

Open **Settings** from the main menu or the in-game quit dialog, or press **F10**.
Choose **Generic** or **Fantasy** under Appearance. The change applies immediately;
**Done**, **Esc**, or **F10** returns to the previous screen.

Generic is the default. Both themes use the same UI and actions. Preferences live
in `settings.json` beside the game's working-directory saves, independently of
world data and multiplayer. Each running player controls their own appearance.

## UI audit and changes

Every game screen uses the shared `Ui` egui context; no independent UI framework
or OS dialog was found. The former desktop appearance came from egui's default
font, rounded gray window chrome, soft shadows and small widget spacing.

| Surface | Treatment |
|---|---|
| Main menu, nickname and join screens | Shared theme and larger buttons; Fantasy adds a code-drawn pixel landscape |
| FPS, health and oxygen HUD | Shared typography and frames; increased vertical clearance between panels |
| Rules/spells list | Existing actions retained; bounded scrolling and wrapping for larger text |
| Resources | Existing inventory/select actions retained; bounded scrollable panel instead of a long list covering the screen |
| Rule source viewer | Themed frame; standard monospace code remains readable and case-sensitive |
| Prompt console | Existing input and generation behavior; larger theme typography |
| Chat and notification toasts | Shared typography and frames; semantic status colors retained |
| Quit confirmation | Existing quit/cancel behavior plus Settings entry |
| Elemental crafting | Shared theme; wider Fantasy layout for all five slots, larger viewport and scrolling; per-theme window sizes |
| Settings | Appearance section, live sample, immediate switching, persistence failure feedback and retry |

Generic keeps the original neutral palette, rounded controls and soft shadows.
Its body/button text is now 16 points, small text 13, headings 24, and code 16.
Fantasy uses 20-point pixel body/button text, 16-point small text, 30-point
headings and the same 16-point code font. It adds square bronze borders, ivory
text, stone/wood-toned controls, gold hover states and hard offset shadows.
The pixel font is original and embedded, with egui fallback fonts for characters
outside its glyph set. No external asset downloads are required at runtime.

Settings takes exclusive keyboard/mouse ownership while open. Closing it restores
the cursor mode appropriate to the previous screen. Simulation and networking
continue as they already do while the console is open.

## Extending settings

`src/settings.rs` owns the serialized `Settings` model and settings panel.
Preferences are grouped by section (`appearance` initially), with serde defaults
for new fields. Add a section/field there and its controls to the panel; route
changes into the corresponding existing subsystem. Do not put device preferences
in world saves or replicate them as authoritative gameplay state.

`src/ui_theme.rs` owns the shared visual definitions and font installation.
Theme changes reconstruct the style/font definitions from defaults, so switching
back cannot leave Fantasy styling behind. `src/ui.rs` integrates the same panel
with menu and gameplay rendering. `src/app.rs` and `src/menu.rs` handle access
and input ownership. New egui screens inherit the active theme automatically.

Preferences use a temporary file followed by rename. Missing settings use Generic;
unreadable/malformed settings fall back safely and show an error in Settings.
Failed writes retain the active selection for the current context and report
that it could not be saved. Future sections can be added without breaking older
files.

## Validation

`cargo test --offline` covers preference defaults, serialization, file replacement,
bad-file handling, theme restoration, embedded font layout, and crafting behavior.

An opt-in GPU check renders the real settings and crafting widgets to PNG files
without opening a game window:

```text
cargo test --offline render_ui_previews -- --ignored --nocapture
```

The four resulting `target/ui-{generic,fantasy}-{settings,crafting}.png` files
allow direct visual comparison at 1280 x 720. The test is ignored in the ordinary
suite because machines without a GPU adapter should still run the unit tests.

The Multiplayer section now selects Direct / LAN or Steam friends for the next session. Steam requires a build with `--features steam`; test App ID 480 is the default. See [Steam multiplayer](STEAM_MULTIPLAYER.md).

## Hotbar and targeting

See [HOTBAR.md](HOTBAR.md) for the nine-slot hotbar, E inventory assignment panel, tool requirements and multiplayer persistence. Settings > Gameplay > Show block targeting outlines toggles the local gold/green/red target previews.
