-- api_version: 1.37.0
-- Interpretation: Creatures close to this device recover health at night.
function on_tick(api)
    local target = api.get_rule_target()
    if not target or target.kind ~= 'device' or not api.is_night then return end
    for _, creature in ipairs(api.find_creatures('any', target.x, target.y, target.z, 4)) do
        api.heal_creature(creature.id, 0.1)
    end
end
