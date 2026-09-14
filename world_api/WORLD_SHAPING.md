# World API 1.33: review and world shaping

The consolidated [World API user guide](USER_GUIDE.md) includes this feature's
usage alongside every other current API capability and the complete reference.

The existing API already exposes authoritative inventories, crafting, machines,
weather/time, combat behavior, spawning and player effects. Its main gaps for
creative rules were cumbersome terrain edits, little material introspection,
no distinction between unloaded space and actual air, and no roof/edited-height
queries. The generation prompts also lagged behind the runtime: they restricted
equipment to three tools and omitted `on_interact` from the allowed callbacks.
Focused prompt retrieval matched method-name fragments, so new shape and roof
methods needed explicit inclusion in their relevant capability groups.

## Implemented capabilities

| Player intent | API building blocks |
| --- | --- |
| Build a platform, bridge, wall, or small room | `fill_box`, `get_player`; use separate boxes for walls and roof |
| Excavate a crater or create a mineral pocket | `fill_sphere` with new material `air` or an ore; filter `stone` to preserve other materials |
| Transform all nearby ores without listing every ore | `find_blocks("ore",...)`, `block_matches`, shape filters |
| Rain changes exposed ground but roofs protect it | `is_exposed_to_sky`, existing weather properties and block edits |
| Place something above actual construction or edited ground | `surface_height`; includes trees and roofs, not the nearest cave floor |
| Make a shrine heal animals for a resource cost | `on_interact`, `take_element`, `find_creatures`, `heal_creature` |
| Inspect or condition on a material's properties | `get_block_kinds`, `get_block_info` |
| Inspect an exact player or creature without repeated loops | `get_player(id)`, `get_creature(id)` |

The tested examples are [a stone platform](examples/stone_platform.lua),
[rain turning exposed soil into mud](examples/rain_softens_soil.lua), and
[a crystal healing shrine](examples/crystal_healing_shrine.lua). These are
ordinary Lua compositions, not special-cased natural-language commands.

## Consistent Lua contract

All capabilities remain flat `api.method(...)` functions. Snapshots are detached
tables: changing returned metadata does not modify engine definitions. Reads see
accepted writes from the same callback. Missing entity/material queries return
`nil`; action failures use the documented false/nil result. Existing older
methods retain their return conventions for compatibility.

`fill_box(x1,y1,z1,x2,y2,z2,kind,filter)` normalizes inclusive integer corners.
`fill_sphere(cx,cy,cz,radius,kind,filter)` selects integer cells by distance from
an integer center. Both return the changed-cell count or `nil`. Zero means a
successful no-op. The optional filter defaults to `any`; `air` builds only in
empty cells and `stone` transforms only stone. Filter categories are shared
with `find_blocks` and `block_matches`, and unknown filters never match.

Calls reserve the complete change before staging it. Exceeding the remaining
edit allowance, encountering a device/bedrock that would change, or requiring
unloaded terrain rejects the whole shape. A rejected call does not undo earlier
successful calls; raise a Lua error if a multi-call structure must roll back as
one transaction. Native budget exhaustion always aborts the entire callback.

Existing limits remain: 32 block changes per rule callback, 300 per cast,
4096 candidate cells per shape, 512 API calls and 65,536 native work units per
callback, plus time/instruction/memory and shared scheduler limits. Larger
construction needs explicit gradual batches. Queries do not load arbitrary
chunks. Shape edits grant no inventory drops and charge no material costs;
use inventory APIs for player-requested costs and crafting APIs for crafting.

## Authority, persistence and scope

Host-only transactions feed the existing block-edit and creature replication
paths. Terrain edits and healed creature health persist through ordinary saves.
Disabling a rule stops future callbacks; it does not undo committed terrain.
Existing world generation and save formats remain compatible.

This extension lets players combine existing materials and creatures into new
behaviors. It does not register new native block types, change textures/models,
rewrite mining physics, or add arbitrary saved components. Lua variables last
only for the current module VM lifetime. Those capabilities would require a
separate design for saved identities, renderer assets and network compatibility.

Schema, generated references, stubs, numeric guards and runtime registration
are checked together. Automated tests exercise real Lua for shape geometry,
filters, staged queries, complete rejection, rollback, native-limit enforcement,
healing persistence, examples, and inclusion in focused LLM context. This is
offline validation; no claim is made about a live local model's success rate.
