# Lighting and surface weather

The forward renderer uses directional hemisphere fill, warm sunlight, ambient
occlusion, four bilinear shadow comparisons, analytic sky reflections, and a
blended filmic curve. Reflections approximate sky illumination; they do not
reflect nearby objects. There are no additional render passes or render targets.

Rain and storms wet exposed terrain over 12 seconds. Sunny, windy, and misty
weather dry it over 45 seconds. Cloud cover transitions smoothly as well.
Wet surfaces darken and gain a view-dependent reflective coat. Water retains its
own animated normals and reflectivity. Roof exposure is a conservative vertical
column test computed during meshing and packed into the existing reflectivity
attribute. Weather changes do not rebuild meshes. Sheltered terrain, cutout
foliage, and creature models do not receive this terrain coat.

## Verification

Run `cargo test --no-default-features render_weather_previews -- --ignored --nocapture`
for offscreen PNGs in `target/render-*.png` and GPU timestamp measurements.
Optionally put the original terrain and sky WGSL sources in
`target/render-before.wgsl` and `target/sky-before.wgsl` for comparison.
The test restores original terrain shading attributes for the baseline.

On an RTX 3060 (Vulkan), the fixed scene at 1280x720 measured median GPU times
of 0.214 ms before, 0.171 ms dry, and 0.180 ms in rain/storms (40 measured frames
after 10 warmup frames). Timings include shadow, sky, and terrain passes, exclude
readback, and are not whole-game FPS measurements or a guarantee on other GPUs.
The savings come from fewer shadow samples, skipping shadows at night/on unlit
faces, and skipping cloud noise in clear weather and star noise during daytime.
