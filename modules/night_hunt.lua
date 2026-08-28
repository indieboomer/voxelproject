-- api_version: 1.0.0
-- Manually written example rule (the LLM pipeline that will eventually
-- generate scripts like this doesn't exist yet -- this is what a rule
-- looks like once it's written).
--
-- Rule: at night, sheep hunt the nearest player carrying a crystal.
--
-- Uses only the World API needed for this one rule: api.is_night,
-- api.players() (with .carrying_crystal), api.creatures(), api.chase.

local CHASE_RADIUS = 18.0

function on_tick(api)
    if not api.is_night then
        return
    end

    local players = api.players()
    local creatures = api.creatures()

    for _, c in ipairs(creatures) do
        if c.kind == "sheep" then
            local nearest, nearest_dist
            for _, p in ipairs(players) do
                if p.carrying_crystal then
                    local dx, dy, dz = p.x - c.x, p.y - c.y, p.z - c.z
                    local dist = math.sqrt(dx * dx + dy * dy + dz * dz)
                    if nearest == nil or dist < nearest_dist then
                        nearest, nearest_dist = p, dist
                    end
                end
            end

            if nearest ~= nil and nearest_dist < CHASE_RADIUS then
                api.chase(c.id, nearest.x, nearest.y, nearest.z)
            end
        end
    end
end
