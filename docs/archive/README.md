# Archived plans and reports

Root Markdown review: 2026-09-15. These files preserve original requirements,
investigations and measurements. They are not current implementation instructions.
Archiving a plan does not mean every proposed feature or acceptance test is complete.

| Archived file | Reason and current reference |
| --- | --- |
| [AGENTS_TESTING_SUBSYSTEM.md](AGENTS_TESTING_SUBSYSTEM.md) | Original proposal; [playtesting status](../PLAYTESTING_PLAN.md) tracks implemented phases and open gates. |
| [AUTOMATION.md](AUTOMATION.md) | Implemented request; [automation guide](../AUTOMATION_GUIDE.md) describes the current system and verification limits. |
| [crafting_instructions.md](crafting_instructions.md) | Implemented request; use [crafting](../CRAFTING.md) and [mana](../mana.md). |
| [HOTBAR.md](HOTBAR.md) | Superseded controls and historical protocol/feature claims; useful details consolidated into [inventory](../inventory.md). |
| [PERFORMANCE.md](PERFORMANCE.md) | Historical benchmark; newer costs and profiling guidance live in [rendering](../rendering.md). |
| [PROMPT_INTENT_ANALYSIS.md](PROMPT_INTENT_ANALYSIS.md) | Investigation preceding the implemented [prompt pipeline](../PROMPT_PIPELINE.md). |
| [WORLD_API_ROADMAP.md](WORLD_API_ROADMAP.md) | Obsolete baseline and work order; [spellcasting](../../SPELLCASTING_PLAN.md) is the active plan. Deferred proposals are summarized in [project state](../project.md). |

## Other root-file decisions

- Kept `AGENTS.md` as development instructions, unchanged.
- Kept `SPELLCASTING_PLAN.md` as the active plan, unchanged.
- Kept `PACKAGING.md` as the Windows distribution entry point.
- Shortened `README.md` to build/play instructions and documentation navigation,
  removing duplicated MVP specification already retained in `AGENTS.md`.
- Moved current references into `docs/`: `CRAFTING.md`, `PLAYER_ANIMATIONS.md`,
  `PROMPT_PIPELINE.md`, `RESOURCES.md`, `STEAM_MULTIPLAYER.md`, `TERRAIN.md`,
  `UI_SETTINGS.md`, and `WORLD_GENERATION.md`. These remain useful feature guides,
  even where they include historical validation results.

Relative Markdown links were updated for the new locations. Links into `target/`
refer to local, regenerable evidence and may be absent in a fresh checkout.
