# Beehives on agent-built trees

The latest request in `target/debug/saves/coplay.log`,
`request-1790617334274-10`, finished partially: the agent built the giant
spruce/birch tree near (-330, 70), reaching Y=94, but reported no beehive
block/entity capability and recorded CAP-0005. The existing hive renderer
only discovered procedural hives near low natural canopies; no writable
hive material or Lua placement action existed.

World API 1.41.0 adds `api.place_beehive(x,y,z)`. Coordinates identify the
empty hive cell beside wood/branches or beside/below leaves, including spruce.
The action returns false for unsupported, occupied, unloaded, invalid-height,
or exhausted-budget sites. Support can be created in the same transaction.

Placed hives use an appended `beehive` block kind, preserving previous IDs.
Normal block edits provide saving, joining-player state, replication, inspection,
removal, and callback rollback. Coplay capability searches and the material
catalog expose the new action/material. Existing hive rendering, buzzing,
orbiting bees, honey harvesting and destruction stings apply to placed hives.
Harvest cooldowns use all three coordinates so stacked hives remain independent.
Bee visuals do not add a separately addressable bee creature type.

Example, after inspecting an empty cell below a leaf at (2,91,2):

```lua
function on_cast(api, event)
    assert(api.place_beehive(2,90,2), "Hive site unavailable")
    assert(api.get_block(2,90,2) == "beehive")
end
```

Regression coverage includes tall staged-tree placement, rejection, rollback,
targeting, serialized edit and chunk reload, and local multiplayer placement/removal.
The original save and CAP-0005 record are retained; a graphical in-game playtest
is still needed to verify appearance in that specific saved tree.

Validation: `cargo test --offline -- --test-threads=1` passed 650 tests,
with 31 existing external-service/GPU/manual tests ignored and no failures.
