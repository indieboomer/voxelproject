-- api_version: 1.39.0
-- Maintains 15 ordinary undead + 1 skeleton sorcerer at night, replacing losses.
-- Group labels are saved. Use a different stable label for each independent rule.
-- Disable/delete: despawn this group first if its creatures should disappear now.
local group = "night-horde-host-v1"
function on_tick(api)
    local ids = api.creatures_in_group(group)
    if not api.is_night then
        for _, id in ipairs(ids) do api.despawn(id) end
        return
    end
    if not api.get_player(0) then return end
    local ordinary, bosses = 0, 0
    for _, id in ipairs(ids) do
        local c = api.get_creature(id)
        if c and c.kind == "skeleton_sorcerer" then bosses = bosses + 1
        elseif c then ordinary = ordinary + 1 end
    end
    -- Four attempts per tick; nil means capacity or placement prevented spawning.
    for _ = 1, 4 do
        local kind
        if bosses < 1 then kind = "skeleton_sorcerer"
        elseif ordinary < 15 then kind = ordinary % 2 == 0 and "zombie" or "skeleton"
        else break end
        local id = api.spawn_creature_near_player(0, kind, 20)
        if not id then break end
        assert(api.set_creature_group(id, group))
        if kind == "skeleton_sorcerer" then bosses = bosses + 1
        else ordinary = ordinary + 1 end
    end
end
