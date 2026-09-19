-- api_version: 1.0.0
-- Manually written example rule.
--
-- Rule: during rain, soil near trees becomes mud that slows players.
--
-- Uses api.weather, api.players(), api.find_blocks, api.get_block,
-- api.replace_block. Mud's actual slowing effect is a passive engine
-- property of the block (see player.rs) -- this rule's only job is
-- deciding where mud appears.

local SEARCH_RADIUS = 12
local MUD_RADIUS = 3

function on_tick(api)
    if api.weather ~= "rain" then
        return
    end

    local players = api.players()
    for _, p in ipairs(players) do
        local px, py, pz = math.floor(p.x), math.floor(p.y), math.floor(p.z)
        local trees = api.find_blocks("wood", px, py, pz, SEARCH_RADIUS)

        for _, tree in ipairs(trees) do
            -- Wood includes canopy branches. Only grounded trunks need a
            -- soil search; scanning every limb would waste the API budget.
            local ground = api.get_block(tree.x, tree.y - 1, tree.z)
            if ground == "soil" or ground == "grass" or ground == "mud" then
                for dx = -MUD_RADIUS, MUD_RADIUS do
                    for dz = -MUD_RADIUS, MUD_RADIUS do
                        if math.sqrt(dx * dx + dz * dz) <= MUD_RADIUS then
                            local bx, by, bz = tree.x + dx, tree.y - 1, tree.z + dz
                            local block = api.get_block(bx, by, bz)
                            if block == "soil" or block == "grass" then
                                api.replace_block(bx, by, bz, "mud")
                            end
                        end
                    end
                end
            end
        end
    end
end
