# Shelter and cave lighting

Rain streaks stop at the highest solid block in their own column, including
player-built roofs and dungeon ceilings. A streak crossing the roof is clipped
at its top surface; rain remains visible outdoors from inside a doorway.
Roof checks use loaded, edited blocks and refresh each frame, so adding or
removing a roof takes effect immediately. Unknown columns suppress rain.

Sheltered terrain, creatures, props, and held equipment receive only a faint
ambient floor (0.012), with no direct sky light, sky reflections, or lightning
washout. Local torch and lantern lights still illuminate them; campfire light
also works beneath a roof during the day. Roof columns are cached for each mesh
build/frame. Terrain receives updated shelter shading when its chunk remeshes.
This is vertical sky occlusion, not full light propagation through windows.

Equip a [torch](TORCHES.md) in inventory's separate left-hand slot for warm
six-block light while wielding a sword or tool. Torches burn forever; crystals
are ordinary resources without held lighting. Campfires and placed lantern
devices also offer stationary light.

Bow and longbow are temporarily disabled in inventory, crafting, attacks, and
script equipment discovery. Legacy counts and enum slots remain in saves, but
their hotbar entries behave as empty. The Ranger's equipment quest now uses a
sword. All multiplayer peers need protocol 38.
