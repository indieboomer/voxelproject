# Elemental crafting

Press **C** during gameplay to open crafting; press **C** or **Esc** to close. Mine blocks, use **Confirm extraction** to turn gathered resources into elements, then build a formula or select one from Starter formulas. Conversion to mana is always explicit. Balances start at zero. The existing mouse/keyboard and egui navigation are used; this project has no controller bindings.

## Architecture

- `src/crafting.rs`: validated registry, composition APIs, ordered matching, checked arithmetic, private account transactions, staged creature output, spawn clearance, tests.
- `src/crafting_ui.rs` and `src/ui.rs`: five ordered slots, totals, mana/result previews, failure feedback, explicit conversion/extraction, pending-request lock.
- `src/player.rs`: existing resource inventory now lives beside elements and mana in `Account`.
- `src/app.rs` and `src/net.rs`: host execution, connection-derived guest identity, reliable requests, authoritative account snapshots, registry synchronization, host validation of remote mining/placement and Lua grants.
- `src/save.rs` and `src/creature.rs`: versioned save envelope, legacy binary fallback, host/guest inventories and world creatures with persistent IDs, positions and health. Existing block edits and Lua module saves remain included.

## Data definitions

`data/crafting.json` is read on host startup. If absent, the compiled copy is used, so a packaged executable still has recipes. Invalid files report an error instead of silently replacing the configuration. The host sends its registry to joined clients; clients never decide outputs. Restart the host after editing balance data.

Recipes contain a unique `id`, one to five `inputs` (`element`, positive integer `amount`), and an `output` (`kind`, `id`, positive integer `quantity`). Canonical block IDs come from `BlockType::from_name`; creature IDs come from `crafting::creature_kind`. Duplicate IDs or ordered formulas fail loading. Amounts and summed costs must fit `u32`. Empty UI slots are normalized away; repeated slots are retained.

`mana_costs` is indexed by occupied slot count minus one: **[0, 2, 4, 16, 256]**. Slot amounts do not affect mana. `conversion_rate` is mana per elemental unit, currently **1**. Overflow and insufficient balances consume nothing.

Compositions are central `compositions` entries with `kind`, canonical `id`, and `elements` in **Earth, Fire, Water, Life, Death** order. Missing entries yield zero. Items/resources/placeable blocks share the existing block registry and composition namespace; creatures have a separate namespace. `Registry::composition`, `Account::add_elements`, `can_afford_elements`, and `consume_elements` provide the gameplay APIs. Mutating helpers are used inside private transactions; network clients only send intentions.

Extraction consumes existing stackable resources. Water and crystal compositions are queryable metadata, but those objects are not stackable in the existing inventory. Creature compositions are metadata; this change does not add creature harvesting or melee.

## Starter recipes

| Ordered slots | Output (quantity 1) | Mana |
|---|---|---|
| Earth x1 | stone (block) | 0 |
| Earth x2 -> Fire x1 | bricks (block) | 2 |
| Fire x1 -> Earth x2 | basalt (block) | 2 |
| Water x1 -> Death x1 | mud (resource) | 2 |
| Death x1 -> Water x1 | redstone (item) | 2 |
| Life x2 | sheep (creature) | 0 |
| Earth x5 -> Life x2 -> Fire x1 | stone_golem (creature) | 4 |
| Earth x1 -> Earth x1 | cobblestone (block) | 2 |
| Earth x1 -> Water x1 -> Life x1 -> Fire x1 | pumpkin (item) | 16 |
| Earth x1 -> Fire x1 -> Water x1 -> Life x1 -> Death x1 | sunscorch (creature) | 256 |

## Starter compositions

Amounts below are `(Earth, Fire, Water, Life, Death)`. All unlisted types yield zero.

| Type | IDs | Composition |
|---|---|---|
| block | grass, soil, sand, cobblestone | (2, 0, 0, 0, 0) |
| block | stone | (1, 0, 0, 0, 0) |
| block | bricks, basalt, copper_ore, gold_ore | (2, 1, 0, 0, 0) |
| block | oak_wood, spruce_wood, cherry_wood, birch_wood | (1, 0, 0, 2, 0) |
| block | oak_leaves, spruce_leaves, cherry_leaves, birch_leaves, short_grass | (0, 0, 1, 2, 0) |
| block | pumpkin | (1, 1, 1, 1, 0) |
| block | mud, redstone | (0, 0, 1, 0, 1) |
| block | diamond_ore, emerald_ore, crystal | (1, 1, 1, 1, 1) |
| block | water | (0, 0, 2, 0, 0) |
| creature | sheep | (0, 0, 0, 2, 0) |
| creature | chicken | (0, 0, 0, 1, 0) |
| creature | cow | (1, 0, 0, 3, 0) |
| creature | wolf | (0, 0, 0, 2, 1) |
| creature | stinger | (0, 0, 0, 1, 1) |
| creature | goblin | (1, 0, 0, 2, 1) |
| creature | stone_golem | (5, 1, 0, 2, 0) |
| creature | sunscorch | (0, 3, 0, 0, 3) |

## New output handlers

Inventory `item`, `block`, and `resource` outputs all add to the current block-resource inventory and can be selected in Resources for placement. There is no separate consumable-item system or fixed slot capacity; `u32` stack overflow reports Inventory full. The poison/preservative/lava examples are therefore adapted to existing mud/redstone/basalt content.

Creature outputs use `CreatureDraft`, the existing ECS staging path and global creature cap. Every creature reserves a distinct nearby position with a solid 3 x 3 floor and 3 blocks of air above it, separated from players and creatures. A failed reservation or exhausted budget discards the whole batch. Host snapshots replicate committed creatures as usual.

To support another output category, extend `ObjectKind`, validate IDs and quantities in `Registry::validate`, then prepare its output in `prepare_transaction`. Reserve all required capacity without touching live state. Add an infallible commit after every preparation succeeds, and connect the output to existing replication and saves. Do not consume balances before a fallible live spawn or inventory write.

## Authority and persistence assumptions

Requests contain an action and the last observed account revision, never balances or a chosen output. The host resolves the sender from its connection and revalidates the transaction. Only the current revision can succeed; reliable-message deduplication and the pending UI lock provide additional protection. Absolute inventory snapshots ignore older revisions. Host mining/placement, client mining/placement, and remote Lua grants all use the host-owned resource balances. Existing client-reported movement and UDP transport remain; this does not add anti-cheat or account authentication.

The MVP has no persistent player IDs. Guest saves are keyed by sanitized nickname; use the same nickname when rejoining. Simultaneously connected guests cannot share a nickname. Nicknames are not authenticated. Host inventory is stored separately. All five elemental balances, mana, resource stacks, and guest accounts survive save/load. Creature IDs, kind, position and health survive; transient animation/chase state resets. Legacy saves initialize missing inventories to zero and retain their existing natural creature generation behavior. No discovery system is introduced.

Saves use the `VOXEL_SAVE_2` envelope and are written to a temporary file then renamed, preserving the previous save if serialization/writing fails. Legacy `WorldSave` binary files still load. Old executables cannot read the new saves or use the new network protocol; multiplayer peers need the same build.

## Validation

Run `cargo build --offline`, `cargo test --offline`, and `cargo clippy --offline --all-targets`. Tests cover formulas, gap normalization, bounds, all mana tiers, insufficient balances, full stacks, failed/blocked/capped creature outputs, successful batches, extraction/conversion, duplicate registries, network request replay, account replication, and both save formats.

Manual check: mine stone/wood/ore, open C, extract materials, convert some into mana, craft reversed formulas, place the output, summon a creature on cleared ground, repeat as a joined guest, save, reload, and reconnect with the same nickname.
