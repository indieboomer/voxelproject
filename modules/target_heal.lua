-- api_version: 1.34.0
-- spell_target: creature
-- Aim at a creature, then Run. Reuses the current target on every cast.
function on_cast(api, event)
    local target = event.target
    assert(target and target.kind == "creature", "Aim at a creature within 18 blocks")
    assert(api.heal_creature(target.id, 20), "The creature can no longer be healed")
end
