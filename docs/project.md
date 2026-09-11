# Current project state

## Playable systems

- First-person movement, collision, jumping, mining and block placement.
- Procedural chunks with mainland rivers/lakes, mountain terrain, vegetation,
  resources, and configurable world-generation presets.
- Resource inventory, crafting, equipment, health and underwater oxygen.
- Host-authoritative multiplayer and saved worlds, creatures and rule modules.
- Wildlife and hostile creatures, including walking zombies/skeletons, rare
  separated dragons with ground/air behavior, and water-constrained fish.
- Day/night, weather, surface wetness, spatial creature sounds, currents,
  visual waterfalls, and persistent generated campfires.
- Local AI prompting, schema-described Lua capabilities, validation, bounded
  execution, callback transactions, and runtime rule management.

## Architecture

`app.rs` coordinates input, rendering, local/remote simulation and UI. Terrain and
block edits live in `voxel/`; generation settings live in `worldgen.rs`. Creature
simulation uses hecs and dedicated dragon/fish behavior. The renderer uses wgpu
with shared mesh/material layouts; egui handles UI. Audio uses rodio.

`scripting.rs`, `script_api.rs`, `script_environment.rs`, `script_transaction.rs`,
`script_budget.rs`, and `script_scheduler.rs` implement the authoritative rule
runtime. Lua callbacks stage mutations and commit only after successful execution.
Networking replicates authoritative results; clients derive cosmetic animation
and sound locally. Saves preserve block edits, so procedural generation changes
can affect regenerated unedited terrain while explicit saved edits take priority.

## Safe entry

Fresh-world starter hostiles spawn 48-72 horizontal blocks from the starting
position; passive animals remain within 24 blocks. Dragon discovery already
keeps new dragons at least 80 blocks from every player.

The host gives each entering player 60 seconds without creature targeting or
attacks, including explicit rule-assigned player targets and dragon attacks.
Late guests receive their own interval. Protection expires normally without
restarting each frame; disconnected players receive a new interval upon re-entry.
It is session state rather than saved progress, so opening a saved session also
grants entry grace. Existing saved creatures are not moved or deleted. This does
not prevent environmental damage or direct rule-driven player damage.

## Rule lifecycle

Describe a rule, generate Lua against the current World API, validate it, review
and activate it on the host, observe shared effects, then revise or disable it.
Modules cannot access arbitrary files, processes, networking, or native libraries.
Instruction, memory, time, API, native-work, spawning and editing limits bound work.
Callback failure rollback is distinct from undoing all effects of a previously
successful module; do not assume disabling a rule restores edited terrain.

## Remaining acceptance work

The target remains a packaged one-to-four-player sandbox proving natural-language
world rules. Automated tests and offscreen GPU checks do not replace extended
multiplayer playtesting. Verify joining, combat, save/reload, rule activation and
failure handling with friends, and profile representative worlds on weaker GPUs.
The original end-to-end acceptance cases remain: sheep hunting crystal carriers
at night, rain creating slowing mud near trees, and three deaths producing a
healing red stone. Avoid adding accounts, public matchmaking, large-server
infrastructure, or broad content systems before that loop is dependable.
