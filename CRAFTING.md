# Elemental crafting

Press **C** during gameplay to open crafting; press **C** or **Esc** to close. Mine blocks, use **Confirm extraction** to turn gathered resources into elements, then build a formula or select one from the Formula book. Conversion to mana is always explicit. Balances start at zero. The existing mouse/keyboard and egui navigation are used; this project has no controller bindings.

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

## Resource catalog

The world has **82 stackable resources**, including 34 resource formulas with two or three ordered slots. See [the complete resource/formula/balance guide](RESOURCES.md). `data/resources.json` is the authoring catalog; `tools/build_resources.py` generates resource entries in `data/crafting.json` while preserving creature recipes. The Formula book is searchable and shows texture icons and mana costs.

Resource outputs use `kind: resource`; no equipment or consumable item system was added. Existing creature formulas remain unchanged and may use one to five slots. Crafted output compositions must be present, and their recoverable elements multiplied by quantity must not exceed inputs in any element; invalid registries fail loading.

Inventory `item`, `block`, and `resource` outputs all add to the current block-resource inventory and can be selected in Resources for placement. There is no separate consumable-item system or fixed slot capacity; `u32` stack overflow reports Inventory full. The poison/preservative/lava examples are therefore adapted to existing mud/redstone/basalt content.

Creature outputs use `CreatureDraft`, the existing ECS staging path and global creature cap. Every creature reserves a distinct nearby position with a solid 3 x 3 floor and 3 blocks of air above it, separated from players and creatures. A failed reservation or exhausted budget discards the whole batch. Host snapshots replicate committed creatures as usual.

To support another output category, extend `ObjectKind`, validate IDs and quantities in `Registry::validate`, then prepare its output in `prepare_transaction`. Reserve all required capacity without touching live state. Add an infallible commit after every preparation succeeds, and connect the output to existing replication and saves. Do not consume balances before a fallible live spawn or inventory write.

## Authority and persistence assumptions

Requests contain an action and the last observed account revision, never balances or a chosen output. The host resolves the sender from its connection and revalidates the transaction. Only the current revision can succeed; reliable-message deduplication and the pending UI lock provide additional protection. Absolute inventory snapshots ignore older revisions. Host mining/placement, client mining/placement, and remote Lua grants all use the host-owned resource balances. Existing client-reported movement and UDP transport remain; this does not add anti-cheat or account authentication.

Steam guest saves use authenticated Steam IDs; Direct guest saves use a separate sanitized-nickname namespace and migrate ordinary legacy nickname accounts. Direct nicknames are not authenticated and cannot be shared by simultaneously connected Direct guests. Host inventory is stored separately. All five elemental balances, mana, resource stacks, and guest accounts survive save/load. Creature IDs, kind, position and health survive; transient animation/chase state resets. Legacy saves initialize missing inventories to zero and retain their existing natural creature generation behavior. No discovery system is introduced.

Saves use the `VOXEL_SAVE_2` envelope and are written to a temporary file then renamed, preserving the previous save if serialization/writing fails. Legacy `WorldSave` binary files still load. Old executables cannot read the new saves or use the new network protocol; multiplayer peers need the same build.

## Validation

Run `cargo build --offline`, `cargo test --offline`, and `cargo clippy --offline --all-targets`. Tests cover formulas, gap normalization, bounds, all mana tiers, insufficient balances, full stacks, failed/blocked/capped creature outputs, successful batches, extraction/conversion, duplicate registries, network request replay, account replication, and both save formats.

Manual check: mine stone/wood/ore, open C, extract materials, convert some into mana, craft reversed formulas, place the output, summon a creature on cleared ground, repeat as a joined guest, save, reload, and reconnect with the same nickname.
