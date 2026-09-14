-- api_version: 1.33.0
-- Prompt: Build a five-by-five stone platform under me, only filling air.
function on_cast(api, event)
    local p = api.get_player(event.player_id)
    if not p then return end
    local x, y, z = math.floor(p.x), math.floor(p.y) - 1, math.floor(p.z)
    local changed = api.fill_box(x - 2, y, z - 2, x + 2, y, z + 2, "stone", "air")
    if changed == nil then
        api.broadcast("Cannot build here: check loaded ground and the edit limit.")
    else
        api.broadcast("Platform: placed " .. changed .. " stone blocks.")
    end
end
