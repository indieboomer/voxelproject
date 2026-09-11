# Prompting with World API 1.22

The host generates and reviews sandboxed Lua modules. The model receives the
generated compact API reference, so schema updates must be regenerated before
rebuilding. All methods live directly under `api`; there is no `api.world` or
`api.player` namespace. See the [complete reference](../world_api/world_api_readme.md)
for signatures, supported callbacks, resource limits and existing creature control.

## New environment capabilities

| Capability | Interface |
| --- | --- |
| Inspect/find campfires | get_campfire(x,y,z), find_campfires(x,y,z,radius) |
| Place a supported fire | place_campfire_near_player(id,radius), place_campfire(x,y,z); remove with replace_block |
| Read water depth/current | get_water(x,y,z) |
| Read drops from a source | get_waterfalls(x,y,z) |
| Check/spawn fish | can_spawn_fish(x,y,z), spawn_fish(x,y,z) |
| Read weather/capabilities | is_raining, campfire_light_radius, fish_spawn_clearance |
| Inspect creature movement | snapshot.can_swim, can_fly, in_water |

Water and campfire queries return snapshots, not writable engine objects. Editing
their Lua table does not change the world. To create a waterfall, edit actual
source water, receiving water and obstructions within the normal edit limits.
Current direction/speed is a read-only visual classification; neither currents
nor waterfalls physically push players. Renderer flicker and moisture are not
authoritative gameplay properties. `can_fly` indicates capability, not flight mode.

## Example: create a campfire nearby

`on_cast` receives only `event.player_id`, not coordinates. Use the nearby placement
helper directly. `nearest_player(x,y,z)` requires three numeric coordinates; it
cannot look up a player by ID. For other caster queries, find the matching `id` in
`api.players()`. Generation review reports this mistake with repair instructions
and exercises placement on loaded ground in its isolated world.

"Create campfire nearby", "place a campfire near me", and "create camfpire nearby"
are one-time spells. The classifier recognizes create/build, and the focused
block context includes campfire and water helpers.

```lua
function on_cast(api, event)
    local fire = api.place_campfire_near_player(event.player_id, 6)
    if not fire then
        api.broadcast('No clear, dry spot nearby. Try a more open area.')
    end
end
```

The helper searches actual loaded blocks, avoids players, and can replace a
small ground plant. It does not require the player to own or craft a campfire.
It charges a bounded search allowance before scanning and consumes one shared
block edit only on success. Radius is clamped to 2..8; 6 is recommended.

## Example: campfires heal nearby players at night

Suggested prompt: "Once per second at night, heal players near a burning campfire
by one health. Do not stack healing from multiple fires."

```lua
local ticks = 0
function on_tick(api)
    ticks = ticks + 1
    if ticks < 10 then return end
    ticks = 0
    if not api.is_night then return end
    for _, player in ipairs(api.players()) do
        local fires = api.find_campfires(
            math.floor(player.x), math.floor(player.y), math.floor(player.z), 8)
        for _, fire in ipairs(fires) do
            if fire.light_active and api.distance(
                player.x, player.y, player.z, fire.x + 0.5, fire.y, fire.z + 0.5
            ) <= api.campfire_light_radius then
                api.heal_player(player.id, 1)
                break
            end
        end
    end
end
```

## Example: summon one fish while standing in suitable water

```lua
function on_cast(api, event)
    for _, player in ipairs(api.players()) do
        if player.id == event.player_id then
            local x, y, z = math.floor(player.x), math.floor(player.y), math.floor(player.z)
            if api.can_spawn_fish(x + 0.5, y + 0.5, z + 0.5) then
                local id = api.spawn_fish(x + 0.5, y + 0.5, z + 0.5)
                if id then api.broadcast('A fish joins the pool.') end
            else
                api.broadcast('Stand in a larger pool with enough underwater clearance.')
            end
            return
        end
    end
end
```

## Example: inspect nearby currents and waterfalls

```lua
function on_cast(api, event)
    for _, player in ipairs(api.players()) do
        if player.id == event.player_id then
            local blocks = api.find_blocks('water', math.floor(player.x),
                math.floor(player.y), math.floor(player.z), 3)
            for _, block in ipairs(blocks) do
                local water = api.get_water(block.x, block.y, block.z)
                if water then
                    local falls = api.get_waterfalls(block.x, water.surface_y - 1, block.z)
                    if #falls > 0 then
                        api.broadcast('Nearby waterfall drop: ' .. falls[1].height .. ' blocks.')
                        return
                    end
                end
            end
            if #blocks > 0 then
                local b = blocks[1]
                local water = api.get_water(b.x, b.y, b.z)
                api.broadcast(water.flowing and 'This water has a current.' or 'This water is still.')
            end
            return
        end
    end
end
```

## Transactions and search limits

New methods read earlier staged edits in the same callback. A failed callback
rolls back campfire placement, creature spawning and other staged effects.
They share existing block/spawn budgets; dedicated helper names do not grant
additional allowances. Direct fish spawning sees staged habitat; the existing
near-player fish search searches committed loaded terrain.

Nearby campfire placement reserves `(2*ceil(radius)+1)^2 * 72` native-work units
times one plus staged edits, plus snapshot cost. Other environment queries/actions
reserve 200 native-work units times one plus staged
edits. Campfire searches reserve twice the bounding cube times that factor and
clamp radius to 12. Native-work exhaustion cannot be bypassed by pcall. Use small
searches, throttle repeated rules, and handle nil/false/empty results. Do not scan
the world or nest maximum-radius searches for every creature on every tick.
All numeric inputs are checked for finiteness and coordinates for supported range.
Creature AI excludes players during their first 60 seconds in a session, even
when a rule assigns them as explicit attack targets. Those targets can become
active after the entry grace expires. Direct damage_player calls remain separate
rule actions and are not creature attacks.

Campfire spawning still requires suitable terrain; fish still need real water.
No helper grants filesystem/network access, unlimited spawns, arbitrary light
sources, forced fish jumping, writable currents, or a full fluid simulation.
