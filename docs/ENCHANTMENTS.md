# Targeted Enchantment spells

An Enchantment is a reusable Spellbook definition. Casting it from the hotbar
creates a separate persistent rule attached to a creature, block or automation
device. The host validates and executes every instance. All players see rule
names and statuses when aiming at an enchanted object.

## Try it

1. Open **Spell Workshop** (`~`) without selecting an object.
2. Select **Enchantment** and describe the behavior, for example: "The enchanted
   creature follows me while I hold a crystal."
3. Generate, inspect the name, description, target requirements and source, then
   choose **Remember**. This saves a template; it does not attach or activate it.
4. In **Spellbook** (`K`), select the Enchantment card. Press **1–9** or click a
   numbered slot button to assign it. Assignment replaces that slot; repeating
   the assignment removes it. The same controls are available for remembered
   results selected in the Workshop via number keys.
5. Close the panel, select the hotbar slot, aim at a compatible object within
   **18 blocks**, and **left-click**. The reticle and status explain readiness;
   block outlines also distinguish valid/invalid casts when outlines are enabled.
6. A successful cast attaches a new instance and displays particles and feedback.
   Aim at the object to inspect its status. Disable or remove instances from the
   Workshop. Save with **F5** to retain templates, assignments and attachments.

There is no **Run** action for Enchantments in either panel. Selecting or assigning
one has no world effect. Left-click casts instead of mining/attacking, and changing
slots restores the selected tool's normal action. Instant spells retain Run.

Templates cost no mana to generate or remember. Each successful attachment costs
the existing **20-mana rule fee**, with the existing **1.5-second spell cooldown**;
mana-free testing still applies. Invalid casts spend nothing and keep the selected
slot. A template can enchant several objects, producing independent instance IDs.
Existing rule-module and tracked-block limits still apply; there is no new per-spell
instance limit.

Direct creation-and-attachment from the Workshop has been removed. Previously
attached saved enchantments continue to load without conversion into templates.
New worlds retain three disabled authoring examples: `attached_crystal_follower.lua`,
`attached_block_ward.lua` (heals nearby creatures during rain), and
`attached_device_ward.lua` (heals nearby creatures at night). Existing saves retain
their module lists; they do not automatically import new starter files.

The generation path supplies the attachment API and bypasses the species-wide
policy shortcut. Review still matters: an attachment provides context and lifecycle,
but does not restrict all of the module's actions to that object.

## Lifetime and identity

| Target/event | Behavior |
| --- | --- |
| Creature dies or is removed | Rule stops; a new creature cannot inherit it. |
| Block is replaced | Rule stops, even if the original material is later restored. |
| Device is packed or replaced | Rule stops; placing the packed device creates a new placement identity. |
| Device configuration or rotation changes | Attachment remains. |
| Block/device chunk unloads | Callbacks pause and resume when loaded. |
| Unrelated world edit | Attachment remains. |
| Disable or detach | Future callbacks stop; previous world changes remain. Detach leaves the module disabled. |

Lost targets are not automatically rebound. Cast a remembered template again to
choose a replacement. Existing temporary chase behavior expires through its normal
lease; this stage does not add general effect rollback.

Each saved world has an identity. Creature IDs retain their allocation high-water
mark across reloads; blocks use a position plus a tracked replacement generation;
devices use a placement ID and base position. Each attachment has its own creation
ID, source spell ID/version, stable owner account, and creator ID. The captured world revision records structural edit/creation
metadata; it is not a revision of every simulation event or a global invalidation
condition. Attached modules and bindings are saved together in the save extension,
separately from ordinary module list positions.

The Spellbook stores the prompt, generated description/source, type, author,
target requirement, icon/artwork and spell revision, without a concrete object
reference. Each attachment stores its resolved source independently. Editing or
deleting a template does not rewrite or remove existing instances; deleting an
instance does not delete its template. Legacy bindings without source/owner fields
keep loading. Missing/replaced targets disable the instance on validation and log
the error; they are never rebound to a new object at the same position.

## Multiplayer

Host and guests use the same cast validation and the normal hotbar request path.
Guests need the physical spell card and the host's **Allow guests to cast** setting.
Use the existing **Give card** control to transfer it. Guest-generated proposals
still require host review; the host remembers approved definitions and manages
sharing. No arbitrary client Lua is accepted as part of a cast request.

Requests reference the spell ID/revision, aimed creature ID or block coordinates,
device placement ID where applicable, facing and observed host world revision.
The host re-resolves range, visibility, target compatibility/existence, selected
slot, card ownership, permission, health, mana and cooldown. A changed world
revision conservatively rejects a guest enchantment request: aim again after the
next snapshot. This can also happen after an unrelated structural edit, but prevents
attaching to a replacement object during transit.

Clients receive creature IDs with picking geometry, read-only spell metadata,
attachment summaries, cast particles and the ordinary authoritative world updates.
Guest cards and hotbar assignments remain in the saved account. On reconnect,
new instances' creator IDs are resolved from their stable owner account; a reused
connection ID cannot impersonate an absent creator. This update uses protocol
**49**; host and guests must update together.

The shared module limit still applies. At most 1,024 block positions can be tracked
for attachments per world. Guest clients receive bounded display summaries, not
Lua source. All peers must use the same build.

## Authoring

World API **1.37.0** adds `api.get_rule_target()`. It returns `nil` without an
attachment, otherwise a table with `creation_id`, `creator_id`, `world_id`,
`world_revision`, and target fields:

| Kind | Target fields |
| --- | --- |
| `creature` | `id`, `species`, current `x`, `y`, `z` |
| `block` | `x`, `y`, `z`, `material` |
| `device` | `id`, `device_type`, base `x`, `y`, `z` |

Guard against `nil` and query current state inside callbacks. See the generated
[API reference](../world_api/world_api_readme.md) and the three starter modules.
Lua globals are recreated on load; arbitrary script-local state is not persisted.

Reusable generated templates carry `-- spell_type: enchantment` and may declare
`-- spell_target: creature`, `block`, `device`, or `any`. Creature/block/device
requirements are checked before charging. Persistent behavior still uses
`on_tick` and `api.get_rule_target()`; the rule language has not changed.

For creature movement, use `api.chase(target.id, player.x, player.y, player.z)`
after checking `target.kind == 'creature'` and looking up the player. The target
does not have `creature_id` or `entity_id` fields. Generation checks now run both
without an attachment and with the requested target kind, including a player
holding a crystal, so guarded attachment code is exercised before review.
Existing generated source is not rewritten by an executable update; regenerate
or correct a rule that already contains an invalid call.

## Validation and manual acceptance

The Direct executable builds successfully. The Enchantment filter passes **17
tests**, and offscreen Workshop/Spellbook previews render in both themes. The
Steam plus `dev-playtest` suite passes **623 tests**, with **32 opt-in tests ignored**,
zero failures and no exclusions.

`cargo test --offline enchantment` covers target-free creation, inert remembering,
Workshop/Spellbook Run visibility, hotbar replacement/removal, host and guest casts,
free failure paths, independent instances, guest creature picking, forged/stale
requests, save/load, reconnect ownership and legacy/missing targets. Existing
attachment tests cover unload/resume, replacement, device packing and callback
rollback. The full suite also checks held-item alignment across all player models.
The Hello gesture's runtime duration now matches its bundled 2.2-second animation,
fixing early cutoff and the previously failing animation-duration assertion.

```powershell
cargo build --offline --no-default-features
cargo test --offline enchantment
cargo test --offline --features steam,dev-playtest --quiet -- --test-threads=1
```

Before packaged multiplayer acceptance, check on a host and a separate guest:

- Generate without aiming, Remember, and verify no world effect or Run action.
- Bind/replace/remove slots; check both UI themes and switching back to tools.
- Cast on creatures, blocks and devices; compare particles, status and behavior
  on both screens. Reuse one template on two objects.
- Try no target, obstruction, range, incompatible category, no mana, cooldown,
  missing card and revoked permission. Check no charge or instance appears.
- Replace/pack a target while a guest cast is in transit; confirm rejection.
- Save/reload and reconnect under the same account; verify cards, assignments,
  attachment targets and ownership, including a different guest joining first.
- Edit/delete the template and confirm earlier instances retain their behavior.
- Load an older world with attached rules and a save with a missing target.

Live model generation and graphical host/guest acceptance remain manual checks.

Next is **2B: owned effects and composition**: cleanup of effects belonging to a
creation, stacking/conflict rules and controlled reversal. Natural-language revision
and history belong to 2C; region rules remain later work.
