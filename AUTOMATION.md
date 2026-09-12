Implement an initial magical production and automation system for Voxel Project, integrated with the existing Rust voxel engine, host-authoritative multiplayer, persistence, crafting, and five elements: Earth, Fire, Water, Life, and Death.

The visual style is low-poly, between Minecraft and World of Warcraft, with Slavic fantasy influences. Machines should resemble objects from a folk mage’s workshop: carved wood, ceramic vessels, copper rings, runestones, and captured sparks.

Each device occupies one voxel cell but may have a non-cubic mesh contained within that cell. Inspect the existing architecture first and reuse its systems where appropriate.

Connections

Implement three separate connection types:

Mana: energy used to operate devices.
Matter: discrete elements, resources, and items.
Signal: boolean states, pulses, or numeric values.

Ports sit at the centers of selected cell faces and rotate with the device. Connections require compatible, opposing ports. Adjacent compatible devices can transfer directly without an intermediate connector.

Keep personal player mana separate from network mana. Players may explicitly charge a network storage device.

Initial devices
Device	Behavior
Mana Collector	Slowly gathers ambient mana. Nearby collectors share a limited local supply, preventing unlimited output from dense placement.
Mana Vessel	Stores network mana. Supports a configurable reserve threshold.
Element Condenser	Converts mana into one selected element. Make conversion costs and production times configurable.
Element Dissipator	Consumes elements to recover mana, always yielding less than their production cost.
Mana Conduit	Transfers mana between compatible ports with limited throughput.
Matter Channel	Moves discrete items in a defined direction. Support straight, corner, and vertical connections.
Filter Splitter	Routes matter by item filter, round-robin distribution, or output priority.
Feeding Chest	Stores items and supplies or receives matter through ports. Exposes inventory counts for automation.
Formula Workshop	Automatically executes one selected existing crafting recipe, consuming ingredients, mana, and processing time.
Threshold Sensor	Measures a selected chest’s inventory or vessel’s mana and outputs a signal. Support separate activation and deactivation thresholds to prevent rapid toggling.
Controlled Valve	Enables or blocks mana or matter transfer based on a signal. Each valve handles one connection type.
Signal Connector	Carries and branches control signals between devices.

For the Formula Workshop, preserve the existing ordered five-slot recipe system and its mana costs. Use a shared matter input and internal ingredient buffers. The configured recipe determines ingredient order; arrival order must not change the result.

Use data-driven definitions for costs, capacity, throughput, processing duration, ports, and recipes. Avoid hardcoding balancing values throughout the implementation.

Simulation requirements
Run all production, transfers, and inventory mutations authoritatively on the host.
Use a deterministic fixed simulation tick, independent of rendering.
No LLM calls during production or network simulation.
Handle network changes when devices are placed, rotated, removed, or disconnected.
Define deterministic allocation when several consumers compete for limited energy or materials.
Prevent duplication or loss during transfers, device removal, and save/load.
A blocked output must pause production safely. Do not discard ingredients or repeatedly charge mana for the same cycle.
Persist device configuration, inventories, stored mana, and production progress.
Replicate relevant state to multiplayer clients.
Do not simulate offline production in this initial implementation.
Interaction and feedback

Provide placement previews showing orientation, occupied cell, and compatible ports.

Add a compact configuration panel for recipes, filters, thresholds, and enabled state. 

create any textures or models necessary. PRovide list of models that could be provided manually.

Display:

Current activity and production progress.
Stored materials and mana where relevant.
Connection directions.
Clear reasons for inactivity: disabled, missing ingredients, insufficient mana, or blocked output.

Devices should communicate activity through inexpensive visual feedback, such as moving sparks, rotating parts, and gate positions. Pair colors with symbols or motion.

Use simple temporary meshes if necessary, while keeping rendering separate from simulation so final GLB assets can replace them.

Balance constraints
Mana → element → mana must always lose energy.
Respect existing recipe costs rather than introducing a competing crafting economy.
Use production time, energy availability, and transport throughput as the initial constraints.
Do not add maintenance, random failures, temperature management, or equipment durability yet.
Scope boundaries

Leave automatic harvesting, mining, block placement, standalone logic gates, clocks, counters, and spell-triggering devices for later. Structure the device system so these can be added without redesigning the core.

Future spell devices should execute previously approved sandboxed actions, with mana costs and cooldowns, rather than generating code during simulation.

Acceptance criteria

Build and verify this complete production loop:

Mana Collector → Mana Vessel → Element Condenser → Formula Workshop → Feeding Chest

Use matter channels and mana conduits as needed. Configure a threshold sensor to stop production at an upper stock limit and resume below a lower limit.

Verify that:

Production consumes the correct ingredients and mana and produces the correct result.
Recipe ordering remains correct regardless of ingredient arrival order.
Full outputs pause safely and resume after space becomes available.
Competing consumers cannot duplicate energy or materials.
Rotation, removal, and reconnection update connectivity correctly.
Save/load preserves the installation’s state.
Multiplayer clients observe consistent host-authoritative results.
Conversion loops cannot generate free mana.

Implement the working system, not only a design document. Add focused tests for simulation correctness and report what was implemented, how it was verified, and any remaining limitations.

At the end update documentation and World API as you seem fit, so production/automation can work with prompting.