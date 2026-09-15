# Describing a new world

Choose **New World**, enter your nickname, and optionally describe the terrain and creature abundance.
For example:

- `A sandy desert with no trees`
- `Small tropical islands in a vast ocean`
- `A flat snow-covered world`
- `Rugged mountains with dense forests`
- `This world is full of sheep but no cows`

Leave the description empty to use the current default generator immediately. A description
uses the configured local model; the full Windows and macOS packages already include
the required runtime and model. A package built without AI needs a running configured
server for this feature. Loading and joining a world require no inference.

The menu stays responsive while the model starts and interprets the text. Cancel
returns to the menu and discards the result; an already-running model request finishes
in the background. A second description waits until that request finishes. Failures
keep your text so you can retry or clear it to create a normal world.

## Implementation plan and supported scope

1. Interpret the description once as schema-constrained JSON, not executable code.
2. Validate shape, surface, relief, tree density, and island spacing against bounded
   settings. Keep the original description alongside these resolved settings.
3. Generate chunks deterministically from the seed and settings, with no inference
   in rendering, chunk generation, or networking. Island worlds have a dry starting
   island around the origin.
4. Save the settings in the existing JSON save wrapper and include them in the
   reliable multiplayer welcome transfer. Old JSON and binary saves default to
   the original terrain. All peers must use the same build.

Shapes are mainland, islands, flat terrain, and mountains. Surfaces are natural,
sand, snow, and stone. Tree density ranges from zero to three times normal; relief
from zero to twice normal; island spacing from 64 to 512 blocks. The model chooses
the closest combination. Existing block resources and creature systems still apply.
Descriptions do not create new assets, buildings, creature species, or gameplay rules;
use the in-game rule console for behavior changes. A desert can have sparse trees,
and islands can have sandy or snowy surfaces.

The terrain settings and original text live in `src/worldgen.rs`; interpretation uses
the same configured llama endpoint as rule generation. No new dependencies, assets,
or platform-specific build steps are required. Rebuild packages with the existing
Windows or separate macOS tools to distribute this version.

## Lake shapes and plant colonies

New worlds use saved `landscape_version: 3`: the original rolling hills and
mountains, with irregular lake basins. The geological faults, rubble, erosion
cuts and undercuts added in versions 1 and 2 are disabled for new worlds.

Lake outlines have seed-selected orientation, elongation, broad bays and
headlands. Each basin keeps a connected interior around its original river
connection. Water levels, winding rivers and raised tributaries are preserved;
terrain outside the old/new lake footprints uses the original height profile.
Visual currents and the water API use the same lake outlines. Flat and island
presets keep their original shape logic; lake changes affect mainland and
mountain presets that use the shared lake generator.
Low cave landings have enough space to pass around staircase treads when a
changed shoreline selects a different entrance height.

Grassy, unshaded, non-wet ground can carry irregular elliptical plant colonies,
roughly 18–54 blocks wide. Some mix lavender, red poppies, clover and flax;
others favor one species in 88% of their selections. Ragged edges and gaps break
up the patches. Existing trees, props and plants take precedence, so actual
density and species proportions vary with habitat and overlapping colonies.
Existing wetland and shade-specific scattering remains available.

Old saves missing the version use `0`, preserving their generated terrain and
vegetation, including newly explored chunks. Create a **new world** to see these
changes. Version-1 and version-2 worlds retain their saved experimental relief.
Both blank and AI-described new worlds select version 3. Clouds are a
separate visual change and appear in existing worlds too. Multiplayer protocol
45 carries the setting; all players need the updated build.

## Verification

Version-3 validation: **589 passed**, 28 opt-in tests ignored in the Steam plus
`dev-playtest` suite; Steam executable rebuilt without warnings.

Lake regressions sample three seeds for irregular outlines, connected wet basin
interiors, still lake centers and unchanged terrain away from lake footprints.
Legacy geology tests remain to protect version-1/2 saves. Generate the inspected
top-down comparison with `cargo test --offline --no-default-features lake_shape_preview -- --ignored --nocapture`.
`target/lake-shapes.png` shows original lakes above and current lakes below for
seeds 7, 42 and 2026. Live multiplayer exploration remains a manual acceptance check.

Automated tests cover blank prompts without an AI server, original height equivalence,
island land/water distribution and dry origins, surface materials and tree removal,
chunk generation order, bounded settings, old-save compatibility, prompted-save
round trips, and terrain metadata in serialized/reordered multiplayer transfers.

Run `cargo test --offline` or `cargo test --offline --features steam`.
For the opt-in real-model test, run
`cargo test --offline live_world_descriptions -- --ignored --nocapture`.
It uses the normal local endpoint, or `WORLDGEN_TEST_URL` when set.

Creature abundance is saved per species: 0 disables natural spawning, 100 is normal, and 1000 is abundant. Unspecified species retain their normal weights. Starting animals and replenished wildlife use the same settings; fish and both dragon colors also respect exclusions. Scripted spawns remain available. Dragons occupy most large territories, selecting high ground with at least 128 blocks between homes and at most eight living dragons.
