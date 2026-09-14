# Resource texture provenance and face map

Every resource texture is generated from drawing primitives in `tools/build_resources.py` and `tools/build_base_textures.py`. No external image is read by either renderer. Rebuild all source tiles with `python tools/build_resources.py`, then pack them with `python tools/build_atlas.py`.

The initial audit found 62 active tiles matching our local generator, 30 active legacy tiles of unverified origin, and one unused legacy placeholder. All legacy tiles were replaced from scratch; every previously generated tile was regenerated too. A dedicated pumpkin underside was added and grass now uses soil on its bottom face. Logs retain distinct bark sides and species-specific end grain on both cut faces. Basalt retains separate side and top tiles.

`texture_audit_before.json` records original hashes and classifications without retaining old images. `texture_provenance.json` records current hashes and generator inputs. `python tools/audit_resource_textures.py` checks exact reproducibility, all face fallbacks, source coverage and atlas pixels. The atlas builder refuses unverified or modified inputs.

The unused legacy atlas backup is overwritten with the new atlas when present. `assets/textures/shadow_zoom.png` is a diagnostic screenshot, not a resource texture or runtime asset; creature/model art is outside this resource-texture audit.

## Resolved faces

| Block | Top | Side | Bottom |
|---|---|---|---|
| grass | [grass.png](grass.png) | [grass_block_side.png](grass_block_side.png) | [soil.png](soil.png) |
| sand | [sand.png](sand.png) | [sand.png](sand.png) | [sand.png](sand.png) |
| spruce_wood | [spruce_log_top.png](spruce_log_top.png) | [spruce_log.png](spruce_log.png) | [spruce_log_top.png](spruce_log_top.png) |
| stone | [stone.png](stone.png) | [stone.png](stone.png) | [stone.png](stone.png) |
| water | [water.png](water.png) | [water.png](water.png) | [water.png](water.png) |
| oak_leaves | [oak_leaves.png](oak_leaves.png) | [oak_leaves.png](oak_leaves.png) | [oak_leaves.png](oak_leaves.png) |
| soil | [soil.png](soil.png) | [soil.png](soil.png) | [soil.png](soil.png) |
| bedrock | [bedrock.png](bedrock.png) | [bedrock.png](bedrock.png) | [bedrock.png](bedrock.png) |
| cobblestone | [cobblestone.png](cobblestone.png) | [cobblestone.png](cobblestone.png) | [cobblestone.png](cobblestone.png) |
| basalt | [basalt_top.png](basalt_top.png) | [basalt_side.png](basalt_side.png) | [basalt_top.png](basalt_top.png) |
| cherry_wood | [cherry_log_top.png](cherry_log_top.png) | [cherry_log.png](cherry_log.png) | [cherry_log_top.png](cherry_log_top.png) |
| cherry_leaves | [cherry_leaves.png](cherry_leaves.png) | [cherry_leaves.png](cherry_leaves.png) | [cherry_leaves.png](cherry_leaves.png) |
| birch_wood | [birch_log_top.png](birch_log_top.png) | [birch_log.png](birch_log.png) | [birch_log_top.png](birch_log_top.png) |
| birch_leaves | [birch_leaves.png](birch_leaves.png) | [birch_leaves.png](birch_leaves.png) | [birch_leaves.png](birch_leaves.png) |
| gold_ore | [gold_ore.png](gold_ore.png) | [gold_ore.png](gold_ore.png) | [gold_ore.png](gold_ore.png) |
| diamond_ore | [diamond_ore.png](diamond_ore.png) | [diamond_ore.png](diamond_ore.png) | [diamond_ore.png](diamond_ore.png) |
| emerald_ore | [emerald_ore.png](emerald_ore.png) | [emerald_ore.png](emerald_ore.png) | [emerald_ore.png](emerald_ore.png) |
| copper_ore | [copper_ore.png](copper_ore.png) | [copper_ore.png](copper_ore.png) | [copper_ore.png](copper_ore.png) |
| oak_wood | [oak_log_top.png](oak_log_top.png) | [oak_log.png](oak_log.png) | [oak_log_top.png](oak_log_top.png) |
| spruce_leaves | [spruce_leaves.png](spruce_leaves.png) | [spruce_leaves.png](spruce_leaves.png) | [spruce_leaves.png](spruce_leaves.png) |
| pumpkin | [pumpkin_top.png](pumpkin_top.png) | [pumpkin_side.png](pumpkin_side.png) | [pumpkin_bottom.png](pumpkin_bottom.png) |
| bricks | [bricks.png](bricks.png) | [bricks.png](bricks.png) | [bricks.png](bricks.png) |
| short_grass | [short_grass.png](short_grass.png) | [short_grass.png](short_grass.png) | [short_grass.png](short_grass.png) |
| iron_ore | [iron_ore.png](iron_ore.png) | [iron_ore.png](iron_ore.png) | [iron_ore.png](iron_ore.png) |
| tin_ore | [tin_ore.png](tin_ore.png) | [tin_ore.png](tin_ore.png) | [tin_ore.png](tin_ore.png) |
| silver_ore | [silver_ore.png](silver_ore.png) | [silver_ore.png](silver_ore.png) | [silver_ore.png](silver_ore.png) |
| coal | [coal.png](coal.png) | [coal.png](coal.png) | [coal.png](coal.png) |
| sulfur | [sulfur.png](sulfur.png) | [sulfur.png](sulfur.png) | [sulfur.png](sulfur.png) |
| rock_salt | [rock_salt.png](rock_salt.png) | [rock_salt.png](rock_salt.png) | [rock_salt.png](rock_salt.png) |
| clay | [clay.png](clay.png) | [clay.png](clay.png) | [clay.png](clay.png) |
| limestone | [limestone.png](limestone.png) | [limestone.png](limestone.png) | [limestone.png](limestone.png) |
| marble | [marble.png](marble.png) | [marble.png](marble.png) | [marble.png](marble.png) |
| granite | [granite.png](granite.png) | [granite.png](granite.png) | [granite.png](granite.png) |
| obsidian | [obsidian.png](obsidian.png) | [obsidian.png](obsidian.png) | [obsidian.png](obsidian.png) |
| quartz | [quartz.png](quartz.png) | [quartz.png](quartz.png) | [quartz.png](quartz.png) |
| amethyst | [amethyst.png](amethyst.png) | [amethyst.png](amethyst.png) | [amethyst.png](amethyst.png) |
| sapphire_ore | [sapphire_ore.png](sapphire_ore.png) | [sapphire_ore.png](sapphire_ore.png) | [sapphire_ore.png](sapphire_ore.png) |
| ruby_ore | [ruby_ore.png](ruby_ore.png) | [ruby_ore.png](ruby_ore.png) | [ruby_ore.png](ruby_ore.png) |
| mithril_ore | [mithril_ore.png](mithril_ore.png) | [mithril_ore.png](mithril_ore.png) | [mithril_ore.png](mithril_ore.png) |
| moonstone | [moonstone.png](moonstone.png) | [moonstone.png](moonstone.png) | [moonstone.png](moonstone.png) |
| amber | [amber.png](amber.png) | [amber.png](amber.png) | [amber.png](amber.png) |
| peat | [peat.png](peat.png) | [peat.png](peat.png) | [peat.png](peat.png) |
| reeds | [reeds.png](reeds.png) | [reeds.png](reeds.png) | [reeds.png](reeds.png) |
| flax | [flax.png](flax.png) | [flax.png](flax.png) | [flax.png](flax.png) |
| wild_herbs | [wild_herbs.png](wild_herbs.png) | [wild_herbs.png](wild_herbs.png) | [wild_herbs.png](wild_herbs.png) |
| iron | [iron.png](iron.png) | [iron.png](iron.png) | [iron.png](iron.png) |
| copper | [copper.png](copper.png) | [copper.png](copper.png) | [copper.png](copper.png) |
| tin | [tin.png](tin.png) | [tin.png](tin.png) | [tin.png](tin.png) |
| silver | [silver.png](silver.png) | [silver.png](silver.png) | [silver.png](silver.png) |
| gold | [gold.png](gold.png) | [gold.png](gold.png) | [gold.png](gold.png) |
| steel | [steel.png](steel.png) | [steel.png](steel.png) | [steel.png](steel.png) |
| bronze | [bronze.png](bronze.png) | [bronze.png](bronze.png) | [bronze.png](bronze.png) |
| mithril | [mithril.png](mithril.png) | [mithril.png](mithril.png) | [mithril.png](mithril.png) |
| diamond | [diamond.png](diamond.png) | [diamond.png](diamond.png) | [diamond.png](diamond.png) |
| emerald | [emerald.png](emerald.png) | [emerald.png](emerald.png) | [emerald.png](emerald.png) |
| ruby | [ruby.png](ruby.png) | [ruby.png](ruby.png) | [ruby.png](ruby.png) |
| sapphire | [sapphire.png](sapphire.png) | [sapphire.png](sapphire.png) | [sapphire.png](sapphire.png) |
| glass | [glass.png](glass.png) | [glass.png](glass.png) | [glass.png](glass.png) |
| charcoal | [charcoal.png](charcoal.png) | [charcoal.png](charcoal.png) | [charcoal.png](charcoal.png) |
| ash | [ash.png](ash.png) | [ash.png](ash.png) | [ash.png](ash.png) |
| lime | [lime.png](lime.png) | [lime.png](lime.png) | [lime.png](lime.png) |
| mortar | [mortar.png](mortar.png) | [mortar.png](mortar.png) | [mortar.png](mortar.png) |
| ceramic | [ceramic.png](ceramic.png) | [ceramic.png](ceramic.png) | [ceramic.png](ceramic.png) |
| planks | [planks.png](planks.png) | [planks.png](planks.png) | [planks.png](planks.png) |
| wood_pulp | [wood_pulp.png](wood_pulp.png) | [wood_pulp.png](wood_pulp.png) | [wood_pulp.png](wood_pulp.png) |
| plant_fiber | [plant_fiber.png](plant_fiber.png) | [plant_fiber.png](plant_fiber.png) | [plant_fiber.png](plant_fiber.png) |
| cloth | [cloth.png](cloth.png) | [cloth.png](cloth.png) | [cloth.png](cloth.png) |
| resin | [resin.png](resin.png) | [resin.png](resin.png) | [resin.png](resin.png) |
| crystal_dust | [crystal_dust.png](crystal_dust.png) | [crystal_dust.png](crystal_dust.png) | [crystal_dust.png](crystal_dust.png) |
| enchanted_glass | [enchanted_glass.png](enchanted_glass.png) | [enchanted_glass.png](enchanted_glass.png) | [enchanted_glass.png](enchanted_glass.png) |
| moon_silver | [moon_silver.png](moon_silver.png) | [moon_silver.png](moon_silver.png) | [moon_silver.png](moon_silver.png) |
| runestone | [runestone.png](runestone.png) | [runestone.png](runestone.png) | [runestone.png](runestone.png) |
| fern | [fern.png](fern.png) | [fern.png](fern.png) | [fern.png](fern.png) |
| clover | [clover.png](clover.png) | [clover.png](clover.png) | [clover.png](clover.png) |
| lavender | [lavender.png](lavender.png) | [lavender.png](lavender.png) | [lavender.png](lavender.png) |
| red_poppy | [red_poppy.png](red_poppy.png) | [red_poppy.png](red_poppy.png) | [red_poppy.png](red_poppy.png) |
| bluebell | [bluebell.png](bluebell.png) | [bluebell.png](bluebell.png) | [bluebell.png](bluebell.png) |
| cattail | [cattail.png](cattail.png) | [cattail.png](cattail.png) | [cattail.png](cattail.png) |
| brown_mushroom | [brown_mushroom.png](brown_mushroom.png) | [brown_mushroom.png](brown_mushroom.png) | [brown_mushroom.png](brown_mushroom.png) |
| glowcap | [glowcap.png](glowcap.png) | [glowcap.png](glowcap.png) | [glowcap.png](glowcap.png) |
| thorn_bush | [thorn_bush.png](thorn_bush.png) | [thorn_bush.png](thorn_bush.png) | [thorn_bush.png](thorn_bush.png) |
| dry_shrub | [dry_shrub.png](dry_shrub.png) | [dry_shrub.png](dry_shrub.png) | [dry_shrub.png](dry_shrub.png) |
| snow | [snow.png](snow.png) | [snow.png](snow.png) | [snow.png](snow.png) |
| meat | [meat.png](meat.png) | [meat.png](meat.png) | [meat.png](meat.png) |
| cooked_meat | [cooked_meat.png](cooked_meat.png) | [cooked_meat.png](cooked_meat.png) | [cooked_meat.png](cooked_meat.png) |
| mud | mud.png | mud.png | mud.png |
| redstone | redstone.png | redstone.png | redstone.png |
| crystal | crystal.png | crystal.png | crystal.png |

All 97 source PNGs have local generation provenance, including the unused placeholder. Air/entities use the synthetic white atlas tile. No required resource texture is missing.

## Source tiles

| Texture | Origin |
|---|---|
| [amber.png](amber.png) | Local procedural generation |
| [amethyst.png](amethyst.png) | Local procedural generation |
| [ash.png](ash.png) | Local procedural generation |
| [basalt_side.png](basalt_side.png) | Local procedural generation |
| [basalt_top.png](basalt_top.png) | Local procedural generation |
| [bedrock.png](bedrock.png) | Local procedural generation |
| [birch_leaves.png](birch_leaves.png) | Local procedural generation |
| [birch_log.png](birch_log.png) | Local procedural generation |
| [birch_log_top.png](birch_log_top.png) | Local procedural generation |
| [bluebell.png](bluebell.png) | Local procedural generation |
| [bricks.png](bricks.png) | Local procedural generation |
| [bronze.png](bronze.png) | Local procedural generation |
| [brown_mushroom.png](brown_mushroom.png) | Local procedural generation |
| [cattail.png](cattail.png) | Local procedural generation |
| [ceramic.png](ceramic.png) | Local procedural generation |
| [charcoal.png](charcoal.png) | Local procedural generation |
| [cherry_leaves.png](cherry_leaves.png) | Local procedural generation |
| [cherry_log.png](cherry_log.png) | Local procedural generation |
| [cherry_log_top.png](cherry_log_top.png) | Local procedural generation |
| [clay.png](clay.png) | Local procedural generation |
| [cloth.png](cloth.png) | Local procedural generation |
| [clover.png](clover.png) | Local procedural generation |
| [coal.png](coal.png) | Local procedural generation |
| [cobblestone.png](cobblestone.png) | Local procedural generation |
| [cooked_meat.png](cooked_meat.png) | Local procedural generation |
| [copper.png](copper.png) | Local procedural generation |
| [copper_ore.png](copper_ore.png) | Local procedural generation |
| [crystal.png](crystal.png) | Local procedural generation |
| [crystal_dust.png](crystal_dust.png) | Local procedural generation |
| [diamond.png](diamond.png) | Local procedural generation |
| [diamond_ore.png](diamond_ore.png) | Local procedural generation |
| [dry_shrub.png](dry_shrub.png) | Local procedural generation |
| [emerald.png](emerald.png) | Local procedural generation |
| [emerald_ore.png](emerald_ore.png) | Local procedural generation |
| [enchanted_glass.png](enchanted_glass.png) | Local procedural generation |
| [fern.png](fern.png) | Local procedural generation |
| [flax.png](flax.png) | Local procedural generation |
| [glass.png](glass.png) | Local procedural generation |
| [glowcap.png](glowcap.png) | Local procedural generation |
| [gold.png](gold.png) | Local procedural generation |
| [gold_ore.png](gold_ore.png) | Local procedural generation |
| [granite.png](granite.png) | Local procedural generation |
| [grass.png](grass.png) | Local procedural generation |
| [grass_block_side.png](grass_block_side.png) | Local procedural generation |
| [iron.png](iron.png) | Local procedural generation |
| [iron_ore.png](iron_ore.png) | Local procedural generation |
| [lavender.png](lavender.png) | Local procedural generation |
| [lime.png](lime.png) | Local procedural generation |
| [limestone.png](limestone.png) | Local procedural generation |
| [marble.png](marble.png) | Local procedural generation |
| [meat.png](meat.png) | Local procedural generation |
| [mithril.png](mithril.png) | Local procedural generation |
| [mithril_ore.png](mithril_ore.png) | Local procedural generation |
| [moon_silver.png](moon_silver.png) | Local procedural generation |
| [moonstone.png](moonstone.png) | Local procedural generation |
| [mortar.png](mortar.png) | Local procedural generation |
| [mud.png](mud.png) | Local procedural generation |
| [oak_leaves.png](oak_leaves.png) | Local procedural generation |
| [oak_log.png](oak_log.png) | Local procedural generation |
| [oak_log_top.png](oak_log_top.png) | Local procedural generation |
| [obsidian.png](obsidian.png) | Local procedural generation |
| [peat.png](peat.png) | Local procedural generation |
| [placeholder.png](placeholder.png) | Local procedural generation |
| [planks.png](planks.png) | Local procedural generation |
| [plant_fiber.png](plant_fiber.png) | Local procedural generation |
| [pumpkin_bottom.png](pumpkin_bottom.png) | Local procedural generation |
| [pumpkin_side.png](pumpkin_side.png) | Local procedural generation |
| [pumpkin_top.png](pumpkin_top.png) | Local procedural generation |
| [quartz.png](quartz.png) | Local procedural generation |
| [red_poppy.png](red_poppy.png) | Local procedural generation |
| [redstone.png](redstone.png) | Local procedural generation |
| [reeds.png](reeds.png) | Local procedural generation |
| [resin.png](resin.png) | Local procedural generation |
| [rock_salt.png](rock_salt.png) | Local procedural generation |
| [ruby.png](ruby.png) | Local procedural generation |
| [ruby_ore.png](ruby_ore.png) | Local procedural generation |
| [runestone.png](runestone.png) | Local procedural generation |
| [sand.png](sand.png) | Local procedural generation |
| [sapphire.png](sapphire.png) | Local procedural generation |
| [sapphire_ore.png](sapphire_ore.png) | Local procedural generation |
| [short_grass.png](short_grass.png) | Local procedural generation |
| [silver.png](silver.png) | Local procedural generation |
| [silver_ore.png](silver_ore.png) | Local procedural generation |
| [snow.png](snow.png) | Local procedural generation |
| [soil.png](soil.png) | Local procedural generation |
| [spruce_leaves.png](spruce_leaves.png) | Local procedural generation |
| [spruce_log.png](spruce_log.png) | Local procedural generation |
| [spruce_log_top.png](spruce_log_top.png) | Local procedural generation |
| [steel.png](steel.png) | Local procedural generation |
| [stone.png](stone.png) | Local procedural generation |
| [sulfur.png](sulfur.png) | Local procedural generation |
| [thorn_bush.png](thorn_bush.png) | Local procedural generation |
| [tin.png](tin.png) | Local procedural generation |
| [tin_ore.png](tin_ore.png) | Local procedural generation |
| [water.png](water.png) | Local procedural generation |
| [wild_herbs.png](wild_herbs.png) | Local procedural generation |
| [wood_pulp.png](wood_pulp.png) | Local procedural generation |
