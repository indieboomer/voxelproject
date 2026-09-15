# Persistent object enchantments (Stage 2A)

The host can attach a continuous Lua rule to one creature, block or automation
device. The relationship survives save/reload, and all players see rule names and
statuses when aiming at the object. Rules execute on the authoritative host.

## Try it

1. Aim at an object within 18 blocks and open the prompt console (`~`).
2. Select **Persistent rule attached to the aimed object** and describe the rule.
   For example: "This creature follows me while I hold a crystal."
3. Submit. The target is captured when the console opens and stays fixed while
   typing and during generation, even if the creature walks away. Close and reopen
   the console to select another object. Missing or replaced targets are rejected;
   the console never substitutes the grass or another object behind them.
4. Review the generated source and target in **Rules**, then **Attach + enable**.
5. Aim at the object to see its enchantments. Use **Disable** or **Detach** in
   Rules to stop it. Save normally to retain activated attachments.

Existing continuous modules can also be attached directly to the current target.
New worlds include three disabled examples: `attached_crystal_follower.lua`,
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

Lost targets are not automatically rebound. Detach and explicitly attach again to
choose a replacement. Existing temporary chase behavior expires through its normal
lease; this stage does not add general effect rollback.

Each saved world has an identity. Creature IDs retain their allocation high-water
mark across reloads; blocks use a position plus a tracked replacement generation;
devices use a placement ID and base position. Each attachment has its own creation
ID and creator ID. The captured world revision records structural edit/creation
metadata; it is not a revision of every simulation event or a global invalidation
condition. Attached modules and bindings are saved together in the save extension,
separately from ordinary module list positions.

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

For creature movement, use `api.chase(target.id, player.x, player.y, player.z)`
after checking `target.kind == 'creature'` and looking up the player. The target
does not have `creature_id` or `entity_id` fields. Generation checks now run both
without an attachment and with the requested target kind, including a player
holding a crystal, so guarded attachment code is exercised before review.
Existing generated source is not rewritten by an executable update; regenerate
or correct a rule that already contains an invalid call.

## Validation and next stage

- Steam plus `dev-playtest`: **585 passed**, 27 opt-in tests ignored, including
  the bound-target generation regression for missing creature IDs.
- Direct: **561 passed**, 24 ignored before the final ward regression and prompt
  changes; the final Steam suite includes those changes.
- Seven attachment regressions cover reference identity, unload/resume, save/reload,
  ordinary-module reordering, device packing, staged edits, rollback and ward effects.
- Offscreen targeting HUD previews render in both themes; Fantasy was inspected.
- Live model generation and graphical host/guest save/reload acceptance remain open.

Next is **2B: owned effects and composition**: cleanup of effects belonging to a
creation, stacking/conflict rules and controlled reversal. Natural-language revision
and history belong to 2C; region rules remain later work.
