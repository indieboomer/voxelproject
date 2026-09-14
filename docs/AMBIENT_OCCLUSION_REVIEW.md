# Ambient occlusion review

AO darkens places where nearby geometry blocks indirect light, making corners,
crevices, and contact with the ground easier to read. It complements the recent
skylight and torch wall-visibility work; those features do not replace contact AO.
The AO refinement and subsequent strength adjustment have been reverted at the
player's request. Current terrain uses the original fixed face diagonals,
brightness curve and local diffuse-light weighting from before that pass.

## Current implementation

`src/voxel/mesher.rs` checks two side neighbors and one diagonal per face corner:
up to 12 block queries per visible opaque quad during mesh creation. It stores
one float per vertex and interpolates brightness levels 1.0, 0.8, 0.6, and 0.45.
AO uses 4 of the current 80 vertex
bytes. Once a chunk is meshed, there are no
AO-specific world queries or render passes each frame; the shader uses simple
multiplications. This is not a measured isolated AO timing: existing meshing
benchmarks include culling, geometry, skylight, and AO together.

Cutout foliage skips AO. Imported model vertices and procedural cuboids start
at AO=1, so they do not receive this voxel contact shading. Terrain quads use
the same fixed 0-2 diagonal in every face. The reverted refinement chose the
darker opposing corners instead; that change was reported to make blocks
appear detached. The original proposed algorithm is described here:
[Algorithm and triangulation](https://0fps.net/2013/07/03/ambient-occlusion-for-minecraft-like-worlds/).

`src/shader.wgsl` applies full AO to ambient fill and sky reflection,
but only modestly to direct sunlight. The combined local diffuse illumination
receives full AO, restoring its original behavior. Wet highlights retain their
separate material treatment. This revert does not change skylight, shadow-map
stability, atlas sampling, collision or camera clipping.

## Options and cost

| Option | Benefits | Costs and limitations |
| --- | --- | --- |
| Refine existing voxel AO (recommended first) | Better corner gradients; fewer diagonal artifacts; consistent dark corners in daylight and torchlight | Diagonal selection adds a few operations per quad at meshing time; tuning shader weights adds essentially no new rendering work. Does not add model-to-ground contact shading. |
| Bake model AO and cache static prop contact shading | Gives bags, machines, and other props more depth using the existing AO vertex field | Bake/preprocessing or CPU sampling cost; no extra screen-sized textures. Static shading must refresh on edits. Baked AO alone cannot represent nearby terrain or moving contacts. |
| Optional half-resolution SSAO/GTAO | Adds contact shading between models and terrain and within model details, including moving creatures | Adds depth sampling, AO evaluation, filtering and integration each frame. More GPU bandwidth, intermediate textures, and pipeline complexity; thin foliage, screen edges and motion can cause halos/noise. |

Screen-space AO only sees the depth representation available from the camera,
so hidden/offscreen geometry is incomplete. GTAO can improve the estimate but
still requires filtering. Intel's XeGTAO implementation uses depth preparation,
AO evaluation and spatial denoising, with TAA providing temporal stabilization
when available. This project's forward renderer has no equivalent temporal
history pipeline. [Intel XeGTAO](https://github.com/GameTechDev/XeGTAO).

For scale, a 1920x1080 R32F depth copy occupies 7.9 MiB; adding its mip chain is
about 10.5 MiB total. Two 960x540 R8 AO buffers add about 1 MiB. These are example
allocations, not a finalized design: extra normals/history/color attachments can
increase them, while directly sampling existing depth can avoid the copy.
Half resolution means one quarter as many AO pixels, but filtering, depth work
and integration prevent assuming one quarter of the total cost.

There is no measured SSAO/GTAO timing for this game. A sensible prototype
acceptance budget would be **at most 0.5 ms additional GPU time at 1080p on the
RTX 3060**, measured at p95 as well as median. That is a proposed ceiling, not an
estimate or promise. At a 16.67 ms frame budget, 0.5 ms consumes 3%; a GPU-bound
60 FPS frame would become about 58.3 FPS without savings elsewhere. Integrated
GPUs need a separate budget and an off setting.

Integrating AO only into indirect lighting is preferable to simply darkening
the final image. In this forward renderer that requires making depth/AO
available before material shading, or separating lighting contributions for a
later pass. Either choice adds more work than a shader-only adjustment. The
left-hand torch/equipment pass, sky, particles, water, and UI need deliberate
handling rather than indiscriminate full-image multiplication.

## Recommended order

1. Improve voxel diagonal selection and tune AO influence on local light. Keep
   current geometry counts, buffers and render passes.
2. Add model/static-prop contact AO where previews show a clear benefit; cache
   calculations and verify block edits invalidate them.
3. Only prototype optional screen-space AO if model-to-terrain contact still
   looks insufficient. Compare identical forest, workshop, bag-on-ground, cave
   combat and four-torch scenes, including camera motion and screen edges.
   Retain it only if the visual gain meets the measured frame-time budget.

Do not add a second strong AO layer over existing voxel AO without adjusting
their combination: otherwise corners become muddy and torch-lit details vanish.

## Historical validation of the reverted refinement

Seven mesher tests pass, including both triangle splits on all six face
orientations, face area/winding preservation, and actual neighboring-block AO.
The existing opt-in GPU preview validates the changed shader and pipelines.
No vertices, indices, textures, or render passes were added.

On the same three 121-chunk exploration fixtures, meshing measured
186.5/193.3/221.0 ms before and 187.1/195.5/209.9 ms after. Triangle counts stayed
255928/263102/272908. These single-run timings show no consistent large change.
The RTX 3060 1080p torch-cave preview measured dry median/p95 GPU time
0.449/0.452 ms before and 0.461/0.464 ms after; rain medians were 0.480/0.493 ms.
The unchanged historical shader also timed slower in the second run, so this
small difference cannot be cleanly attributed to AO. These are fixture timings,
not whole-game FPS measurements. Comparison images are
`target/ao-torch-before.png` and `target/ao-torch-after.png`.
