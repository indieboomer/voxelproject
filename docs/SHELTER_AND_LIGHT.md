# Shelter and cave lighting

Rain streaks stop at the highest solid block in their own column, including
player-built roofs and dungeon ceilings. A streak crossing the roof is clipped
at its top surface; rain remains visible outdoors from inside a doorway.
Roof checks use loaded, edited blocks and refresh each frame, so adding or
removing a roof takes effect immediately. Unknown columns suppress rain.

Sheltered terrain, creatures, props, and held equipment receive only a faint
ambient floor (0.012), with no direct sky light, sky reflections, or lightning
washout. Local crystal and lantern lights still illuminate them; campfire light
also works beneath a roof during the day. Roof columns are cached for each mesh
build/frame. Terrain receives updated shelter shading when its chunk remeshes.
This is vertical sky occlusion, not full light propagation through windows.

Select a glowing crystal in the hotbar for a six-block portable light. For sword
combat, the proposed next step is a belt-light slot: equip one crystal there to
keep its light while holding a weapon. The belt slot is a proposal, not implemented
in this change. Existing campfires and placed lantern devices offer stationary
light while using a sword.

Bow and longbow are temporarily disabled in inventory, crafting, attacks, and
script equipment discovery. Legacy counts and enum slots remain in saves, but
their hotbar entries behave as empty. The Ranger's equipment quest now uses a
sword. All multiplayer peers need protocol 37.
