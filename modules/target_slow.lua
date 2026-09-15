-- api_version: 1.36.0
-- spell_target: creature
-- Interpretation: Slow the aimed creature for ten seconds.
function on_cast(api, event)
    assert(event.target and event.target.kind == "creature", "Aim at a creature")
    assert(api.apply_creature_status(event.target.id, "slow", 10), "Target unavailable")
end
