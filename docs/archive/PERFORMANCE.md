> Archived 2026-09-15. Historical exploration benchmark; newer rendering work has changed the measured costs.
> See [current documentation](../rendering.md). Claims and measurements below describe the historical revision.

# Exploration performance

The investigation found two travel-related costs. All 11 new chunks at a chunk-boundary crossing were generated synchronously, followed by up to eight mesh builds/uploads in the same frame. Additionally, the world retains chunks out to radius 7 for reuse (up to 225), but the renderer submitted every retained mesh to both terrain and shadow passes. Spawn initially has only the radius-5 neighborhood (121 chunks), so exploration could substantially increase rendered work even when standing still afterward.

Changes:

- Cache a chunk's 18x18 terrain height grid, including a one-cell border for mountain slopes. Reuse those heights for surface materials, deposits, ground vegetation and snow support. Previously those passes recalculated the same procedural terrain repeatedly.
- Stream missing chunks nearest-first, at most two per frame with a 2 ms time budget checked between chunks. Rebuild/upload at most two dirty meshes per frame with a 3 ms budget checked between meshes. One indivisible operation can exceed its time budget; these are scheduling limits, not hard execution deadlines.
- Always load the player's immediate 3x3 collision neighborhood before physics, including after teleportation. Host interaction requests retain their existing authoritative terrain loading. Distant visual terrain can fill in over subsequent frames.
- Render terrain only within the intended radius 5 and camera frustum. Cull shadow chunks against the sun's separate frustum so offscreen shadow casters are retained when needed. Cached radius-7 terrain remains available for backtracking.
- Invalidate neighboring meshes when chunks arrive or leave, refreshing hidden boundary faces and ambient occlusion without rebuilding everything at once.

Measurements on this machine in the existing optimized debug Steam test profile, seed 42:

| Player X | Generate 121 chunks before / after | Generate incoming 11 before / after | Full-area triangles before / after |
|---|---|---|---|
| 0 | 160 / 62 ms | 13.5 / 5.6 ms | 227,668 / 227,668 |
| 2,048 | 151 / 60 ms | 13.7 / 5.9 ms | 238,776 / 238,776 |
| 16,384 | 152 / 63 ms | 13.3 / 5.5 ms | 248,674 / 248,674 |

This is about a 60% reduction in generation CPU time. Meshing itself remains approximately 1.6-2.0 ms per chunk here, but is now spread across frames. A 16:9, 70-degree, forward-looking culling test submits 46 camera chunks at all three distances rather than all 225 retained chunks. Actual counts vary with camera pitch, direction, and loaded terrain. Wind geometry measured about 0.3 ms per frame and was not the dominant measured CPU cost.

These are CPU benchmarks and draw-submission checks, not a measured in-game FPS guarantee. Live GPU timing and the user's particular save/rules/session were not profiled. Resource distribution, deterministic generation, collision, saved edits, multiplayer, and frustum boundary/shadow tests pass.

Reproduce:

```powershell
cargo test --features steam profile_exploration -- --ignored --nocapture
cargo test --features steam profile_wind_geometry -- --ignored --nocapture
cargo test --features steam retained_ring -- --nocapture
cargo test --features steam
.\tools\build_steam.ps1
```
