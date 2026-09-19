# Voxel Project documentation

Voxel Project is a Rust voxel sandbox with a custom wgpu renderer, authoritative
multiplayer simulation, and local-AI-assisted sandboxed Lua rules.

**Active plan:** [Targeted spells and persistent enchantments](../SPELLCASTING_PLAN.md).
Read [project state](project.md) for the implementation baseline and open acceptance
work. [Archived plans and reports](archive/README.md) retain historical material.

## Build and feature references

| Guide | Contents |
| --- | --- |
| [Spellcasting implementation](SPELLCASTING.md) | Targeted casts, persistent host Spellbook, validation and pending stages |
| [Spell artwork and cards](SPELL_ART.md) | Live icon compositions, card collection, image persistence and multiplayer |
| [Enchantment spells](ENCHANTMENTS.md) | Reusable templates, aimed hotbar casts, persistent instances and target-loss behavior |
| [Windows packaging](../PACKAGING.md) / [macOS](../macos/README.md) | Build and distribute playable bundles |
| [Steam and Direct multiplayer](STEAM_MULTIPLAYER.md) | Hosting, joining, identity, compatibility and open manual checks |
| [Crafting](CRAFTING.md) / [Resources](RESOURCES.md) | Ordered formulas, catalog, authoring and save behavior |
| [Automation](AUTOMATION_GUIDE.md) | Devices, connections, production and World API |
| [Prompt pipeline](PROMPT_PIPELINE.md) | Interpretation, validation, review, guest proposals and measured latency |
| [World generation](WORLD_GENERATION.md) / [Terrain](TERRAIN.md) | Prompted presets, geography and underwater presentation |
| [UI and settings](UI_SETTINGS.md) | Themes, preferences and targeting feedback |
| [Player animations and chat](PLAYER_ANIMATIONS.md) | Gestures, held items, bubbles and emoticons |
| [Adventure](ADVENTURE_GUIDE.md) / [Storage and underground](STORAGE_AND_UNDERGROUND.md) | Camp progression, exploration, chests and saves |
| [Development playtesting](PLAYTESTING_PLAN.md) | Development-only agent tooling and unfinished acceptance gates |

## System guides

| Guide | Contents |
| --- | --- |
| [Project state](project.md) | Implemented systems, architecture, remaining acceptance work |
| [Prompting](prompting.md) | World API usage, examples, transactions and limits |
| [Campfires](campfires.md) | Generation, visuals, light, persistence, API |
| [Fish](fish.md) | Habitat, swimming, spawning and rule control |
| [Wildlife](wildlife.md) | Creature visibility, exploration population, recycling and limits |
| [Creature loot](creature-loot.md) | Death puffs, player-kill rewards and ground pickups |
| [Player names](player-names.md) | Settings, AI suggestions and multiplayer name tags |
| [Inventory](inventory.md) | Elements, equipment, resources and numbered hotbar assignment |
| [Hunger and eating](HUNGER.md) | Gentle hunger timing, meals, damage floor, multiplayer and saves |
| [Equipment and recipe books](EQUIPMENT_AND_RECIPE_BOOKS.md) | 16 specialist tools, weapons, magic items and exploration charms; discoverable recipes |
| [Mana and recipes](mana.md) | Spell/rule costs, mana recovery and equipment decomposition |
| [Inventory scripting](inventory-api.md) | Transactional resource, equipment, element and mana APIs |
| [Water](water.md) | Currents, raised tributaries, waterfalls and audio |
| [Rendering](rendering.md) | Lighting, wetness, effects and performance checks |
| [Development and testing](development.md) | Build, generation tools and validation |

The complete API reference is generated from
[`world_api/schema.yaml`](../world_api/schema.yaml). Its
[reference](../world_api/world_api_readme.md),
[compact prompt context](../world_api/world_api_compact.md), and
[Lua completion stubs](../world_api/world_api_stubs.lua) are generated together.
Do not edit generated API files by hand. Feature guides describe current behavior;
they are not claims that every multiplayer acceptance scenario has been played
through in a packaged build.
