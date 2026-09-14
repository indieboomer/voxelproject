# Creature behavior (World API 1.14)

For all current capabilities and the full reference, see the consolidated
[World API user guide](USER_GUIDE.md). The examples below describe the original
behavior-control API; the user guide also covers newer temporary protection policies.

Behavior runs on the host. Movement, attack animation, player damage, and creature removal use existing multiplayer replication. Clients do not run a second AI simulation.

| Call | Effect |
|---|---|
| `get_behavior(id)` | Read aggression, mode, and explicit target; nil for missing creature. |
| `select_target(id, "chicken", 32)` | Select nearest chicken within 32 blocks; excludes self, returns target ID or nil. Use `"player"` to select a player. |
| `set_target(id, "creature", other_id)` | Select an exact creature; use `"player"` for player IDs. |
| `chase_target(id)` | Follow selected target without attacking. |
| `attack(id)` | Follow and attack selected target in melee range on cooldown. |
| `ignore(id)` | Clear target and suppress aggression; wander peacefully. |
| `set_aggressive(id, true)` | Clear selection and restore automatic attacks against nearby players, even for normally passive species. False restores peaceful wandering. |
| `die(id)` | Immediate death, sound and on_death event; false if already gone. |
| `chase(id, x, y, z)` | Existing temporary coordinate chase; refresh each tick. Suppresses automatic attacks while active. |

Selecting a target alone does not start combat. `attack` and `chase_target` return false without a selection. A missing selected target is cleared on the next simulation update; select again to find another. Creature and player IDs are separate namespaces.

Attack damage/range/cooldown stay species-specific. Normally passive sheep, chickens and cows receive a basic melee attack: 2 damage, 1.5 block reach and 2 second cooldown. Damage is applied by simulation, not repeatedly by a Lua rule. Behavior mode describes intent; `attack` mode does not mean a hit has landed.

Explicit overrides persist until changed and survive saves. Disabling a rule does not undo its previous changes. Conditional rules should reset behavior in their else branch.

## Wolves hunt chickens at night

```lua
function on_tick(api)
    for _, c in ipairs(api.creatures()) do
        if c.kind == "wolf" then
            if api.is_night and api.select_target(c.id, "chicken", 32) then
                api.attack(c.id)
            else
                api.ignore(c.id)
            end
        end
    end
end
```

## Make nearby wolves peaceful once

```lua
function on_cast(api, event)
    for _, p in ipairs(api.players()) do
        if p.id == event.player_id then
            for _, c in ipairs(api.find_creatures("wolf", p.x, p.y, p.z, 32)) do
                api.ignore(c.id)
            end
        end
    end
end
```

Example prompts: "At night wolves hunt and attack chickens; during the day they ignore all targets", "Make wolves peaceful", "Make sheep chase chickens without attacking". Regenerate older modules to use the new API; existing chase-only code is not rewritten automatically.
