-- api_version: 1.36.0
-- spell_target: creature
-- Interpretation: Summon a wolf beside the aimed creature, on nearby clear ground.
function on_cast(api, event)
    assert(event.target and event.target.kind == "creature", "Aim at a creature")
    local c = assert(api.get_creature(event.target.id), "Target unavailable")
    for _, offset in ipairs({{3,0},{-3,0},{0,3},{0,-3}}) do
        local x, z = math.floor(c.x + offset[1]), math.floor(c.z + offset[2])
        local floor = api.surface_height(x,z)
        if floor and math.abs(floor + 1 - c.y) <= 3 then
            local y, clear = floor + 1, true
            for dx=-1,1 do for dz=-1,1 do for dy=0,1 do
                if not api.is_block_loaded(x+dx,y+dy,z+dz)
                    or api.get_block(x+dx,y+dy,z+dz) ~= 'air' then clear=false end
            end end end
            if clear then
                assert(api.spawn_creature('wolf',x+0.5,y,z+0.5), "Spawn budget reached")
                return
            end
        end
    end
    error("No clear ground beside that creature")
end
