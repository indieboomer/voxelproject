# Storage, underground exploration, and named saves

Implementation uses the existing authoritative device transactions, deterministic chunk generator, and versioned save format. Chests, mana devices, loot, and enemies share the host's state with guests.

## Chests

- Open the workshop with **B**, select **Storage Chest**, and place it in one empty voxel. Building costs four oak wood. The mesh and embedded wood/iron texture come from `models/chest.glb`.
- Aim at a chest and press **F** to open its storage panel. Your inventory and chest contents appear side by side. Select an item and amount, then **Store** or **Take**. **Store all** and **Take all** transfer the selected item type in one transaction.
- Chests start in storage mode. They have no gameplay slot or stack limit; the engine's integer accounting supports up to 4,294,967,295 total units per chest. They hold resources, elements, tools, and bound creatures. Packed machines retain their separate inventory slots.
- Left-clicking a chest packs it with all contents preserved. Replace the packed chest from the workshop. Transfers check reach, obstruction, available stock, and inventory overflow on the host; failed transfers change neither inventory.
- The chest panel only offers inventory transfers: no machine settings, mana, signals, rotation, or ejection controls. Existing saved automation connections and settings remain compatible.

## Caves and dungeons

Cave walls retain a reduced selection of deposits exposed by excavation. Additional small patches
provide iron, copper, coal, tin, silver, gold, sulfur, rock salt and quartz.
Emerald, amethyst, sapphire and ruby patches appear at Y=16 or below; diamond,
mithril and moonstone patches are rarer and limited to Y=10 or below. Existing
natural veins keep their own depth ranges. Previously the cave lining replaced
other resources with stone/bricks and offered only copper and gold.

This material change also applies to existing cave layouts when chunks regenerate
(including after loading a save). Cave shapes remain the same and saved block
edits take precedence, so mined deposits and player construction stay intact.

- Newly created worlds place one open system per suitable 96 by 96 block land region. Each has nine chambers connected by a seeded depth-first spanning-tree algorithm, with at least 224 blocks of connecting corridors, a spiral descent, and an additional loop. Rooms include ore-rich walls, three loot chests, and skeleton guards.
- Every system has an open, stone-framed surface entrance. New-world startup reports a nearby entrance location. Spiral stairs descend to floor Y=3; terrain generation retains its original elevation profile, while the build ceiling is now Y=127. Systems gain additional scale horizontally. Deep water crossings receive a solid lining.
- New cave version 2 uses a **3-block-wide outer gate** with a **1-block-wide, 3-block-high opening**. The entrance stair passage is one block wide with four blocks of headroom. Connecting corridors vary between **1–2 blocks wide and 3–4 blocks high**; rooms are **4–5 blocks across in each horizontal direction and 3–4 blocks high**. The bottom stair junction turns inward so the narrow exit cannot run back beneath its own stair treads. Guards fit inside the smaller rooms.
- Terrain is generated from world coordinates, independent of chunk load order. The host initializes each landmark's rewards once and saves its discovery marker. Emptying or packing a chest, killing guards, leaving the area, and reloading does not replenish rewards.
- The cave-generation version is saved and replicated. Older saves keep their original terrain generation, including the larger version-1 rooms; create a new world to get the compact systems. New underground terrain is not retroactively carved into them. Generated chests share the existing 128-device world limit; when it is reached, undiscovered rewards wait for space.

Cave-wall ore and mineral deposits now retain approximately half their previous
abundance for every resource, including natural deposits exposed by cave carving.
The selection preserves small patches, relative rarity and depth limits; added
ore covers about 7% of otherwise plain cave lining instead of 14%. Ordinary buried
veins and chest rewards are unchanged. Regenerated chunks use the reduced density,
while saved player block edits remain authoritative.

## Mana and appearance

- New lanterns, dark altars, and shrines receive **30 mana**. A lantern spends one mana per ten seconds; altars and shrines spend one per second. Their paid final interval completes before effects and light stop.
- Refill through the inspection panel or a connected mana supply. Packing preserves charge and the remaining paid interval; it never refills the device. The explicit **Mana-free actions (testing)** setting bypasses spending.
- Machine and ritual-device surfaces use the existing pixel terrain atlas, tinted to preserve their material colors. Chests use their own embedded model texture.

## Save and load

- Enter a unique world name during creation. **F5** saves to that world's file and displays success or an error. Hosts also save on exit. Guests cannot save the host's world.
- **Load World** offers a saved-world selector. Files live under `saves/world-<name>.bin`; the original `saves/world.bin` appears as **world**.
- Replacement writes and synchronizes a temporary file first, retaining the previous save in `.bin.bak`. A failed or corrupt load reports an error instead of creating a replacement world. To recover a backup manually, copy it over its corresponding `.bin` file with the game closed.
- Saves include terrain edits, generation settings, rule modules, inventories, packed devices, chest contents, remaining device power, creature state, underground discovery, dropped loot, player health/status, and the weather cycle.

## Chat logs

Manual **F5** saves and save-on-exit also export the host's complete chat and
notification scrollback as UTF-8 text to `saves/<worldname>_chat.log`.
Player names and message contents are retained, including multiplayer chat and
lines that have left the limited on-screen history. The transcript is stored in
the world save too, so subsequent sessions continue it. Repeated saves replace
the export without duplicating lines; the previous export is kept as `.log.bak`.
Older saves start with an empty transcript; historical messages that were never
recorded cannot be recovered. Export failures are reported separately after the
world save succeeds. Joined players rely on the host's world save/export.

## Verification

Run `cargo test --offline`. Tests cover large atomic chest transfers, preserved packed contents, initial mana and exhaustion, legacy saves, file replacement and backups, state round trips, deterministic underground generation, and one-time landmark rewards. GPU previews are opt-in through the existing `render_weather_previews` test with `VOXEL_MACHINE_PREVIEW=1` or `VOXEL_AURA_PREVIEW=1`.

## Map and building height

Press **M** to open or close the map, or **Esc** to close it. It shows procedural land and water, cave entrances (purple), placed/modified solid blocks and devices (yellow), and your current position (white/red). Drag to pan, scroll or use the buttons to zoom, and use **Center on me** to return to your position. Hover an entrance for coordinates. Markers use the same world generation version as the save. Opening the map does not load chunks or initialize cave loot.

The vertical build range is now **Y=0..127**. Existing terrain elevations and saved worlds retain their shapes. Empty upper layers are implicit and are skipped by terrain meshing; upper storage is allocated when blocks are placed there. Building large structures still adds geometry and memory in proportion to the construction.

Health, poison and oxygen are positioned beneath the measured shortcut-panel bounds, including wrapped text in the fantasy theme.

### Height benchmark

`cargo test --offline profile_exploration -- --ignored --nocapture` on the current machine, averaging three fixed exploration areas:

| Work | 48-block ceiling | 128-block ceiling |
|---|---:|---:|
| Generate 121 chunks | 156.47 ms | 159.20 ms |
| Mesh 121 chunks | 234.22 ms | 233.57 ms |
| Stream 11 new chunks | 14.05 ms | 14.22 ms |

Triangle counts were identical in every area. Generation averaged about 1.7% slower, meshing about 0.3% faster; this benchmark showed no material regression for existing terrain. This does not claim that arbitrarily large new towers are free to render. Detailed outputs are in `target/height48-benchmark.log` and `target/height128-final-benchmark.log`.
