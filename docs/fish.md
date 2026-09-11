# Fish

Fish spawn in suitable loaded water and swim between reachable underwater goals.
They can occasionally jump above the surface and return along a checked corridor.
Water/obstacle checks constrain their movement; they do not pathfind over land.
Small puddles are excluded: spawning requires a full 7x7 water layer centered on
the floored position, plus clearance for the fish's body (half-width 0.7 and
half-height 0.35). This is three cells beyond the center in both horizontal axes.

Discovery is bounded and driven by authoritative simulation. Fish are replicated
as creatures and their behavior state participates in saves. Removing water can
strand fish; do not rely on them surviving arbitrary habitat destruction.

## Prompting

- `api.can_spawn_fish(x,y,z)` tests habitat in loaded chunks, including staged edits.
- `api.spawn_fish(x,y,z)` returns a stable id or nil and shares the ordinary
  creature-spawn budget. Failed attempts also consume a spawn attempt.
- `api.spawn_creature('fish',x,y,z)` also validates staged habitat.
- `api.spawn_creature_near_player(id,'fish',radius)` searches committed loaded
  water with bounded attempts; it may return nil even when some water exists.
- `api.find_creatures('fish',...)` and `api.nearest_creature('fish',...)` use the
  normal creature interfaces. Snapshots expose can_swim, can_fly and in_water.
- `api.chase(id,x,y,z)` can guide fish through connected water; refresh its lease
  each tick. This does not override collision/water constraints or force a jump.

`api.fish_spawn_clearance` is 3. Use block-center positions such as x+0.5,y+0.5,z+0.5
for spawn checks. A fish's in_water field checks its center and may be false during
a jump; can_swim describes its capability. Other creature kinds may be immersed
without having dedicated fish swimming behavior. There is no exposed jump timer,
fish-size setter, or unrestricted teleport method.
