# Voxel Project

A Rust multiplayer voxel sandbox where players describe world rules in natural
language, review generated Lua, and activate it in a shared authoritative world.

## Current work

[SPELLCASTING_PLAN.md](SPELLCASTING_PLAN.md) is the active development plan:
reusable targeted spells, a saved Spellbook, and hotbar casting first; persistent
object enchantments, owned effects, and revision history second. Targeted host casts
and the saved Spellbook are implemented, including an inventory Spells column,
host/guest hotbar casting, a casting HUD and replicated particles. Temporary
slow/stun effects and collision-checked creature pushes extend the spell API. See
[implementation status](docs/SPELLCASTING.md) for the available flow and limits.

See [project state](docs/project.md) for implemented systems and remaining
acceptance work, and [AGENTS.md](AGENTS.md) for development instructions.

## Build and play

```powershell
cargo build --no-default-features
cargo test --no-default-features --quiet -- --test-threads=1
```

On Windows, run `target/debug/voxelproject.exe` from the repository root.
Choose New World to start, Load World to resume, or join a host. A blank world
description starts without AI; optional descriptions use the configured local model.

- [Development and testing](docs/development.md)
- [Windows packaging](PACKAGING.md): `make_package.bat` builds a shareable bundle.
- [macOS setup and packaging](macos/README.md)
- [Steam and Direct/LAN multiplayer](docs/STEAM_MULTIPLAYER.md): up to four players;
  all peers need the same build. Separate-account acceptance testing remains open.

## Main controls and guides

| Control | Guide |
| --- | --- |
| I: inventory; 1–9 / mouse wheel: hotbar | [Inventory and hotbar](docs/inventory.md) |
| K: Spellbook; Remember in Rules | [Remembered spells](docs/SPELLCASTING.md#stage-1b-persistent-spellbook) |
| C: crafting | [Crafting](docs/CRAFTING.md), [resources](docs/RESOURCES.md), [mana](docs/mana.md) |
| B: build; R: rotate; F: interact/configure | [Automation](docs/AUTOMATION_GUIDE.md) |
| J: journal; M: waypoints | [Adventure](docs/ADVENTURE_GUIDE.md) |
| F5: save | [Storage and underground exploration](docs/STORAGE_AND_UNDERGROUND.md) |
| F10: settings | [UI and settings](docs/UI_SETTINGS.md) |
| T: chat; comma / period / slash: gestures | [Player animation and chat](docs/PLAYER_ANIMATIONS.md) |

## World rules

The [World API user guide](world_api/USER_GUIDE.md) combines prompting instructions,
Lua examples, API methods, multiplayer/save behavior, and limits.
[Prompt pipeline](docs/PROMPT_PIPELINE.md) describes interpretation, validation,
host review, and optional guest proposals. Lua runs on the host with resource
budgets and callback transactions. Disabling a rule does not undo all of its
previously committed effects.

The runtime uses wgpu, winit, hecs, egui, serde, sandboxed Lua via mlua, and local
llama.cpp inference. Direct UDP and optional Steam share gameplay authority.
The API contract lives in [world_api/schema.yaml](world_api/schema.yaml);
generated references must be rebuilt with `tools/gen_world_api.py`.

Browse the [documentation index](docs/README.md) for feature guides.
[Archived plans and reports](docs/archive/README.md) retain historical requirements
and measurements without defining the current work order.
