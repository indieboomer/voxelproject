# Rivers, lakes, mountains and underwater rendering

World generation now includes narrow winding rivers joining large lake basins and occasional rocky mountains. Terrain remains a deterministic function of world seed and position, independent of chunk loading order.

## Generation

`src/voxel/terrain.rs` layers the features over the existing rolling terrain. Rivers meander along several sine/noise scales, with lake centers placed directly on their paths. Channels are generally around 5-8 blocks wide, widening where they meet lakes or existing lowlands. Lake radii are roughly 24-52 blocks, with noise-varied shorelines and up to six blocks of water. Rivers and lakes share the existing sea level (Y=18), guaranteeing connected water without a flow simulation. Their paths cross chunk boundaries using global coordinates. The MVP uses roughly spaced river corridors; it does not model drainage basins, currents or waterfalls.

Mountain candidates are sparse, seed-selected patches with variable elliptical footprints and roughened peaks. Peaks fit inside the existing 48-block world height, reaching roughly Y=41-44. At Y=34 and above the surface and subsoil become rock; tree and grass scattering stop there. Lower slopes retain the existing biome and resources. Rivers and lakes carve valleys through terrain after mountain elevation is applied.

The seed-42 survey over a 1024x1024-block region, sampled every two blocks, recorded 2,201 rocky samples out of 262,144 (0.84%), 20,868 water samples (7.96%), and a highest sampled point of Y=43. These are sample statistics, not quotas for every seed. The generated map is `target/terrain-map.png` (blue water, green lowland, gray rocky tops). Detailed statistics are in `target/terrain-survey.txt`.

## Underwater view

The camera's eye position is checked against actual water blocks each frame. Being in water up to the player's feet does not enable the effect. When the eyes submerge, world geometry and creatures receive a blue tint and blue fog over 1.5-26 blocks; the distant sky becomes the same underwater blue. Distant birds and rain streaks are hidden underwater. The HUD and other egui windows remain readable and untinted. Leaving the water immediately restores normal rendering. This is local presentation, independent of the existing drowning/oxygen mechanics.

The effect uses the reserved `weather_fx.z` uniform component, preserving uniform layouts and existing lighting/weather fields.

## Compatibility and checks

Existing saved block edits and inventories are preserved. Unedited terrain regenerates with the new geography when a save loads; player edits can still obstruct a generated river. Use New World for a clean example of the new terrain. Multiplayer protocol is now 5 so old terrain generators cannot silently join a new session: all Direct/Steam players need this build.

Tests verify continuous wet paths between lake centers, orthogonally connected channel steps, positive/negative chunk boundaries and reverse load order, lake width, sparse high peaks, terrain height bounds, natural resource availability and buried ore placement. Eye-position tests distinguish submersion from wading, and an opt-in GPU check validates both modified WGSL shaders.

```powershell
cargo test --offline
cargo test --offline --features steam
cargo test --offline --features steam validate_underwater_shaders -- --ignored
.\tools\build_steam.ps1
```
