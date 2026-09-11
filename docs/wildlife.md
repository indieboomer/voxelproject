# Creature visibility and population

Host and guest renderers filter creature snapshots before constructing animated
meshes. A creature's chunk must have a terrain mesh and be within the same
five-chunk draw radius as terrain. Retained chunks outside that radius no longer
leave distant creatures visible against missing ground. Terrain depth testing
continues to hide creatures behind drawn terrain.

The host replenishes ordinary wildlife during exploration. Every two seconds it
tries at most two spawns per connected player, targeting 20 land creatures within
88 horizontal blocks. Spawn candidates use loaded, edited ground with four clear
blocks overhead, remain 48–72 blocks from the selected player and at least 48
blocks from every other player, and avoid crowding existing creatures. Existing
species weights and the 60-second entry protection still apply.
The host also streams terrain around distant guests using the existing shared
two-chunk/time generation budget per frame. Guest regions need no host GPU meshes.

Natural land creatures and fish beyond 128 horizontal blocks from all players
are recycled without death events, loot, or death sounds. This larger removal
radius avoids repeatedly spawning/removing creatures near the discovery boundary.
Recycled fish release their source pool for rediscovery. Killed fish retain the
existing consumed-region behavior. Dragons retain their solitary territory and
death persistence.

Natural discovery stops when the world contains 128 creatures in total; fish
discovery additionally stops at 32 fish and dragon discovery at 8 dragons.
These are discovery limits, not a destructive limit on existing saves or Lua
spawns. Natural spawn tracking survives saves. Script-spawned creatures and
wildlife with changed aggression, explicit targets or custom behavior are kept.
Older saves have no spawn tracking, so their existing creatures are preserved.
Rules can therefore exceed the natural population budget; their existing script
budgets still apply. Distant preserved creatures are not deleted by this system.
