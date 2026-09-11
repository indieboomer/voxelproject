# Campfires

Campfires are append-only `campfire` blocks generated sparsely on level, dry
grass, soil, sand, or stone above the shoreline. Placement requires a flat 3x3
patch and four blocks of clearance. Eligible 48x48 regions try up to eight
candidate sites and place at most one fire. Small ground plants can be cleared;
trees, water, and uneven ground cause a candidate to be rejected.
Placement is deterministic from seed and chunk coordinates. Existing saved edits
are applied afterward, so removed fires stay removed after reload.
This is sparse placement, not a guarantee of a campfire in every region. Changed
generation takes effect when chunks regenerate; already loaded chunks are not
rewritten in place. Natural-generation tests survey seeds 7, 42, and 2026 and
write counts and nearby coordinates to `target/campfire-generation.txt`.
The default mainland survey found 69 fires for seed 7, 86 for seed 42, and 68 for
seed 2026 across 169 eligible chunks per seed. Nearby examples (center x, base y,
center z) are (24.5,25,-24.5), (22.5,23,21.5), and (23.5,22,69.5), respectively.
These coordinates depend on the current generator/settings and can be overridden
by saved block edits.

The base uses existing stone and log textures. Three animated flame cards and
five rising, pixelated smoke puffs provide the effect without new texture assets.
Flames use a 12x12 pixel grid and a small color palette; smoke uses an 8x8 grid
with a particle-local dissolve. Animation advances in eight discrete frames per
second. The former screen-space stippling has been removed.
At most eight nearby fires render effects. They use one additional draw in the
existing terrain pass and do not enter the sunlight shadow pass.

At night, up to four nearby fires illuminate surfaces within eight blocks with
warm flickering light and smooth distance falloff. This is an inexpensive local
light approximation without additional shadow maps; intervening walls do not
occlude the light. Daylight fades out the local illumination while fire continues
to animate. Smoke uses cutout pixels rather than a blending pass.

`sounds/campfire.mp3` provides looping positional crackling. At most two nearby
fires play simultaneously; attenuation/panning follows the listener, fading to
silence at 24 blocks. Removing a fire, its support, or its loaded chunk stops
its audio. These loops are independent of rain and waterfall ambience.

There is no fuel consumption, weather extinguishing, damage, crafting recipe,
or collectible inventory entry. Empty hands (one hit) or a pickaxe (two hits)
salvage 2 stone (`stone`), 2 oak wood (`oak_wood`), and 4 Fire elements. The stone
ring and logs return existing resources; the flame releases elemental Fire into
the crafting balance. The host grants all rewards together, once, to the breaking
player. A full balance rejects harvesting without partial rewards or removal.
Script removal grants no mining rewards. A missing support suppresses fire/light
effects. Placement and
edits use the existing authoritative block and save systems, while animation is
local and requires no effect replication packets.

## Prompting with World API 1.21

For "create campfire nearby", use
`api.place_campfire_near_player(event.player_id,6)` once in `on_cast`. The helper
searches loaded edited terrain within the requested horizontal radius (clamped
to 2..8) and four blocks above/below the player's feet. It keeps two horizontal
blocks away from every player, accepts suitable solid support and clear space,
and may replace a small ground plant. It returns a CampfireSnapshot or nil when
no site/player/budget is available. It uses one shared block edit and rolls back
with the callback. This avoids using procedural terrain_height on edited land.

`api.get_campfire(x,y,z)` returns burning, light_active, light_radius, and
requires_fuel, or nil for a different block. `api.find_campfires(x,y,z,radius)`
returns snapshots within a radius capped at 12. `api.place_campfire(x,y,z)` uses
one shared block edit and requires loaded supported ground and four clear air
cells. Rule placement checks one support cell; natural placement checks a 3x3
clearing. Remove a fire with `api.replace_block(x,y,z,'air')`.

Use `api.is_night` and a fire's `light_active` for rules such as shelter, healing,
or creature avoidance. Those behaviors are rule effects, not built-in campfire
benefits. `api.campfire_light_radius` is 8. Queries observe staged edits in the
same callback; failed callbacks roll back placements.

Run `cargo test --no-default-features campfire` for placement, persistence,
serialization, and effect-budget checks. Set `VOXEL_CAMPFIRE_PREVIEW=1` and run
`cargo test --no-default-features render_weather_previews -- --ignored --nocapture`
for an offscreen night preview in `target/render-dry.png`.
