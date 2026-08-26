-- Manually written example rule.
--
-- Rule: three creature deaths in one location spawn a red stone that heals
-- nearby creatures.
--
-- Uses the on_death event and api.replace_block. Demonstrates persistent
-- module state: a plain top-level Lua table survives between calls because
-- each module keeps its own long-lived Lua VM, so this remembers recent
-- deaths without any special engine support for it.

local CLUSTER_RADIUS = 6.0
local CLUSTER_WINDOW = 3

recent_deaths = recent_deaths or {}

local function dist(x1, y1, z1, x2, y2, z2)
    local dx, dy, dz = x1 - x2, y1 - y2, z1 - z2
    return math.sqrt(dx * dx + dy * dy + dz * dz)
end

-- This rule only reacts to deaths; on_tick must still exist (the sandbox
-- requires it) but has nothing to do every frame.
function on_tick(api)
end

function on_death(api, event)
    table.insert(recent_deaths, { x = event.x, y = event.y, z = event.z })
    if #recent_deaths > CLUSTER_WINDOW then
        table.remove(recent_deaths, 1)
    end
    if #recent_deaths < 3 then
        return
    end

    local a, b, c = recent_deaths[1], recent_deaths[2], recent_deaths[3]
    if dist(a.x, a.y, a.z, b.x, b.y, b.z) <= CLUSTER_RADIUS
        and dist(b.x, b.y, b.z, c.x, c.y, c.z) <= CLUSTER_RADIUS then
        api.replace_block(math.floor(c.x), math.floor(c.y), math.floor(c.z), "redstone")
        api.broadcast("A red stone rises where creatures fell...")
        recent_deaths = {}
    end
end
