-- api_version: 1.33.0
-- Prompt: Rain turns exposed soil near players into mud, but roofs protect it.
function on_tick(api)
    if not api.is_raining then return end
    local changed = 0
    for _, p in ipairs(api.players()) do
        local cells = api.find_blocks("soil", math.floor(p.x), math.floor(p.y), math.floor(p.z), 2)
        for _, b in ipairs(cells) do
            if changed >= 4 then return end
            if api.is_exposed_to_sky(b.x, b.y + 1, b.z) == true then
                if api.replace_block(b.x, b.y, b.z, "mud") then changed = changed + 1 end
            end
        end
    end
end
