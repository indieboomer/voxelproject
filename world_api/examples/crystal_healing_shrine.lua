-- api_version: 1.33.0
-- Prompt: Interacting with crystal heals nearby sheep, spending one life element.
function on_tick(api) end

function on_interact(api, event)
    if event.kind ~= "crystal" then return end
    local sheep = api.find_creatures("sheep", event.x, event.y, event.z, 6)
    local injured = false
    for _, c in ipairs(sheep) do
        if c.health < c.max_health then injured = true end
    end
    if not injured then return end
    if not api.take_element(event.player_id, "life", 1) then
        api.broadcast("The shrine needs one life element.")
        return
    end
    for _, c in ipairs(sheep) do
        if c.health < c.max_health then api.heal_creature(c.id, 4) end
    end
end
