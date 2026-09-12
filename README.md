# Project Codename: Voxel Project

> A lightweight multiplayer voxel sandbox whose rules can be rewritten from inside the game using natural language.

To create a shareable Windows build with the AI model, llama server and installer,
run `make_package.bat`. See [PACKAGING.md](PACKAGING.md) for options and installation steps.

Separate Mac setup and `.app`/DMG packaging tools are in [macos/](macos/README.md).

New World accepts an optional AI terrain description, such as a sandy desert or
an island world. Blank descriptions keep the original terrain. See
[WORLD_GENERATION.md](WORLD_GENERATION.md) for supported options and behavior.

## MVP Goal

Build a small but playable first-person voxel game that proves one idea: a local LLM can translate a player's prompt into sandboxed game code, activate it at runtime, and change the same authoritative world for every connected player.

This is a side project and technical experiment. Prioritize a working end-to-end build over scale, content, polish, or production infrastructure.

## MVP Player Experience

- Start a new procedural world and enter it immediately, similar to Minecraft or Valheim.
- Play alone or host a session for up to four players.
- Multiplayer players receive a random model from `models/player/player1..4.glb` and a random hat from `hat1..4.glb` or no hat. The host assigns unique model/hat combinations among connected players and replicates them to everyone; appearances stay fixed until disconnect. Assets are embedded in the executable. All peers must use the same build (protocol 21).
- Hosts can enable guest prompting in Settings. Guests generate locally and submit disabled proposals for host review; only the host activates world rules.
- Move in first person, run, jump, collide with terrain, and respawn.
- Use comma to dance, period for angry, and slash for a jump gesture. Multiplayer characters animate movement, combat, and work, with held items attached to the right hand. Chat includes eight original emoticons, a picker, and overhead bubbles. See [PLAYER_ANIMATIONS.md](PLAYER_ANIMATIONS.md).
- Break, collect, select, and place voxel blocks.
- Explore one biome with terrain, water, trees, a day/night cycle, and 2–3 simple creatures.
- Open a prompt console and describe a new rule for the world.
- Let a local LLM generate a sandboxed game module using the documented World API.
- Review and activate the module; its effects run on the host/server and are replicated to all players.
- Disable, revise, delete, or revert generated rules.
- Save the world, active modules, original prompts, and module history.

Example target prompt:

> At night, sheep hunt players carrying a crystal, but they are afraid of red light.

## Architecture

Use a lightweight custom runtime rather than a general-purpose game engine.

- **Language:** Rust
- **Rendering:** `wgpu` with chunked voxel meshes, hidden-face culling, greedy meshing, fog, sunlight, simple shadows, and voxel ambient occlusion
- **Window/input:** `winit`
- **ECS:** `bevy_ecs` or `hecs`
- **UI/tools:** `egui`
- **Serialization:** `serde`
- **Networking:** server-authoritative listen server; QUIC via `quinn` is preferred
- **Scripting:** sandboxed Lua via `mlua` for the MVP
- **Local AI:** `llama.cpp` with a configurable GGUF coding model

Keep the renderer independent from simulation so the same executable can later run as a headless dedicated server.

## Runtime Rule Pipeline

1. The host submits a natural-language prompt.
2. The LLM receives the prompt, World API documentation, and relevant existing modules.
3. It generates a small Lua module, not native engine code.
4. The game validates syntax, allowed API calls, resource limits, and basic behavior.
5. The host reviews and activates the module.
6. The authoritative server executes it and replicates resulting state changes.
7. Errors or budget violations automatically disable the module.

Lua must not access files, networking, processes, native libraries, or unrestricted engine memory. Remove `os`, `io`, `package`, `require`, and `debug`. Enforce instruction, time, memory, spawn, and block-edit budgets.

## Initial World API

Expose a small, stable capability API rather than engine internals:

- World queries: time, weather, blocks, entities, nearby objects
- World actions: spawn entity, replace blocks, change weather, broadcast message
- Entity data: position, health, tags, inventory, state
- Entity actions: set target/state, add or remove component, damage, destroy
- Events: tick, day/night started, spawn, damage, death, block broken, interaction

Add API functions only when required by a concrete playable rule.

## Multiplayer Scope

- Single-player uses the same local authoritative server as multiplayer.
- One player hosts; up to three friends join by direct address or simple session code.
- The host owns the save and is initially the only player allowed to generate or activate rules.
- The host uses local AI. When guest prompting is enabled, guests use their own local AI to submit proposals. Joining without prompting does not require a model.
- The session ends when the host leaves.

No dedicated servers, host migration, accounts, matchmaking, public server browser, or anti-cheat in the MVP.

## Implementation Order

1. First-person controller and procedural voxel chunks.
2. Block breaking/placing, inventory hotbar, and world persistence.
3. Day/night cycle and simple ECS creatures.
4. Authoritative listen server with two players, then expand to four.
5. Sandboxed Lua modules written manually against the World API.
6. Module activation, budgets, errors, persistence, disable, and rollback.
7. Local LLM integration that generates the same modules.
8. Prompt/rule UI and multiplayer notifications.
9. Package a playable build and test with friends.

Do not integrate the LLM until a manually written Lua module can reliably modify the multiplayer world.

## Definition of Done

The MVP is complete when all of the following work in a packaged build:

- A player can create, enter, play, save, and reload a voxel world.
- One to four players can share the same host-authoritative session.
- A local LLM can generate at least three materially different rules without hard-coded handling for those prompts.
- Generated rules behave identically for all connected players.
- Rules survive save/reload and can be inspected, disabled, revised, and reverted.
- Invalid or runaway code cannot freeze the game or access the host system.

Reference acceptance tests:

1. At night, sheep hunt players carrying a crystal.
2. During rain, soil near trees becomes mud that slows players.
3. Three creature deaths in one location spawn a red stone that heals nearby monsters.

## Explicitly Out of Scope

Crafting depth, survival needs, quests, story, multiple biomes, procedural art or music, generated animations, large public servers, marketplace/workshop, monetization, production backend, and Everwind-level visuals.

Visual ambition may grow later. The MVP should look clean and coherent using simple textures, block-based creatures, good color, fog, lighting, and ambient occlusion. The product risk to validate is programmable world behavior, not content volume or graphical fidelity.

## Development Principle

Build the smallest playable vertical slice that tests the complete loop:

**enter world → play → describe a rule → generate code → validate → activate → observe multiplayer consequences → revise or revert**

Avoid general engine features unless they directly support this loop.

## Elemental crafting

Press **C** in game. See [CRAFTING.md](CRAFTING.md) for controls, starter formulas, compositions, configuration, multiplayer, and save behavior.

## UI appearance

Open **Settings** in the main menu or press **F10** in game to switch between **Generic** and **Fantasy**. Both use larger text. See [UI_SETTINGS.md](UI_SETTINGS.md) for the UI audit, controls, persistence, and extension guide.

## Steam multiplayer (development)

Optional Steam friends sessions use test App ID **480**. Direct / LAN remains available in Settings. See [Steam build, implementation plan and validation checklist](STEAM_MULTIPLAYER.md).

## Resource expansion

The game now has 82 stackable resources, natural deposits, short elemental formulas and pixel textures. See [resource catalog, balance and migration notes](RESOURCES.md).

## Landscape

Winding rivers connect large lakes, occasional mountains have rocky summits, and submerged views have a blue tint. See [terrain behavior and compatibility](TERRAIN.md).
