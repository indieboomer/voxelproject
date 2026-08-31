-- api_version: 1.1.0
-- Manually written example instant spell.
--
-- Spell: "add 100 stone" -- places 100 stone blocks around the caster's
-- feet, snapped to terrain height so they don't float or bury underground.
--
-- Demonstrates the on_cast contract: unlike on_tick, it defines no
-- persistent state and runs its whole effect in one call, using
-- event.player_id to find the caster in api.players() and a loop over
-- api.replace_block to do all the placing at once (on_cast gets a much
-- higher per-call block-edit budget than on_tick precisely so a loop like
-- this fits in a single call).

function on_cast(api, event)
    local caster = nil
    for _, p in ipairs(api.players()) do
        if p.id == event.player_id then
            caster = p
            break
        end
    end
    if caster == nil then
        return
    end

    local placed = 0
    local radius = 1
    while placed < 100 and radius < 20 do
        for dx = -radius, radius do
            for dz = -radius, radius do
                if placed >= 100 then
                    break
                end
                if math.abs(dx) == radius or math.abs(dz) == radius then
                    local x = math.floor(caster.x) + dx
                    local z = math.floor(caster.z) + dz
                    local y = api.terrain_height(x, z) + 1
                    if api.replace_block(x, y, z, "stone") then
                        placed = placed + 1
                    end
                end
            end
        end
        radius = radius + 1
    end
end
