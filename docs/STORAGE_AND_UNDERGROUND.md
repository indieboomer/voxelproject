# Storage, underground exploration, and named saves

Implementation uses the existing authoritative device transactions, deterministic chunk generator, and versioned save format. Chests, mana devices, loot, and enemies share the host's state with guests.

## Chests

- Open the workshop with **B**, select **Storage Chest**, and place it in one empty voxel. Building costs four oak wood. The mesh and embedded wood/iron texture come from `models/chest.glb`.
- Aim at a chest and press **F** to open its storage panel. Your inventory and chest contents appear side by side. Select an item and amount, then **Store** or **Take**. **Store all** and **Take all** transfer the selected item type in one transaction.
- Chests start in storage mode. They have no gameplay slot or stack limit; the engine's integer accounting supports up to 4,294,967,295 total units per chest. They hold resources, elements, tools, and bound creatures. Packed machines retain their separate inventory slots.
- Left-clicking a chest packs it with all contents preserved. Replace the packed chest from the workshop. Transfers check reach, obstruction, available stock, and inventory overflow on the host; failed transfers change neither inventory.
- The chest panel only offers inventory transfers: no machine settings, mana, signals, rotation, or ejection controls. Existing saved automation connections and settings remain compatible.

## Caves and dungeons

- Newly created worlds have sparse, seeded underground landmarks. Natural chambers and brick-lined dungeon rooms have ore-rich walls, loot chests, and skeleton guards.
- Stair passages sometimes open at the surface; others stop short and need excavation. Look away from lakes and riverbeds, which remain sealed against underground generation.
- Terrain is generated from world coordinates, independent of chunk load order. The host initializes each landmark's rewards once and saves its discovery marker. Emptying or packing a chest, killing guards, leaving the area, and reloading does not replenish rewards.
- Older saves keep their original terrain generation. New underground terrain is not retroactively carved into them. Generated chests share the existing 128-device world limit; when it is reached, undiscovered rewards wait for space.

## Mana and appearance

- New lanterns, dark altars, and shrines receive **30 mana**. A lantern spends one mana per ten seconds; altars and shrines spend one per second. Their paid final interval completes before effects and light stop.
- Refill through the inspection panel or a connected mana supply. Packing preserves charge and the remaining paid interval; it never refills the device. The explicit **Mana-free actions (testing)** setting bypasses spending.
- Machine and ritual-device surfaces use the existing pixel terrain atlas, tinted to preserve their material colors. Chests use their own embedded model texture.

## Save and load

- Enter a unique world name during creation. **F5** saves to that world's file and displays success or an error. Hosts also save on exit. Guests cannot save the host's world.
- **Load World** offers a saved-world selector. Files live under `saves/world-<name>.bin`; the original `saves/world.bin` appears as **world**.
- Replacement writes and synchronizes a temporary file first, retaining the previous save in `.bin.bak`. A failed or corrupt load reports an error instead of creating a replacement world. To recover a backup manually, copy it over its corresponding `.bin` file with the game closed.
- Saves include terrain edits, generation settings, rule modules, inventories, packed devices, chest contents, remaining device power, creature state, underground discovery, dropped loot, player health/status, and the weather cycle.

## Verification

Run `cargo test --offline`. Tests cover large atomic chest transfers, preserved packed contents, initial mana and exhaustion, legacy saves, file replacement and backups, state round trips, deterministic underground generation, and one-time landmark rewards. GPU previews are opt-in through the existing `render_weather_previews` test with `VOXEL_MACHINE_PREVIEW=1` or `VOXEL_AURA_PREVIEW=1`.
