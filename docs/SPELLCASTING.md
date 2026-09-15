# Spellcasting implementation

## Plan assessment

[The active plan](../SPELLCASTING_PLAN.md) separates reusable casts from persistent
enchantments. Phase 1 can reuse the existing Lua sandbox, callback transactions,
creature queries/healing/damage, block edits, inventory authority and cast fees.
Stage 1B now stores a distinct saved spell definition alongside legacy module
saves. Hotbar entries still contain only resource or equipment references.

Keep implementation in the plan's order. Stage 1A establishes hand-written target
semantics before generation depends on them. Stage 1B adds stable spell IDs,
revisions, target requirements, mana/cooldown/range metadata, compatibility review,
saved definitions and the dedicated Spellbook. Stage 1C can then add hotbar spell
references, host-validated guest requests, replay protection and targeting HUD.
Stage 1D expands capabilities based on playable spell requirements.

Durable world identity/revisions need an explicit definition in those stages;
a world seed or a module's list index is not a durable identity. A global revision
must not accidentally invalidate every cast when an unrelated block changes.
General owned modifiers and enchantment lifecycle remain Phase 2 work.

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

### Stage 1A scope

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

### Next stage and remaining checks

Stage 1C adds hotbar spell references, target/cooldown HUD, host-to-guest metadata
delivery and permitted guest cast requests with replay protection. The Stage 1B
book is host-managed; guests do not receive its definitions yet. Costs/range are
saved but fixed to supported defaults in this stage. Source editing, natural-language
revision and revision history are not added by rename or duplicate.

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
