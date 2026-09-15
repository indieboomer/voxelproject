-- api_version: 1.37.0
-- Interpretation: Creatures near this specific block recover health during rain.
function on_tick(api)
    local target = api.get_rule_target()
    if not target or target.kind ~= 'block' or api.weather ~= 'rain' then return end
    for _, creature in ipairs(api.find_creatures('any', target.x, target.y, target.z, 4)) do
        api.heal_creature(creature.id, 0.1)
    end
end
