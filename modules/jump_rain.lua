-- Manually written example rule.
--
-- Rule: when a player jumps, it starts raining.
--
-- Demonstrates detecting a one-shot event (like a jump) purely from
-- on_tick, since the engine has no separate "on jump" callback. The naive
-- check `p.on_ground and p.vertical_speed > 0` can NEVER be true: a
-- player's on_ground flips to false in the same instant vertical_speed
-- becomes positive from jumping, so the two conditions never overlap.
-- Instead, remember each player's on_ground state from the previous tick
-- (the same persistent-module-state trick redstone_healing.lua uses) and
-- watch for it flipping from true to false while rising -- that's the
-- moment the jump started.

was_on_ground = was_on_ground or {}

function on_tick(api)
    for _, p in ipairs(api.players()) do
        local previously_on_ground = was_on_ground[p.id]
        if previously_on_ground and not p.on_ground and p.vertical_speed > 0 then
            api.start_rain()
        end
        was_on_ground[p.id] = p.on_ground
    end
end
