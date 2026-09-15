-- api_version: 1.34.0
-- spell_target: creature
function on_cast(api, event)
    local target = event.target
    assert(target and target.kind == "creature", "Aim at a creature within 18 blocks")
    assert(api.get_creature(target.id), "The creature no longer exists")
    api.damage(target.id, 20)
end
