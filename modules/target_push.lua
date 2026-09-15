-- api_version: 1.36.0
-- spell_target: creature
-- Interpretation: Push the aimed creature four blocks away, stopping at walls.
function on_cast(api, event)
    assert(event.target and event.target.kind == "creature", "Aim at a creature")
    local c = assert(api.get_creature(event.target.id), "Target unavailable")
    local p = assert(api.get_player(event.player_id), "Caster unavailable")
    local dx, dz = c.x - p.x, c.z - p.z
    local length = math.sqrt(dx * dx + dz * dz)
    assert(length > 0.01, "Target too close")
    assert(api.push_creature(c.id, dx / length * 4, 0, dz / length * 4), "No clear space to push")
end
