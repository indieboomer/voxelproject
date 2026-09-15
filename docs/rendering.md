# Lighting and surface weather

## Visual voxel clouds

The sky shader renders an 8-block voxel grid at Y=160–184, above the build ceiling.
Coherent noise produces connected, irregular footprints with one to three layers
of thickness. Face shading distinguishes their undersides and vertical edges.
The whole field drifts continuously at 0.65 blocks/second along X and 0.22 along Z;
individual cells do not flicker or reroll as the camera moves. This is steady
visual wind, independent of simulation weather forces.

Clear weather has scattered clouds; rain and storms increase cover and darken it.
Clouds dim at night, obscure celestial lights and fade into distant horizon haze.
They are purely visual: no blocks, collision, saves, shadow-map geometry or network
entities. A bounded traversal of at most 48 cells per sky pixel keeps work limited.
The underwater sky path skips them.

Offscreen previews: `cargo test --offline --no-default-features render_weather_previews -- --ignored --nocapture`.
Set `VOXEL_LANDSCAPE_PREVIEW` to `geology` or `meadow` for generated-world scenes;
`VOXEL_CLOUD_TIME` selects an elapsed time in seconds for checking drift. Images
are written to `target/render-*.png`. Timings are fixed-scene GPU measurements,
not whole-game frame-rate guarantees.

## Exploration scheduling

Terrain generation caches height samples and streams missing chunks nearest-first.
The immediate collision neighborhood loads before physics. Terrain and shadow
passes cull cached meshes to their respective visible areas, while neighboring
chunk changes invalidate boundary meshes. The [original exploration report](archive/PERFORMANCE.md)
preserves earlier benchmarks; use the newer measurements and profiling commands
below when assessing the current renderer.

## Cave movement and contact shading

The AO refinement and later strength adjustment have been reverted. Terrain
again uses the original fixed 0-2 face diagonal, brightness levels
1.0/0.8/0.6/0.45, and full AO on combined local diffuse illumination.
Atlas UV corners are inset by half a base texel; with the existing nearest
filtering, all seven mip levels stay inside the intended tile. Previously an
exact tile boundary could select neighboring black or transparent texels,
and alpha rejection could expose geometry behind a solid face.

Player collision splits movement into steps no larger than 0.25 blocks per
axis. Endpoint-only checks could skip a one-block floor during a fast fall or
a wall with a speed boost and a long frame. Blocked axes stop while other axes
keep sliding. This adds collision queries for large steps, not render passes.
The near plane is now 0.05 blocks (previously 0.1), keeping its corners inside
the player's head clearance through 32:9 aspect ratios at the default FOV.
This trades some distant depth precision for preventing close ceiling cuts.
Regression tests cover tile edges at every mip, thin-wall/floor traversal,
and the near-plane clearance at wide aspect ratios. These address concrete
failure paths; the original intermittent gameplay report has no recorded replay.

The latest [wet-surface pass](WET_SURFACES.md) adds material-specific blocks,
character moisture that dries under shelter, and local-light specular
highlights. See that page for current behavior and costs; historical
measurements below describe their named revisions.

Sun-shadow edge stability: the light camera's orientation now comes only from
sun direction, before applying player translation. Previously `look_at` formed
its direction by subtracting two large world positions; float rounding changed
the orientation as the player moved, defeating texel-grid snapping. Projection
translation still snaps to whole texels, and sunlight continues moving normally.
Cutout casters use a separate linear sampler and footprint-selected atlas mips,
clamped inside each 64-pixel tile at both mip levels. Main color textures retain
their pixel filtering. A weighted 3x3 tent replaces sparse rotated shadow taps,
using the same four bilinear depth comparisons and 2048-square shadow map.

The ignored render preview supports `VOXEL_SHADOW_TEMPORAL=1` together with
`VOXEL_FOREST_PREVIEW=1`: it shifts the fixture to x/z=16384, fixes the viewing
camera/sun/wind, and moves only the shadow coverage center by tiny increments.
It reports pixels whose RGB changes by more than 16/255 between frames. The
reproduced dry-scene instability changed from median/p95 550/550 pixels to 0/0
on RTX 3060/Vulkan at 1280x720; GPU median changed from 0.211 to 0.218 ms.
This isolates spurious movement-induced flicker, not legitimate moving tree or
sun shadows. CPU regression tests also check position-independent orientation
and whole-texel grid translation at positive and negative distant coordinates.
For a historical A/B run only, `VOXEL_SHADOW_LEGACY=1` uses saved pre-fix shaders
at `target/shadow-edge-before.wgsl` and `target/shadow-caster-before.wgsl`, plus the
old camera calculation. This test-only switch is absent from the game build.

Terrain AO has returned to its pre-refinement implementation. See the
[AO review](AMBIENT_OCCLUSION_REVIEW.md) for current behavior and historical
measurements of the reverted pass.

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
attribute. Weather changes do not rebuild meshes. Sheltered terrain and cutout
foliage do not receive this terrain coat. Characters use accumulated actor
moisture instead of the terrain exposure flag.

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
  For partially exposed cells, inset face samples supplement the centre ray so
  mining a recess does not leave it black until the torch moves. Solid cells are
  skipped, and complete walls still block every sample. No GPU layout or shader
  sampling cost changes are required.
  Unchanged lights reuse their volume. This is coarse wall occlusion, without
  local shadows from moving creatures or soft penumbrae.
  After the mined-recess visibility fix, the four-moving-torch fixture measures
  about 1.4 ms CPU median for volume refreshes (up to 2.1 ms p95), compared with
  the earlier 0.55 ms figure below. Stationary, unchanged lights still reuse their
  volumes. The RTX 3060 preview passes at 1280x720 with about 0.34 ms dry and
  0.35 ms rainy GPU medians; these are fixture timings, not whole-game frame times.
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
