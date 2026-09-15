-- api_version: 1.36.0
-- spell_target: block
-- Interpretation: Toggle the machine I am aiming at on or off.
function on_cast(api, event)
    local t = event.target
    assert(t and t.kind == "block", "Aim at a machine")
    local d = assert(api.get_device(t.x, t.y, t.z), "That block is not a machine")
    assert(api.set_device_enabled(d.x, d.y, d.z, not d.enabled), "Machine unavailable")
end
