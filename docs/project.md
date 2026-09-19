# Current project state

## Active development plan

[Spellcasting](../SPELLCASTING_PLAN.md) defines the next work order: Phase 1 targeted
casts, persistent Spellbook and hotbar assignment, followed by Phase 2 persistent
object enchantments, owned effects, revision history and regions.

Existing callback transactions, budgets, interpreted prompting, saved accounts and
creature IDs are foundations to reuse. Hotbar entries support resources, equipment
and stable remembered spell IDs. The
[spellcasting implementation](SPELLCASTING.md) supplies host-targeted creature/block
casts and a persistent host Spellbook with stable IDs, compatibility checks and
management controls. Host and guest hotbar casting are implemented. Enchantments
are now remembered templates cast onto world targets, with independent saved
instances; see [the Enchantment guide](ENCHANTMENTS.md).

The [older World API roadmap](archive/WORLD_API_ROADMAP.md) is historical. Its
bulk-construction jobs, blueprints, explicit Lua state, broader events/timers,
conflict-aware world undo and creation bundles remain proposals to reassess when a
concrete spellcasting stage needs them. Archiving does not mark those items complete.
The [playtesting plan](PLAYTESTING_PLAN.md) separately retains its unfinished gates.

Compatibility sources of truth are `src/net.rs::PROTOCOL_VERSION` (49 after the
targeted Enchantment workflow update) and `world_api/schema.yaml` (1.37.0). Version
numbers in historical implementation results describe those revisions only.

## Playable systems

- First-person movement, collision, jumping, mining and block placement.
- Procedural chunks with mainland rivers/lakes, mountain terrain, vegetation,
  resources, and configurable world-generation presets.
- Resource inventory, crafting, equipment, health and underwater oxygen.
- [Gentle hunger](HUNGER.md): delayed warnings and damage, a 50-health floor,
  relief from every successful meal, and authoritative guest HUD replication.
- Optional campkeeper dialogue, three personal saved contracts, a field journal,
  camp rest and automatic recovery after defeat; see [the adventure guide](ADVENTURE_GUIDE.md).
- Portable left-hand torch light, map waypoints/recovery markers, aimed creature health
  and contextual interaction/damage feedback.
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

`adventure.rs` holds camp transactions, progression checks and safe recovery search;
`adventure_app.rs` connects them to authoritative accounts/networking, and
`adventure_ui.rs` renders the journal/feedback. Keepers reuse animated player assets
and are derived from fires rather than added to the creature ECS. Progress is saved
inside crafting accounts. The shared protocol carries camp requests, recovery and creature
health; World API 1.27 exposes journal state and current held items to reviewed rules.

The [sandbox/RPG review](SANDBOX_RPG_REVIEW.md) records strengths, weaknesses,
the two-hour plan, implementation choices and follow-up priorities.

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
