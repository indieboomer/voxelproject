-- api_version: 1.36.0
-- spell_target: creature
-- Interpretation: Stun the aimed creature for three seconds.
function on_cast(api, event)
    assert(event.target and event.target.kind == "creature", "Aim at a creature")
    assert(api.apply_creature_status(event.target.id, "stun", 3), "Target unavailable")
end
