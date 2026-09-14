# Torches

Read **The Wayfarer's Handbook** to learn the torch recipe, then craft it in
**C** or inventory **I**: **2 oak wood + 1 resin + 2 mana**.

Open **I** and click **Equip torch** in the left-hand row. The torch stays in
your left hand while the hotbar controls your sword, pickaxe, or another tool
in your right hand. **Put torch away** stops its light and sound. A torch has no
fuel, duration, durability, or ongoing mana cost. It does not take a hotbar slot.

The supplied `models/torch.glb` provides the handle, texture, emissive flame and
looping flame animation. Its six-block yellow light flickers and moves slightly.
It uses a spatial loop of `sounds/campfire.mp3` at reduced gain (0.28 versus
campfire 0.45). Up to four nearby torches are audible, fading out at 12 blocks.

Equipping is a host-validated inventory transaction. The left-hand choice is
saved independently of the hotbar; guests see each other's torch, light and
sound. A missing torch stack or defeated player produces no torch light.
Crystals no longer illuminate terrain when held. Survey lanterns keep their
existing cool light. Bow and longbow remain disabled.

Older saves gain an empty torch stack and an empty left hand. Multiplayer
requires matching builds (protocol 38).
