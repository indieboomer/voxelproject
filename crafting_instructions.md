Implement a complete Elemental Crafting system in the existing Voxel Project, including gameplay logic, data definitions, persistence, multiplayer authority, and an in-game crafting UI.

First inspect the repository and identify:

* the engine/framework and current project structure;
* inventory, item, block, creature, mana, save/load, UI and multiplayer systems;
* existing coding conventions and reusable UI components;
* how player interactions and input bindings are implemented.

Adapt the implementation to the existing architecture. Do not introduce a parallel framework or replace working systems.

## Core rules

The game has five fundamental elements:

* Earth
* Fire
* Water
* Life
* Death

Resources, items, blocks and creatures may have an elemental composition consisting of integer quantities of these elements.

The player has:

* an elemental inventory containing quantities of all five elements;
* a mana balance.

Crafting uses recipes containing between one and five ordered slots.

Each occupied slot contains:

```text
element: one of Earth, Fire, Water, Life or Death
amount: positive integer
```

Rules:

* Slot order matters.
* The same element may appear in multiple slots.
* Empty trailing slots are allowed and ignored.
* Empty gaps between occupied slots should be normalized away.
* A recipe matches only when the ordered elements and their quantities match exactly.
* The same input must always produce the same result.
* Recipe matching must be deterministic and must not involve the LLM.

Examples:

```text
[Water ×1] [Death ×1] -> Light Poison
[Death ×1] [Water ×1] -> Preservative
[Earth ×2] [Fire ×1] -> Brick
[Fire ×1] [Earth ×2] -> Lava
[Earth ×5] [Life ×2] [Fire ×1] -> Stone Golem
```

These examples may be adapted to identifiers already present in the project.

## Mana cost

Mana cost is determined exclusively by the number of occupied recipe slots:

```text
1 slot  = 0 mana
2 slots = 2 mana
3 slots = 4 mana
4 slots = 16 mana
5 slots = 256 mana
```

Keep this mapping in one configurable data structure rather than scattering constants through the code.

The quantities inside slots determine the elemental material cost. The number of occupied slots determines the mana cost.

Crafting must be possible only when:

* the ordered slot sequence matches a registered recipe;
* the player owns all required elemental quantities;
* the player has enough mana;
* the output can be created safely.

Crafting must atomically:

1. validate the recipe;
2. validate elemental inventory and mana;
3. consume the required elements;
4. consume mana;
5. create the output;
6. report success or failure to the UI.

No materials or mana may be consumed if output creation fails.

## Data-driven recipes

Create a data-driven recipe registry. Recipes must not be hardcoded inside UI or interaction logic.

Each recipe should contain at least:

```text
id
ordered input slots
output type
output id
output quantity
optional display metadata
```

Support outputs such as:

* inventory items;
* placeable blocks or resources;
* creatures/entities.

Use the project’s existing spawning and inventory systems. Item outputs should enter the player inventory. Creature outputs should spawn at a validated nearby position rather than entering the inventory.

Add a small starter set of recipes using existing project content where possible. Include examples demonstrating that reversed slot order can produce a different result.

Detect duplicate recipe definitions during loading and report a clear error.

## Elemental composition

Add a data-driven way to assign an elemental composition to existing resources, items, blocks and creature types.

Do not manually modify every asset if the project already has a central item/entity definition registry. Extend that registry instead.

Provide APIs equivalent to:

```text
getElementalComposition(objectType, objectId)
addElements(player, composition)
canAffordElements(player, composition)
consumeElements(player, composition)
```

If no elemental inventory exists, implement it as part of player state.

Element-to-mana conversion should use a centrally configurable conversion rate. Use a temporary default of one elemental unit converted into one mana unless the project already defines another rate. Make the rate easy to rebalance later.

Conversion must be explicit: the player chooses an element and quantity and confirms the conversion. Do not automatically convert missing materials into mana or mana into elements.

## Crafting UI

Create an in-game Elemental Crafting screen consistent with the game’s current visual style and input system.

The screen should contain:

1. Current mana balance.
2. Current quantities of Earth, Fire, Water, Life and Death.
3. Five ordered crafting slots displayed from left to right.
4. An element selector for every slot.
5. An editable positive integer amount for every occupied slot.
6. A clear/remove action for each slot.
7. A clear-all action.
8. A visible mana cost calculated from the number of occupied slots.
9. A result preview when the current sequence matches a recipe.
10. A clear “Unknown formula” state when it does not match.
11. A Craft button.
12. Feedback explaining why crafting is unavailable:

* unknown formula;
* insufficient element;
* insufficient mana;
* inventory full;
* invalid spawn location;
* other relevant failure.

13. A small element-to-mana conversion section.

Use recognizable colors and icons where suitable, but always include text or tooltips so the interface does not depend on color alone.

Suggested elemental colors:

```text
Earth: brown or ochre
Fire: orange or red
Water: blue
Life: green
Death: violet, dark grey or desaturated purple
```

When a slot is selected, let the player choose one of the five elements and set its amount. The same element must remain selectable in later slots.

Changing any slot must immediately update:

* normalized input sequence;
* matched result;
* required elemental totals;
* mana cost;
* affordability indicators;
* Craft button state.

Make the UI usable with the input methods already supported by the project. Preserve existing mouse/keyboard and controller conventions where applicable.

## Mana conversion UI

The conversion section should allow the player to:

* choose one of the five elements;
* enter a quantity;
* see the amount of mana that will be received;
* confirm the conversion;
* receive clear success or failure feedback.

Keep the conversion rate visible or explain it in a tooltip.

## Multiplayer and authority

The authoritative host/server must validate and execute crafting and element-to-mana conversion.

Clients may calculate previews locally, but they must not be able to grant themselves:

* elements;
* mana;
* items;
* blocks;
* creatures.

Replicate updated elemental inventory, mana and created outputs using the project’s existing networking architecture.

Prevent duplicate crafting caused by repeated requests, latency or UI double-clicking.

## Persistence

Persist at least:

* player elemental inventory;
* player mana;
* any recipe discovery state only if such a system already exists;
* crafted world outputs through the existing world save system.

Do not introduce a new recipe-discovery mechanic unless the project already contains one.

Loading older saves must not crash. Initialize missing elemental data with safe defaults.

## Tests

Add automated tests appropriate to the existing test setup.

Cover at least:

* exact ordered recipe matching;
* reversed order not matching or producing a different registered output;
* repeated elements in separate slots;
* normalization of empty gaps;
* rejection of zero and negative amounts;
* mana cost for one through five occupied slots;
* insufficient elements;
* insufficient mana;
* atomic rollback when output creation fails;
* successful consumption and output creation;
* duplicate recipe detection;
* element-to-mana conversion;
* multiplayer/server-side validation where practical;
* serialization and loading of elemental inventory.

## Quality constraints

* Keep the implementation straightforward and data-driven.
* Do not involve the LLM in deterministic recipe resolution.
* Do not add blueprint, catalyst, refinement, quality-tier or workstation mechanics.
* Do not implement long chains of intermediate resources unless needed by existing sample content.
* Avoid unrelated refactors.
* Reuse current UI, inventory, entity and networking systems.
* Keep balancing values easy to edit.
* Add concise developer documentation explaining how to define:

  * elemental compositions;
  * recipes;
  * mana costs;
  * new output handlers.

## Acceptance criteria

The feature is complete when:

* the player can open the crafting screen in normal gameplay;
* all five elemental balances and mana are visible;
* the player can construct a one-to-five-slot ordered formula;
* reversing slots can change the result;
* the correct mana cost is displayed;
* valid recipes consume the correct elements and mana;
* invalid recipes consume nothing;
* item and creature outputs are handled correctly;
* state persists after saving and loading;
* crafting is authoritative in multiplayer;
* tests pass;
* the project builds and runs without new errors.

After implementation:

1. run the relevant build, lint and test commands;
2. fix any failures caused by the change;
3. provide a concise summary of modified files and architecture;
4. list the starter recipes and elemental compositions added;
5. clearly identify any assumption forced by missing existing systems.
