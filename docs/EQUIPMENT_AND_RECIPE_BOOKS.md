# Equipment and recipe books

Press **C** for the equipment catalog. Search by name or effect, filter by path, or show only recipes you can afford. The separate elemental tab retains ordered formulas, extraction and mana conversion. After crafting, open **I** and assign the item to a hotbar slot. Equipment effects apply while that item is selected and owned; carrying several charms does not stack their effects.

Look for floating books and the **Recipe book [F]** label on dry land. Look directly at a volume within five blocks and press **F**. Reading plays `sounds/lore_book.mp3`, opens its recipes, and teaches that set permanently to your character. Books use `models/lore_book.glb`, including its embedded texture. Individual volumes are randomly scattered on dry land, at least 17 blocks apart. Books stand upright with a gentle hover and a few golden motes. Locations remain stable across reloads and are shared by every player; small islands may contain only some of the four recipe paths. Obstructed or flooded books become readable when the obstruction is removed. Books remain for friends, and rereading costs nothing.

The original axe, pickaxe, sword and bow remain available without a book. New recipes consume iron, oak wood, mana and a specialist material. The catalog shows exact costs and missing materials. Salvage returns at most half the specialist recipe's materials and also costs mana. Each player's discoveries and inventory persist separately; the host validates reading and crafting for guests too.

| Book / path | New item | Gameplay | Specialist ingredient |
|---|---|---|---|
| The Greenhand's Almanac / Harvesting | Forester axe | Twice the wood/plant harvesting power | 2 resin |
| | Prospector pick | Twice the stone/ore harvesting power | 2 quartz |
| | Spade | Three times the soil/sand harvesting power | 1 copper |
| | Sickle | Harvest plants in one stroke | 3 plant fiber |
| The Warden's Arsenal / Combat | Spear | 4.8-block reach; 10 damage per 0.55s | 2 flax |
| | Dagger | 2.4-block reach; 7 damage per 0.20s | 2 obsidian |
| | Warhammer | 3-block reach; 25 damage per 1.1s | 6 stone |
| | Longbow | 36-block reach; 16 damage per 1s; 2 mana | 6 plant fiber |
| The Living Elements / Magic | Ember wand | 18-block bolt; 22 damage per 0.9s; 4 mana | 2 ruby |
| | Tide wand | 24-block bolt; 8 damage per 0.30s; 1 mana | 2 sapphire |
| | Life staff | Left-click: heal yourself 15 HP per 2s; 5 mana | 6 wild herbs |
| | Survey lantern | Held terrain/cave illumination; no running mana cost | 2 enchanted glass |
| The Wayfarer's Handbook / Exploration | Trail charm | 30% faster walking and sprinting | 2 amber |
| | Leaping charm | 35% stronger jumps | 2 emerald |
| | Diving charm | One-quarter normal oxygen consumption | 6 reeds |
| | Feather charm | Limits descent to 3 blocks per second | 4 cloth |

Weapon strikes and bolts respect terrain occlusion, ownership, mana and cooldowns. Harvesting upgrades change the number of strikes, not the quantity of block drops. The life staff heals only its user and cannot revive a defeated player.

Equipment currently uses procedural held models and matching inventory silhouettes. These are placeholders for future supplied models; definitions live in `src/gear_catalog.rs`, and held geometry lives in `src/held_item.rs`. Gear enum order is append-only for saved hotbar references. All peers must run the updated build (protocol 36).
