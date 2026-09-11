# Inventory scripting (World API 1.22)

All APIs target a connected player ID, including guests. Queries include changes
staged earlier in the callback and committed by earlier callbacks. Returned
tables are detached; editing them does not grant resources. Errors and script
budget failures roll back every staged inventory, mana and world action.

| Capability | Calls |
| --- | --- |
| Complete inventory | `get_player_inventory(id)` → `{resources, items, elements, mana}` |
| Resource counts | `get_inventory(id)` (legacy resource map), `get_resource_count`, `has_resource` |
| Item counts | `get_item_count`, `has_item` accept resource IDs and `axe`, `pickaxe`, `sword` |
| Currency queries | `get_mana(id)`, `get_element_count(id, element)` |
| Explicit rewards/costs | `give_item`, `take_item`, `give_mana`, `take_mana`, `give_element`, `take_element` |
| Equipment creation/salvage | `craft_item(id, kind)`, `decompose_item(id, kind)` |
| Resource extraction | `decompose_resource(id, resource, amount)` |
| Mana conversion | `convert_elements_to_mana(id, element, amount)` |
| Formulas | `get_item_recipe(kind)`, `get_resource_elements(resource)` |

Elements use keys `earth`, `fire`, `water`, `life`, `death`; element arguments
also accept capitalized names. Items use lowercase equipment IDs. Resources use
the existing catalog IDs, for example `iron`, `oak_wood`, `stone`. The retired bow
and world campfire are not inventory items. Mana also appears in `players()` and
`nearest_player()` snapshots.

Queries return nil for missing players or unknown IDs. Actions return false on
invalid input, insufficient materials/mana, or overflow, without partial changes.
New amount arguments must be positive and no greater than `item_grant_max`.
Equipment crafting/salvage handles one item per call; optional amount must be 1.
Resource extraction and element conversion default to one unit. Existing resource
grants retain their amount-clamping and balance-saturation behavior for compatibility.

Creation/decomposition uses the host's current crafting registry and the same
equipment formulas as inventory. Resource decomposition costs 1 mana per unit;
gear recipe tables contain `create` and `decompose`, each with `resources` and
`mana`. The former consumes materials; the latter returns them. Element conversion
uses the host registry rate. Explicit grants bypass crafting, so use them only
when a rule intentionally awards items or currency.

The engine reserves `api.instant_mana_cost` (5) before `on_cast`, and refunds it
on failure. Lua sees the remaining spendable mana. Do not charge this fee again.
`api.rule_mana_cost` is 20, charged once on generated rule creation. A custom
`take_mana` cost is additional. `api.mana_regen_cap` is 100; connected players
recover one mana every five seconds below that cap. Rule rewards may exceed it.

Example instant: craft a pickaxe using the caster's materials and recipe mana,
in addition to the engine's cast fee:

```lua
function on_cast(api, event)
    if not api.craft_item(event.player_id, "pickaxe") then
        error("Crafting needs 3 iron, 2 oak wood and 6 available mana")
    end
end
```

Example instant: decompose two stones, inspecting its transactional result:

```lua
function on_cast(api, event)
    if not api.decompose_resource(event.player_id, "stone", 2) then
        error("Need two stones and two available mana")
    end
    local inventory = api.get_player_inventory(event.player_id)
    api.broadcast("Earth elements: " .. inventory.elements.earth)
end
```

Returning normally after a failed boolean action is still a successful cast and
spends its engine fee. Use `error(...)` to abort and refund a failed spell.
