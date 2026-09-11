# Development and verification

Run commands from the repository root. Rust/Cargo are required; the optional Steam
feature is unnecessary for ordinary local builds. Asset paths are compiled or
resolved according to the existing runtime paths; retain the asset directories.

```powershell
cargo build --no-default-features
cargo test --no-default-features --quiet -- --test-threads=1
```

The executable is `target/debug/voxelproject.exe` on Windows. Serial tests avoid
unrelated machine contention affecting tight script time budgets. Ignored tests
include GPU, preview, and local-model integration checks; a normal passing suite
does not imply those external capabilities were exercised.

## World API changes

Edit `world_api/schema.yaml` alongside the Rust runtime implementation, then run:

```powershell
py -3 tools/gen_world_api.py
cargo test --no-default-features environment_ -- --nocapture
```

The generator requires Python and PyYAML. It updates the Rust validator registry,
full reference, compact AI context and Lua stubs. Append new block discriminants
to preserve saves; non-CSV block IDs belong in the generator's extras list.
API additions need argument validation, bounded native-work charges, transaction
visibility and failure-rollback tests, not documentation alone.

## Generation and GPU previews

```powershell
cargo test --no-default-features campfire -- --nocapture
$env:VOXEL_CAMPFIRE_PREVIEW='1'
cargo test --no-default-features render_weather_previews -- --ignored --nocapture
Remove-Item Env:VOXEL_CAMPFIRE_PREVIEW
$env:VOXEL_WATER_PREVIEW='1'
cargo test --no-default-features render_weather_previews -- --ignored --nocapture
Remove-Item Env:VOXEL_WATER_PREVIEW
```

Previews write `target/render-*.png` and report GPU timings when supported. They
overwrite earlier previews. Campfire generation writes a separate survey to
`target/campfire-generation.txt` with counts and coordinates for three seeds.
The synthetic campfire preview checks appearance; the generation survey checks
that natural terrain actually contains fires. Water previews use a generated
tributary. Neither preview is a full multiplayer or whole-game FPS benchmark.
