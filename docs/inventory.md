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

Select a food stack and click **Eat 1** to consume one item, restore health and
clear [hunger](HUNGER.md):

| Food | Health restored |
| --- | ---: |
| Raw meat | 8 |
| Cooked meat | 25 |
| Pumpkin | 10 |
| Wild herbs | 5 |
| Brown mushroom | 6 |

Healing stops at 100 health. The resource **Eat 1** button currently remains
disabled at full health, including while hungry, and for defeated players.
Harvested foods and dishes use a separate **Eat** button, available at full health.
Ordinary food does not cure poison; herbal purifying stew does, while glowcaps and
toxic dishes cause poison. The host validates inventory revisions, consumes one
item, and applies healing and fullness; duplicate requests cannot consume twice.
Raw and cooked meat are inventory resources and cannot be placed as terrain.
Aim at a campfire and press **F** to cook raw meat individually or in batches of up to 64.

## Hotbar use and authority

During gameplay, **1–9** selects a slot and the **mouse wheel** cycles assigned
slots with available entries. New accounts assign axe, pickaxe and sword
to slots 1–3. Clearing a slot leaves its inventory intact and enables empty hands.
When a stack reaches zero, equipment is no longer owned, or a spell becomes
unavailable, all its hotbar bindings are cleared automatically. The selected slot
stays selected and becomes empty; replenishing the stack does not restore its old
bindings. Reassign it from inventory if desired. This also applies after loading
a save and to authoritative guest inventory updates.

Resources place blocks; compatible tools mine; weapons use their equipment action.
Empty hands gather explicitly hand-pickable plants. Pending actions referencing
depleted stacks are rejected; cleanup leaves the slot empty for the next action.
Axes handle wood/plants and pickaxes handle
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
Host spell cards have a Delete button. Deleting a source module from Rules also
deletes its remembered cards (including renamed and duplicated copies), using
the saved source and original prompt rather than the display name. Other spells
with the same name remain. Deleting a card alone leaves its Rules module available.
Spells needing compatibility review remain visible but cannot be assigned or cast.
Their hotbar bindings are cleared too; low mana or cooldown alone does not clear a slot.
K still opens the full Spellbook for inspection, rename, duplicate, delete and casting.
Successful casts show short purple-and-gold particles, also visible to guests.
The HUD shows the target, mana cost, cooldown and readiness. The host can enable
**Allow guests to cast this spell** in K → Spellbook; guests then see it in their
inventory and read-only Spellbook. The host checks every guest cast and charges
that guest's mana. Revoking permission clears the guest's corresponding bindings.
Spell definitions remain host-managed. All players need matching game builds.
