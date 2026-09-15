> Archived 2026-09-15. Historical roadmap with an obsolete baseline and work order; spellcasting is the active product plan. Unfinished proposals remain historical, not completed.
> See [current documentation](../../SPELLCASTING_PLAN.md). Claims and measurements below describe the historical revision.

**World API and prompt-driven sandbox roadmap**

Prepared 2026-09-07 from the current working tree, including its uncommitted changes. This is an implementation plan, not a claim that the proposed capabilities exist. Prioritize the World API and the player creation loop; keep the existing Rust runtime, local LLM, Lua, and four-player listen-server scope.

**Implementation progress — 2026-09-07**

The first Phase 0 increment is implemented as World API v1.10.0:

- Shared execution guard for initialization and all five callbacks, with independent instruction and cooperative wall-time limits; protected Lua calls cannot suppress budget exhaustion.
- Source-size limit, shared API-call cap, broadcast count/byte limits, and numeric/coordinate validation before native API calls.
- Execution limits generated from the schema and consumed by Rust; corrected documentation of UDP ordering and nontransactional effects.
- Externally timed subprocess regressions for initialization/callback loops, protected calls, memory exhaustion, and API/broadcast limits; live API registry conformance checks.

Validation: 178 tests passed, 8 live-LLM tests ignored; all 13 sandbox subprocess cases passed. The development build compiles successfully.

The second increment, World API v1.11.0, adds callback transactions:

- Extracted Lua bindings/event dispatch into `src/script_api.rs` and staged effects into `src/script_transaction.rs`. Creature commands commit without cloning or modifying the live ECS during Lua execution.
- All five callbacks commit only after execution and final budget checks succeed. Failure discards block/player/inventory/creature effects, death events, broadcasts, time/weather changes, and tentative spawn IDs/randomness.
- Queries observe accepted writes and earlier committed callbacks in the host dispatch. Inventory operations reserve funds, including across modules, and match the real player's saturating arithmetic.
- Failed VMs are rebuilt before retry/reactivation; their Lua globals reset. No save format changes or persistent state facility are included yet.
- Added regression coverage for all callback types, runtime/instruction/memory failures, own-write queries, multiple-module ordering, death handlers, retry state, and applying inventory commands to a real player.

Validation for v1.11.0: 186 tests passed, 8 live-LLM tests ignored; development build successful. Multiplayer transport itself was not changed or playtested in this increment.

The third increment, World API v1.12.0, adds bounded scheduling:

- Shared per-dispatch callback, instruction, API-call, native-work and cooperative time budgets. Admission reserves full callback allowances, including VM initialization on retry.
- Round-robin recipient queues preserve accepted events without replay, coalesce pending ticks, cancel obsolete activations, and visibly report queue saturation. Limits are 512 pending callbacks total and 64 per module.
- Conservative native query/action work limits, time checkpoints inside block scans, and a 256-creature cap for World API spawning.
- A 64-loaded-module limit; overflow and invalid saved sources survive future saves. Runtime recipient IDs prevent queued events from following shifted module indices; these IDs are not yet durable save identities.
- Generated World API and prompt documentation describes deferred execution, current-state queries, saturation, and transient queues.

Validation for v1.12.0: 196 tests passed, 8 live-LLM tests ignored, all 14 externally timed sandbox subprocess cases passed; development build successful without warnings. Multiplayer playtesting and frame-time measurements were not performed.

Phase 0 remains in progress. Next work package: extract the authoritative simulation command boundary and measure scheduler/commit costs in a multiplayer session. Versioned saves, durable transaction/module IDs, explicit persistent Lua state, and user-facing undo/history remain outstanding. Callback transactions provide failure rollback, not atomic network delivery or reversal of previously committed gameplay. Native calls are not forcibly preempted; the 20ms dispatch target is provisional, not a measured frame-time guarantee. Each tick/cast entry point has its own dispatch budget; there is no whole-render-frame cap across multiple entry points. Pending events are not saved on world exit.

**Product direction**

The next milestone should let a player create a small place, give it behavior, test that behavior with a friend, change it conversationally, and recover an earlier version after saving and reloading. The engine supplies reusable capabilities; prompts combine them into creations. New example prompts should not require new Rust branches for their wording or their particular rule.

A useful target session:

> Build a stone sanctuary here, with an entrance facing me. Name it Haven. At night, sheep outside Haven hunt players carrying crystals; inside, everyone is safe. Make the sanctuary twice as wide. Change the hunters to wolves. Undo that last rule change.

This exercises spatial grounding, building, persistent named objects, region-scoped rules, actual inventory, creature behavior, revision, and undo. It stays within the existing visual vocabulary.

**What the project already has**

| Area | Observed implementation | Consequence for the plan |
|---|---|---|
| API contract | v1.9.0; 30 methods, three properties, five callbacks. Schema generates docs, Lua stubs, and a Rust name registry. | Preserve `world_api/schema.yaml` as the contract; expand its executable checks. |
| Rule execution | One Lua VM per module, restricted libraries, 8 MiB memory limit, callback timer checked every 1,000 instructions. Host runs Lua. | Good base for capabilities, with execution gaps to close first. |
| Generation | Local HTTP request with API docs and five examples; keyword-based rule/action classification; 800 output-token limit; one corrective retry. | Suitable for short standalone rules; lacks world context and revision context. |
| Review | Generated modules start disabled; host can view code, enable/disable, delete, or run an instant action. | Extend this existing panel into a creation workflow. |
| World modification | Individual block edits, creature spawning/chasing/damage, player effects, weather and time. | Add a small set of spatial and behavioral primitives instead of expanding individual themed commands. |
| Persistence | One `saves/world.bin`; seed, block edits, camera/player position, time, and module name/prompt/source/enabled. | Add versioned worlds and durable rule state before creations depend on them. |
| Multiplayer | Custom UDP retry/ack channel and best-effort snapshots. Player movement is client-simulated; inventory queries/removal work only for the host. | Lua authority exists, but gameplay authority is incomplete. |

Evidence: [schema](../../world_api/schema.yaml), [runtime](../../src/scripting.rs), [generation](../../src/llm.rs), [generation orchestration](../../src/app.rs), [UI](../../src/ui.rs), [save code](../../src/save.rs), [networking](../../src/net.rs).

**Gaps that directly limit player creativity**

1. **Validation can itself stall the game.** `Module::load` executes top-level source while `call_start` is `None`; the installed hook does not enforce elapsed time in that state. The hook interval is not an independent instruction-count ceiling. Runtime API work also needs its own bounds because a Lua hook cannot interrupt a long native operation midway.
2. **A failed rule can leave partial effects.** Creature and weather actions mutate live state, while blocks and player effects are queued. Disabling after an exception does not undo what already happened. Queries do not consistently see a callback's queued writes. Repeated `take_item` calls check the same inventory snapshot, allowing multiple apparent successes against the same balance.
3. **Rules do not compose predictably.** Direct speed/weather/target setters can overwrite one another. Disable/delete does not remove all previously applied effects. Per-call budgets reset for each event, with no aggregate simulation-step ceiling evident in the dispatcher.
4. **A creation has no durable identity or history.** Modules are addressed by vector index in the UI. Lua counters reset on reload; creatures and player inventories are not in the save. There is no creation revision graph, saved named region, or world-edit undo journal.
5. **The model cannot resolve “here,” “this sheep,” or “change that rule.”** Generation receives the prompt and static documentation/examples, without the selected object, raycast position, named places, current world facts, or relevant existing modules.
6. **Large construction cannot fit the current action contract.** An instant action gets 300 block edits and must finish in one callback. Bulk building needs engine-managed jobs, bounds, progress, and cancellation.
7. **The API still exposes special cases.** `carrying_crystal` records a historical flag instead of current inventory; mud slowing is in `player.rs`; redstone healing is in `app.rs`. The example healing rule creates a block, while the engine supplies its healing behavior. General status effects, healing, tags, and spatial triggers would let players author variations themselves.
8. **Reliable delivery is not ordered delivery.** `ReliableChannel` retries and deduplicates but has no receive-order buffer. Reordered edits to the same block can leave a stale final value. `Welcome` includes the entire edits vector in one UDP message, which will not support large authored worlds reliably. The API documentation currently overstates ordering.

These are source-level findings. This review did not benchmark the local model or conduct a new multiplayer playtest.

**Design decisions**

Use three explicit creation types: **Action**, **Rule**, and **World setup**. A compound creation can contain a construction job and several rules. An action is requested once but may complete over several simulation ticks; it should not need to masquerade as a permanent rule. World setup combines a bounded generation recipe with saved creations.

Keep Lua as the behavioral language. Let Rust implement expensive geometry, movement, queries, transactions, persistence, and scheduling. Do not ask the LLM to emit thousands of block placements or implement pathfinding.

Introduce a small internal `CreationDraft`: stable ID, parent revision, original prompt, resolved references, requested capabilities, parameters, Lua sources and/or build specification, and validation results. The engine owns IDs and history. Model-produced descriptions are proposals; review counts, affected bounds, errors, and diffs come from validation and actual planned commands.

Use additive flat API names initially, retaining existing calls as compatibility wrappers. Namespace migration can wait for an intentional major API version. Every addition must specify argument ranges, return/error behavior, cost, events produced, persistence, replication, and disable/undo semantics.

**Phase 0 — Make execution and effects trustworthy**

Start here, before new creative capabilities.

- Arm time and instruction limits during module initialization and every callback. Enforce an independent instruction ceiling and a sticky exhausted flag. Verify protected calls cannot swallow a budget failure and continue forever. Keep memory limits active throughout.
- Add a simulation-step scheduler budget across modules and events, fair dispatch, queue limits, and caps on queries, messages, effect commands, and total spawned entities. Individual native API calls must validate finite numbers, coordinates, sizes, and bounded work before execution.
- Extract an authoritative simulation entry point from `App` incrementally. It should run without `wgpu`, `winit`, or `egui` so tests and previews can use the real simulation. Move rule mutation behind a command interface without rewriting the renderer.
- Give each callback a bounded transaction overlay: reads see its accepted writes, inventory debits reserve the balance, and all commands commit only after successful execution. On failure, discard commands and disable the module. Disable a VM whose private Lua state was partly modified; reactivation rebuilds it from the last committed explicit state.
- Commit callbacks in stable module/event order. Later callbacks see earlier committed state. Record an origin/module/revision ID for every effect. After-commit events carry a cause ID and have a bounded propagation depth.
- Reconcile generated documentation with runtime behavior, particularly ordering and budgets. Add schema/runtime conformance checks; text scanning remains a diagnostic aid, not the sandbox boundary.

**Exit:** a module that loops during loading, exceeds a native query limit, errors after editing/spawning, or repeatedly removes the same inventory cannot hang the process, leak partial world effects, or double-spend. Run potentially hanging cases in a test subprocess with an external timeout.

**Phase 1 — Durable creations and authoritative shared state**

- Add a save envelope with format version, world ID, generator version, and migration from the exact existing bincode layout. Keep a backup and write snapshots atomically through a temporary file.
- Save named worlds separately. Persist entity IDs/state, player inventories and relevant attributes, weather/time, module IDs/revisions, explicit module state, scheduled timers, regions, and active jobs as they are introduced.
- Expose bounded `state_get`/`state_set` for JSON-like values, isolated per module. Do not serialize arbitrary Lua globals, closures, or userdata. Commit explicit state with world commands. State migration for a revised rule runs with budgets; incompatible migration leaves the old revision active or requires an explicit reset.
- Separate durable player identity in a saved world from connection IDs. A local reconnect token mapped by the host is enough for this scope; no account system is needed.
- Move inventories and item spending for all players onto the host. Derive “carrying crystals” from inventory contents. Handle client interaction/mining/placement as validated intents with sequence IDs. Keep local movement prediction, with host validation/reconciliation for rule-sensitive movement and attributes.
- Add revisioned authoritative deltas and bounded snapshot transfer in chunks, acknowledgement, resynchronization, and queue backpressure. Use ordered transport or explicit revision handling; never assume retry alone implies order. Keep the current transport if this is a small change; choose QUIC only after assessing migration cost, not as a separate infrastructure project.
- Give each world mutation transaction a journal entry. Retain bounded history and periodic checkpoints instead of logging unlimited per-frame state.

**Exit:** a guest's inventory drives the same crystal rule as the host's; save after two creature deaths and reload before the third without losing the counter; packet loss/reordering and joining a substantially edited world converge to the host's state.

**Phase 2 — Contextual prompting and a complete revision loop**

- Add explicit Action / Rule / World setup intent to the console, with automatic suggestion that the player can correct. Replace keyword classification as the final authority.
- Capture a bounded context snapshot: player ID/position/facing, hit block or entity, selected area, named places, available assets, relevant rule IDs/revisions, world revision, and necessary world facts. Freeze “here” and “that object” to visible references; show the resolved selection before applying.
- Support prompts such as “make it larger,” “only during rain,” and “change the selected rule.” Include the selected revision and a compact conversation summary; avoid sending every module and every past message.
- Generate a structured draft containing interpreted intent, parameters, required capabilities, code/build spec, and any unresolved reference. Validate the envelope deterministically. Ask a short in-game clarification only when the ambiguity materially changes the outcome; otherwise display an editable assumption.
- Select API documentation by capability and include its dependencies and exact schemas. Keep this a deterministic registry lookup initially; a vector database is unnecessary.
- Detect truncated output, use configurable output limits suited to each draft type, show generation progress, support cancellation, and reject stale generation results after changing worlds or revisions.
- Validate envelope, capabilities, source/entrypoints, and API arguments; execute smoke scenarios in an isolated simulation fixture using the actual runtime. Feed bounded diagnostics into a limited repair loop. Never let retry silently change the requested creation type.
- Present an effect summary, editable parameters, source/diff, test results, and affected region. Provide Preview, Apply, Revise, Disable, History, and Undo where supported. “Explain this rule” should include real runtime observations: last trigger, affected entity, failed condition, cost, and errors.
- Validate a replacement alongside the running revision, then swap atomically at a tick boundary. Keep the previous version active if validation fails. No automatic runtime repair activates without host review.

**Exit:** “make the selected rule affect wolves instead of sheep” updates one existing creation, shows a diff, and can restore its previous revision after save/reload. A missing capability produces an honest unsupported result instead of plausible invalid Lua.

**Phase 3 — Build places through prompts**

Implement this first creative vertical slice using the foundations above. Candidate API spellings below are proposals, not a finalized public contract.

| Capability | Initial surface | Player example |
|---|---|---|
| Spatial references | Selected bounds, hit position/normal, facing, named anchors | “Build a wall between these points.” |
| Bulk geometry | `fill_box`, `hollow_box`, `replace_in_region`; then line and cylinder when needed | “Build a stone sanctuary with an entrance.” |
| Terrain operations | Surface query over edited terrain, flatten, bounded raise/lower | “Flatten this area for a courtyard.” |
| Reuse | Save a selected structure as a blueprint; stamp with position and quarter-turn rotation | “Put another tower at the other corner.” |
| Places | Create/query/name a box region; membership and enter/leave events | “Call this place Haven.” |
| Job control | Job ID, estimated edits, progress, cancel, undo | “Stop building and undo the completed part.” |

Represent geometry declaratively and generate edits inside Rust. Define block-coordinate conventions, inclusivity, clipping, overwrite filters, and maximum volume. Start with boxes and a small material palette; do not build a general CAD system.

Preview the exact planned edit set with translucent blocks or bounds plus material/count summaries. Revalidate affected blocks at commit if the world changed since preview. Large jobs commit in bounded batches, yielding to simulation and replication; do not promise atomic visibility for a multi-tick building operation. Mark a compound creation “building” and activate its dependent rules only when its required structure succeeds.

Journal before/after values and write revisions for completed batches. Undo restores a cell only if its current write revision still belongs to that operation; newer player edits become visible conflicts rather than being overwritten. Include secondary edits such as flooding where an operation produces them. Cancellation retains a clearly reported completed portion until the player chooses undo.

**Exit:** build a roughly 2,000-block courtyard across ticks without exceeding the scheduler budget; a guest sees its progress; cancellation and reload resume/undo consistently; a player's later alteration survives an older job's undo.

**Phase 4 — Rules that compose into ecosystems**

Add capabilities in the order demanded by the acceptance scenarios.

| Capability | Initial contract | What it enables |
|---|---|---|
| Tags and bounded entity data | Persistent tags and typed data on entities/regions; filtered bounded queries | “Only marked wolves guard this village.” |
| Complete events | Day/night, weather change, spawn, damage/death, block placement, region enter/leave, item change | Rules avoid rebuilding transition detection in every Lua module. |
| Event identity | Entity/player IDs, position, source/cause, damage amount, transaction ID | Per-creature memory, attribution, bounded rule chains. |
| Timers | Named timers by simulation time, persisted with pause/reload semantics | “Open the gate 30 seconds after the bell is used.” |
| Creature control | Target, move/flee/guard, aggression override, heal, bounded attributes | Passive sheep can attack; hostile wolves can become neutral guards. |
| Scoped effects | Add/remove keyed health/speed/jump/damage modifiers with duration or refreshed lease | Rain slowing and a sprint blessing coexist without permanent overwritten values. |
| Inventory predicates | Has/count/give/take for every player, with atomic spending | “Spend a crystal to activate the shrine.” |
| Semantic light sources | Registered color, position, radius, enabled state; nearby query | “Sheep flee red light.” Rendering can initially reuse simple emissive blocks. |

Avoid exposing arbitrary ECS components or engine memory. Define a small whitelist of useful data and actions. Add a bounded navigation improvement only when move/guard acceptance tests expose a concrete obstacle.

Define composition explicitly: stable order for additive commands; documented stacking/clamps for modifiers; priorities for conflicting AI targets and weather overrides, with stable ID tie-breaks and conflict reporting. Source-owned modifiers are removed by the engine on disable/delete/crash even if a Lua cleanup callback fails.

Make disable, revision restore, and world undo distinct operations. Disable stops execution and removes leased effects; it does not reverse every historical death or construction. Revision restore changes code/parameters/state according to migration policy. Undo reverses supported journaled mutations with conflict checks; a whole-world checkpoint restore is the explicit option for wider causal history. Spawns may be removable while owned and unmodified; do not promise that undo can resurrect all downstream consequences safely.

Migrate mud slowing and redstone healing into authored rules using the generic capabilities. Keep creature models and basic locomotion in the engine. Add block-category tags such as `tree_trunk` so prompts need not enumerate all wood names.

**Exit:** all three original reference rules run through general APIs, with save/reload and a joined player. Also test two overlapping modifiers, rule disable after a crash, changing hostile species to friendly, and the red-light extension to the sheep rule.

**Phase 5 — Worlds as saved, editable creations**

- Introduce a `WorldRecipe`: seed, versioned terrain parameters, sea level, tree density, spawn location, bounded starter structures/regions, and initial rule references. The LLM proposes validated data; Rust generates terrain deterministically.
- Begin with variations inside the current biome and assets. Persist generator version/recipe so exploration after reload does not change terrain generation unexpectedly. Recipe revisions affect a new world or an explicitly selected regeneration region; never silently regenerate existing player builds.
- Offer “Create world from prompt” and named local save slots. A complex request becomes small linked drafts that the player can preview and refine. Avoid one enormous Lua module tasked with generating an entire world.
- Allow local world duplication and export/import of a versioned creation bundle containing blueprint data, rules, prompts, parameters, required API/assets, and optional initial state. Imported code passes the same validation and stays disabled pending review.
- Later, if existing assets prevent useful prompts, add constrained data-defined variants: a named creature based on an existing model with tint/stats/tags, or a block variant with approved material/physical properties. Persist and replicate definitions before instances. Defer generated models, textures, native code, and arbitrary content pipelines.

**Exit:** create, name, save, duplicate, and reopen two differently prompted worlds; their terrain and rules remain stable; unsupported asset requests are explained; restoring one world's history cannot affect another world.

**Validation and measures of progress**

Maintain hand-written runtime fixtures independently from model generation. They prove API semantics before asking a model to use them. Use a separate held-out prompt corpus, not just the five examples included in the system prompt. Start with 24 prompts split across construction, conditional rules, and revision/unsupported requests; include paraphrases and compound requests.

For each prompt record model/configuration, first-pass contract success, bounded repair success, observed behavioral assertions, time to a reviewable draft, and runtime cost. A compiling module is not a successful rule. A preview is bounded evidence, not a proof of every future interaction. Label untested future conditions in the UI.

Suggested release gates, to be tuned after measuring the baseline:

- All runtime/persistence/transaction regression cases pass; no live effects from failed validation.
- At least 20 of 24 held-out prompts meet their behavioral checks within two repairs. Unsupported requests count as successful only when correctly identified and clearly reported.
- Two-player convergence under delayed, dropped, duplicated, and reordered packets; then a four-player session covering construction and concurrent rules.
- Save/reload equivalence for persistent counters, timers, entities, inventory, jobs, and enabled revisions.
- A 2,000-edit construction job and ten simple rules remain within a measured configurable script/work budget. Benchmark on the host hardware; choose a target such as 2 ms of scheduled rule work per simulation step and report actual frame-time percentiles.
- Disable, revision restore, and conflict-aware undo pass separate tests.

**Recommended first implementation sequence**

1. Close initialization budget gaps; add independently bounded instructions and native work; fix schema claims.
2. Extract the simulation command boundary and add callback transactions with inventory reservations and error rollback.
3. Add versioned saves, stable creation/entity IDs, explicit state, revision storage, and host inventory authority; implement the replication fixes required for their consistency.
4. Add selected-world context and Revise/History/Preview to the existing console for a small rule; validate and switch revisions safely.
5. Add one box-building job, one named region, and undo. Deliver the sanctuary session with two players, introducing only the creature/effect primitives it requires.
6. Complete reusable events/effects and the original three rule scenarios; then add world recipes and local bundles.

These are dependency-ordered work packages, not a request for a broad engine rewrite. Estimate calendar time only after packages 1–2 reveal the cost of the simulation boundary. Keep renderer polish, new creature rosters, general crafting, accounts, marketplaces, dedicated servers, and autonomous background LLM world control outside this roadmap's first deliverable.
