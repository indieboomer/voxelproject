-- Manually written example rule.
--
-- Rule: breaking a Crystal block while sprinting summons a storm -- start
-- rain and spawn a sheep near the player who broke it.
--
-- Demonstrates the on_block_break event, the player-state fields on
-- api.players() (specifically `running`), api.start_rain, and
-- api.spawn_creature_near_player.

function on_tick(api)
end

function on_block_break(api, event)
    if event.kind ~= "crystal" then
        return
    end

    local running = false
    for _, p in ipairs(api.players()) do
        if p.id == event.player_id and p.running then
            running = true
            break
        end
    end
    if not running then
        return
    end

    api.start_rain()
    api.spawn_creature_near_player(event.player_id, "sheep", 4)
    api.broadcast("A storm gathers as the crystal shatters...")
end
