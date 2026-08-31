# World API changelog

Every entry corresponds to a `version` bump in `world_api/schema.yaml`. After
changing the schema, regenerate everything with:

```
python tools/gen_world_api.py
cargo build && cargo test
```

Then update this file by hand with what actually changed and why -- the docs
regenerate automatically, but "what changed and why" is not mechanically
derivable from a diff of the schema alone.

## 1.2.0 -- 2026-08-31 (player health, poison, attributes, inventory)

Adds a health attribute (0-100, starting at 100), a poison status effect,
adjustable movement attributes, and rounds out inventory manipulation in the
World API. Deliberately does NOT add anything that inflicts poison or heals
it in the world yet (no poison blocks/creatures, no medication item) -- only
the underlying mechanic and the World API hooks a future one would use.

- **Added `health`/`poisoned`/`speed_multiplier`/`jump_multiplier`** to
  `Player` (`src/player.rs`) and to `PlayerSnapshot`, so every rule/spell
  can read them via `api.players()`/`api.nearest_player()`. Every player
  starts every session at 100 health, unpoisoned, 1.0 multipliers -- see
  `persistence.player_state_not_saved` below.
- **Added `api.damage_player`/`api.heal_player`** (clamped into [0, 100]),
  **`api.set_poisoned`** (starts/stops a 1-health-per-`poison_tick_secs`
  drain, engine-driven, no Lua callback fires for the tick itself), and
  **`api.set_player_speed`/`api.set_player_jump`** (multipliers on base
  walk/sprint/jump speed, clamped into
  `[attribute_multiplier_min, attribute_multiplier_max]` = [0.1, 5.0]).
- **Added `api.take_item`/`api.get_resource_count`**, rounding out
  `api.give_item` (1.1.0) for inventory manipulation. Both are honestly
  scoped to the host's own player only -- a remote player's Resources
  counts are never reported up to the host, so there's nothing accurate to
  answer with for anyone else; both return `nil`/`false` for a non-host
  `player_id` rather than guess. `give_item` doesn't have this limitation
  (granting doesn't need to know what's already held).
- **Player attribute replication**: health/poisoned/speed_multiplier/
  jump_multiplier ride the existing best-effort `Snapshot` broadcast
  (~20Hz, self-healing) the same way `carrying_crystal` already does --
  broadcast to *every* connected player, not just the one they belong to.
  Introduced `net::SnapshotPlayer` (a named struct) in place of the
  previous `(PlayerId, [f32;3], f32, bool)` tuple, since an 8-tuple was no
  longer readable. Each client applies its own entry's attributes to its
  local `Player` (the one that actually simulates its own movement
  physics); other players' entries update their `RemotePlayer` record
  purely for future use (nothing reads another player's copy yet).
- **Poison tick**: a new host-only 10-second timer (`App::apply_poison_ticks`)
  damages every currently-poisoned player by 1 health, independent of the
  Lua tick loop -- pure engine state, not a World API action.
- **Rules panel HUD**: a small always-visible Health readout (color-coded,
  plus a POISONED tag) below the FPS counter.
- **`PlayerEffect`**: introduced as the single deferred-effect channel for
  every player-targeted action (give/take item, health, poison, both
  multipliers), replacing 1.1.0's narrower `item_grants: Vec<(PlayerId,
  BlockType, u32)>` field on `TickInput`/`TickOutcome`. One enum instead of
  a growing set of parallel `Vec` fields -- see `scripting.rs`'s doc
  comment on `PlayerEffect` for why.
- Added automated test coverage: `Player`'s health/poison/multiplier
  defaults and clamping; `take_resources`' atomic all-or-nothing behavior;
  every new `api.*` action's Lua binding (connection checks, clamping,
  host-only enforcement for take_item/get_resource_count) via
  `ScriptHost::run_cast` outcomes; `PlayerSnapshot` round-tripping the four
  new fields through `api.players()`/`api.nearest_player()`;
  `SnapshotPlayer` encode/decode.

### Compatibility

No breaking changes to the save format. `ModuleSaveEntry` is untouched (as
always); the new `Player`/`RemotePlayer` fields are runtime-only, following
the `player_state_not_saved` precedent below. `UnreliableMsg::Snapshot`'s
wire *shape* did change (tuple -> `SnapshotPlayer` struct, with new fields)
-- this breaks compatibility between differently-versioned host/client
binaries (an old client can't decode a new host's Snapshot, or vice versa),
but that was already true of any `Packet` shape change in this project;
there's no version negotiation at the protocol level yet, host and client
are always expected to be the same build.

## 1.1.0 -- 2026-08-31 (instant spells)

Adds a second module kind alongside the existing continuous RULE: an
INSTANT SPELL that runs its whole effect exactly once, immediately, instead
of ~10/sec for as long as it's enabled. Motivating prompts: "add 100
stone", "spawn a dozen chickens around me" -- one-time actions that don't
belong on a tick.

- **Added the `on_cast(api, event)` event**, mutually exclusive with
  `on_tick` -- a module now defines EXACTLY ONE of the two (`Module::load`
  rejects both-defined or neither-defined). Which one a module defines is
  entirely how the engine tells a RULE from an INSTANT SPELL
  (`Module::is_instant`); see "Compatibility" below for why this isn't a
  saved field. `event` is a new `CastEvent` type carrying `player_id` (the
  player who clicked Run -- currently always the host, since running a
  spell is host-only like generating/activating a rule).
- **Added `api.give_item(player_id, kind, amount)`**, for the one case
  `replace_block` can't cover: a prompt that explicitly asks to add to a
  player's *inventory* rather than the world (e.g. "add 100 stone to my
  inventory" vs. plain "add 100 stone", which places blocks). Amount is
  clamped to the new `item_grant_max` budget (500). Reaches a remote
  player's inventory via a new targeted `ReliableMsg::GrantItem` (see
  `replication.item_grants`); the host's own grant applies directly, no
  network round-trip.
- **Added higher one-shot action budgets for `on_cast`**:
  `block_edits_per_cast_call` (300) and `creature_spawns_per_cast_call`
  (30), vs. the regular per-tick 32/4 -- an instant spell runs once, not
  ~10/sec, so it can afford to do more work in that one call.
- **Rules panel UI**: an instant spell shows a `[SPELL]` status instead of
  ON/OFF/ERR, and a **Run** button (host-only, re-runnable any number of
  times) instead of Enable/Disable. View Code and Delete work identically
  to a RULE.
- **Prompt classification**: `App::start_generation` runs a cheap
  deterministic heuristic (`llm::classify_prompt`, the same kind of
  keyword-based approach `derive_rule_name` already uses for naming) to
  guess RULE vs. INSTANT SPELL from the prompt's own phrasing (conditional/
  temporal language like "when"/"if"/"at night" vs. an imperative one-shot
  verb like "add"/"spawn"/"give me"), and tells the model which one to
  write via a directive prepended to the user turn. If the model's output
  doesn't match the guess, one corrective retry is attempted (reusing the
  existing validation-retry pipeline); if it *still* doesn't match, the
  module is accepted as whatever kind it actually turned out to be rather
  than failing generation outright -- the heuristic is a hint to the model,
  not a hard requirement enforced against it.
- Added a new starter/example module, `modules/add_stone.lua` -- an instant
  spell placing 100 stone blocks around the caster in an expanding ring,
  snapped to terrain height. Doubles as the system prompt's on_cast
  few-shot example, the same way the four existing starter rules already
  double as on_tick/on_death/on_block_break examples.
- Added automated test coverage: `Module::load` rejecting on_tick+on_cast
  and neither; a cast's block edits/item grants/spawns applying exactly
  once per Run click, not repeating; a cast's `api.destroy` chaining into
  another module's `on_death` (same chaining `run_tick` already does for
  ticks and block breaks); `classify_prompt`'s heuristic across both
  RULE- and INSTANT-leaning example prompts; `ReliableMsg::GrantItem`'s
  encode/decode round-trip.

### Compatibility

No breaking changes. `ModuleSaveEntry`'s serialized shape (`name`,
`prompt`, `source`, `enabled`) is unchanged, following the precedent
`api_version_tagging` set in 1.0.0: whether a module is a RULE or an
INSTANT SPELL is derived every load from `source` (which of on_tick/
on_cast it defines), never stored as its own field, so this needed zero
changes to `WorldSave`'s bincode layout and cannot break old saves. Every
pre-1.1.0 rule defines `on_tick`, so every one of them keeps loading and
running as a RULE exactly as before.

## 1.0.0 -- 2026-08-28 (baseline)

First versioned snapshot of the World API. This release did not add any new
`api.*` capability; it establishes the schema-driven documentation/tooling
pipeline around the API that already existed, and fixes two bugs found while
auditing it for that pipeline to describe correctly:

- **Established `world_api/schema.yaml`** as the single authoritative,
  machine-readable description of every property, method, event, type,
  budget, persistence rule, and multiplayer-authority note in the World API.
  `tools/gen_world_api.py` generates from it:
  - `world_api/world_api_readme.md` -- full human-readable docs.
  - `world_api/world_api_compact.md` -- concise docs, spliced into
    `prompts/system_prompt.txt` at `{WORLD_API_COMPACT}` and so directly
    fed to the local model doing rule generation (previously this section
    was a hand-maintained, hand-copied block of the system prompt that
    could silently drift from the real Rust implementation).
  - `world_api/world_api_stubs.lua` -- EmmyLua-style type stubs for editor
    autocomplete against `api.*`.
  - `src/world_api_gen.rs` -- a Rust registry (method/property names, block
    kinds, creature kinds, the version string) consumed by the new
    pre-flight validator.
- **Added `src/world_api_validate.rs`**, a pre-flight lint run on
  LLM-generated Lua *before* it's ever loaded into a real Lua VM
  (`App::poll_generation`). Flags calls to unknown `api.*` members (with a
  "did you mean" suggestion) and literal block-kind strings passed to
  `api.replace_block` that aren't real block ids -- both feed a specific,
  actionable message into the existing one-retry generation loop instead of
  only surfacing as an opaque runtime error the first time a bad rule ticks.
- **Added API version tagging**: every rule's Lua source may begin with a
  `-- api_version: X.Y.Z` comment recording which World API version it was
  written against (`world_api_validate::tag_with_api_version`). Applied
  automatically to every LLM-generated rule going forward, and retroactively
  to the five hand-written starter rules under `modules/`. Purely
  informational (never validated against the running engine), so it's
  fully backward compatible -- an untagged rule (anything generated or
  hand-written before this release) keeps loading and running exactly as
  before; `Module::api_version()` just returns `None` for it, shown in the
  Rules panel as "api version unknown" instead of a version number.
- **Fixed `api.get_block`** (and the `kind` field of the `on_block_break`
  event) returning a block's *display name*, lowercased -- e.g. `"oak
  wood"` with a space -- instead of the snake_case id `api.replace_block`/
  `api.find_blocks` actually accept (`"oak_wood"`). Any multi-word block
  never round-tripped correctly; single-word blocks (grass, soil, stone,
  ...) happened to work by coincidence, which is why no existing rule
  hit this. Fixed by adding a canonical `BlockType::id()` (generated from
  `textures/blocks.csv`, the same source of truth `tools/build_atlas.py`
  already uses) and switching both call sites to it.
- **Fixed `api.broadcast`** only logging server-side despite being
  documented (and named) as a player-facing notification -- it never
  actually reached any client. `ScriptHost::run_tick` now returns the
  broadcast messages a tick produced, and `App` relays each through the
  same host-to-all `Notify` path every other rule-triggered message
  (rule enabled/disabled, player joined, ...) already uses.
- **No respawn system exists yet** anywhere in the codebase (confirmed by
  audit, not just absence from this API) -- there is no World API surface
  tied to it, so no "respawn behavior" tests were added this release. Noted
  here rather than silently skipped.
- Added automated test coverage across: pre-flight validation
  (`world_api_validate`'s own test module), sandbox restrictions (already
  covered, unchanged), persistence (`ModuleSaveEntry` bincode round-trip,
  and that a module's runtime Lua state does *not* survive a save/reload
  while its enabled flag and source do), multiplayer replication
  (`net.rs`'s `BlockEdit`/`Snapshot`/`Notify` encode/decode round-trips),
  and "existing rules keep working" (every file under `modules/` still
  passes `Module::load` and carries a version tag).

### Compatibility

No breaking changes. `ModuleSaveEntry`'s serialized shape (`name`, `prompt`,
`source`, `enabled`) is unchanged -- API versioning deliberately lives inside
`source` as a comment instead of a new struct field, specifically so it
doesn't touch `WorldSave`'s bincode layout and cannot break old saves. Every
existing hand-written or previously-generated rule keeps loading and running
identically; only `get_block`'s return value and `on_block_break`'s
`event.kind` changed *shape* for multi-word block names specifically (see
above) -- nothing in `modules/*.lua` depended on the old, buggy space-
separated form (verified during the audit), so this is not expected to
affect any real rule in practice, but a rule saved elsewhere that happened to
compare `event.kind`/`get_block(...)` against a multi-word display name
string (e.g. `"oak wood"`) would need updating to the id form (`"oak_wood"`).

### How to evolve `ModuleSaveEntry` safely in the future

Bincode has no field-presence markers -- it reads a struct as a fixed
sequence of fields in declaration order, so adding, removing, or reordering
a field breaks every previously-saved world immediately, `#[serde(default)]`
notwithstanding (that attribute only helps formats that *can* signal "this
field is absent", which bincode cannot). If a future API version genuinely
needs new per-module save state:

- Prefer encoding it inside `source` (as this release did for the version
  tag) if it's naturally a property of the rule's code.
- Otherwise, introduce a new top-level `Vec` in `WorldSave` (alongside
  `modules`) rather than a new field on `ModuleSaveEntry` itself, and join
  it back up by module name/index on load -- `WorldSave` gaining a field is
  itself still a breaking bincode change, so pair it with a version byte at
  the very start of the save file and a loader that branches on it, the way
  a real migration would.
