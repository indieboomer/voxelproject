# Voxel Project documentation

Voxel Project is a Rust voxel sandbox with a custom wgpu renderer, authoritative
multiplayer simulation, and local-AI-assisted sandboxed Lua rules.

| Guide | Contents |
| --- | --- |
| [Project state](project.md) | Implemented systems, architecture, remaining acceptance work |
| [Prompting](prompting.md) | World API 1.22, examples, transactions and limits |
| [Campfires](campfires.md) | Generation, visuals, light, persistence, API |
| [Fish](fish.md) | Habitat, swimming, spawning and rule control |
| [Wildlife](wildlife.md) | Creature visibility, exploration population, recycling and limits |
| [Creature loot](creature-loot.md) | Death puffs, player-kill rewards and ground pickups |
| [Player names](player-names.md) | Settings, AI suggestions and multiplayer name tags |
| [Inventory](inventory.md) | Elements, equipment, resources and numbered hotbar assignment |
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
