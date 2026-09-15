> Archived 2026-09-15. Original playtesting proposal; implementation status and open phases are tracked separately.
> See [current documentation](../PLAYTESTING_PLAN.md). Claims and measurements below describe the historical revision.

Build a **development-only AI playtesting system**, starting with one agent completing short tasks through normal player actions. Expand only after it produces reproducible, useful findings.

The first goal is to discover broken or confusing gameplay sequences. Feedback about enjoyment should remain a hypothesis for human playtesting.

Agent(s) operating ingame should be started from the game (option in settings), start near current player and be visible ingame as player with red Agent1 Agent2 etc name overhead.

### Phase 1 — Player interface and observability

**Goal:** Make the game controllable and observable without an LLM.

Create a structured observation API exposing:

* Player position, health, mana, oxygen, equipment, and inventory.
* Nearby observable terrain, creatures, drops, and devices.
* Known recipes and available interactions.
* Recent events and the outcome of the previous action.

Limit observations to information a player could reasonably know. Keep complete world state available separately for technical diagnostics.

Expose a small action API: move, look, interact, collect, equip, mine, place, craft, attack, and wait. Actions must use existing player validation: reach, collision, tools, costs, cooldowns, and inventory capacity.

Each action returns a structured result, including failure reason. Record world seed, build version, initial save, simulation ticks, actions, and outcomes.

**Exit criterion:** A scripted controller can collect materials and craft an item through this interface, with an inspectable event log.

### Phase 2 — Reliable movement and task execution

**Goal:** Separate gameplay decisions from low-level control.

Implement a deterministic controller for actions such as:

* Move to a reachable position.
* Approach and interact with a target.
* Mine a selected block.
* Follow or attack a creature.
* Cancel the current task.

The controller handles pathfinding, movement inputs, aiming, and action timing. It must detect blocked paths, disappearing targets, lack of progress, and timeouts.

Start with walking on land. Add oxygen count underwater after the basic controller works.

**Exit criterion:** Scripted scenarios complete reliably enough that controller failures can be distinguished from game failures.

### Phase 3 — One AI player, short sessions

**Goal:** Let an LLM choose goals and actions.

Use a bounded decision loop:

1. Receive an observation and recent history.
2. Choose a short-term goal and a supported action.
3. Let the controller execute it.
4. Reconsider after completion, failure, or a significant event.

Require structured responses containing the selected action and a brief explanation. Validate all arguments before execution.

Do not request an LLM response every frame. Immediate safety behavior, such as stopping when a path becomes invalid, belongs in the controller. Define safe behavior while requests are pending or fail.

Set limits on session duration, decisions, token usage, and repeated unsuccessful actions. Record the model and configuration used.

**Initial tasks:**

* Gather ingredients and craft a basic tool.
* Build a small shelter.
* Survive an encounter and return to a marked location.

**Exit criterion:** The agent completes some tasks autonomously and produces useful traces when it fails.

### Phase 4 — Evidence-based feedback

**Goal:** Turn sessions into actionable reports.

Keep three responsibilities separate:

| Component          | Responsibility                                      |
| ------------------ | --------------------------------------------------- |
| AI player          | Makes decisions using limited observations          |
| Technical observer | Checks authoritative events, state, and invariants  |
| Report generator   | Summarizes outcomes and identifies candidate issues |

Every reported issue should include:

* Intended goal and observed result.
* Relevant action sequence and event references.
* Expected behavior and its basis: specification, invariant, or hypothesis.
* Classification: game bug, controller problem, agent mistake, possible usability issue, or uncertain.
* Severity, confidence, and reproduction information.

Use deterministic checks for conservation of resources, crafting costs, invalid movement, inventory changes, and persistence. Do not rely solely on the LLM to establish that a bug occurred.

Automatically attempt to reproduce promising findings from the initial save and recorded actions. Preserve failures to reproduce as uncertain findings.

**Exit criterion:** The system discovers and reproduces at least one previously unknown game issue with enough evidence for a developer to fix it.

### Phase 5 — Broader gameplay coverage

**Goal:** Exercise interacting systems rather than isolated features.

Add scenarios incrementally:

| Area        | Example task                                                             |
| ----------- | ------------------------------------------------------------------------ |
| Survival    | Cross a river, collect a resource, and return safely                     |
| Combat      | Fight a suitable enemy, recover drops, and retreat when necessary        |
| Automation  | Build a production line and maintain a target stock level                |
| Recovery    | Diagnose a blocked output or insufficient mana                           |
| Persistence | Continue a task after saving and reloading                               |
| World rules | Request a behavior change through the player-facing flow, then verify it |
| Economy     | Search for unusually profitable conversion cycles                        |

For generated rules, verify the resulting behavior independently. The same model should not be the sole judge of whether its own interpretation was correct.

Add a few controlled play styles—cautious, exploratory, and resource-efficient—while keeping scenario definitions and budgets comparable.

**Exit criterion:** A repeatable scenario suite reports completion, failures, stalls, and candidate exploits across several systems.

### Phase 6 — Multiplayer testing

**Goal:** Test actual client–host interaction with two agents.

Run agents through separate clients connected to the same authoritative host. Direct host API calls alone do not test replication or client behavior.

Start with:

* One agent gathering while another builds.
* Shared storage and competing withdrawals.
* Concurrent device interaction.
* Combat and drop collection.
* Disconnecting and reconnecting during a task.

Compare each client’s observable state with authoritative state, allowing for expected replication delay.

**Exit criterion:** Reports distinguish gameplay failures from synchronization issues and include correlated client and host logs.

### Phase 7 — Scheduled regression sessions

**Goal:** Make the system useful during daily development.

Run a small fixed suite on selected builds, followed by optional exploratory sessions.

Produce a concise report containing:

* Completed and failed scenarios.
* Regressions against the previous comparable build.
* Reproduced issues, grouped to avoid duplicates.
* Uncertain findings requiring review.
* Links to logs, saves, and available playback.
* Runtime and model cost.

Use scripted scenarios for dependable regression gates. Initially treat open-ended AI findings as advisory, because model decisions can vary between runs.

### Scope and practical limits

* Keep agent tooling behind a development flag; exclude it from normal releases.
* Reuse the game’s simulation and interaction paths rather than creating permissive shortcuts.
* Save observations and actions so investigations do not require another identical LLM response.
* Prefer structured logs and snapshots first; add visual playback when it materially helps debugging.
* Agents using structured state will not adequately test visual clarity, animation quality, hotbar discoverability, or general interface usability.
* Defer large agent populations, elaborate personalities, and accelerated simulation until the basic system proves useful.

**Recommended first milestone:** one agent, three short scenarios, legal player actions, reliable movement, and a final report with reproducible evidence. That is enough to evaluate whether the approach saves development time before expanding it.
