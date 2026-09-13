# Phase 3: one AI player

The implementation is available in the `dev-playtest` build. Start a solo world,
open **F10 > Development playtesting**, choose an OpenAI scenario, then start Agent1.
The current executable is built with `cargo build --offline --features dev-playtest`.

Set `OPENAI_API_KEY` and `OPENAI_PLAYTEST_MODEL` inside the `aiapi` object in local
`settings.json`, or in the launching process environment. Nonempty file values
take priority for each field. The file is read whenever an AI session starts, so
file edits do not require restarting the game. Settings saves preserve both values.
The key stays in local configuration; it is excluded from debug output, world
saves, prompts and session reports. `settings.json` is ignored by Git.

```json
{
  "aiapi": {
    "OPENAI_API_KEY": "YOUR_API_KEY",
    "OPENAI_PLAYTEST_MODEL": "YOUR_MODEL_ID"
  }
}
```

Merge this object with your existing settings. If using Windows environment
variables instead, restart the launcher/terminal after changing them.

| Scenario | Player actions | Independent completion check |
| --- | --- | --- |
| Gather and craft a tool | Mine resources; construct or inspect a smelter; disable ejection or collect its metal drops; deposit ore and fuel; withdraw iron; craft an axe, pickaxe or sword. | A new tool was produced by an accepted crafting transaction. Starting tools do not count. |
| Build a small shelter | Gather solid building blocks and place the marked blueprint, using reachable support faces and normal jumping when needed. | Agent1 placed all 23 required blocks: two-block walls, a full roof, a clear two-block doorway and interior, with intact ground. |
| Survive and return | Move at least two blocks from the start marker, engage a hostile creature or survive its attack, then return. | Actual hostile engagement, positive health, return within 0.75 blocks, and two seconds with no hostile creature within four blocks. |

Shelter runs need a clear, flat 3 x 3 patch with three blocks of headroom around
the agent spawn. The start/build site has an in-world marker. Encounter runs use
existing world creatures; they do not grant gear or spawn an easy enemy for the AI.

The model receives bounded player observations, named inventory balances, relevant
recipes, local targets, objective progress and recent decisions/results. It chooses
an offered typed command plus a short goal and explanation. The game validates
the structured response and executes the command through the existing player
validators. No generated code or arbitrary engine mutation is accepted.

Movement, aiming, jumping, mining, attacks and placement are local controller tasks.
Placement checks the intended destination before spending a resource. Smelting
uses the same device transactions and simulation as human players. Device contents
are only shown after inspection (or after constructing that device).

The decision loop reconsiders task completion, failure and damage. A damage event
cancels the task and starts a local retreat to the start; a reply based on the old
observation is discarded and its usage still accounted for. Pending requests and
network failures do not keep issuing movement input. Dead/submerged agents stop.
Long return journeys use reachable segments of observed land.

Run limits: 10 minutes, 96 decisions, 160,000 total tokens, 1,024 output tokens per
request, six unsuccessful controller tasks, or eight rejected player actions.
Only one request can be in flight. Transport timeout is 18 seconds with no automatic
retry. An input-byte reservation plus output/protocol allowance is checked before
each request. These are caps, not a claim that every generated world is solvable
within them.

Settings displays the current goal, pending/running state, decisions, tokens and
artifact directory. `session.json`, `events.jsonl` and `report.json` record model
configuration, exact requests/commands, accepted actions, outcomes, objective
evidence and usage. Unique session directories prevent concurrent runs from
overwriting each other's traces. [Phase 4](PLAYTESTING_PHASE4.md) now adds independent
invariant findings and isolated action replay; this does not establish live AI
acceptance for phase 3.

Validation includes real gathering/smelting/crafting from an empty material
inventory, all shelter placements and resource costs, combat against a simulated
hostile followed by a safe return, mocked provider decisions driving a mining task,
damage invalidating an in-flight decision, malformed responses, and budget stops.
No live OpenAI call was possible during implementation: the process and Windows
user API-key settings were absent. Live autonomous completion is therefore **not
claimed**, even though all phase-3 scenarios and controls are implemented.

Subsequent connectivity check using the locally configured credentials reached
OpenAI but returned HTTP 429 with "You have no credits remaining." Autonomous
acceptance remains open until scenarios complete. After credits were added, the
same live structured-decision test succeeded using the current configuration
(150 total tokens). This verifies connectivity and decision parsing, not full
autonomous scenario completion. A session stopped by an earlier API error must
be stopped/removed and started again; it does not retry automatically.
HTTP failures now include bounded, credential-redacted provider error details.

To explicitly send one small live structured-decision request using the game's
actual request/response code (this can incur API usage):

```powershell
cargo test --offline --features dev-playtest live_openai_decision_request -- --ignored --nocapture
```

This test is ignored in ordinary test runs and never prints the configured key.
