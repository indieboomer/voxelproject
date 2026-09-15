# Spellcasting implementation

## Plan assessment

[The active plan](../SPELLCASTING_PLAN.md) separates reusable casts from persistent
enchantments. Phase 1 can reuse the existing Lua sandbox, callback transactions,
creature queries/healing/damage, block edits, inventory authority and cast fees.
Stage 1B now stores a distinct saved spell definition alongside legacy module
saves. Hotbar entries now also contain stable spell references.

Keep implementation in the plan's order. Stage 1A establishes hand-written target
semantics before generation depends on them. Stage 1B adds stable spell IDs,
revisions, target requirements, mana/cooldown/range metadata, compatibility review,
saved definitions and the dedicated Spellbook. Stage 1C adds hotbar spell
references, host-validated guest requests, replay protection and targeting HUD.
Stage 1D adds bounded temporary statuses and creature pushes; existing APIs cover
healing/damage, block transforms, spawning, devices and inventory operations.

[Stage 2A](ENCHANTMENTS.md) now defines saved world and object identities,
attachment lifecycle and target-loss handling. Unrelated block changes do not
invalidate attachments. General owned modifiers and composition remain Stage 2B.

## Stage 1A implementation

World API **1.34.0** adds optional `event.target`, `origin`, `facing`,
`hit_position`, `hit_normal` and a process-local `cast_id` to host-targeted casts.
Vectors expose x/y/z. Creature targets expose `kind = "creature"` and `id`;
block targets expose `kind = "block"`, integer x/y/z and `material`.

The host picks a creature body or block along its current aim, within 18 blocks.
Picking shares creature hit proxies with weapons and the terrain raycast with
block interactions. The cast boundary rechecks caster position, target identity,
block material, range, terrain obstruction and loaded visibility. Supplied hit
coordinates are recomputed. Missing, changed or invalid targets cannot commit
the reference spells' effects.

Execution uses the existing transactional callback and normal replicated outcome
path. The existing Run action reserves the base fee (normally 5 mana) before Lua,
then refunds it if the cast fails. No LLM request occurs during Run. Targeted
casts require an idle scheduler and cancel unexecuted work so a refunded request
cannot execute on a later tick.

### Try the three reference spells

Start a **new world** to load the starter files under `modules/`. Aim during normal
play, open the existing Rules panel, and click **Run** for:

- `target_heal`: heal the aimed creature by up to 20 health, capped by its species.
- `target_damage`: damage the aimed creature by 20.
- `target_stone`: replace the aimed block with stone.

Close the panel, aim at a different creature/block and run the same spell again.
Target IDs are supplied at casting time, not embedded in the Lua source.
Modules use normal rule-save persistence; adding starter files does not insert
them into an existing saved world's module list.

### Original Stage 1A scope (historical)

- The initial fixtures run through the Rules panel. Stage 1B below adds a
  Spellbook for these and generated instant spells; hotbar assignment is pending.
- Guest-authored proposals retain untargeted execution with their author's caster
  identity. Guests cannot submit targeted casts yet.
- Range is fixed at 18. Stage 1B adds required-target metadata and a Spellbook
  cooldown. Configurable costs/range, durable world revisions and network replay IDs
  remain pending.
- Target context is immediate and transient; `world_revision` and `selected_region`
  are not exposed. `cast_id` is not persisted and is not a network deduplication key.
- Player, device, loot and empty-position targets are pending. Creature hit normals
  are zero because picking uses body proxies, not exact mesh-surface intersections.
- Target context does not restrict a reviewed script's other API capabilities.
  The reference spells assert their required kind before modifying anything.
- Multiplayer effects use the existing replication path; graphical multi-player
  acceptance and live-model targeted generation have not been verified here.

## Checks

```powershell
cargo test --offline --no-default-features targeted_ -- --test-threads=1
cargo test --offline --no-default-features -- --test-threads=1
```

Targeted regressions cover reusing a healing module on different creatures,
damage, block replacement, wrong kinds, stale/dead targets, obstructions,
invalid aim/origin, range, unloaded visibility, recomputed hit metadata and
transaction rollback after damage, block edits and mana commands.

Validation on 2026-09-15: **535 tests passed** in both Direct and Steam
configurations; 24/25 opt-in tests were ignored respectively. The API references
were regenerated from the schema, documentation links resolve, and `git diff
--check` passes. The Steam development executable also builds successfully with
`cargo build --offline --features steam` at `target/debug/voxelproject.exe`.
Live-model and graphical multiplayer checks remain unperformed.

## Stage 1B: persistent Spellbook

### Player flow

1. Generate an instant spell normally, review its effect, and click **Remember**
   beside it in the Rules panel. Existing saved-world spells can be remembered too.
2. Press **K** during gameplay or use **Spellbook** in the Rules panel. **Esc** or
   the window close button returns to play. Search by name or description.
3. Select a remembered spell to inspect its effect, author, original prompt,
   target requirement, cost, range, cooldown, revision and compatibility status.
   Generated source and structured interpretation remain under Advanced.
4. Rename, duplicate or delete entries. **Save changes** applies a new name/target
   requirement and increments the revision. Duplicates receive a new stable ID.
5. Aim before opening the book, then use **Cast at current aim**. Successful casts
   cost 5 mana (zero with host mana-free testing) and have a 1.5-second cooldown.
   Invalid targets, insufficient mana, cooldown rejection and callback failure
   spend no mana. Casting does not contact the LLM.
6. **F5** saves remembered spells with the world. Changes survive normal save/load;
   Spellbook edits do not independently autosave the world.

**Spell-defined** preserves existing generated code's targeting behavior. No target
category is guessed from the player's wording. Choose Creature or Block to add an
engine check before casting; this does not rewrite or restrict the code's effects.
The three supplied target spells declare their category with a `spell_target`
source comment. Aim checks still use Stage 1A's 18-block range.

### Persistence and validation

`src/spellbook.rs` owns up to 64 remembered definitions. Each stores a world-local
monotonic ID, revision, original source/prompt, author, interpretation, target
requirement, API version, icon parameters, costs and validation metadata. Deleting
an entry never reuses its ID; list order is not identity. Rename/target updates
preserve source and prompt. Remembering the same source/prompt again selects its
existing entry rather than silently creating another.

The book lives in the JSON save envelope's `crafting.spellbook` field, using the
existing atomic world-save path. Legacy `ModuleSaveEntry` binary layout is unchanged.
Old JSON and binary saves start with an empty book and retain their old modules.
Remember makes an independent copy: deleting a Rules-panel draft does not delete
its remembered spell, and deleting a remembered spell does not undo cast effects.

On load, saved validation badges are discarded and recomputed without an LLM.
Checks cover supported API versions, literal API names, syntax, bounded sandbox
loading, the instant callback and supported metadata. Compatible older source stays
byte-for-byte intact. Unsupported versions, unknown methods, invalid source,
duplicate identities and excess entries remain preserved but require review and
cannot be cast. These checks do not prove the meaning of arbitrary Lua.

The current host casts remembered spells as themselves, even if the original
author was a guest. Legacy guest proposals keep their existing author-caster
behavior outside the Spellbook. Execution temporarily loads the definition into
the existing bounded script runtime, applies normal host outcomes, then removes
that execution module. It never leaves a second saved module or running rule.
Lua globals reset between remembered casts. Cooldown timers are session-local;
the configured duration is saved, but reopening a world starts ready.

### Stage 1C: hotbar and guest casting

The inventory now has an independent Spells column. Select a remembered spell,
press 1–9 or click a hotbar slot, then close inventory and left-click with that
slot selected. The Spellbook retains all its management and cast controls.
Stable hotbar references save with accounts, follow renames and clear on deletion.
Incompatible spells cannot be assigned or cast. Mana, cooldown and aim failures
appear as gameplay toasts. Successful casts produce bounded, short-lived emissive
particles, replicated from host to guests. Failed casts produce no particles.
World API 1.36.0 reports an available selected spell as `spell:<id>` through
`get_equipped_item`. Network protocol 41 requires matching game builds.

The HUD shows target, cost, cooldown, invalid aim and pending-host status.
In the host's Spellbook, enable **Allow guests to cast this spell**. New spells
and duplicates default to private. Shared spells appear in guests' inventory and
read-only Spellbook; guests can assign or cast them but cannot edit definitions.
Revoking permission removes guest bindings. Permission and assignments save with
the world. The grant permits the reviewed code's normal World API capabilities.

Only bounded display metadata travels to guests; Lua stays on the host. Requests
contain connection token, sequence, spell ID/revision, direction and expected target.
The host derives caster identity and eye position, re-resolves aim, checks permission,
health, mana, cooldown and scheduler readiness, then executes the saved definition.
Repeated/out-of-order requests and previous-connection tokens cannot execute.
Failed callbacks refund the fee; cooldown and particles start only on success.
Guest cooldowns are per account/spell and survive reconnection within the session.
They reset when reopening the world, like host cooldowns.

Costs/range remain fixed at supported defaults (5 mana, 1.5 seconds, 18 blocks).
Source editing, natural-language revision and revision history remain Phase 2 work.

### Stage 1D: composable spell actions

- `apply_creature_status(id, "slow" | "stun", seconds)` accepts 0–30 seconds.
  Slow halves movement/action speed; stun pauses them. Reapplication replaces the
  duration and 0 clears that status. Stun takes precedence; durations do not multiply.
  Remaining duration saves with creatures and expires on host simulation time.
- `push_creature(id, dx, dy, dz)` moves up to 8 blocks per call, checking the whole
  body path against loaded terrain and staged block edits. It stops at obstacles.
  Dragons are unsupported; fish must remain in water. Normal body separation and
  movement resume afterward. Changes roll back with failed callbacks.
- Existing heal/damage, block replacement, spawning, player teleport, device enable/
  configuration, and give/take/craft/decompose APIs supply the other action categories.
- New starter spells: `target_slow`, `target_stun`, `target_push`, `target_summon`,
  and `target_device_toggle`. New worlds load these from `modules/`; existing saves
  retain their module lists. Generate equivalent spells using the updated API docs.

### Next: acceptance and Phase 2B

Verify a host and guest sharing a spell, casting at different targets, invalid casts,
permission revocation, reconnect and save/reload in a live session. Automated checks
cover the underlying paths; they do not replace this graphical multi-PC check.
Phase 2A now attaches persistent rules to durable creature/block/device references,
saves those relationships and defines what happens when their targets disappear.
See the [enchantment guide](ENCHANTMENTS.md) for usage and validation. Stage 2B adds
owned effects, cleanup and composition; disabling a 2A rule stops future callbacks
but does not undo its previous world changes.

The user reported successful generated casting (including killing a cow in front
of the player) after Stage 1A. Automated checks and UI previews are separate evidence;
an end-to-end graphical save/reload and multi-PC acceptance session remains useful.

### Stage 1B validation (2026-09-15)

- Direct suite: **542 passed**, 24 opt-in tests ignored.
- Steam plus `dev-playtest` suite: **564 passed**, 27 opt-in tests ignored.
- Seven added regressions cover save-envelope migration, stable IDs/revisions,
  incompatible-source preservation, saved-status revalidation, attribution/plan
  retention and repeated casting of a restored definition without extra saved modules.
- The offscreen Spellbook preview passed and was inspected in Generic and Fantasy
  themes. Primary cast/duplicate/delete actions remain visible in the initial view.
- Documentation links resolve and `git diff --check` passes.

### Inventory/hotbar validation (2026-09-15)

- Direct suite: 546 passed, 24 opt-in tests ignored before the final equipped-spell regression.
- Steam plus `dev-playtest`: 569 passed, 27 opt-in tests ignored, including that regression.
- Regressions cover spell assignment permissions, keyboard assignment, save/reload,
  rename/delete bindings, staged inventory queries, particle bounds and expiration.
- Inventory previews inspected in Generic and Fantasy themes; particle scene rendered
  on the GPU. Live gameplay and a multi-PC session were not run for this change.

### Stage 1C/1D validation (2026-09-15)

- Steam plus `dev-playtest`: **577 passed**, 27 opt-in tests ignored.
- Direct: 553 passed before the final two reference-spell regressions; all seven
  targeted tests then passed, including guest slow/stun/push/summon and device casting.
- Tests cover metadata packet bounds, permissions and revocation, stale spell revisions,
  changed/obstructed targets, invalid directions, repeated/reordered requests and old
  connection tokens, rate limits, staged terrain collision, rollback, status expiry,
  slowdown behavior and saved remaining durations.
- Host and guest Spellbook and casting-HUD GPU previews passed in Generic and Fantasy
  themes. A live host/guest game session remains an acceptance task.
