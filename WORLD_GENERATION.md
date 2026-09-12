# Describing a new world

Choose **New World**, enter your nickname, and optionally describe the terrain and creature abundance.
For example:

- `A sandy desert with no trees`
- `Small tropical islands in a vast ocean`
- `A flat snow-covered world`
- `Rugged mountains with dense forests`
- `This world is full of sheep but no cows`

Leave the description empty to use the original generator immediately. A description
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
   the original terrain. Protocol 21 requires peers to use the updated build.

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

## Verification

Automated tests cover blank prompts without an AI server, original height equivalence,
island land/water distribution and dry origins, surface materials and tree removal,
chunk generation order, bounded settings, old-save compatibility, prompted-save
round trips, and terrain metadata in serialized/reordered multiplayer transfers.

Run `cargo test --offline` or `cargo test --offline --features steam`.
For the opt-in real-model test, run
`cargo test --offline live_world_descriptions -- --ignored --nocapture`.
It uses the normal local endpoint, or `WORLDGEN_TEST_URL` when set.

Creature abundance is saved per species: 0 disables natural spawning, 100 is normal, and 1000 is abundant. Unspecified species retain their normal weights. Starting animals and replenished wildlife use the same settings; fish and both dragon colors also respect exclusions. Scripted spawns remain available. Dragons occupy most large territories, selecting high ground with at least 128 blocks between homes and at most eight living dragons.
