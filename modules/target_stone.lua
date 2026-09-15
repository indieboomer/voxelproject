-- api_version: 1.34.0
-- spell_target: block
function on_cast(api, event)
    local target = event.target
    assert(target and target.kind == "block", "Aim at a block within 18 blocks")
    assert(api.replace_block(target.x, target.y, target.z, "stone"), "The block cannot be replaced")
end
