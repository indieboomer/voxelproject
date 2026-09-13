# Development playtesting implementation plan

Source: [AGENTS_TESTING_SUBSYSTEM.md](../AGENTS_TESTING_SUBSYSTEM.md).

Implement phases in order and keep exit criteria as gates, rather than claiming an
AI report proves a game bug. No API credentials are required for phases 1–2.

1. **Player interface and observability.** Add the opt-in `dev-playtest` Cargo
   feature, a Settings start/stop control, one visible Agent1, restricted JSON
   observations, validated actions, a gather/decompose/craft script, and session
   artifacts containing the initial save, configuration, ticks, actions and results.
   Verify successful crafting and rejection of illegal actions using game fixtures.
2. **Movement controller.** Add bounded land navigation, approach, mining and
   follow/attack tasks, cancellation and explicit blocked/stalled/timeout results.
   Exercise gather/craft, construction and return journeys before considering
   controller reliability established. Underwater navigation follows land tests.
3. **AI decisions.** Connect a bounded structured decision provider to the tested
   action interface. Keep movement and emergency stopping local. Record provider,
   model, limits and usage; accept credentials through environment configuration.
   Run the three short tasks and retain failures as evidence.
4. **Evidence and reproduction.** Add independent invariant checks, replay from
   the recorded starting state, and issue reports with action references and
   classifications. This gate requires an actual previously unknown reproduced
   issue; implementing a report generator alone cannot pass it.
5. **Broader scenarios.** Add survival, combat, automation, recovery, persistence,
   rules and economy incrementally, using comparable budgets and deterministic
   checks. Promote reliable scenarios to the regression suite.
6. **Multiplayer.** Run separate connected clients; correlate host/client logs and
   test concurrent storage/device actions and reconnection. Host-local agents do
   not satisfy this phase.
7. **Regression scheduling.** Run scripted gates on selected builds, compare
   comparable runs, deduplicate reproduced findings and attach artifact links and
   model usage. Open-ended AI findings remain advisory.

Normal builds must compile without the subsystem. Session tooling must not
overwrite the named world save. Full technical snapshots are separate from the
observations offered to a decision provider. Visual/UI quality still needs human
or screenshot testing.

## Implementation status

| Phase | Status and evidence |
| --- | --- |
| 1 | Implemented. Script gathers, decomposes and crafts through the game validators, starting without materials or mana. JSONL trace and a snapshot readable by the normal save reader are tested. Settings starts/stops one nearby Agent1 with a red overhead name. |
| 2 | Initial land milestone implemented. Bounded breadth-first navigation, approach/interact/mine, follow/attack, cancellation, obstruction, target loss, stall and timeout results. Controlled fixtures complete gather/craft, walk/return and approach/mine. Complex terrain and underwater recovery are not established. |
| 3 | Implemented: all three initial AI scenarios, bounded decisions, verified placement and smelter actions, damage-triggered retreat/reconsideration, and independent completion checks. Game-action and mocked-provider tests pass. **Live model acceptance remains unverified because no API key is configured.** See [phase 3 details](PLAYTESTING_PHASE3.md). |
| 4 | Initial evidence milestone implemented: independent transaction/movement/support checks, isolated snapshot/action replay, and uncertain divergence reports. A controlled agent fixture discovered and reproduced unsupported ShortGrass after support mining (35 matching actions). Live AI discovery and full concurrent-world replay are not established; phase-3 live acceptance remains open. See [phase 4 details](PLAYTESTING_PHASE4.md). |
| 5–7 | Pending preceding gates. No multiplayer replication coverage or scheduled regression service is claimed. |

## Running the current milestone

```powershell
cargo run --offline --features dev-playtest
```

Enter a solo hosted world, open Settings (F10), choose a Development playtesting
scenario and press **Start / stop Agent1**. The agent changes the current world
through normal actions. Stop removes the agent; its inventory is not merged into
the host's inventory. A completed agent remains visible until stopped. Joining a
guest stops the session because this stage does not implement separate clients.

Three deterministic scenarios are available: gather/craft a resource, walk/return,
and approach/mine. Unavailable local terrain is reported as a scenario/controller
failure. The controller only routes through observed dry land, with a 512-node
search limit, 30-second task timeout and two-second movement stall threshold.

The three optional OpenAI scenarios (tool crafting, shelter, and encounter/return) read `OPENAI_API_KEY`
and `OPENAI_PLAYTEST_MODEL` from `settings.json` under `aiapi`, with the launching
process environment as fallback for each empty field. File configuration is read
at each session start; see the [configuration example](PLAYTESTING_PHASE3.md).
The model must support Responses API
structured outputs. No model or price is assumed. Local settings preserve explicitly
configured credentials; credentials are excluded from playtest artifacts. The request format follows the
[official Structured Outputs guide](https://developers.openai.com/api/docs/guides/structured-outputs).
Limits are 600 seconds, 96 decisions, 160,000 total tokens and 1,024 output tokens
per decision. A conservative input-byte reservation prevents starting a request
that would exceed the remaining token budget. Failed/in-flight calls can have
unreported usage; reports mark usage completeness and leave monetary cost unknown.
Requests run on a worker, use an 18-second transport timeout and have no automatic
retries. The player waits while a decision is pending; submerged/dead states stop
the AI session. Damage cancels the current task, invalidates pending decisions and triggers a local retreat toward the start marker. Six unsuccessful tasks or eight rejected actions stop a run. No API call is made by the scripted tests.

Artifacts are in `target/playtests/<session>/`:

- `session.json`: build version and executable fingerprint, seed, generation,
  agent starting state, registry, scenario and model limits.
- `initial.bin`: full host world snapshot, separate from the named world save.
- `events.jsonl`: sensed observations, tick-indexed actions/results, edits,
  authoritative events, and AI requests/validated decisions if enabled.
- `report.json`: completion or failure, artifact links, usage, invariant findings
  and automatic isolated replay attempts when findings exist. Reproduced findings
  are distinguished from uncertain divergence and still need cause classification.

Physics/actions advance at 20 Hz; visibility sensing runs at 4 Hz. Intermediate
records omit the unchanged sensory payload. Geometry comes from bounded rays;
buried resources and unopened device inventories are not exposed. A nearby inspected device exposes the same contents/configuration as its player panel. Agent effects participate
in host creature targeting and rule effects. This still does not provide separate
network clients, visual testing or full-world deterministic
reproduction of concurrent human/rule/creature activity.

```powershell
cargo test --offline --features dev-playtest
cargo test --offline
```

Normal builds do not include the playtest module, settings controls or provider.

Phase-3 validation: 468 development-feature tests and 451 normal-build tests passed
(22 opt-in tests ignored in each suite). Both Settings themes rendered in the
offscreen preview. The approach/mining scenario additionally asserts that the
player moved before mining. No paid API request or live AI scenario was run;
`OPENAI_API_KEY` was absent from the development process environment.

Phase-4 validation: 471 development-feature tests and 451 normal-build tests passed
(22 opt-in tests ignored in each suite). The new replay test checks a real
gather/craft trace, altered outcomes and player state, and explicit external-event
boundaries. The support-loss discovery fixture confirms the world-state violation
and reproduces its independent finding from the initial save. Successful replay
alone is never counted as a game issue.
