# Voxel Project: current state and development potential

**Assessment: 29 September 2026 · branch `experimental` · commit `b8586da`.**
Based on source, schemas, feature documentation and a fresh automated test run; no graphical, live-model or multi-PC playtest was performed for this review. Intended length: approximately four printed pages.

## Overall assessment

The project is a substantial playable prototype, considerably beyond the original MVP in breadth. It combines a custom voxel game, multiplayer authority, a bounded Lua runtime, local natural-language rule generation, reusable magic, automation and light adventure progression. Its distinctive opportunity is already present: players can invent behavior and make it part of a shared world.

It is **not yet a verified completion of the original MVP**. Packaged multiplayer acceptance, dependable generation of materially different rules, and general revision/reversion remain incomplete. More content has been implemented than the original scope required, while evidence that the central loop consistently works for new players is still limited.

Status meanings: **Implemented** = working implementation exists in the reviewed code, not a guarantee of release readiness; **Partially implemented** = usable foundations with material gaps or unresolved acceptance; **TODO** = the described capability or acceptance milestone remains outstanding.

## Feature assessment

| Area | Status | Current implementation and limits |
|---|---|---|
| Core voxel play | **Implemented** | First-person movement, collision, jumping, mining, collecting, placement, inventory/hotbar, health, oxygen and defeat recovery. |
| World and exploration | **Implemented** | Procedural chunks, terrain presets, mountains, rivers/lakes, vegetation, underground exploration, resources, campfires, map and waypoints. Existing saves carry generation compatibility information. |
| Rendering and atmosphere | **Implemented** | wgpu rendering, hidden-face culling, voxel AO, lighting/shadows, fog, weather, wet surfaces, clouds, water effects, animation and audio. This review does not establish visual quality or representative frame rates. |
| Greedy meshing and renderer separation | **Partially implemented** | Chunk meshing and several simulation modules are separated, but the current mesher emits individual visible faces; greedy merging is absent. `app.rs` still coordinates rendering, input, networking and simulation in roughly 7,000 nonblank lines. |
| Creatures and combat | **Implemented** | Passive wildlife, hostile creatures, dragons, fish, creature behaviors, damage, drops and temporary magical statuses. Considerably broader than the original 2–3-creature target. |
| Survival and adventure | **Implemented** | Crafting, equipment, mana, hunger/food/cooking, storage, camp rest, journal/contracts, quest NPCs and recipe/lore books. These establish activities but do not prove a satisfying progression curve. |
| Automation | **Implemented** | Mana/matter transport, storage, production, routing, sensors, valves and aura devices; authoritative simulation and saved production state. Bounded installations; no offline production or general harvesting/mining machines. |
| One-to-four-player networking | **Partially implemented** | Direct UDP and optional Steam transport, admission/version checks, state replication, inventory authority, reconnect handling and host-loss behavior exist. Uses UDP/Steam rather than the originally preferred QUIC. Separate-account internet gameplay acceptance remains open. |
| World persistence | **Implemented** | World edits, player/account state, creatures, devices, rules, prompts, Spellbook and attachments have save paths and compatibility handling. Arbitrary Lua globals are not saved. |
| Lua sandbox and World API | **Implemented** | Restricted libraries, instruction/time/memory/API/native-work limits, spawn/edit budgets, scheduling and callback transactions. Failed callbacks discard staged effects; failed rules are disabled. Current test instability needs investigation. |
| Local AI rule generation | **Partially implemented** | llama.cpp integration, bundled-server startup, interpreted plans, generic policy compilation, custom Lua generation, linting, smoke/behavior checks, repair and review UI exist. Live results are historical and selective; arbitrary prompt correctness and latency remain unproven. |
| Runtime rule management | **Partially implemented** | Review, activation, execution, disable/delete and persistence exist. Disabling stops future callbacks but does not reverse successful edits, spawns or damage. General revision history and world-effect rollback are absent. |
| Reusable targeted spells | **Implemented** | Saved Spellbook, stable IDs/revisions, cards, hotbar casting, mana/cooldowns, target checks, particles, host-permitted guest casting, slow/stun/push and other API actions. Casting reuses code without inference. Live multiplayer acceptance remains open. |
| Object enchantments | **Partially implemented** | Reusable templates attach independent saved rules to creatures, blocks and devices. Identity, replacement, unloading and ownership are handled. General effect ownership/cleanup, stacking/conflict rules, revision history and region enchantments remain unfinished. |
| Agentic Co-Play | **Implemented** | Optional host-only cloud authoring: inspection, sandboxed actions, rule revision, verification receipts, conversation/history persistence and capability backlog. Conflict-checked undo restores rule definitions only. Historical live checks exist; it is a separate workflow from local generation. |
| Scripted world authoring | **Implemented** | Recent APIs expose quest NPC/loot-bag creation and interactive beehive placement. Existing NPC quests are reused; a merchant NPC is not a general trading system. These are concrete capabilities, not unlimited content generation. |
| Distribution and testing | **Partially implemented** | Windows packaging/installer, AI and no-AI bundles, macOS packaging support, automated tests, GPU previews and optional agentic playtest/replay tools exist. Clean-machine acceptance, sustained multiplayer sessions and weaker-GPU profiling remain outstanding. |

Evidence anchors: [source layout and dependencies](Cargo.toml), [runtime](src/scripting.rs), [mesher](src/voxel/mesher.rs), [network protocol](src/net.rs), [saves](src/save.rs), [Spellcasting](docs/SPELLCASTING.md), [Enchantments](docs/ENCHANTMENTS.md), [Co-Play](docs/ASTRA_COPLAY.md), and [packaging](PACKAGING.md).

## Verification and original MVP gaps

Fresh command: `cargo test --offline --no-default-features --quiet -- --test-threads=1`.

**Result: 636 passed, 14 failed, 31 ignored.** Nine failures explicitly reported script time-budget exhaustion; five enchantment failures reported missing expected callback output/effects. Two representative failures—device enchantment and placement behavior validation—passed immediate isolated reruns. This indicates possible timing sensitivity, but does not establish the cause of every failure or justify calling the suite green. Diagnose runtime cost, budget accounting and test isolation before weakening limits. Steam, GPU, live inference and packaged gameplay were not exercised by this command.

The three original acceptance examples exist as hand-written modules with automated coverage: night-hunting sheep, rain-created mud, and a death-triggered red stone. Their existence does **not** prove that a local model can independently generate all three and sustain correct shared behavior through save/reload. The red-stone example also uses in-memory Lua death history, illustrating the persistence gap for script-local counters.

Remaining acceptance work:

- **TODO:** demonstrate three materially different, freshly generated local-model rules in a packaged host/guest session; verify intended behavior, failure handling and save/reload rather than generation success alone.
- **TODO:** repeat with four players, late joining, reconnects, conflicting actions and host departure, including separate Steam accounts and internet connections.
- **Partially implemented:** revision/revert. Co-Play has definition history/undo and spells have revision numbers; neither provides general reversal of committed world effects or a complete shared editing workflow.
- **TODO:** establish clean-install success, first-session usability, sustained performance and practical host hardware requirements.

Documentation has drifted: current code uses **protocol 51** and the schema is **World API 1.41.0**, while the project overview still lists 49/1.37.0. Historical passing counts describe earlier runs, not this checkout's current verification. The original scope excludes survival depth and quests, yet both now exist; reconcile the intended product scope explicitly.

## Game potential

**The strongest direction is a small cooperative magic sandbox:** explore, encounter a problem, invent a spell, share it, then enchant the environment. Targeting, physical spell cards and persistent attachments make programmable rules understandable as game actions. Generation happens between actions rather than on every cast, allowing responsive moment-to-moment play.

Existing survival, creatures and machines provide useful reasons to invent magic. Their value is greatest when they create situations with several possible solutions: protecting a camp, redirecting a dangerous creature, or controlling production during bad weather. Additional content that does not create such decisions risks making the game larger without strengthening its identity.

The principal design risk is that authoring bypasses the game. Powerful reviewed Lua can grant resources or make broad changes despite an apparently narrow target. Uniform spell fees and a target label do not constitute a balanced capability system. Creative authoring and progression-constrained play need clear expectations. Otherwise players may either trivialize challenges or feel misled by inconsistent restrictions.

Other risks are generation delays, semantic mistakes, irreversible consequences and interface complexity across Workshop, Spellbook, rules and Co-Play. Local inference also increases installation size and host hardware demands; cloud Co-Play introduces a separate online/cost dependency. No retention study or market research was performed, so commercial potential remains a hypothesis.

## Most promising next steps, in order

1. **Stabilize and prove one complete session.** Investigate the failing tests, then package a 30–60-minute two-player session with exploration, three invented spells/rules, an enchantment, a revision and reload/reconnect. Record time to first successful cast, generation failures, desyncs and frame stalls. Expand to four players after this works reliably.

2. **Finish owned effects and composition (Phase 2B).** Give persistent modifiers explicit ownership, predictable stacking and cleanup. Distinguish reversible enchantments from permanent edits; make target loss and disable behavior visible. Then add revision/history (2C) with clear boundaries around what undo actually restores. This directly increases players' confidence to experiment.

3. **Build a short cooperative discovery loop.** Use existing camps, creatures, mana and machines for a few authored situations that reward different player-created solutions. Introduce one spell and one enchantment early. Improve target feedback and explain generation failures in gameplay terms before adding species, biomes or survival systems.

4. **Develop enchanted automation selectively.** Conditional wards, creature-assisted work and weather-reactive devices combine existing systems into distinctive play. Add APIs only when repeated observed player requests require them; use the Co-Play backlog as evidence. Regions and large construction should follow demonstrated demand.

5. **Consolidate around measured needs.** Clarify the roles of local Workshop and cloud Co-Play, update stale docs and extract tightly scoped authority/casting/save responsibilities from `App`. Profile before investing in greedy meshing or broader renderer rewrites. Defer large RPG expansion, public servers and marketplace infrastructure.

The next milestone should demonstrate **reliable, understandable player-created magic with friends**. The project already has enough supporting systems to test whether that experience is compelling.
