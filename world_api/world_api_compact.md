World API v1.0.0 (the `api` table passed into on_tick/on_death/on_block_break)

Queries:
- api.players(): Every connected player. Returns PlayerSnapshot[]. Empty array if nobody is connected.
- api.nearest_player(x, y, z): The single closest connected player to a point. Prefer this over looping players() yourself when you just need the closest one. Returns PlayerSnapshotWithDistance | nil. nil if no players are connected.
- api.creatures(): Every creature in the world. Returns CreatureSnapshot[]. Empty array if none exist.
- api.find_creatures(kind, cx, cy, cz, radius): Creatures of one kind within a radius of a point. kind: "sheep", "chicken", or "any". An unrecognized string is treated the same as "any" -- it is NOT an error, so a typo silently matches every creature instead of failing loudly. radius: Clamped into [0, find_radius_max]. Returns CreatureSnapshot[]. Up to find_results_max entries; excess matches are silently dropped.
- api.nearest_creature(kind, x, y, z): The single closest creature of one kind to a point. No radius argument -- it searches every creature in the world unconditionally. kind: "sheep", "chicken", or "any" -- same unrecognized-string-means-any behavior as find_creatures. Returns CreatureSnapshotWithDistance | nil. nil if no creature of that kind exists.
- api.get_block(x, y, z): The block at an integer position. Returns string. Snake_case block id -- see block_kinds. Positions outside the loaded/generated world, or above/below the vertical bounds, read as "air", never an error.
- api.find_blocks(kind, cx, cy, cz, radius): Positions of every block of one kind within a radius of a point. kind: A block id from block_kinds, OR the special category alias "wood" (matches any tree species: oak_wood, spruce_wood, birch_wood, cherry_wood). Unlike find_creatures' kind, an unrecognized string here returns an EMPTY result, not "match everything" -- this asymmetry is a real gotcha, not a design you can rely on being consistent across query methods. radius: Clamped into [0, find_radius_max]. Returns BlockPos[]. Up to find_results_max entries; excess matches are silently dropped.
- api.terrain_height(x, z): The integer y of the topmost solid ground block at a column. Useful for placing/spawning things at the correct height instead of guessing. Returns integer.
- api.distance(x1, y1, z1, x2, y2, z2): Straight-line distance between two points. Prefer this over writing your own sqrt(dx*dx+dy*dy+dz*dz). Returns number.
- api.time_of_day: number. 0 = sunrise, 0.25 = noon, 0.5 = sunset, 0.75 = midnight.
- api.is_night: boolean. True once the sun is below the horizon (matches the point the sky finishes fading to full night).
- api.weather: string. Current weather. One of: clear, rain.

Table shapes -- PlayerSnapshot, PlayerSnapshotWithDistance, CreatureSnapshot, CreatureSnapshotWithDistance, BlockPos, DeathEvent, BlockBreakEvent (fields with a caveat noted inline):
- PlayerSnapshot = {id [Stable per-connection player id; 0 is always the host.], x, y, z, carrying_crystal [True once this player has ever broken a Crystal block this session. One-shot: never resets to false from this API (there is no 'drop crystal' action).], speed [Horizontal speed in blocks/sec -- excludes vertical (fall/jump) motion.], vertical_speed [Signed vertical speed: positive while rising (e.g. jumping), negative while falling.], on_ground [True when standing on solid ground. Never true the same tick vertical_speed is positive -- a jump sets both at once, so 'on_ground and vertical_speed > 0' can never be true. There is no separate on_jump/on_land event; detect a one-shot transition by remembering the previous tick's on_ground per player id.], running [True while sprinting, as opposed to walking.], in_water [True while standing in a water block.]}
- PlayerSnapshotWithDistance = PlayerSnapshot + {distance}
- CreatureSnapshot = {id [Stable creature id, valid until it despawns (destroy/damage-to-zero).], kind, x, y, z, health, max_health [Fixed per kind (sheep 12, chicken 6); never changes at runtime.]}
- CreatureSnapshotWithDistance = CreatureSnapshot + {distance}
- BlockPos = {x, y, z}
- DeathEvent = {kind, x [Last position before death.], y, z}
- BlockBreakEvent = {kind [Snake_case id of the block that was broken -- see block_kinds. Same vocabulary as get_block's return value.], x, y, z, player_id [id of the player who broke it (matches PlayerSnapshot.id).]}

Actions (budget-limited per call: 32 block edits, 4 creature spawns; failures are silent -- check return values, not errors):
- api.chase(creature_id, x, y, z): Makes a creature move toward a point at increased speed. Call it again every tick to keep it chasing -- stop calling it and the creature reverts to normal wandering within about a third of a second (there is no separate "stop chasing" call). Returns nil. Always nil, even if creature_id doesn't exist -- there is no way to tell from Lua whether the call actually found the creature.
- api.damage(creature_id, amount): Reduces a creature's health. Fires on_death (to every module, not just the caller) if this brings health to zero or below, and despawns it. Returns nil. Always nil, whether or not creature_id exists.
- api.destroy(creature_id): Removes a creature immediately regardless of remaining health, and fires on_death. Returns nil. Always nil, whether or not creature_id exists.
- api.spawn_creature(kind, x, y, z): Spawns a new creature at a position. kind: "sheep" or "chicken"; anything else (including a typo) silently becomes "sheep" -- not an error. Returns integer | nil. The new creature's id, or nil if this call's spawn budget is already used up.
- api.spawn_creature_near_player(player_id, kind, radius): Spawns a new creature at a random point within radius blocks of a player (horizontally; height snaps to terrain). Prefer this over spawn_creature plus your own random offset when the rule is about spawning near a specific player. kind: Same silent-default-to-sheep behavior as spawn_creature. radius: Clamped into [1, find_radius_max] -- note the floor is 1, not 0, unlike the find_* methods. Returns integer | nil. The new creature's id, or nil if player_id isn't connected OR the spawn budget is used up -- these two failure cases are indistinguishable from Lua.
- api.replace_block(x, y, z, kind): Changes one block. kind: A block id from block_kinds (e.g. "air" to clear a block). Returns boolean. false if kind isn't a recognized block id, OR this call's block-edit budget is used up -- these two failure cases are indistinguishable from Lua. true otherwise.
- api.set_weather(name): Sets the weather directly. Returns nil. An unrecognized name is silently ignored -- not an error.
- api.start_rain(): Shorthand for set_weather("rain"). Returns nil.
- api.stop_rain(): Shorthand for set_weather("clear"). Returns nil.
- api.set_time_of_day(t): Sets the time of day directly. t: Wrapped into [0,1) (e.g. -0.25 and 1.25 both become 0.75), same scale as api.time_of_day. Returns nil.
- api.set_time_dawn(): Shorthand for set_time_of_day(0.0). Returns nil.
- api.set_time_night(): Shorthand for set_time_of_day(0.5) -- sunset, night begins. Returns nil.
- api.broadcast(message): Shows a message as a notification to every connected player, and logs it server-side. Use sparingly -- there is no per-call rate limit, only the shared time/instruction budget. Returns nil.

Creature kinds: sheep, chicken.
Block kinds (27): basalt, bedrock, birch_leaves, birch_wood, bricks, cherry_leaves, cherry_wood, cobblestone, copper_ore, diamond_ore, emerald_ore, gold_ore, grass, oak_leaves, oak_wood, pumpkin, sand, short_grass, soil, spruce_leaves, spruce_wood, stone, water, air, mud, redstone, crystal.

function on_tick(api) [required]: The only required callback -- Module::load rejects a source file that doesn't define this.
function on_death(api, event) [optional]: Fires once per creature death, from any cause (this module's or another enabled module's api.damage/api.destroy). Does NOT chain: if an on_death handler itself kills another creature, that second death does not fire on_death again within the same tick.
function on_block_break(api, event) [optional]: Fires once per real player-caused block break (mining). A rule's own replace_block calls, even ones that set a block to air, never trigger this.

Every World API call executes ONLY on the host -- Lua does not run on joined clients at all (ScriptHost is host-only state). A module's effects (block edits, creature spawns/state, weather, time of day, broadcasts) reach clients purely through the replication channels below; there is no separate "multiplayer mode" a rule opts into. Only the host can generate or activate rules (a menu/UI restriction, not something this API itself enforces or exposes).
