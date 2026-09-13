# Lighting and surface weather

Creature meshes now obey the terrain draw radius and require a terrain mesh in
their chunk, on host and guests. See [wildlife](wildlife.md) for population limits.

Wind ribbons use short, crisp rectangular strokes with stepped heights and four
opacity bands. Their shape animates at eight frames per second while wind drift
stays continuous. The existing particle count, vertex budget and draw call are
unchanged; no textures or extra rendering passes are needed.

The forward renderer uses directional hemisphere fill, warm sunlight, ambient
occlusion, four bilinear shadow comparisons, analytic sky reflections, and a
blended filmic curve. Reflections approximate sky illumination; they do not
reflect nearby objects. These material improvements require no extra render
targets. Campfire effects add a bounded draw within the existing terrain pass.

Rain and storms wet exposed terrain over 12 seconds. Sunny, windy, and misty
weather dry it over 45 seconds. Cloud cover transitions smoothly as well.
Wet surfaces darken and gain a view-dependent reflective coat. Water retains its
own animated normals and reflectivity. Roof exposure is a conservative vertical
column test computed during meshing and packed into the existing reflectivity
attribute. Weather changes do not rebuild meshes. Sheltered terrain, cutout
foliage, and creature models do not receive this terrain coat.

River currents reuse the vertex wind scalar for a direction and animate
world-space ripples. Lake surfaces retain stationary waves. Waterfalls reuse
the rain line pipeline with a fixed particle budget; see [water](water.md).
Campfire flames and smoke use pixelated procedural cards, excluded from sun
shadows: 12x12 flame pixels, 8x8 smoke pixels, and eight animation frames per second.
Up to four nearby campfires add warm local light at night with an eight-block
range. These local lights do not use shadow maps; see [campfires](campfires.md).
World API queries describe authoritative blocks and derived features, not a
client's transient wetness, visible particle count, or audio state.

Held crystals contribute a steady cool six-block light, active during the day as
well as at night. Local and nearby remote players share the existing four-light
uniform array with campfires/machinery, with the local held crystal taking priority.
This adds no render targets, shadow passes or terrain remeshing. Local lights are
unoccluded and can leak through walls. Up to four nearby campkeepers reuse the
animated player model/hat meshes within the existing entity draw and shadow pass.

The opt-in `render_weather_previews` test accepts `VOXEL_ADVENTURE_PREVIEW=1` for a
keeper/camp scene and `VOXEL_CRYSTAL_PREVIEW=1` for a compact midday cave; use
`VOXEL_CRYSTAL_PREVIEW=unlit` for the same geometry without portable light. These
write `target/render-*.png`. UI previews accept `UI_PREVIEW_PANEL=journal`,
`adventure`, `recovery` or `map` and write images for both themes. See the
[sandbox review](SANDBOX_RPG_REVIEW.md) for measured fixture timings and their limits.

## Verification

Run `cargo test --no-default-features render_weather_previews -- --ignored --nocapture`
for offscreen PNGs in `target/render-*.png` and GPU timestamp measurements.
Optionally put the original terrain and sky WGSL sources in
`target/render-before.wgsl` and `target/sky-before.wgsl` for comparison.
The test restores original terrain shading attributes for the baseline.

During the original lighting change, an RTX 3060 (Vulkan) fixed scene at 1280x720 measured median GPU times
of 0.214 ms before, 0.171 ms dry, and 0.180 ms in rain/storms (40 measured frames
after 10 warmup frames). Timings include shadow, sky, and terrain passes, exclude
readback, and are not whole-game FPS measurements or a guarantee on other GPUs.
These historical measurements predate the added campfire/current effects; use
the preview test for the current revision and representative game profiling for
whole-frame performance. GPU previews require a working graphics adapter.
The savings come from fewer shadow samples, skipping shadows at night/on unlit
faces, and skipping cloud noise in clear weather and star noise during daytime.
