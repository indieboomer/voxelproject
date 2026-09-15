# Inventory and hotbar

Health and mana appear in the top row. Selecting equipment exposes creation and
decomposition; selecting a resource exposes elemental extraction. See
[mana and recipes](mana.md) for costs, recovery and exact tool formulas.

Press **I** to open or close inventory, or **Esc** to close it. Gameplay movement
and mouse actions are suspended while the cursor is available for inventory.

The **Left hand** row equips or puts away a [torch](TORCHES.md) independently of
the active hotbar tool. Learn its recipe from The Wayfarer's Handbook.

The top row shows Earth, Fire, Water, Life and Death balances. Items on the left
include three starter tools/weapons and [16 specialist items](EQUIPMENT_AND_RECIPE_BOOKS.md); selecting unowned equipment shows its creation recipe. Specialist crafting requires discovering its recipe book. Resources
on the right show owned stacks alphabetically with counts and world-texture icons.
Both columns scroll independently and impose no inventory slot limit.

Click an entry to select it, then press **1–9** to assign it to that hotbar slot.
You can also click a hotbar slot after selecting an entry. Selecting an inventory
entry alone does not change equipment. Without a selection, numbers select the
active slot. Clear the active slot to use empty hands. Assignment references the
existing stack; it does not consume or duplicate items. Depleted entries cannot
be assigned, and resource and element balances remain host-authoritative.

Select a food stack and click **Eat 1** to consume one item and restore health:

| Food | Health restored |
| --- | ---: |
| Raw meat | 8 |
| Cooked meat | 25 |
| Pumpkin | 10 |
| Wild herbs | 5 |
| Brown mushroom | 6 |

Healing stops at 100 health. Full-health or defeated players cannot eat and keep
their food. Eating does not cure poison. The host validates inventory revisions,
consumes one item, and applies healing; duplicate requests cannot consume twice.
Raw and cooked meat are inventory resources and cannot be placed as terrain.
Aim at a campfire and press **F** to cook raw meat individually or in batches of up to 64.

## Hotbar use and authority

During gameplay, **1–9** selects a slot and the **mouse wheel** cycles assigned
slots, including depleted assignments. New accounts assign axe, pickaxe and sword
to slots 1–3. Clearing a slot leaves its inventory intact and enables empty hands.
Existing depleted assignments become usable again when their stack is replenished.

Resources place blocks; compatible tools mine; weapons use their equipment action.
Empty hands gather explicitly hand-pickable plants. A depleted resource assignment
does not count as an empty hand. Axes handle wood/plants and pickaxes handle
stone/ore/soil; specialist equipment supplies additional categories. Bow and longbow
are currently disabled by `Gear::enabled` in `src/equipment.rs`.

Accounts save equipment counts, all nine assignments and the selected slot. The host
validates item actions against ownership, revisions, range, visibility, tool rules,
placement constraints and balances. Guests send intentions; authoritative snapshots
replicate held items and inventory. Steam accounts use Steam identity; Direct guests
should retain their nickname to recover saved inventory.

Target outlines are configurable in [settings](UI_SETTINGS.md), and remote held-item
animation is described in [player animations](PLAYER_ANIMATIONS.md).

## Spells

The third inventory column lists remembered spells. Select a spell, then press
1–9 or click a hotbar slot to assign it. Close inventory, select that slot and
left-click to cast at your aim. Spell slots show a rune and the selected spell's
name. They consume mana rather than items; right-click does nothing.

Bindings save with the world, follow renames, and clear when the spell is deleted.
Spells needing compatibility review remain visible but cannot be assigned or cast.
K still opens the full Spellbook for inspection, rename, duplicate, delete and casting.
Successful casts show short purple-and-gold particles, also visible to guests.
The HUD shows the target, mana cost, cooldown and readiness. The host can enable
**Allow guests to cast this spell** in K → Spellbook; guests then see it in their
inventory and read-only Spellbook. The host checks every guest cast and charges
that guest's mana. Revoking permission clears the guest's corresponding bindings.
Spell definitions remain host-managed. All players need matching game builds.
