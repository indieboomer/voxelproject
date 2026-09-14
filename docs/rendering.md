# Lighting and surface weather

Terrain AO selects each quad's diagonal from opposing corner brightness sums,
connecting the darker pair to reduce triangular interpolation artifacts. Ties
keep the original diagonal. Vertex/index counts, UVs, AO levels, and render
passes are unchanged. Local lights apply full AO to soft fill and the same mild
AO weight as sunlight to their direct component, preserving torch-lit corner
detail alongside voxel wall visibility. See [AO review](AMBIENT_OCCLUSION_REVIEW.md).

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

Left-hand torches contribute warm flickering six-block light, active during the
day and at night. Crystals no longer cast portable light. Local and nearby remote
players share the four-light uniform array with campfires/machinery; the local torch takes priority.
This adds no render targets, shadow passes or terrain remeshing. A cached voxel
visibility texture now blocks local light through opaque walls. Up to four nearby campkeepers reuse the
animated player model/hat meshes within the existing entity draw and shadow pass.

The opt-in `render_weather_previews` test accepts `VOXEL_ADVENTURE_PREVIEW=1` for a
keeper/camp scene and `VOXEL_TORCH_PREVIEW=1` for a cave with a left-hand torch and
right-hand sword. `VOXEL_CRYSTAL_PREVIEW=1` now shows the same cave without portable light. These
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

## Rendering pass B

- Sun shadows snap to the existing 2048-square shadow map's texel grid to reduce
  movement shimmer. Cutout models and foliage use alpha-tested shadows; foliage
  deformation matches the main pass. No additional shadow maps or passes.
- The block atlas has seven independently generated mip levels, using
  alpha-weighted linear-light color averaging. Nearest texels preserve the pixel
  style; interpolation between mip levels reduces distant texture crawling.
  Tiles are never averaged together. Model textures keep their existing layout.
- Wetness darkening is gentler and its reflective coat respects material
  reflectivity, reducing the uniform glossy appearance of wet wood/soil.
- [Sheltered skylight](SHELTER_AND_LIGHT.md) fades through openings. A temporary
  eight-cell halo is discarded after propagation; resident cache storage is
  256 bytes per stored vertical layer per chunk, at most 32 KiB per chunk.
- Four bounded 18-cubed visibility volumes occupy about 23 KiB on the GPU.
  They refresh for movement in quarter-block increments or changed nearby
  terrain/device occupancy, using cached column lookups and voxel ray traversal.
  Unchanged lights reuse their volume. This is coarse wall occlusion, without
  local shadows from moving creatures or soft penumbrae.
- Roof heights are maintained on edits. Terrain meshing reads local chunk
  blocks directly when no devices require occupancy overrides. Inactive device
  geometry is cached; configuration, rotation, neighbor ports, and activity
  changes invalidate it. Animated mechanisms still rebuild, and the combined
  presentation mesh is still copied/uploaded each frame.

Resolution, draw distance, entity limits, and the four-light budget are unchanged.
Vertex size rises from 68 to 72 bytes; atlas mipmaps add roughly one third to the
block atlas texture storage. These tradeoffs avoid full-screen postprocessing.

For current checks, use `cargo test --offline --features dev-playtest -- --test-threads=1`, plus
the ignored `profile_exploration`, `profile_static_props`, and
`render_weather_previews` tests. In PowerShell, set `$env:VOXEL_PREVIEW_1080='1'`
before the GPU test for 1920x1080. Separate optional scene flags are
`VOXEL_FOREST_PREVIEW`, `VOXEL_MACHINE_PREVIEW`, `VOXEL_UNDERGROUND_PREVIEW`,
`VOXEL_ENTRANCE_PREVIEW`, and `VOXEL_TORCH_PREVIEW`. Add
`VOXEL_FOUR_TORCHES_PREVIEW` to measure four moving light-volume updates and render
four lights. Unset scene variables between runs. Outputs overwrite
`target/render-*.png`; preserve them under distinct names when comparing scenes.
Reported GPU median/p95 cover 40 frames after 10 warmups and exclude readback,
simulation, UI, and visibility rebuild CPU time (reported separately).

### Measured results (2026-09-14)

RTX 3060, Vulkan, driver 591.86, development profile with optimization level 1.
CPU exploration uses the same seed, positions, and 121 chunks as the baseline
captured immediately before B. Triangle counts are identical.

| CPU fixture | Before B | After B |
| --- | ---: | ---: |
| Mesh 121 chunks, three areas | 272–307 ms | 185–211 ms |
| Generate 121 chunks | 165–183 ms | 184–188 ms |
| Generate 11 crossing chunks | 14.6–15.3 ms | 15.6–16.6 ms |
| Build geometry for 64 idle workshops | 0.660 ms/frame (uncached) | 0.108 ms/frame (cached) |

Roof maintenance slightly increases generation cost; combined generation and
meshing improve. Rebuilding all four moving light volumes costs approximately
0.55 ms CPU median, 0.62–0.71 ms p95 in the cave fixture; steady lights reuse them.

| Current GPU fixture, 1920x1080 | Dry median | Rain median | Rain p95 |
| --- | ---: | ---: | ---: |
| Foliage rows | 0.304 ms | 0.325 ms | 0.326 ms |
| Machinery | 0.350 ms | 0.370 ms | 0.374 ms |
| Generated dungeon | 0.312 ms | 0.342 ms | 0.350 ms |
| Generated cave entrance | 0.542 ms | 0.554 ms | 0.557 ms |
| Cave with four torches | 0.595 ms | 0.627 ms | 0.631 ms |

The original small 1280x720 outdoor fixture changed from 0.174/0.183 ms
dry/rain immediately before B to 0.184/0.194 ms during this pass. The visual
improvements have a small GPU cost; they are not computationally free.
Historical `before` shader outputs in the preview logs predate B and are not
used as the baseline in these tables. These fixtures do not establish whole-game
FPS or four-client network performance. Full gameplay at the player's hardware
and settings remains the final frame-time check.

Validation: 521 tests passed, 25 opt-in tests ignored with the serial command.
One concurrent full-suite run hit an existing Lua wall-clock budget assertion;
the serial suite passed without changing sandbox limits.
