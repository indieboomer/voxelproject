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
