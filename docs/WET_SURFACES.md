# Wet blocks and characters

Rain darkens porous blocks more than polished stone, glass, and metal. Wood and
soil retain rougher wet surfaces; smooth materials catch sharper highlights.
Broad stable variation breaks up uniform wetness without extra textures. Top
faces receive more water than vertical faces. Exterior walls become damp below
their top block; adjacent roof columns protect overhangs. Undersides and cutout
vegetation retain their existing shading.

Characters, NPCs, guides, creatures, and local held equipment accumulate cosmetic
moisture. Full exposure takes 12 seconds to soak in rain or 8 in storms. Under
shelter or in dry weather, a soaked character dries over 45 seconds. Five nearby
roof-column samples refresh at most five times per second per tracked actor;
partial exposure slows soaking. Terrain retains the global 12-second wetting
and 45-second drying transition.

Wet surfaces catch specular highlights from the existing four local lights,
including torches/lanterns, sharing their radius, flicker, and wall visibility.
Emissive flames and particles are excluded from actor moisture application.

## Models

Absorption derives from imported material roughness and metallic factors.
Materials can set `extras.wetAbsorption` from 0 to 1: low for polished nonporous
surfaces, high for cloth/fur. Missing parameters use legacy defaults. Existing
models using one atlas/material for skin, clothing, and equipment share a wet
response. Finer distinctions require authored masks/materials; the renderer does
not infer material type from skin color.

## Cost and limitations

Two floats increase vertex size from 72 to 80 bytes (about 11% more vertex
storage/upload traffic). Shader arithmetic and small CPU caches are added.
There are no new render passes, textures, particles, network fields, or save
fields. Weather transitions do not rebuild terrain; edits refresh exposure
through affected-chunk invalidation.

Moisture is client presentation state. Players/NPCs/guides use stable keys.
Creature visual snapshots lack IDs, so same-kind observations are matched to
nearest previous positions in spatial buckets, consuming each track once.
Overlapping identical creatures may exchange cosmetic moisture. Newly observed
creatures start dry; tracks expire after 600 unseen update frames. Unseen cached
actors do not simulate continuous offscreen moisture changes. Clients can differ
immediately after joining, and moisture resets on a new session.

Terrain moisture remains a global amount masked by current exposure, not water
simulation per block. Placing a roof can change its wet appearance at remeshing.
Puddles, runoff, ripples, splashes, and reflections of nearby objects are outside
this pass. Existing analytic sky reflections remain.

## Validation

Run `cargo test --offline --features dev-playtest -- --test-threads=1`.
Tests cover model absorption/overrides, roof transitions, independent moisture
across reordered creature snapshots, flame exclusion, and wall overhangs.

For character previews set `$env:VOXEL_WET_PREVIEW='1'`,
`$env:VOXEL_NPC_PREVIEW='1'`, and `$env:VOXEL_PREVIEW_1080='1'`, then run
`cargo test --offline render_weather_previews -- --ignored --nocapture`.
Moisture changes with each weather case; output is `target/render-*.png` plus
GPU median/p95. Use `VOXEL_TORCH_PREVIEW` instead of NPCs to inspect wet held
equipment under torchlight. These are fixture measurements, not whole-game FPS.

Measured on the RTX 3060/Vulkan at 1920x1080, the matching machinery fixture's
GPU median changed from 0.349 to 0.365 ms dry and 0.370 to 0.386 ms in rain.
Rain p95 changed from 0.374 to 0.387 ms. The character fixture measured
0.383 ms median / 0.385 ms p95 in rain. A stress fixture with every cave surface
soaked and four torches measured 0.686 ms median / 0.693 ms p95; enable
`VOXEL_WET_STRESS_PREVIEW` plus the four-torch and wet-preview flags to reproduce.
These GPU timings include scene rendering, but exclude CPU moisture updates,
simulation, UI, and readback. The preview's historical `before` shader is not
the immediate pre-change baseline used for the machinery comparison.

The separate `profile_wet_actors` test measured 128 tracked creatures at
0.127 ms CPU median / 0.161 ms p95, including periodic exposure refreshes but
excluding model animation/mesh generation. Exploration still produced the same
255928/263102/272908 triangles across the three 121-chunk fixtures, with meshing
times of 189/202/212 ms. The full serial test suite passed 531 tests; GPU previews
separately validated shader compilation and the actual pipelines.
