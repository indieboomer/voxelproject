-- api_version: 1.37.0
-- Interpretation: This creature follows the rule creator while they hold a crystal.
-- Attach + enable this continuous rule while aiming at a creature.
function on_tick(api)
    local target = api.get_rule_target()
    if not target or target.kind ~= 'creature' then return end
    local creator = api.get_player(target.creator_id)
    if creator and api.get_equipped_item(creator.id) == 'crystal' then
        api.chase(target.id, creator.x, creator.y, creator.z)
    end
end
