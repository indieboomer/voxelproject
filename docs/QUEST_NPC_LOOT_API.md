# Quest NPC and loot bag creation — World API 1.40.0

The merchant-stand request in `agentic-test`, request `request-1790612866741-33`,
completed its 24 block edits but correctly reported CAP-0002: the engine had NPCs
and loot bags, while scripting could create neither. This update exposes those
existing systems rather than introducing new merchant or inventory mechanics.

New calls:

```lua
-- Coordinates are loaded integer feet cells with solid support and two air cells.
assert(api.spawn_npc("merchant", 147, 21, 40))
assert(api.spawn_loot_bag(145, 21, 40, {stone=math.random(1,4), amber=1}))
local people = api.find_npcs(147, 21, 40, 8)
local bags = api.find_loot_bags(147, 21, 40, 8)
```

Example coordinates are illustrative: inspect terrain before using them. Quest NPC
types are `sage`, `elf_ranger`, `warrior`, `merchant`, `fire_sorceress`, and
`necromancer`. They wander locally and retain existing dialogue and per-player
quests; another merchant does not reset completed rewards or provide shop trading.
NPCs are separate from combat creatures. Spawn calls return booleans, not IDs.

Bags accept a map of 1–16 canonical collectible resources, 1–256 items per entry,
and at most 1,024 total. Equipment and arbitrary item names are unsupported.
Contents can be randomized in Lua. Ordinary bags retain the existing two-second
pickup delay and 120-second lifetime, and save their age and contents. The API
does not charge inventory: it is an authoring action like existing item grants.

Both creations share creature spawn budgets: four attempts per persistent callback,
30 per instant callback. Failed placements consume attempts. NPC creation stops at
256 total NPCs and bag creation at the existing 48-drop cap. Placement checks read
staged edits, and same-dispatch queries/cap checks include earlier committed spawns.
Any failed callback discards its NPC and bag effects. Position and table limits
use the existing sandbox guards; invalid types/contents raise errors.

The App captures authoritative NPC/drop state before host tick, normal cast,
targeted cast, guest cast and Co-Play execution. Successful effects append through
the existing App effect path; NPCs use reliable nearby NPC updates and bags use
the normal world snapshot. Existing save fields carry both. Query results are
detached snapshots without stable IDs. Co-Play inspection now includes nearby
`npcs`, `loot_bags`, and an NPC truncation flag, enabling verification of NPC type
and bag contents after creation. Live observation is still required before
marking CAP-0002 resolved; the original save/backlog are not rewritten here.

Validation: `cargo test --offline --features dev-playtest -- --test-threads=1`
passed 668 tests with 33 environment-dependent tests ignored. The four new focused
tests exercise all six NPC types, staged terrain/spawn reads, duplicate and cap
checks across callbacks, malformed loot rollback, shared creature/NPC/bag budgets,
save/network round trips, delayed pickup, and expiry. No live model or graphical
multiplayer playtest was performed.

After final placement-bound checks, the four focused API tests passed again in the
default build. A further focused-prompt regression passed after exposing the new
signatures in creature, inventory and block capability groups, so both Co-Play and
the spell generator can discover the creation/query calls.
