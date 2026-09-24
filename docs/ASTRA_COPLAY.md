# Astra Co-Play implementation plan

Work stays on `experimental`. Reuse `scripting::Module`, transactional callbacks,
`ScriptHost` scheduler cancellation, generated `world_api_compact.md`, persistent
`WorldIdentity` references and the existing `CraftingSave` JSON sidecar.

1. Add a local Responses API worker and bounded, game-thread tool dispatcher.
2. Support capability discovery, bounded inspection, instant authoring and stable
   named rule revisions. Apply the existing sandbox budgets without spell fees.
3. Save conversations, references and revision history alongside world content.
   Undo restores only rule definitions, with conflict checks; downstream effects
   of rules and one-shot scripts are not reversed.
4. Store evidence-backed capability gaps independently in `coplay/backlog.json`,
   with human annotations, dismissal, explicit recheck and Markdown export.
5. Add an F8 panel with progress, details, cancellation and undo. Hosts only;
   credentials never enter replicated state or world saves.
6. Test orchestration, persistence, authority, retries, rule replacement and Lua
   execution. Report live-provider and graphical checks separately.

The provider protocol follows the official Responses function calling guide:
https://developers.openai.com/api/docs/guides/function-calling

## Using Co-Play

Start or load a world, press **F8**, and enable **world authoring**. Type the
request and press Send. You can close the panel with F8/Escape and continue
playing while Astra works. Only the host can use this mode; guests receive
ordinary authoritative world effects, never provider configuration.

The configured model is used verbatim. Existing local settings work unchanged:

```json
{
  "aiapi": {
    "OPENAI_API_KEY": "your-local-key",
    "OPENAI_PLAYTEST_MODEL": "your-configured-model"
  },
  "coplay": {
    "steps": 24,
    "corrections": 3,
    "seconds": 300,
    "tokens": 160000,
    "output_tokens": 6000
  }
}
```

Environment variables with the same names are fallbacks for empty settings.
Settings stay local and are git-ignored. Provider errors omit response bodies;
the worker redacts the configured credential before parsing returned content.
No provider reasoning items or credentials are stored in world saves.

Limits apply per user request. The UI cancels future work immediately; an
already-running HTTP request can finish in its worker, but its result is discarded.
Calls have a 60-second transport timeout, no automatic application retry, and a
bounded response size. Input bytes plus maximum output tokens conservatively
reserve the remaining model budget before each call. Hard ceilings are 64 tool
steps, 8 correction attempts and 16,000 output tokens per call.

## Authoring and verification

The local Responses worker emits one tool call at a time. `App` validates and
executes it on the game thread. Tools discover generated World API signatures,
inspect bounded regions/entities, read current rule source, revise a rule,
execute an instant Lua script, compare an expected observation, report/recheck
capability gaps, and finish. Saved request records include actual results and
usage; expandable UI details retain these after completion.

Rule names beginning `coplay-` are stable IDs. Follow-ups read the definition and
supply its exact previous source as a precondition. Replacement removes pending
subscriptions for the old runtime before registering the new one. Target
references use existing world IDs, creature IDs, device IDs and block generations;
a replaced block, even of the same material, invalidates its reference.

Instant scripts run in the existing transactional sandbox. A failed callback
rolls back its staged effects, and successful actions receive persistent receipts.
Reusing a receipt returns the prior result; reusing its ID with different arguments
is rejected. Execution success and rule registration remain distinct from
verified gameplay. `verify` compares an expected fact with a fresh observation;
without such evidence the final status explicitly labels effects unverified.

F5 and normal save/quit persist world changes, definitions, conversation,
request evidence, references and revision history using the existing save system.
Existing accepted rules run locally without OpenAI. Co-Play rules with an API
version mismatch reload disabled with their definitions retained.

**Undo last rule revision** restores only that rule's definition and subscriptions.
It detects later source/enabled-state edits and refuses conflicting restoration.
It does not rewind blocks, creatures, inventory, weather, damage, or other
downstream simulation effects. One-shot actions do not have automatic undo.
Lua globals reset on reload/revision as in the existing runtime; persistent
behavior should derive its state from the world and supported persistent APIs.
Requests needing an unavailable persistent state mechanism belong in the backlog.

## Capability backlog

`coplay/backlog.json` is the independent, durable development backlog. It is never
included in world undo and is reused across worlds/restarts. A corrupt file causes
an explicit error; the game does not replace it with an empty backlog.

The panel supports viewing evidence, human notes, dismissal, explicit rechecking,
and exporting **`coplay/world_api_backlog.md`**. Entries use canonical capability
keys to merge occurrences and preserve multiple evidence examples. Times are Unix
milliseconds. Build IDs include a repository source fingerprint. Proposed API
extensions are labeled proposals. Resolution requires a human-requested recheck
and a passing gameplay observation in that build; the evidence is retained.

No speculative gameplay gaps are seeded. The first unsupported request creates
the actual entries. Use the Export Markdown button even for an empty backlog.

## Validation commands

```powershell
cargo test --offline coplay -- --nocapture
cargo test --offline --features dev-playtest
cargo build --offline --features dev-playtest
target/debug/voxelproject.exe --coplay-check
```

The last command is an explicit paid integration check. It starts a separate
world in the real game loop with a hidden window, uses the configured provider,
creates a rain rule, revises the same rule to mist, saves/reloads its definition,
undoes the revision, and requests a custom UI button twice to check evidence-backed
capability reporting and deduplication. It writes `target/coplay-live.json` with observed game
state and provider/tool evidence, and exports the backlog. It does not test
manual mouse/keyboard usability or a second connected client.

Validation on 2026-09-24: all nine focused Co-Play tests passed. The complete
development suite passed 657 tests with 32 environment-dependent tests ignored.
The live configured-provider check passed all six phases: rain, same-ID mist
revision, reload, undo, missing UI capability reporting and repeated-request
deduplication. Its four requests consumed 213,363 reported tokens in total, each
within its configured per-request budget. `CAP-0001` in the exported backlog is
the confirmed custom-UI capability gap from that integration scenario.

Manual panel interaction and multiplayer with a second client were not exercised.
Host authority is covered by automated guards/tests; mutations reuse the normal
replication path. The live reload check restores saved rule definitions in the
running game; it does not restart the executable. Backlog restart persistence is
covered by the filesystem round-trip test.
