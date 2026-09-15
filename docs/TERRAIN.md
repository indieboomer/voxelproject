# Rivers, lakes, mountains and underwater rendering

World generation now includes narrow winding rivers joining large lake basins and occasional rocky mountains. Terrain remains a deterministic function of world seed and position, independent of chunk loading order.

New worlds retain the original land relief and add irregular lake outlines with
bays and headlands. See [world generation](WORLD_GENERATION.md#lake-shapes-and-plant-colonies)
for lake shapes, plant colonies and compatibility. The baseline survey below
describes the version-0 landscape with its earlier rounded lakes.

## Generation

`src/voxel/terrain.rs` layers the features over the existing rolling terrain. Rivers meander along several sine/noise scales, with lake centers placed directly on their paths. Channels are generally around 5-8 blocks wide, widening where they meet lakes or existing lowlands. Lake radii are roughly 24-52 blocks, with noise-varied shorelines and up to six blocks of water. Rivers and lakes share the existing sea level (Y=18), guaranteeing connected water without a flow simulation. Their paths cross chunk boundaries using global coordinates. The MVP uses roughly spaced river corridors; it does not model drainage basins or fluid flow. Raised tributaries, visual currents and waterfalls are described in [water](water.md).

Mountain candidates are sparse, seed-selected patches with variable elliptical footprints and roughened peaks. Peaks fit inside the existing 48-block world height, reaching roughly Y=41-44. From Y=32 upward, coherent snow patches extend onto upper slopes. Snow becomes more likely with elevation, while steep faces retain more exposed rock. Above Y=34 the remaining ground is predominantly rock, with rare grassy ledges. Snow is one block deep and always has stone directly beneath it; trees stop below the alpine zone. Lower slopes retain the existing biome and resources. Rivers and lakes carve valleys through terrain after mountain elevation is applied.

The seed-42 survey over a 1024x1024-block region, sampled every two blocks, recorded 2,201 rocky samples out of 262,144 (0.84%), 20,868 water samples (7.96%), and a highest sampled point of Y=43. These are sample statistics, not quotas for every seed. The generated map is `target/terrain-map.png` (blue water, green lowland, gray rocky tops). Detailed statistics are in `target/terrain-survey.txt`.

## Underwater view

The camera's eye position is checked against actual water blocks each frame. Being in water up to the player's feet does not enable the effect. When the eyes submerge, world geometry and creatures receive a blue tint and blue fog over 1.5-26 blocks; the distant sky becomes the same underwater blue. Distant birds and rain streaks are hidden underwater. The HUD and other egui windows remain readable and untinted. Leaving the water immediately restores normal rendering. This is local presentation, independent of the existing drowning/oxygen mechanics.

The effect uses the reserved `weather_fx.z` uniform component, preserving uniform layouts and existing lighting/weather fields.

## Compatibility and checks

Existing saved block edits and inventories are preserved. Unedited terrain regenerates with the new geography when a save loads; player edits can still obstruct a generated river. Use New World for a clean example of the new terrain. Protocol checks prevent mismatched builds from joining: all Direct/Steam players need the same build.

Tests verify continuous wet paths between lake centers, orthogonally connected channel steps, positive/negative chunk boundaries and reverse load order, lake width, sparse high peaks, terrain height bounds, natural resource availability and buried ore placement. Eye-position tests distinguish submersion from wading, and an opt-in GPU check validates both modified WGSL shaders.

```powershell
cargo test --offline
cargo test --offline --features steam
cargo test --offline --features steam validate_underwater_shaders -- --ignored
.\tools\build_steam.ps1
```

The seed-42 surface survey samples 508 snow, 1,634 exposed-rock and 59 grass columns above Y=34 (about 23%, 74%, and 3%), plus 834 snow patches sampled on lower Y=32-33 slopes. These are seed-specific observations.

## Visible wind

Airborne motes and curved white wind ribbons use translucent, soft-edged triangles with terrain depth testing and no depth writes. They fade over their lifetimes and near the camera/draw-volume boundary. Wind strength comes from the same weather values used by vegetation: mist 0.5, sunny/rain 1, windy 2, storm 2.5. A smoothed response avoids snapping when weather changes; periodic gusts vary speed, while stronger weather increases ribbon length, visibility and particle density. Rain and storm hide both wind motes and ribbons; mist shows motes only. Sunny and windy weather show both. The underlying particle state continues updating so effects resume automatically after weather changes.

The effect is local presentation of the replicated weather, so no new network protocol or simulation changes are needed. Particles move through world space around each player's view and are respawned within a bounded volume. Air particles are suppressed underwater, inside solid blocks, and beneath solid roofs. Up to 144 motes and 48 eight-segment ribbons use one draw call and at most 3,168 vertices. The effect uses procedural geometry and no downloaded textures.
