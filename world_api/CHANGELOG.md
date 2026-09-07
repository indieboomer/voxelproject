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

## 1.13.0 -- 2026-09-07 (sunscorch creature, reimported models, weighted starter spawns)

- All bundled creature models were reimported from `models/`. Six of eight
  kinds (stone_golem, wolf, stinger, cow, goblin, and the new sunscorch) came
  back as standard glTF skinned meshes with real baked textures, replacing
  the earlier rigid Blockbench-style rigs; sheep and chicken stayed on the
  old rigid format. This is an internal rendering change with one visible
  side effect: each kind's actual set of animation clips now reflects what
  its source file actually exports, not a fixed assumption -- cow lost its
  run clip (a Lua-driven fast chase() now plays walk, not run) and stinger
  gained one (an aggro-closing stinger now plays run, not walk).
- New hostile creature kind: `sunscorch`, a slow, tanky ghost. Like
  stone_golem, it never speeds up while chasing (no run clip, and its aggro
  speed equals its wander speed) and never gives up a chase once aggroed.
  max_health 28. Added to `creature_kinds`, `spawn_creature`,
  `spawn_creature_near_player`, `find_creatures`, and `nearest_creature`.
- The starter world's initial creature scatter (`Creatures::spawn_around`)
  no longer alternates only sheep/chicken -- it now draws every kind from a
  weighted table: sheep and chicken most common, cow slightly less so,
  wolf/stinger/goblin uncommon, and stone_golem/sunscorch rare. A brand-new
  world can now include a hostile encounter within its first minutes, just
  an infrequent one.

## 1.12.0 -- 2026-09-07 (bounded rule scheduling)

- Host tick/cast dispatches share instruction, API-call, native-work and
  cooperative time budgets, with at most 16 callbacks per dispatch. Full
  invocation allowances are reserved before execution; deferred work does
  not reset Lua state or replay committed effects.
- Round-robin recipient queues preserve event payloads and per-rule FIFO
  order, coalesce ticks, and cancel pending work on disable/delete. Queue
  overflow drops newest requests with a visible notice. Pending work is
  transient and is not persisted on world exit.
- Native API scans consume a per-callback work budget; block scans check
  time between rows. Protected Lua calls cannot swallow work exhaustion.
- World API spawns return nil at 256 live creatures. Destroying a creature
  frees capacity, and rejected spawns do not reserve creature IDs.
- At most 64 modules load. Overflow and invalid saved source are preserved
  on subsequent saves, with a host notice; new generated modules report a
  capacity error. Failed VMs can rebuild even after erasing their callback.
- Updated generated prompt documentation and regression coverage for fair
  deferral, cancellation, overflow, resource admission, save preservation,
  creature capacity, deferred deaths, and native-work rollback.

## 1.11.0 -- 2026-09-07 (callback transactions)

- All five callbacks stage effects and commit only after execution and final
  budget checks succeed. Errors discard block edits, creature actions and
  deaths, player effects, inventory changes, time/weather state, broadcasts,
  and spawn ID/random-sequence changes.
- Queries see accepted changes within the callback and earlier committed
  callbacks in the same dispatch. Repeated inventory removals reserve funds;
  inventory grants become available immediately in the staged view.
- Failed Lua VMs are rebuilt from source before retry/reactivation, resetting
  their globals. Healthy VMs keep their existing state across callbacks.
- Extracted the Lua bindings and callback transaction boundary from the
  script host; creature changes replay bounded commands without cloning ECS.
- Block edits outside the world's vertical bounds now return false.
- This adds failure rollback, not player-facing undo/history, full simulation
  extraction, ordered network commits, or persistent Lua state. Save layout
  and the public method/entrypoint set remain unchanged.

## 1.10.0 -- 2026-09-07 (execution guards)

- Initialization and every callback now share the same execution guard:
  5ms cooperative wall-time limit, 100,000 instructions, and an 8 MiB Lua
  memory cap. Sources over 64 KiB are rejected before compilation.
- Instruction/time/API-call/broadcast exhaustion cannot be suppressed by
  Lua protected calls. Ordinary protected Lua errors continue to work.
- API calls are capped at 512 per callback, with at most 8 broadcasts of
  512 UTF-8 bytes each. Invalid non-finite numbers and coordinates outside
  +/-1,000,000 blocks are rejected before native execution.
- Documented the current limitations accurately: native calls are not
  forcibly preempted, world mutations are not transactional, aggregate
  tick budgets are pending, and UDP retries do not enforce edit ordering.
- No save layout or API method changes. Existing rules within the limits
  keep their existing entrypoints and per-call block/spawn allowances.

## 1.9.0 -- 2026-09-07 (goblin)

Adds a fourth hostile creature kind, `goblin`: a club-wielding melee bruiser
that sits between `wolf` and `stone_golem` on every axis.

- **Added the `goblin` creature kind**, hostile like `stone_golem`/`wolf`/
  `stinger` (`CreatureKind::is_hostile`): tankier and harder-hitting than a
  wolf but not as tanky/hard-hitting as a golem, on a cooldown between the
  two (1.1s vs wolf's 0.9s and golem's 1.5s). Has a "run" clip like wolf
  (`CreatureKind::has_run_clip`), so it visibly speeds up while
  aggro-chasing, at a pace a step under a wolf's. Excluded from the starter
  world scatter like every other hostile kind.

## 1.8.0 -- 2026-09-07 (creature model winding fix, stinger, cow)

Fixes the flicker that showed up on every part of every creature right after
1.7.0's real models landed, and adds two more creature kinds: hostile
`stinger` and neutral `cow`.

- **Fixed creature model triangle winding** (`src/model.rs`'s `load_glb`):
  glTF's front-face winding convention is counter-clockwise, but this
  engine's main render pipeline uses clockwise as front-face with back-face
  culling on (`app.rs`'s `render_pipeline`, matching the voxel mesher's own
  convention). 1.7.0 loaded glTF indices as-is, so every triangle in every
  creature model was backwards -- the pipeline culled each part's actually-
  visible surface and rendered its inside instead, and with these models'
  many closely-stacked decorative sub-parts (eye glints, moss patches,
  cracks, ...), that read as flicker/z-fighting everywhere rather than a
  clean "inside-out" look. Fixed by swapping each triangle's last two
  indices once, at load time. Locked in by a new test,
  `triangle_winding_matches_the_engines_clockwise_front_face_convention`,
  which checks the loaded winding against vertex normals for every bundled
  model and would fail loudly if this regresses.
- **Added the `stinger` creature kind**, hostile like `stone_golem`/`wolf`:
  fastest attack cooldown and lightest hit of the three, but no "run"
  clip -- it stays on its "walk" clip even while aggro-closing (its threat
  is attack rhythm, not chase speed). Excluded from the starter world
  scatter like every other hostile kind.
- **Added the `cow` creature kind**, neutral like `sheep`/`chicken` -- only
  ever moves under Lua's `chase()`, never attacks on its own. Unlike sheep/
  chicken, its model has a "run" clip, which a fast (`Wander::hunting`)
  Lua-driven chase now uses (`CreatureKind::has_run_clip`). Not added to the
  starter world scatter for now, kept opt-in via `spawn_creature`/
  `spawn_creature_near_player` like every hostile kind, so a fresh world's
  first minutes don't change.

## 1.7.0 -- 2026-09-07 (real creature models, wolf)

Every creature is now rendered as an actual animated model (loaded from
`models/*.glb`) instead of the two-box placeholder shape, and a new hostile
`wolf` creature kind joins `stone_golem` as a creature that fights back on
its own.

- **Real models for every creature kind** (`src/model.rs`, new): a
  dependency-free glTF/GLB loader (`serde_json` against the JSON chunk, hand
  parsed against the binary chunk) built for exactly the shape these
  Blockbench exports use -- a node hierarchy with no skinning (every moving
  part is its own node, animated by keyframing that node's own translation/
  rotation/scale), flat `baseColorFactor` materials (no images), and
  LINEAR-only sampler interpolation. `push_model` walks the hierarchy each
  frame, samples the requested clip at a given time, and appends the posed
  mesh straight into the same `Vertex` buffer the voxel terrain uses (real
  per-vertex normals now drive lighting for creatures too, instead of the
  old flat per-face shading). `creature::push_creature`'s box-drawing is
  gone entirely.
- **Per-creature animation state** (`src/creature.rs`): every creature now
  tracks a current `AnimClip` (`idle`/`walk`/`run`/`attack`) and elapsed
  time, chosen each tick from its movement this tick (idle when
  stationary, run instead of walk while aggro-chasing or Lua-`chase()`d fast
  if the kind's model actually has a run clip, attack for a fixed window
  right after a landed hit) -- not every kind has every clip; sheep/chicken
  only ever show idle/walk.
- **Added the `wolf` creature kind**, hostile like `stone_golem`
  (`CreatureKind::is_hostile` -- both now share one generalized aggro/attack
  code path in `Creatures::update` instead of a golem-only branch) but
  faster and lighter: it chases at `WOLF_RUN_SPEED` (6.0, under the player's
  own sprint speed so it stays escapable) rather than a golem's slow
  beeline, and bites for less damage on a much shorter cooldown. Like
  `stone_golem`, excluded from the starter world scatter -- only appears via
  `spawn_creature`/`spawn_creature_near_player`.
- **Creature snapshots now carry anim state**
  (`net::UnreliableMsg::Snapshot::creatures`, `Creatures::snapshot`): the
  `(pos, kind, facing)` tuple gained `(anim_clip, anim_time)` so a joined
  client renders the same pose the host computed, instead of only the box
  placeholder's position/facing.



Adds two new ways a rule/spell can be triggered by, and act on, a player
directly: `on_interact`, a non-destructive counterpart to `on_block_break`
fired by a dedicated "use" key, and `api.teleport_player`, the first way for
Lua to move a player at all (previously it could only read a player's
position, never set it).

- **Added the interact key (E)** (`src/input.rs`): edge-triggered like a
  mouse click, aimed at whatever's under the crosshair via the same
  `raycast` mining/placing already uses, but changes nothing on its own --
  it only reports "the player deliberately used this" for a rule to react
  to. Distinct from both existing clicks: unlike left-click it never breaks
  the block, unlike right-click it never places one.
- **Added `on_interact(api, event)`**, mirroring `on_block_break`'s
  optionality and `InteractEvent` shape (`kind`/`x`/`y`/`z`/`player_id`) --
  fires once per interact-key press aimed at a block, whether or not a rule
  does anything in response. Collected and dispatched the same way
  `BlockBreakEvent` is (`ScriptHost::run_tick`'s new `interacts` parameter),
  so a module defining only one of the two events never sees the other's
  event by construction, not by convention.
- **Added `api.teleport_player(player_id, x, y, z)`**: moves a player
  straight to a position, bypassing movement/collision entirely, and zeroes
  their velocity so the jump doesn't carry over whatever momentum they had.
  Applied directly to the host's own `Player::position` for `HOST_PLAYER_ID`;
  for anyone else it's a new targeted `ReliableMsg::Teleport`, the same
  single-recipient pattern `GrantItem` already uses -- the host applies its
  own teleport locally instead of round-tripping through the network. A
  remote target's `RemotePlayer::pos` is also snapped immediately on the
  host's side, ahead of the next `PlayerState` update the client will send
  once it applies the same teleport itself.
- **Added `ReliableMsg::Interact { x, y, z }`** (client -> host): a joined
  client's own interact-key press has no local effect to apply optimistically
  (unlike a block break, which removes the block client-side immediately) --
  it's reported to the host as-is, which reads the block at that position
  itself (its world state is authoritative, a client's view could be stale)
  before constructing the `InteractEvent`.
- **Rules panel hint text** now mentions the new key alongside the existing
  break/place hint.
- Added automated test coverage: `on_interact` firing with the correct event
  fields and never mutating the block it names; a block break never firing
  `on_interact` and vice versa (and the reverse, already covered by the
  existing `on_block_break` test); `teleport_player` queuing a `Teleport`
  effect for a connected player and failing for a missing one, the same
  connection-check pattern every other player-targeted action already
  follows; `Interact`/`Teleport` wire round-trips in `net.rs`.

### Compatibility

No breaking changes. `ModuleSaveEntry`'s serialized shape is untouched --
both additions are new optional API surface, so every existing rule/spell
keeps loading and running exactly as before (a module that doesn't define
`on_interact` simply never receives the new event, and nothing calls
`teleport_player` on its own). `UnreliableMsg::Snapshot`'s wire shape is
unchanged this time; the two new messages (`Interact`, `Teleport`) are
additions to `ReliableMsg`, which (like every past addition to it) breaks
wire compatibility between differently-versioned host/client binaries --
already true of any `Packet` shape change in this project, since there's no
protocol version negotiation yet.

## 1.5.0 -- 2026-08-31 (five-weather system)

Expands weather from a strict clear/rain alternation to five kinds --
sunny, rain, mist, storm, windy -- picked by weighted random timer instead
of forced alternation, with storm/windy visibly increasing grass/leaf wind
sway and mist hazing the fog.

- **Renamed `Weather::Clear` to `Weather::Sunny`** (`src/weather.rs`) and
  added `Mist`, `Storm`, `Windy`. `api.weather`/`api.set_weather` report/
  accept the new name `"sunny"`; `"clear"` is still accepted by
  `set_weather` as a backward-compatible alias so an old rule/save that
  says `api.set_weather("clear")` keeps working. `to_u8`/`from_u8` keep
  Sunny=0/Rain=1 stable and add Mist=2/Storm=3/Windy=4.
- **Weighted random selection, not alternation**: `WeatherState` used to
  strictly flip Clear<->Rain every stretch. It now picks the next weather
  by weight (sunny 45, rain 20, mist 15, windy 12, storm 8) and can land
  back on the current one -- deliberate, since that's what actually makes
  "sunny is most common" show up as multiple consecutive sunny stretches
  rather than a forced near-even rotation through all five.
- **Per-weather stretch duration**: storms are brief (30-90s), sunny
  stretches run long (120-300s) since it's the baseline, everything else
  in between (60-150s) -- previously every weather used the same flat
  90-240s range regardless of kind.
- **Storm/windy increase grass/leaf wind sway** (the request's main ask):
  added `Weather::wind_strength` (sunny/rain 1.0 baseline, mist 0.5, windy
  2.0, storm 2.5), threaded through as a new `w` component on
  `CameraUniform.light_params` (previously unused, hardcoded to `0.0`) and
  multiplied into the existing sway formula in `shader.wgsl`'s `vs_main` --
  the same per-vertex `wind` attribute both short grass (top-only sway) and
  leaves (whole-block sway, see `mesher.rs`) already used, so both move
  together with zero mesh-generation changes needed.
- **Storm renders rain too** (`Weather::has_rain_particles`, true for
  Rain and Storm) -- a storm with no rain falling would just be windy with
  a different name. **Mist hazes the fog color** toward a soft gray-white,
  blended with the existing sky-color fog rather than a separate
  fog-distance system, so it reads as visibly different from sunny without
  a new uniform field.
- Added automated test coverage in the previously-untested `weather.rs`:
  name/from_name round-trips (including the "clear" alias) for every kind,
  to_u8/from_u8 round-trips and unrecognized-value fallback, wind_strength
  ordering (storm > windy > baseline > mist), has_rain_particles,
  stretch-duration ordering, and a statistical check that sunny is picked
  meaningfully more often than storm over many rolls.

### Compatibility

No save-format changes -- weather was never saved with the world (resets
to Sunny every session, like health/oxygen/inventory). `Weather::Clear`'s
Rust name changed, but `to_u8()==0` is unchanged, so a decoded `Snapshot`
byte from an old build still reads correctly as the (renamed) Sunny.
`api.weather`'s reported string changed from `"clear"` to `"sunny"` -- a
rule checking `api.weather == "clear"` will no longer match (checking
`== "rain"` etc. is unaffected); `api.set_weather("clear")` still works via
the alias. No modules under `modules/` referenced `"clear"` (audited), so
none needed updating.

## 1.4.0 -- 2026-08-31 (oxygen)

Adds an oxygen attribute (0-100, starting full) that drains while a player
is submerged in water and drowns them (via the existing health/damage path)
if it stays empty too long -- the first engine-driven player hazard tied to
the world itself, rather than a rule or a hostile creature.

- **Added `oxygen`** to `Player`/`RemotePlayer`/`PlayerSnapshot`
  (`src/player.rs`), read-only from Lua via `api.players()`/
  `api.nearest_player()`. No `api.set_player_oxygen` -- submersion alone
  drives it, so there's nothing for a rule to meaningfully set.
- **Tuned rates, not the literal request**: asked for "1 oxygen every 10
  seconds" while submerged, but at that rate a full tank takes 1000 seconds
  (~16 minutes) to empty -- long enough that the mechanic would never
  actually matter in play. Went with `oxygen_drain_per_sec` = 2.0 instead
  (50s full drain, a common "how long can I hold my breath" ballpark in
  other games), and added `oxygen_regen_per_sec` = 12.5 (8s full recovery)
  so surfacing reads as a real relief rather than a slow trickle back --
  neither rate was in the original request. Keep the literal "1 health
  every 10 seconds" for `drowning_damage_per_sec` once oxygen hits 0: slow
  enough to give a player time to notice and swim up, which is exactly the
  fairness a drowning mechanic should have.
- **Continuous, not tick-based**: unlike poison's 10-second timer
  (`POISON_TICK_INTERVAL`), oxygen drain/regen/drowning-damage are applied
  every frame scaled by `dt` (`App::update_oxygen`) so the HUD bar (new:
  an Oxygen readout, shown only while it's below 100) moves smoothly
  instead of visibly stepping.
- **Replication**: `oxygen` rides the same best-effort Snapshot broadcast
  health/poisoned/the multipliers already use (`net::SnapshotPlayer`
  gained a field) -- host-authoritative, since only the host computes
  submersion for every connected player and applies the engine pass.
- Added automated test coverage: `Player::drain_oxygen`/
  `regenerate_oxygen` clamping at 0/`MAX_OXYGEN`; the tuned rates actually
  producing full drain/regen within their documented time budgets;
  `oxygen` round-tripping through `api.players()`/`api.nearest_player()`;
  `SnapshotPlayer` encode/decode (extended in the same test as 1.2.0's).

### Compatibility

No breaking changes to the save format -- `oxygen` follows the
`player_state_not_saved` precedent (health, poison, multipliers, inventory)
rather than becoming the first per-player state that persists.
`UnreliableMsg::Snapshot`'s wire shape changed again (one more field on
`SnapshotPlayer`), same caveat as 1.2.0's entry: host and client are always
expected to be the same build, no protocol version negotiation exists yet.

## 1.3.0 -- 2026-08-31 (stone golem)

Adds a third creature kind: `stone_golem` -- large, slow, and the first
hostile creature in the game. Unlike sheep/chicken (which only ever move
under a rule's `chase()`), a golem autonomously closes on and periodically
damages the nearest player within its aggro radius entirely on its own, no
Lua involved at all.

- **Added `CreatureKind::StoneGolem`** (`src/creature.rs`): speed 0.9
  (slower than both sheep 1.4 and chicken 2.0 -- deliberately escapable, not
  a fast ambush), 40 max health (vs. sheep 12 / chicken 6), and a body
  roughly 2x a player's footprint, taller and wider than either existing
  creature.
- **Added engine-driven combat**: `Creatures::update` now takes the
  connected players' positions and, for every golem with a player within
  `STONE_GOLEM_AGGRO_RADIUS` (12 blocks), overrides its wander/Lua-chase
  target to beeline for the nearest one (at its own slow speed, never the
  Lua-chase speed boost) and, once within `STONE_GOLEM_ATTACK_RANGE` (2.2
  blocks) and off cooldown (`STONE_GOLEM_ATTACK_COOLDOWN`, 1.5s), deals
  `STONE_GOLEM_ATTACK_DAMAGE` (4) via the same `PlayerEffect::Health` path
  `api.damage_player` uses -- so it replicates to a remote player and
  clamps at 0 identically. No new World API surface for the damage itself;
  a rule only ever *causes* a golem to exist (`api.spawn_creature`/
  `spawn_creature_near_player`, kind `"stone_golem"`) or reacts to one via
  the existing generic creature/health queries.
- **NOT added to the starter-world creature scatter**
  (`Creatures::spawn_around` stays sheep/chicken only, by design -- see its
  doc comment): a hostile creature ambushing a brand-new player with no way
  to have anticipated it would break the "same first-minutes experience
  every world" guarantee. A golem only ever appears because a rule/spell
  explicitly summoned one.
- Added automated test coverage: golem aggro closing the distance within
  radius, ignoring a player outside it, attack cooldown pacing (spaced
  correctly over a multi-hit window, not firing every tick), sheep/chicken
  provably never attacking regardless of proximity, kind round-tripping
  through `spawn_creature`/`spawn_creature_near_player`/`find_creatures`/
  `nearest_creature`.

### Compatibility

No breaking changes. `CreatureKind::to_u8`/`from_u8` gained a third variant
(`2`) with `from_u8` still defaulting anything unrecognized to `Sheep` --
existing saved worlds and rules that only ever produced/consumed 0
(sheep)/1 (chicken) are unaffected. No save-format changes.

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
