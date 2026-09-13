# Phase 4: independent evidence and isolated reproduction

The `dev-playtest` build now observes transactions independently of the AI decision
provider. Checks run before periodic mana regeneration, so a rejected action on a
regeneration tick is not mistaken for an inventory mutation. The observer checks:

- Rejected actions preserve the account and emit no block edits.
- Crafting, extraction, conversion, equipment crafting/salvage, mining and placement
  have the expected resource, element, gear and mana deltas.
- Passive actions do not change economic balances.
- Movement stays finite and does not move a previously clear player into terrain.
- Direct block edits preserve the solid support required by decorations.

Costs come from the recorded registry and equipment formulas; the observer performs
its own balance arithmetic rather than invoking the transaction implementation.
Successful collection and device transfers do not yet have complete cross-inventory
conservation checks. Support checks describe the direct edit; later rule callbacks
could change the result and require investigation.

Session schema 2 adds findings, authoritative player state and collision-player
positions to action records. These diagnostics are not supplied to the AI. Reports
retain the scenario goal, original action/tick references, expected and observed
behavior, specification/invariant basis, severity, confidence, classification and
artifact paths. General findings remain `uncertain` until their cause is reviewed.

When a session has an invariant finding, report generation attempts isolated replay
automatically. It reloads `initial.bin` using the real save reader, restores the
recorded agent and registry, and runs the recorded actions through the existing
validators. No model request or named-world write is involved. Only findings that
recur in the matching trace prefix appear in `reproduction.findings_reproduced`.
A matching trace without such a finding does not establish that a bug occurred.

To replay any session manually without opening a window:

```powershell
cargo run --offline --features dev-playtest -- --playtest-replay target/playtests/<session>
```

The command prints JSON. `status: matched` means the isolated actions matched;
`uncertain` retains the first differing tick, field and expected/actual state.
An explicit external authoritative event stops replay at an uncertain boundary.
Malformed artifacts fail with an error. Schema 1 traces are also accepted, with
their more limited recorded state.

This is not full server playback. Creature AI, loot aging, automation ticks, weather,
Lua callbacks and concurrent human actions are not replayed. Host-world save
round-tripping is covered by the existing real-save-reader test; task continuation
across a live save/reload remains phase 5 work. Separate clients remain phase 6.

## Discovery fixture

Run the controlled support-loss investigation with:

```powershell
cargo test --offline --features dev-playtest reproduces_unsupported_decoration -- --nocapture
```

The fixture records its initial terrain and inventory, places ShortGrass legally on
a stone pillar, then mines its support through normal equipment actions. It checks
the resulting world against `BlockDef::only_on_top` and replays the recorded actions.
Artifacts include `session.json`, `initial.bin`, `events.jsonl` and
`support-issue.json`. The test prints the unique artifact directory. This is a
characterization of the current bug, not a passing regression gate for its fix;
when support handling is fixed, replace the bug assertion with the corrected
behavior and update the finding status.

Live autonomous phase-3 acceptance remains unverified. Controlled scripted discovery
and replay do not establish that an LLM completed the initial tasks autonomously.
