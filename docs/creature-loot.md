# Creature deaths and pickups

Actual creature deaths create a 0.6-second puff of shrinking square particles,
animated in discrete steps. The creature leaves simulation immediately; the puff
occupies its place as it vanishes. The host sends the effect to guests. Natural
population recycling does not produce a death puff or loot.

Direct player weapon kills create one loot bag containing all rewards on supported
ground nearby, checking alternative columns on slopes and at loaded chunk edges.
Walk within 1.5 blocks to collect; pickups remain visible for at least two seconds
before collection is enabled. Terrain blocks pickup through walls. The host awards each
bundle once and synchronizes inventory; full counters leave the entire pickup in place.
The bag uses the embedded `models/loot_bag.glb` model with its burlap and gold
vertex colors. It bobs vertically, with three small golden pixels drifting upward.
Geometry is cached once; particles use an emissive material without extra lights.
Collection removes it and shows the collector an animated "Acquired" list with
resource icons, names and quantities. The list slides in and fades out after four
seconds; successive pickups combine quantities and restart the timer. Multiplayer
feedback is sent reliably only to the collector.
The collector hears `sounds/loot.mp3` once per collection batch.
Creature-versus-creature combat and script kills do not create pickup rewards.

| Creature | Reward |
| --- | --- |
| Sheep / cow | 2 plant fiber |
| Chicken / fish | 1 plant fiber |
| Wolf | 1 plant fiber + 1 resin |
| Stinger | 1 resin |
| Goblin | 1 iron ore + 1 cloth |
| Stone golem | 3 stone + 1 iron ore |
| Sunscorch | 2 sulfur + 1 ash |
| Zombie | 1 ash + 1 cloth |
| Skeleton | 2 ash |
| Green dragon | 3 resin + 2 crystal dust |
| Red dragon | 3 sulfur + 2 crystal dust |

These use existing catalog resources and elemental compositions, not new food,
bone, feather or hide currencies. Biological drops represent recoverable organic
residue; humanoids leave cloth/materials, undead leave ash, and magical predators
leave sulfur/resin and crystal residue. Resource tooltips identify loot sources.
Normal paid resource decomposition remains available after pickup.

Performance limits: 16 concurrent puffs with 12 particles each, 48 pickup bundles,
120-second pickup lifetime. Additional drops are omitted when full or no supported
loaded ground is found. Pickups are transient and are not saved across sessions.
Effects and pickups obey the terrain draw boundary. Network protocol 18 requires
host and guests to run the same build.
