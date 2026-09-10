# Resources and elemental balance

The catalog contains **84 stackable, placeable resources**: **56 obtainable naturally** (six also craftable), and **28 crafting-only**. This adds 61 resources to the original 23. Air, water and bedrock are not counted. Surface blue crystals now stack in inventory; harvesting still sets the legacy crystal rule flag. No equipment, tools, consumable items, stations or animal harvesting were added.


Snow forms solid, opaque patches from Y=32 on mountain slopes and summits, mixed with exposed rock and rare grass. Generated snow is one block deep with stone immediately underneath. It is hand-pickable (one use), placeable from the hotbar, and extracts to Water 1. Snow is natural-only with no crafting recipe; it does not melt or spread. Its original, seamless 64px powder texture is shared by top, side and bottom faces, with no emission or shiny glimmer.

## Play and progression

Open Resources to search by name/category, filter to owned resources, and select a material for placement. Hover a name for its source and mining hardness. Press **C** and open the searchable **Formula book** to fill the ordered slots. Every resource formula uses **two or three slots**, costing **2 or 4 mana** respectively, and produces one resource. Order matters. Existing creature formulas remain separate and unchanged.

The crafting system is elemental transmutation: extract a gathered block into Earth/Fire/Water/Life/Death, explicitly convert spare elements into mana, then craft. Iron ore is a natural block and iron is a crafted material. Ore is a useful elemental source, not an identity-locked ingredient: equivalent elements from other sources also work. Rare geology therefore offers variety and concentrated materials, not a hard progression gate. Specific ore requirements would be a separate change to the crafting rules.

Example with no starter mana: extract **one iron ore** (Earth 3, Fire 1) and **one coal** (Earth 1, Fire 3). Convert **Fire 2** into two mana. Craft **[Earth 3] [Fire 2] -> Iron**. You retain Earth 1; the ore, coal, crafting elements and mana are consumed.

Early materials use 2-5 elemental units and two slots; alloys and arcane materials generally use 6-9 units and three slots. Mana can come from any element, keeping common plant/soil gathering useful. Refined material blocks can be placed, mined and extracted again, but their extraction never returns more of any element than the recipe spent, and never refunds mana. The registry rejects amplification, including quantity-multiplier overflow. This component-wise conservation rule prevents multi-recipe recycling profit as well as direct loops.

## Sources and formulas

E = Earth, F = Fire, W = Water, L = Life, D = Death. Arrows show slot order; `-` means no resource recipe. Extraction lists the elemental yield of one block. All entries have textures.

| Resource | Family | Availability | Formula | Mana | Extraction E/F/W/L/D |
|---|---|---|---|---:|---|
| Snow | Water and ice | Mountain snow patches, Y 32+ | - | - | 0/0/1/0/0 |
| Grass | Plants | Trees and open grassland | - | - | 1/0/0/1/0 |
| Soil | Earth and stone | Surface terrain | - | - | 2/0/0/0/0 |
| Stone | Earth and stone | Underground pockets + crafting | E1 -> W1 | 2 | 1/0/0/0/0 |
| Sand | Earth and stone | Surface terrain | - | - | 2/0/0/0/0 |
| Oak Wood | Plants | Trees and open grassland | - | - | 1/0/0/2/0 |
| Oak Leaves | Plants | Trees and open grassland | - | - | 0/0/1/2/0 |
| Cobblestone | Earth and stone | Underground pockets + crafting | E1 -> E1 | 2 | 2/0/0/0/0 |
| Bricks | Earth and stone | Rare buried ruins + crafting | E2 -> F1 | 2 | 2/1/0/0/0 |
| Spruce Wood | Plants | Trees and open grassland | - | - | 1/0/0/2/0 |
| Spruce Leaves | Plants | Trees and open grassland | - | - | 0/0/1/2/0 |
| Cherry Wood | Plants | Trees and open grassland | - | - | 1/0/0/2/0 |
| Cherry Leaves | Plants | Trees and open grassland | - | - | 0/0/1/2/0 |
| Birch Wood | Plants | Trees and open grassland | - | - | 1/0/0/2/0 |
| Birch Leaves | Plants | Trees and open grassland | - | - | 0/0/1/2/0 |
| Basalt | Earth and stone | Underground pockets + crafting | F1 -> E2 | 2 | 2/1/0/0/0 |
| Gold Ore | Metals | Underground pockets | - | - | 4/2/0/0/0 |
| Diamond Ore | Gems | Underground pockets | - | - | 4/2/3/0/0 |
| Emerald Ore | Gems | Underground pockets | - | - | 3/0/2/3/0 |
| Copper Ore | Metals | Underground pockets | - | - | 2/1/0/0/0 |
| Pumpkin | Plants | Trees and open grassland + crafting | E1 -> W1 -> L2 | 4 | 1/0/1/2/0 |
| Short Grass | Plants | Trees and open grassland | - | - | 0/0/1/2/0 |
| Mud | Earth and stone | Wet ground near sea level + crafting | W1 -> D1 | 2 | 0/0/1/0/1 |
| Redstone | Arcane | Crafting or active world rules | D1 -> W1 | 2 | 0/0/1/0/1 |
| Iron Ore | Metals | Underground, Y 3-30 | - | - | 3/1/0/0/0 |
| Tin Ore | Metals | Underground, Y 8-32 | - | - | 2/0/1/0/0 |
| Silver Ore | Metals | Underground, Y 3-20 | - | - | 2/0/1/0/1 |
| Coal | Minerals | Underground, Y 4-32 | - | - | 1/3/0/0/0 |
| Sulfur | Minerals | Underground, Y 3-20 | - | - | 1/2/0/0/1 |
| Rock Salt | Minerals | Underground, Y 5-24 | - | - | 1/0/3/0/0 |
| Clay | Earth and stone | Underground, Y 10-27 | - | - | 2/0/2/0/0 |
| Limestone | Earth and stone | Underground, Y 5-32 | - | - | 3/0/1/0/0 |
| Marble | Earth and stone | Underground, Y 3-22 | - | - | 4/0/1/0/0 |
| Granite | Earth and stone | Underground, Y 3-28 | - | - | 4/1/0/0/0 |
| Obsidian | Earth and stone | Underground, Y 2-12 | - | - | 2/3/0/0/1 |
| Quartz | Gems | Underground, Y 5-28 | - | - | 2/0/2/0/0 |
| Amethyst | Gems | Underground, Y 3-18 | - | - | 2/0/1/0/2 |
| Sapphire Ore | Gems | Underground, Y 3-14 | - | - | 2/0/4/0/0 |
| Ruby Ore | Gems | Underground, Y 3-14 | - | - | 2/4/0/0/0 |
| Mithril Ore | Arcane | Underground, Y 2-10 | - | - | 3/0/2/1/1 |
| Moonstone | Arcane | Underground, Y 2-12 | - | - | 1/0/3/0/3 |
| Amber | Gems | Underground, Y 8-28 | - | - | 1/1/0/3/0 |
| Peat | Plants | Wet ground near sea level | - | - | 1/1/1/0/2 |
| Reeds | Plants | Wet ground near sea level | - | - | 0/0/2/2/0 |
| Flax | Plants | Open grassland | - | - | 0/0/1/3/0 |
| Wild Herbs | Plants | Open grassland | - | - | 0/0/1/2/1 |
| Iron | Metals | Elemental crafting only | E3 -> F2 | 2 | 3/2/0/0/0 |
| Copper | Metals | Elemental crafting only | E2 -> F2 | 2 | 2/2/0/0/0 |
| Tin | Metals | Elemental crafting only | E2 -> W1 -> F1 | 4 | 2/1/1/0/0 |
| Silver | Metals | Elemental crafting only | E2 -> W2 -> F1 | 4 | 2/1/2/0/0 |
| Gold | Metals | Elemental crafting only | E3 -> F3 | 2 | 3/3/0/0/0 |
| Steel | Metals | Elemental crafting only | E4 -> F3 -> D1 | 4 | 4/3/0/0/1 |
| Bronze | Metals | Elemental crafting only | E3 -> F2 -> W1 | 4 | 3/2/1/0/0 |
| Mithril | Arcane | Elemental crafting only | E4 -> W3 -> L2 | 4 | 4/0/3/2/0 |
| Diamond | Gems | Elemental crafting only | E4 -> W3 -> F2 | 4 | 4/2/3/0/0 |
| Emerald | Gems | Elemental crafting only | E3 -> L3 -> W2 | 4 | 3/0/2/3/0 |
| Ruby | Gems | Elemental crafting only | E3 -> F4 | 2 | 3/4/0/0/0 |
| Sapphire | Gems | Elemental crafting only | E3 -> W4 | 2 | 3/0/4/0/0 |
| Glass | Construction | Elemental crafting only | E1 -> F2 | 2 | 1/2/0/0/0 |
| Charcoal | Plants | Elemental crafting only | L2 -> F1 | 2 | 0/1/0/2/0 |
| Ash | Minerals | Elemental crafting only | F1 -> D1 | 2 | 0/1/0/0/1 |
| Lime | Construction | Elemental crafting only | E2 -> F1 -> W1 | 4 | 2/1/1/0/0 |
| Mortar | Construction | Elemental crafting only | E3 -> W2 | 2 | 3/0/2/0/0 |
| Ceramic | Construction | Elemental crafting only | E3 -> W1 -> F2 | 4 | 3/2/1/0/0 |
| Planks | Construction | Elemental crafting only | L2 -> E1 | 2 | 1/0/0/2/0 |
| Wood Pulp | Plants | Elemental crafting only | L1 -> W2 | 2 | 0/0/2/1/0 |
| Plant Fiber | Plants | Elemental crafting only | L2 -> W1 | 2 | 0/0/1/2/0 |
| Cloth | Construction | Elemental crafting only | L3 -> W2 | 2 | 0/0/2/3/0 |
| Resin | Plants | Elemental crafting only | L3 -> F1 | 2 | 0/1/0/3/0 |
| Crystal Dust | Arcane | Elemental crafting only | E2 -> W2 -> D1 | 4 | 2/0/2/0/1 |
| Enchanted Glass | Arcane | Elemental crafting only | W3 -> F2 -> L2 | 4 | 0/2/3/2/0 |
| Moon Silver | Arcane | Elemental crafting only | E3 -> W3 -> D3 | 4 | 3/0/3/0/3 |
| Runestone | Arcane | Elemental crafting only | E4 -> L2 -> D2 | 4 | 4/0/0/2/2 |
| Fern | Plants | Shaded ground beneath tree canopies | - | - | 0/0/1/2/0 |
| Clover | Plants | Open grassy meadows | - | - | 0/0/0/2/0 |
| Lavender | Plants | Open grassy meadows | - | - | 0/0/1/2/0 |
| Red Poppy | Plants | Open grassy meadows | - | - | 0/1/0/2/0 |
| Bluebell | Plants | Shaded ground beneath tree canopies | - | - | 0/0/2/1/0 |
| Cattail | Plants | Wet ground near sea level | - | - | 0/0/2/2/0 |
| Brown Mushroom | Plants | Shaded ground beneath tree canopies | - | - | 0/0/1/1/1 |
| Glowcap | Plants | Shaded ground beneath tree canopies | - | - | 0/0/1/1/2 |
| Thorn Bush | Plants | Open grassy meadows | - | - | 1/0/0/2/0 |
| Dry Shrub | Plants | Dry upland grassland | - | - | 0/1/0/1/1 |

## Natural distribution and compatibility

Deposit parameters live in `data/resources.json`; the generator emits `src/voxel/resource_veins.rs`. Ore veins (including coal) get twice the base attempts and two extra random-walk steps. They replace underground stone or buried soil; other mineral deposits replace only stone. All deposits stay within their configured height bands, preserve bedrock, and use deterministic seed/position salts. Surface clay, peat and mud favor elevations near sea level. Reeds favor that same wet ground; flax and herbs occupy open grassland. Decorations only replace air above solid ground, preserving trees and existing plants. Existing ore families and four tree species remain.

The automated survey samples **432 chunks across seeds 7, 42 and 2026**, spanning positive and negative coordinates. It finds every naturally available resource and no crafting-only material. Sample block totals: iron ore 12,331, coal 13,711, mithril ore 1,243, moonstone 462, peat 1,594 and flax 911. Iron and coal are about three times as frequent as before this ground-cover expansion (previously 4,025 and 4,492). Basic stone remains over half of solid terrain. Full survey output is `target/resource-distribution.csv`; these are deterministic sample counts, not guaranteed yields for every world or spawn point. Underground access and player pacing still warrant playtesting.

Legacy enum values and the first 23 inventory positions are unchanged. Old 23-slot and 72-slot JSON inventory arrays are padded with zeroes for the new slots; host and guest accounts retain their counts. Existing saved block edits remain. Unedited terrain is regenerated from the seed, so loading an older world exposes the new deposits and plants even in previously visited terrain. Back up an old save if its exact unedited landscape matters. Network protocol is now **5**: all Steam/Direct participants need the new build.

Gold, diamond and emerald ore extraction yields were increased to better reward their rarity and hardness. Grass now yields Earth and Life; grass tufts and grass terrain have distinct display names. Stone now uses Earth 1 -> Water 1; pumpkin uses Earth 1 -> Water 1 -> Life 2. Redstone and pumpkin are classified as resources, with no new item system. Water remains nonstackable; use leaves, reeds or rock salt for extractable Water.

## Textures and rebuilding

All **94 resource source PNGs** are now drawn by local procedural generators, including the unused placeholder. The live atlas contains **93 resource face tiles plus the synthetic white tile**. The provenance audit identified 62 previously generated active tiles and 30 legacy active tiles; the legacy art was replaced from scratch, and all locally generated art was regenerated too. Separate log bark/end-grain, basalt faces, grass sides/soil bottom and pumpkin top/side/underside are preserved or corrected. Leaves and plants retain cutout transparency. No external image files are inputs to the generators. Glass remains an opaque patterned material block rather than a window pane.

See [texture inventory](textures/RESOURCE_TEXTURES.md). `target/resource-textures.png` is the generated contact sheet. All new texture art is generated locally from deterministic drawing code; it has no external asset dependency.

Resource authoring uses `data/resources.json`; base terrain and multi-face texture specifications use `data/base_textures.json`. Resource edits generate crafting entries while preserving existing creature entries:

```powershell
python tools/build_resources.py
python tools/build_atlas.py
python tools/audit_resource_textures.py
python tools/gen_world_api.py
cargo test --offline
cargo test --offline --features steam
.\tools\build_steam.ps1
```

The generator checks catalog/formula uniqueness, 2-3 resource slots, source classifications and extraction conservation. Rust tests also exercise actual craft/extract transactions for every formula, iron progression from raw resources, old/new account serialization, network registry size, terrain availability, crafting-only exclusions, bedrock and deterministic generation. The Steam-enabled development build retains the existing Direct option. Custom runtime edits to `data/crafting.json` apply on restart but will be replaced for resources on the next catalog regeneration. If adding resource types later, append enum/inventory entries before regeneration; never reorder existing positions.

Verification on 2026-09-09: 263 automated tests passed in each of the Direct and Steam builds; the opt-in GPU preview check passed for Generic/Fantasy settings, crafting and resource icons. The Steam development executable and DLL were rebuilt. Clippy completes with warnings, including pre-existing project warnings and the larger inline network packet variants; strict lint cleanliness is not claimed. Multi-account Steam gameplay was not retested for this resource expansion.

## Ground-cover expansion

Ten additional natural resources provide ground-level visual variety: fern, clover, lavender, red poppy, bluebell, cattail, brown mushroom, glowcap, thorn bush and dry shrub. Each has a distinct pixel texture, extraction composition and inventory entry. Flowers and shrubs favor meadow/upland terrain, cattails favor wet ground, and ferns, bluebells and mushrooms favor tree shade. Coarse deterministic patches and local placement rolls create clumps; existing terrain decorations are preserved. Glowcaps have gentle self-emission. These resources are gathered and extracted, with no new crafting formulas or consumable-item effects.

The same 432-chunk survey found all ten additions, including 238 glowcaps and 1,245 dry shrubs. Ore coverage now includes the buried soil stratum while leaving surface blocks and bedrock intact. Both Direct and Steam participants must update to protocol 5.

Landscape generation now also includes rivers, lakes and rocky mountain tops. See [terrain notes](TERRAIN.md). The latest distribution counts above include these features.
