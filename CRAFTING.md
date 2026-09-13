# Elemental crafting

Press **C** during gameplay to open crafting; press **C** or **Esc** to close. Mine blocks, use **Confirm extraction** to turn gathered resources into elements, then build a formula or select one from the Formula book. Conversion to mana is always explicit. Balances start at zero. The existing mouse/keyboard and egui navigation are used; this project has no controller bindings.

## Tools and weapons

The initially expanded **Tools and weapons** section in C exposes these recipes:

| Item | Materials | Mana |
|---|---|---:|
| Sword | 2 iron + 1 oak wood | 4 |
| Axe | 3 iron + 2 oak wood | 4 |
| Pickaxe | 3 iron + 2 oak wood | 6 |
| Bow | 1 iron + 4 oak wood | 6 |

These use the same host-authoritative transactions as inventory crafting. Missing
materials disable crafting. The third campkeeper contract requires a newly crafted
sword while active; old completion credit is preserved.

The bow fires an immediate mana shot up to 20 blocks for 9 damage, costing
1 mana per shot with a 0.7-second cooldown. Terrain blocks shots. Assign the bow
in **I**, select its hotbar slot, then left-click. Bows can be stored in machines
as `item:bow`, and existing saved bows are retained.

**Craft bound sheep figurine** in C uses the sheep formula (2 Life and the
formula's normal mana cost) but puts `creature:sheep` into your carried production
goods instead of spawning it. Deposit it into a chest's matter storage and use
**Release** to bring it to life. Workshops producing sheep also create figurines.
This craft completes the Necromancer's "An Unexpected Interest" objective.

## Additional formulas

There are now **49 ordered elemental formulas**. Experiment or open the searchable
Formula book to see and load them. There is no discovery unlock gate. Order matters.

| New formula | Ordered inputs | Output |
|---|---|---|
| Chicken | Life 1 → Water 1 | 1 chicken |
| Cow | Life 4 → Earth 1 | 1 cow |
| Wolf | Life 3 → Death 1 | 1 wolf |
| Stinger | Life 2 → Death 2 | 1 stinger |
| Goblin | Life 2 → Earth 2 → Death 1 | 1 goblin |
| Zombie | Death 3 → Life 1 | 1 zombie |
| Skeleton | Death 3 → Earth 2 | 1 skeleton |
| Fish | Water 2 → Life 1 | 1 fish; requires a suitable nearby pool |
| Crystal | Earth 1 → Water 2 → Death 1 | 1 crystal |
| Oak wood | Earth 1 → Life 2 → Water 1 | 1 oak wood |
| Stone batch | Earth 8 → Water 2 | 8 stone |
| Bricks batch | Earth 8 → Fire 4 | 4 bricks |

Land creatures need clear supported space. Failed spawning refunds the entire
transaction. Hostile recipes create hostile creatures. Mana uses existing slot-count
costs; mana-free testing waives mana, not materials or elements.

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

The world has **84 stackable resources**, with 38 resource formulas and 11 creature formulas. See [the complete resource/formula/balance guide](RESOURCES.md). `data/resources.json` is the authoring catalog; `tools/build_resources.py` generates single-output resource entries in `data/crafting.json` while preserving creature and batch recipes. The Formula book shows icons, output quantities and mana costs.

Resource outputs use `kind: resource`. Equipment uses the separate material recipes above. Creature formulas may use one to five slots. Resource compositions must be present, and recoverable elements multiplied by output quantity must not exceed inputs in any element; invalid registries fail loading.

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
