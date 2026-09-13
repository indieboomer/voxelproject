-- api_version: 1.27.0
-- Optional example: paste into a reviewed rule. Not loaded automatically.
-- Completing the campkeeper contracts makes a held crystal ward off goblin melee.
function on_tick(api)
    for _, player in ipairs(api.players()) do
        local journal = api.get_player_journal(player.id)
        if player.health > 0 and journal and journal.complete
            and api.get_equipped_item(player.id) == "crystal" then
            api.protect_player(player.id, "goblin")
        end
    end
end
