# Shelter and cave lighting

Rain streaks stop at the highest solid block in their own column, including
player-built roofs and dungeon ceilings. A streak crossing the roof is clipped
at its top surface; rain remains visible outdoors from inside a doorway.
Roof checks use loaded, edited blocks and refresh each frame, so adding or
removing a roof takes effect immediately. Unknown columns suppress rain.

Skylight now spreads through openings, fading over seven blocks into covered
spaces. Fully enclosed rooms retain a faint ambient floor (0.012). Terrain,
creatures, props, and held equipment use the same field. Local torch and lantern
lights still illuminate them; campfire light also works beneath a roof during
the day. Roof heights are updated on block edits. Skylight is cached per chunk;
edits and chunk streaming invalidate affected neighbors, and terrain updates
when remeshed. Opaque blocks stop propagation; this is bounded skylight, not
multi-bounce global illumination. Glass currently follows its existing opaque
block definition.

Local lights use cached voxel visibility to stop illumination through opaque
walls, with no additional shadow passes. Moving lights refresh after a quarter
block of movement; nearby block edits refresh their visibility too. Shadows are
coarse voxel silhouettes; animated creatures do not cast local-light shadows.

Equip a [torch](TORCHES.md) in inventory's separate left-hand slot for warm
six-block light while wielding a sword or tool. Torches burn forever; crystals
are ordinary resources without held lighting. Campfires and placed lantern
devices also offer stationary light.

Bow and longbow are temporarily disabled in inventory, crafting, attacks, and
script equipment discovery. Legacy counts and enum slots remain in saves, but
their hotbar entries behave as empty. The Ranger's equipment quest now uses a
sword. All multiplayer peers need protocol 38.
