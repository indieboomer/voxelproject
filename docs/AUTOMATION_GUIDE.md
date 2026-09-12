# Magical production

Implemented from [AUTOMATION.md](../AUTOMATION.md). Sixteen devices run on the authoritative host, including in single-player. Production does not call the LLM.

## Build and operate

- **B** opens the device palette. Each button lists its resource cost and is enabled when affordable. Select a device, aim at an adjacent empty cell, **R** to rotate, and **left or right click** to place. **B** or **Escape** exits placement and restores the previous tool. Machine mode hides the held item, suppresses tool actions, and grays out and locks the hotbar, including number keys and the wheel.
- The preview outlines the occupied cell green/red, shows rotated face ports, and marks compatible neighbors green. Mana has one mark, matter two, and signal three, alongside blue/amber/violet colors.
- Aim at a device and press **F** to inspect/configure it. Choose settings, then **Apply settings**. Reopen F if another player changes the configuration while you're editing; stale drafts cannot overwrite their changes.
- **Left click outside build mode** or **Pack intact** moves the complete device into your packed inventory, including contents, mana, configuration, and paid production progress. B places it again without another construction charge. Eight packed devices fit per player. Packing fails safely if full.
- The F panel exposes live activity, progress, storage, mana, signal, and world-facing port directions. Use **Deposit/Withdraw** with an item ID and quantity. Vessels, lanterns, dark altars, and shrines accept explicit personal mana refills; network mana never automatically draws from the player.
- Workshops transport creature recipes internally as `creature:<kind>` figurines. By default the feeding chest releases these as living creatures nearby. In storage mode, you can still withdraw a figurine and use **Release one carried bound creature nearby** manually.

Devices have full-cell collisions even where their temporary mesh leaves gaps. Construction and inventory interactions require nearby unobstructed space and are validated by the host. Existing mining and block-edit APIs cannot destroy stored machinery or its cargo.

Feeding chests default to **Eject contents nearby**. Each second, an enabled chest throws up to two units of one resource/item/element in a loot bag, or releases one creature. Bags follow a short arc from the chest and land nearby; walk close after the two-second pickup delay to collect them. Creatures use their normal behavior immediately; fish require suitable water. The outlet alternates between stored item types.

Turn **Eject contents nearby** off in F to retain stock or feed an adjacent machine through the east matter output. In ejection mode that output is disabled. Blocked throw paths, unavailable ground, full loot capacity, and failed creature spawn checks retain the goods and show **BlockedOutput**. Ejection waits for the area to be loaded. Machine bags survive save/load and do not expire; the shared 48-bag limit stops further ejection until bags are picked up. Pickup and creature release are host-authoritative and remove each unit exactly once.

## Lantern, Dark Altar, and Shrine

Build these from **B**, then aim anywhere on the part and press **F** to refill it with personal mana or enable/pause it. Each accepts mana from any horizontal side or below at its base, so a collector immediately west can power it directly. Capacity is 60 mana. All have a one-block footprint; placement, collision, and block-edit protection cover their full height.

| Part | Height | Operation | Mana |
| --- | --- | --- | --- |
| Street Lantern | 3 blocks | Victorian iron post and framed lantern with a steady white-blue light at the top | 1 per 10 seconds |
| Dark Altar | 2 blocks | Black-purple particles; deals 3 damage to every creature within a 6-block radius each second | 1 per second |
| Shrine | 2 blocks | Green-white motes; restores 3 health to every creature within a 6-block radius each second, capped at maximum health | 1 per second |

Auras affect friendly and hostile creatures, including flying creatures within the spherical radius, but do not affect players. Overlapping auras each apply their effect. The first pulse occurs when a mana cycle starts. Pausing, packing, and saving preserve remaining paid runtime; offline time grants no effects. Mana-free testing powers all three without consuming mana. Unpowered/disabled parts stop their light, particles, and working sound. The supplied `sounds/dark_altar.mp3` and `sounds/shrine.mp3` play as nearby spatial loops (at most four), stopping when the part stops working. Lanterns share the nearest-four local-light budget with campfires and smelters.

## Ore smelter

Build **Ore Smelter** from B for **4 stone + 2 clay + 1 oak wood**; it needs no metal to build. It converts one copper, iron, tin, silver, gold, or mithril ore into one corresponding metal. Gem ores and alloys are not smelting recipes.

Deposit ore and optional coal/wood through F, or connect a matter input. Smelting is immediate on the next host tick (100 ms), up to two ores per tick, with no timed production batch. Each ore costs **1 network mana**, or fuel heat: **one wood smelts two ores; one coal smelts eight**. All tree wood types work. Stored heat is spent first, then available mana, then coal, then wood. Unused heat survives save/load and packing; no fuel is burned while idle or blocked. Mana-free testing mode consumes neither mana nor fuel, but still consumes ore.

At rotation 0, **west accepts mana and matter**, **north accepts matter**, and **east outputs finished metal**. Place a collector immediately west to provide mana. For automatic ore/fuel supply, place a feeding chest north of the smelter, rotate it once so its output faces south, and turn that chest's ejection off.

**Eject finished metal nearby** defaults on and throws finished metal in pickup bags. Turn it off to feed a chest or channel immediately east. Only finished metal leaves the smelter; input ore, fuel, and remaining heat stay inside. A full or closed output pauses smelting before spending more fuel. F shows stored heat, supported conversions, and a missing-fuel message. Balance and conversion recipes are in `data/automation.json` under `smelting`.

The temporary model is a stone-and-ceramic furnace with a grate, glowing coals, and chimney. While smelting it emits black smoke puffs and a small flickering orange light, fading briefly after production stops. Stored fuel heat alone does not keep the effect running. A replacement `ore_smelter.glb` should follow the one-cell model contract below.

## Machine feedback

Nearby machines briefly glow and emit particles when something changes:

| Event | Visual | Sound from `sounds/machine` |
|---|---|---|
| Production completed | Gold sparks rising from the device | `clunk3.mp3` |
| Mana/matter received or sent, including deposits and withdrawals | Blue pulse and small motes | `silent_clunk.mp3` |
| Activity, signal, rotation, or configuration changed | Violet pulse | `clunk1.mp3` |
| Blocked, missing ingredients/mana, closed, or disabled | Orange-red pulse | `clunk2.mp3` |
| Placed | Green pulse | `medium_loud_clunk.mp3` |
| Packed/removed | Brown motes at the former position | `clunk4.mp3` |

Effects last 0.8 seconds and operate within 24 blocks. Sounds use distance falloff and stereo position. Frequent transfers are coalesced per device; at most four nearby cues play per second, with production preferred over routine transfer noise. No continuous idle clunk plays. The configuration panel still supplies exact counts and inactivity reasons.

Host event counters travel with device snapshots, so an item passing through a channel between two client snapshots still produces feedback. Loading or joining establishes a silent baseline rather than replaying historical activity. Effects are cosmetic and never change production, inventories, or timing. `VOXEL_MACHINE_PREVIEW=1 cargo test --bin voxelproject render_weather_previews -- --ignored --nocapture` renders an opt-in GPU test scene (set the environment variable separately in PowerShell).

## Verified example: stock three sheep figurines

Use any clear horizontal row above ground; coordinates below are relative to its first cell, not absolute world coordinates. All devices have rotation 0 (west input, east output). Put the sensor one cell above the valve. Read actual cell coordinates in F when setting its target.

| Relative cell | Device | Settings |
|---|---|---|
| (0,0,0) | Mana Collector | Enabled |
| (1,0,0) | Mana Vessel | Reserve 0; optional personal charge |
| (2,0,0) | Element Condenser | Life; reserve 16 recommended to retain its next conversion cost |
| (3,0,0) | Formula Workshop | `sheep` recipe |
| (4,0,0) | Controlled Valve | Matter; **Open when signal is OFF** |
| (5,0,0) | Feeding Chest | Enabled; **Eject contents nearby OFF** to accumulate stock |
| (4,1,0) | Threshold Sensor | Target chest's actual cell; item `creature:sheep`; upper 3, lower 1; State mode |

The collector supplies one mana per second. Each Life element costs 16 mana and takes 3 seconds once funded; the existing sheep recipe consumes two Life and no additional mana, then takes 4 seconds. Expect a slow startup unless you explicitly charge the vessel. The automated acceptance test runs this loop with the shipped balance, without manually inserting mana or ingredients.

At three figurines the sensor closes the adjacent output valve and pauses the workshop. Withdrawing one leaves the signal latched; withdrawing another reaches the lower threshold and resumes production. Sensors measure at the start of a tick; buffered items in other layouts can cause stock to exceed a threshold by in-transit quantities. Place the controlling valve directly after the workshop for immediate backpressure. For more complex routes insert channels, conduits, and signal connectors.

For a resource recipe, select `stone`: one Earth followed by one Water costs the existing two recipe mana. Deposit the elements in either arrival order; the workshop reconstructs the configured ordered recipe. When using one condenser, switch its selected element between batches to provide both ingredients; changing settings never changes a paid batch. Multiple condensers can also merge through channels into the shared input. A matter channel carries no mana, so route a separate mana conduit to the workshop's west face if the preceding device doesn't output both networks.

## Connections and control

Ports must be adjacent, opposing, and of the same type, with an output facing an input. No intermediate connector is required. Rotation turns local east toward south; up/down remain vertical. Channels, conduits, splitters, and valves let you choose a local input and ordered output faces, including corners and vertical routes.

| Device | Default ports / behavior |
|---|---|
| Collector | Mana east out; enabled collectors within eight blocks form a shared supply cluster, including chains |
| Vessel | Mana west in, east out; retains its configured reserve |
| Condenser | Mana west in; matter and surplus mana east out; selected element |
| Dissipator | Matter west in, mana east out; selected element only |
| Conduit | Mana west in, east out; configurable faces |
| Channel | Matter west in, east out; configurable faces |
| Splitter | Matter west in; east/north/south out; configurable faces |
| Chest | Matter west in; ejects by default, or east out when ejection is off; 128 total stored units |
| Workshop | Matter and mana west in; matter east out; 64 buffered units |
| Smelter | Mana/matter west in, matter north in; metal east out when ejection is off; 32 buffered units |
| Sensor | Signal out on all six faces; chest inventory or vessel mana within 16 blocks |
| Valve | Mana west in/east out by default; switch to matter while empty; signal up/down in |
| Signal connector | Signal bidirectional on all six faces; branches combine by maximum |

Splitter round-robin distributes among compatible available destinations. Priority tries the configured outputs in order; deselect/reselect faces to change that order. Filter sends the selected item only to the first configured face and other items to the remaining faces; disconnecting the primary output does not silently redirect matching items.

Sensors latch on at `value >= upper` and off at `value <= lower`. State emits 0/1, Pulse emits a single tick on the rising edge, Numeric emits the measured count. Valves treat any nonzero value as on and optionally invert it. Disabled or disconnected signal paths clear on the next tick. There are no standalone logic gates or spell actuators.

## Simulation, balance, and persistence

**Settings → Gameplay → Mana-free actions (testing)** disables mana requirements and spending for crafting/decomposition, gear, generated rules, instant casts, explicit Lua `take_mana`, condensers, workshops, and personal vessel charging. Materials, processing time, storage limits, and other checks still apply. The option defaults off and is saved in `settings.json`. The host controls it for all players; changing it while joined only changes your preference for a future hosted session. Turn it off to restore costs; existing paid batches finish with their original payment. Actual balances are preserved rather than filled to an artificial maximum. Custom Lua checks against hard-coded mana thresholds still see real balances; use the API cost constants for engine fees.

[data/automation.json](data/automation.json) contains build costs, capacities, throughput, durations, default ports, and element conversion costs/returns. It is embedded at build time, like the existing recipe registry. Edit and rebuild all peers together. Existing ordered recipes and their mana costs remain in [data/crafting.json](data/crafting.json).

The default is 64 active devices and a 100 ms fixed host tick. Each tick measures/propagates signals, gathers shared ambient supply, transfers mana, transfers matter, then advances production. Stable cell/port order with rotating source and mana-destination priority makes allocation deterministic. Sources cannot forward newly received stock in the same tick; both inbound and outbound throughput are bounded. No network graph cache needs invalidation when devices rotate or disappear.

Ingredients and mana are reserved once into a paid batch. Full local outputs or blocked connected downstream storage hold production; clearing space resumes that same batch. Disconnected outputs can accumulate in the producer's bounded buffer. Disabled devices retain all state. Condensing then dissipating always loses mana: costs `[10,12,14,16,20]`, returns `[4,5,6,7,8]` for Earth/Fire/Water/Life/Death. Invalid balance definitions are rejected.

Saves preserve active devices, sensor latch, routing cursor, inventories, mana, paid batches, and packed player devices. Older saves default to an empty installation. There is no offline simulation; a frame stall catches up at most one second to bound work. Devices continue operating outside rendered chunks while the host runs.

Multiplayer uses protocol **26**. Clients send requests; only the host changes installations and accounts. Reliable snapshots split devices into groups of four and publish every half second (also scheduled by edits). Clients validate and install a complete revision atomically, tolerate reordered/duplicate packets, and never simulate production themselves. This MVP replicates the bounded whole installation rather than using spatial subscriptions or deltas.

## Prompting / World API 1.25.0

The generated [World API reference](world_api/world_api_readme.md), compact prompt documentation, and Lua stubs include:

- `get_device(x,y,z)` and `get_devices()` for host snapshots, inventory counts, and activity.
- `set_device_enabled(x,y,z,enabled)` for conditional production rules.
- `configure_device(x,y,z,settings)` for recipe, element, reserve, routing, filtering, and sensor/valve settings.
- `place_device(player_id,kind,x,y,z,rotation)` to construct from the connected player's resources, with ordinary reach/space checks.

Mutations share the block-edit budget and the existing callback transaction. An error rolls back device changes and construction costs together. No API grants free network mana or allows destruction of stored cargo. Use `on_cast` for one-time placement/configuration; ordinary production is built in and needs no Lua timer. Disabling a device also stops transfers; use a downstream valve for stock control while allowing upstream mana to recharge. Example pause spell:

```lua
function on_cast(api, event)
    for _, device in ipairs(api.get_devices()) do
        if device.kind == "workshop" then
            api.set_device_enabled(device.x, device.y, device.z, false)
        end
    end
end
```

## Art and replacement assets

Original procedural meshes in [src/automation_mesh.rs](src/automation_mesh.rs) use carved-looking wooden frames, ceramic pots, copper rings, stone markers, moving sparks, workshop rotation, and visible valve positions. They need no downloads, textures, or external licenses. Simulation and rendering are separate; final assets are not loaded yet.

Manually supplied models can replace these props: `mana_collector.glb`, `mana_vessel.glb`, `element_condenser.glb`, `element_dissipator.glb`, `mana_conduit.glb`, `matter_channel.glb`, `filter_splitter.glb`, `feeding_chest.glb`, `formula_workshop.glb`, `threshold_sensor.glb`, `controlled_valve.glb`, and `signal_connector.glb`. A distinct matter-valve variant and modular straight/corner/vertical connector pieces would improve readability.

Replacement contract: one metre/cell, geometry inside `[0,1]` on X/Z and `[0,height]` on Y (3 for lantern, 2 for altar/shrine, 1 otherwise), origin at the base cell minimum corner, +Y up, +X east, +Z south. Ports belong at base-cell face centers and are drawn from device definitions; avoid baking fixed connection markers into a configurable connector model. Optional named nodes `rotor`, `gate`, and `spark` can support animation when a GLB adapter is added. Keep materials low-poly wood/ceramic/copper/stone and emission restrained.

## Verification and limits

Implementation verification: **32 automation tests passed**; full suite **440 passed, 22 opt-in tests ignored** (run with `--test-threads=1`). GPU previews verified working smelters and the lantern/altar/shrine scene at night. The shipped-balance end-to-end test verifies the upper/lower stock limits, pauses, and resumption without injected ingredients or mana. Aura tests cover radius boundaries, healing caps, deaths, fixed-tick catchup, mana payment, pausing, persistence, full-height collision, personal refilling, and collector power.

Run `cargo test --bin voxelproject automation` for production, ordered recipe payment, conservation and competition, output blocking/resume, vertical/rotated/reconnected routes, packing, persistence, Lua transactions, complete multiplayer snapshot assembly, and occupied-height mesh bounds. Run the full suite with `cargo test --bin voxelproject -- --test-threads=1`. Set `VOXEL_AURA_PREVIEW=1` and `VOXEL_MACHINE_NIGHT_PREVIEW=1`, then run `cargo test --bin voxelproject render_weather_previews -- --ignored --nocapture` for the new parts' nighttime render preview.

Offscreen panel previews: set `UI_PREVIEW_PANEL=automation`, then `cargo test --bin voxelproject render_ui_previews -- --ignored --nocapture`; output is `target/ui-*-automation.png`. The executable builds with `cargo build`.

Remaining scope limits: temporary procedural art; no offline production, harvesting/mining/block-placement machines, maintenance, standalone logic gates, or spell devices. Multiplayer correctness is covered at the command/snapshot and simulation layers; a live two-player visual playtest is still recommended.
