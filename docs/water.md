# Currents and waterfalls

See [fish](fish.md) for aquatic creatures and [prompting](prompting.md) for rules.

Mainland rivers carry directional ripples following their seeded channel tangent.
Lake interiors, upper feeder pools, and other unclassified water retain gentle
stationary waves without a current. The shader uses world-space ripples instead
of scrolling atlas UVs, so neighboring texture tiles cannot bleed into the water.
The existing wind vertex scalar stores water headings; vertex size is unchanged.

Mainland generation now includes sparse raised feeder pools between lakes, short
streams, and 5–8 block drops into receiving pools connected to the main river.
Placement and bank heights depend only on world coordinates, seed, and generation
settings. Islands, flat worlds, and mountain presets retain their own terrain.
Existing saved block edits are still applied after generation.

Waterfalls are visual effects rather than falling blocks or a fluid simulation.
When a chunk mesh changes, a bounded vertical scan finds exposed water beside an
open drop into lower water (2–16 blocks). This also supports suitable edited pools.
Solid obstructions, source removal, and unloaded neighbors suppress the effect.
Neighbor chunk arrival, departure, and border edits invalidate the mesh/cache.

Up to 32 nearby spill columns render falling streaks and impact spray, with 80
line segments each, reusing the rain pipeline and draw. Effects fade by 64 blocks.
Adjacent columns share sound sources, with at most two spatial loops using the
embedded `sounds/waterfall.mp3`; volume fades to zero by 48 blocks. No water flow
forces, damage, new collectible resources, or network effect packets are added.

## World API 1.20

`api.get_water(x,y,z)` returns nil for non-water, otherwise surface_y, depth,
flowing, flow_x, flow_z, flow_speed, and visual_only, plus the queried position.
Depth counts vertically contiguous water cells. Surface_y is the surface height,
one above the uppermost water block; query surface_y-1 when looking for a spill.
Still water has zero direction/speed; classified currents have a unit direction
and visual speed 1.4. Water properties describe the seeded visual classification,
not measured hydrodynamics, and there is no set_water_flow method.

`api.get_waterfalls(x,y,z)` returns up to four drops from that source surface
block: source position, bottom_y, height, flow_x/z, sound_radius, and visual_only.
It uses the same detection routine as rendering and sees staged edits. A source
must have open sky and an adjacent unobstructed drop into lower loaded water.
Alter water and obstructions using budgeted block edits to create/remove spills;
waterfalls themselves are not entities and cannot be spawned with spawn_creature.
Use small `find_blocks('water',...)` searches followed by targeted queries, rather
than repeatedly scanning large volumes inside nested loops.

## Checks

`cargo test --no-default-features -- --test-threads=1` checks generation, currents,
lake exclusions, blocked/missing/source-removed spills, particle bounds, and MP3
decoding. To render a generated waterfall without launching a window, set
`VOXEL_WATER_PREVIEW=1` and run
`cargo test --no-default-features render_weather_previews -- --ignored --nocapture`.
The preview writes `target/render-*.png`. Seed 42 has an example near x=-1006,
z=215. GPU timings in this fixture include the waterfall particles but exclude
audio, simulation, and readback.
