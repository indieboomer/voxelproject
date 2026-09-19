## Start here

The World API lets Lua modules change the shared game world. You can describe
an outcome in ordinary language and have the local model generate Lua, or
write a module yourself. A rule changes what happens over time; an instant
spell performs an action when run. You do not need to learn every method to
start: choose a concrete outcome, review the generated behavior, and test it.

1. Open Spell Workshop with the backquote/tilde key, describe your request,
   choose a spell type, and click **Generate Spell** or press **Ctrl+Enter**.
2. Read the interpretation and any generation/validation feedback. Unsupported
   requirements should be clarified rather than silently replaced with a smaller effect.
3. In Spell Workshop, use **View Code** to inspect the result. Generated modules
   are not automatically active.
4. The host clicks **Enable** for a continuing rule or **Run** for an instant
   spell. **Disable** stops future rule callbacks; **Delete** removes the module.
5. Test the trigger, the effect, and what happens when the condition ends.
   Save with **F5** when you want to retain the world and its modules.

For **Enchantment**, no object is selected during generation. Choose **Remember**
to store the reusable definition in Spellbook (`K`), assign it with **1–9** or a
numbered slot button, close the panel, aim at a valid object within **18 blocks**,
and **left-click**. There is no Enchantment **Run** action in either panel.
Remembering and assigning do not activate behavior. Each successful hotbar cast
creates its own persistent attachment, with source spell/version, owner and target;
later edits to the template leave earlier instances unchanged. Templates are free
to generate/remember; each attachment uses the existing 20-mana rule fee and
1.5-second spell cooldown. Invalid casts cost nothing. Guests need the spell card
and host casting permission. See [the Enchantment player guide](../docs/ENCHANTMENTS.md)
for target categories, save compatibility and multiplayer checks.

The host controls activation and the authoritative world. When the host enables
guest prompting, a guest with local generation available can submit a proposal.
The host still reviews and activates it. A guest-requested instant, run by the
host, uses the requesting guest as caster and requires that guest to be connected.
Receiving a proposal does not execute it.

Normally a successfully created rule costs 20 mana once, with no upkeep, and a
successful instant cast costs 5 mana. The engine handles these fees; generated
Lua must not charge them again. Failed casts refund the reserved cast fee.
Host mana-free testing changes the effective fees to zero. Read
`api.rule_mana_cost` and `api.instant_mana_cost` rather than assuming fixed costs.
Custom material or mana costs requested as part of a rule are separate.

## Write prompts that produce useful rules

Describe **who or what**, **the trigger**, **the action**, **the area or amount**,
and **how the effect ends**. Use the game's material and equipment names when
you know them. The reference below lists every supported block and creature ID.

| Request | What to specify |
| --- | --- |
| One-time construction | Shape, dimensions, material, location and which existing blocks may change |
| Conditional creature behavior | Species, target species/players, condition and behavior afterward |
| A shrine or switch | Trigger block, interaction, target, cost and cooldown if wanted |
| A reward | Event, recipient, resource/item and amount |
| Weather-sensitive terrain | Material, radius, whether roofs protect it, and whether the change is permanent |
| A production machine | Device, recipe, inputs, power source and output destination |

Examples:

- "Build a five-by-five stone platform under me, filling only empty space."
- "At night wolves hunt chickens; during the day they wander peacefully."
- "Rain turns exposed soil within two blocks of players into mud; roofs protect it."
- "Interacting with a crystal heals sheep within six blocks and spends one life element."
- "Give the player who mines iron ore one additional coal."
- "Set the time to dawn once."

Distinguish protecting players from pacifying a species: preventing sheep from
hurting players need not prevent sheep fighting other creatures. Distinguish
following from attacking, owning a tool from equipping it, and granting resources
from crafting them using owned ingredients. A persistent instruction should say
whether to restore the old behavior when its condition ends.

One cast cannot modify an unlimited area. Keep instant construction within
300 changed blocks, or explicitly request gradual construction in small batches.
Avoid prompts that require arbitrary new native materials, models, general
physics changes or undocumented callbacks; those capabilities are not exposed.

## Choose callbacks and understand events

Every module defines exactly one primary callback: `on_tick` or `on_cast`.
Either module type may also define the three optional event callbacks.

| Callback | When it runs | Event data |
| --- | --- | --- |
| `on_tick(api)` | About ten times per second while a rule is enabled | No event argument |
| `on_cast(api, event)` | Once per host Run action | `player_id` only |
| `on_death(api, event)` | Creature death | `kind, x, y, z`; no killer ID |
| `on_block_break(api, event)` | A player mines a block | `kind, x, y, z, player_id` |
| `on_interact(api, event)` | A player presses F while aiming at a block | `kind, x, y, z, player_id` |

For an event-only rule, define an empty `on_tick` and implement the relevant
event callback. There is no built-in `on_jump`, `on_rain`, `on_enter_region`,
general `on_damage`, or chat callback. Use current snapshots and remembered
previous values in `on_tick` to detect transitions where appropriate. That
remembered Lua state is temporary and resets when the module VM reloads.

Scripted block edits do not trigger player mining/interaction events, and
interaction alone does not destroy or consume anything. Death handlers cannot
recursively generate an unlimited chain of death callbacks in the same tick.
Events may wait in bounded queues and query current world state when dispatched.

## Essential Lua conventions

All methods live directly under `api`: use `api.get_player(id)`, not
`api.player.get(id)` or `api:get_player(id)`. Properties such as `api.is_night`
are values, not functions. Call functions with parentheses.

```lua
function on_cast(api, event)
    local p = api.get_player(event.player_id)
    if not p then return end
    api.broadcast("Caster position: " .. p.x .. ", " .. p.y .. ", " .. p.z)
end
```

Player IDs are not array indexes. Player and creature IDs are separate
identities, even when their numbers happen to match. Position fields are
`x, y, z`, not a nested `position` table. Block coordinates are integers;
use `math.floor` on player coordinates, particularly at negative positions.
Player positions refer to feet.

Query results are detached snapshots. Modifying a returned table does not
change the world. After an action, query again if you need the updated state.
Queries within the callback see accepted staged writes from that callback.

Check documented results: methods can return `nil`, `false`, an empty list,
or a number. Lua treats zero as true. For shape edits, zero is a successful
no-op and `nil` is rejection. Older methods have different failure conventions;
the complete reference describes each one. Unknown `find_creatures` kinds
retain the legacy behavior of matching any creature, while unknown block
filters match nothing. Do not use misspelled IDs to mean "any".

Use `ipairs` for returned lists and `pairs` for maps such as inventory resources.
The available standard libraries are `table`, `string`, and `math`, along with
the sandbox's basic Lua functions. Files, networking, processes, imports and
engine memory are not accessible. Keep Lua short and use the documented helpers.

## Capabilities by task

### Players, movement and progression

Use `players`, `get_player` and `nearest_player` for position, movement, health
and environmental context. `nearest_player` takes three coordinates, never an
ID. Guest movement flags are approximated from network state.

`damage_player`, `heal_player`, `set_poisoned`, `set_player_speed`,
`set_player_jump` and `teleport_player` apply player effects. Movement
multipliers are bounded. Teleport bypasses collision and clears velocity;
the caller must choose a suitable destination. These are explicit actions,
not declarations that automatically undo themselves when a condition ends.

`get_player_journal` reports built-in progress, recovery location, quests and
recipe-book discovery. It is read-only: editing its table does not claim a
quest or reward. Peaceful quest-giver NPCs are separate from combat creatures.

### Inventory, equipment, elements and mana

`get_player_inventory` returns four parts: `resources`, `items`, `elements`
and `mana`. Resource keys are block/resource IDs; equipment keys are the
available tool/charm IDs. Element keys are `earth`, `fire`, `water`, `life`,
and `death`. Read counts with `get_resource_count`, `get_item_count`,
`get_element_count` and `get_mana`; use `has_resource` or `has_item` for checks.
The older `get_inventory` is resource-only.

Use `give_item`, `take_item`, `give_element`, `take_element`, `give_mana` and
`take_mana` for explicitly requested rewards or costs. Grants bypass crafting
costs. For actual crafting, use `craft_item`, `decompose_item`,
`decompose_resource` and `convert_elements_to_mana`, which use the host economy.
Inspect `get_item_recipe` and `get_resource_elements` instead of inventing costs.
Specialist equipment may require a discovered recipe book.

Ownership and selection differ: `get_equipped_item` reports the selected
right-hand resource/tool. Torches use a separate left-hand slot and do not
replace that selection. Crystals are ordinary resources and do not light the
world when held. The legacy `carrying_crystal` flag is not a substitute for
an actual inventory or equipment query. Granting an item does not automatically
equip it.

### Creatures, combat and protection

Inspect creatures with `creatures`, `get_creature`, `find_creatures` and
`nearest_creature`. Spawn with `spawn_creature` or
`spawn_creature_near_player`; fish have dedicated habitat-aware helpers.
Check returned IDs because creature counts, spawn budgets and habitat rules
can prevent spawning. Dragons have additional placement constraints.

Target selection and action are separate:

1. Choose a target with `set_target` or `select_target`.
2. Use `chase_target` to follow without damage, or `attack` to pursue and fight.
3. Use `ignore` to cancel pursuit and aggression, or `set_aggressive` to
   restore automatic hostile/peaceful behavior.

`chase` steers toward coordinates and needs refreshing. `get_behavior` reports
intent, not proof a hit landed. Combat damage and cooldowns remain species-specific.
Do not emulate ordinary melee by calling `damage` every tick. Direct `damage`,
`destroy`/`die`, and `heal_creature` are available for explicit magical effects;
healing caps at species maximum and cannot resurrect dead creatures.

For temporary restrictions, `protect_player` prevents a specified creature
attacking one player and `suppress_creature_attacks` prevents that creature
attacking any target. These are rule-owned policies: refresh them only while
the condition holds. They expire without permanently changing behavior settings.
Ordinary behavior overrides persist until explicitly changed, including saves.
Initial player protection can suppress creature attacks even when a rule
assigns an explicit target; direct scripted player damage is a separate action.

### Blocks, materials and construction

`get_block` reads a cell; `get_block_kinds` lists the material catalog and
`get_block_info` reports properties such as solidity, hardness and emission.
Metadata is read-only. Use `block_matches` and `find_blocks` for exact IDs or
the categories `any`, `wood`, `ore`, `leaves`, `plant`, `solid`, and `liquid`.
`ore` includes coal; `liquid` currently means water. Search results are bounded
and are not necessarily sorted nearest-first.

`replace_block` edits one cell. `fill_box` and `fill_sphere` create, excavate,
or transform groups of cells. The new `kind` is the material to place;
optional `filter` selects existing material. For example `kind="air"`
excavates, while `filter="air"` builds only in empty space.

Boxes include both corners and normalize their order. Spheres use an integer
center and coordinate-distance radius. Shape edits require loaded cells,
protect bedrock and occupied device cells, and reject the entire call if it
cannot complete within the remaining edit allowance. They cannot place bedrock.
All block-edit methods share the callback allowance; helpers do not grant
extra changes. No-op shape cells consume no edits. These world-edit powers
do not automatically pay materials or grant mining drops.

Use `is_block_loaded` because `get_block` also returns `air` for unloaded
terrain. `terrain_height` is procedural elevation. `surface_height` scans
actual loaded terrain and includes roofs, trees, devices and staged edits.
Neither is a query for the cave floor immediately below a player.

### Weather, daylight, shelter, campfires and water

Read `weather`, `is_raining`, `is_night` and `time_of_day`. Current weather
names are `sunny`, `rain`, `mist`, `storm`, and `windy`; rain and storm produce
rainfall. Time runs from sunrise at 0 through noon at 0.25, sunset at 0.5,
and midnight at 0.75. `set_weather`, `start_rain`, `stop_rain`,
`set_time_of_day`, `set_time_dawn` and `set_time_night` change those values.
Do not repeatedly reset time or restart the weather timer unless requested.

`is_exposed_to_sky` checks rain-blocking cells at or above a loaded position;
query one cell above a surface. It returns `nil` when terrain is unknown,
not proof of shelter. Leaves block rain; water does not. This is an exposure
test, not a numeric light-intensity query.

Place a campfire using `place_campfire` or `place_campfire_near_player`.
The latter searches suitable nearby terrain. `get_campfire` and
`find_campfires` report burning/light state. `get_water` reports depth,
surface and visual current; `get_waterfalls` reports eligible spills.
`can_spawn_fish` and `spawn_fish` require real water and clearance. Editing
water or obstructions can change those environmental effects; there is no
general fluid simulation or direct writable current/light-source override.

### Machines, storage and production

`get_devices` and `get_device` inspect automation. `place_device` builds a
device using a connected player's resources; it checks reach and full-height
clearance. `configure_device` changes supported settings and
`set_device_enabled` starts/stops its operation. Device snapshots are copies.
Occupied device cells cannot be replaced as ordinary terrain; use player
packing controls to preserve their contents.

Device matter maps use typed keys such as `resource:stone`, `item:pickaxe`,
`element:earth`, and `creature:sheep`. Inspect current recipes/configuration
and check action results. Production runs on host simulation ticks, without
an LLM call each cycle. Do not imitate crafting or smelting with free grants.
Disabling a device also stops transfers; for stock limits prefer the existing
sensor/valve arrangement. Paid batches keep their recipe until completion.

Smelters process supported ores into their corresponding metals using mana
or fuel. Feeding chests can eject contents or retain them for transfers;
`eject_contents=false` selects retention. `configure_device` documents all
supported settings in the full reference below; undocumented table keys are
not a way to create new machine capabilities.

### Notifications and diagnostics

Use `broadcast` for a player-visible message. Keep messages short and emit
them on meaningful transitions rather than every tick. Broadcasts share a
per-callback count and byte limit and roll back if the callback fails.

Chat and notification history is exported when the host saves as
`saves/<worldname>_chat.log`, including lines no longer visible in the on-screen
log. The transcript survives reload and repeated saves do not duplicate it.
This is the game's save feature, not Lua filesystem access or a chat event API.

## Complete Lua examples

These three examples are embedded from the executable example modules used
by the sandbox regression checks. Each code block is a separate module.

### One-time platform construction

Request: "Build a five-by-five stone platform under me, only filling air."

{{STONE_PLATFORM}}

### Weather-sensitive ground

Request: "Rain turns exposed soil near players into mud, but roofs protect it."
This example makes permanent terrain changes and limits work to four accepted
changes per callback. It does not turn mud back into soil when the rain ends.

{{RAIN_SOFTENS_SOIL}}

### A resource-powered interaction shrine

Request: "Interacting with crystal heals nearby sheep, spending one life element."
The no-op `on_tick` makes this a rule; the interaction callback performs its action.
It only spends the element when an injured sheep is present.

{{CRYSTAL_HEALING_SHRINE}}

## Transactions, stopping rules and saving

Every callback stages effects and commits them only after successful execution
and budget checks. A callback error discards its staged terrain, inventory,
creature, weather/time, notification and player changes. It does not undo
earlier successful callbacks or the event that triggered it.

An ordinary action rejection (`false`/`nil`) does not automatically fail the
callback. If a structure or purchase involves several operations that must
succeed together, check every result and raise `error("Cannot complete action")`
on failure. A shape's rejection guarantees that shape staged nothing; earlier
successful calls in the callback still need explicit failure for rollback.
Budget exhaustion cannot be swallowed with `pcall` or `xpcall`.

Disabling a rule stops future work; it is not world history undo. Block edits,
inventory changes and explicit creature behavior overrides already committed
remain. For temporary rules, define the condition's exit behavior. Rule-owned
attack protection policies are the exception designed to expire automatically.

Saves retain module names, prompts, source and enabled flags, along with normal
saved world/player/creature state. Lua global variables, timers and remembered
tables do not survive world reload or rebuilding a failed module VM. Do not
promise permanent custom counters using plain Lua variables.

Current named worlds are stored in `saves/world-<name>.bin`; the default world
uses `saves/world.bin`. The preceding save is retained as `.bin.bak`.
Guest inventories are persisted by the host, but guest transient vitals reset
on reconnect. See the complete persistence and replication details below for
the practical limits of multiplayer state and save compatibility.

## Troubleshooting and working within limits

| Symptom | Check or change |
| --- | --- |
| Nothing happens after generation | Review the result, then host Enable or Run; proposals are not automatic activation |
| Player lookup is nil | Use `get_player(event.player_id)`; the requesting guest may have disconnected |
| Wrong creature species is affected | Check the exact kind ID; legacy unknown creature filters mean any |
| Shape returns nil | Loaded cells, dimensions, Y bounds, protected cells, material/filter spelling and remaining edit allowance |
| Shape returns zero | The operation succeeded but no matching cells needed changing |
| A roofed cell is mistaken for exposed | Compare exposure to `true`; `nil` means unknown, and surfaces must be queried at y+1 |
| Creature follows but does not fight | Select a target and call `attack`, not just `chase_target`; entry protection and cooldowns still apply |
| A conditional effect remains after Disable | Ordinary effects persist; implement exit behavior or an explicit cleanup action |
| Progress stored in a Lua table disappeared | VM state is not saved; use built-in saved systems where applicable |
| Insufficient materials/mana | Inspect inventory and recipe, then check the action result; do not double-charge engine fees |
| Rule exceeds execution budget | Reduce scan radii, avoid nested broad scans, update fewer actors/cells per tick and throttle repeated work |
| Unknown API member/material | Use the exact signatures/IDs below; there are no nested api.world/api.player namespaces |

Rules receive 32 block edits and four creature spawns per callback; instant
casts receive 300 edits and 30 spawns. Methods share 512 API calls and 65,536
native-work units per callback, alongside time, instruction and memory limits.
Shape scans consider at most 4096 cells; other query radii and result caps are
method-specific. A radius request larger than the documented cap is not a way
to query the whole world. Scans become more expensive after many staged edits.

Use small searches, avoid repeatedly scanning every creature's neighborhood,
and handle full/no-match results. The full reference that follows includes
every current method, property, callback, returned field, vocabulary and limit.
Runtime creation of native block types, models, textures, arbitrary saved
components and unrestricted engine access remains unsupported.
