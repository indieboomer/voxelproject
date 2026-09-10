# Interpreted local rule generation

The game continues to use the configured Qwen2.5-Coder 7B model and local llama.cpp server. No new model or service is required.

## Generation

1. Interpret the original request into a schema-constrained plan: execution type, condition, affected species, desired effect, relevant API groups, requirements and unsupported assumptions.
2. Validate enums, sizes and contract consistency. Resolve protection beneficiaries independently of the effect name to avoid conflating protection of players with suppression of all combat. Typed policies use explicit actor/target scope validation and executable checks; the custom Lua path reviews the actual generated code with a bounded correction attempt. Standalone model criticism of abstract plans was removed after live testing found false rejections of valid requests. Unsupported or still contradictory interpretations return an error without adding a rule.
3. Compile supported temporary attack policies from generic capabilities. This supports weather or day/night conditions, any creature species, and either player protection or suppression of all that species' attacks. It is not a lookup table of natural-language prompts.
4. Run compiled policies against isolated rain/sunny/storm/mist, day/night and condition-exit scenarios before accepting them. Tests cover rule disable and the normal sandbox. More complex requests use a focused API prompt, Lua generation, API linting, isolated runtime smoke scenarios and a bounded model review/repair cycle; those are not automatically proven semantically correct.
5. Show the interpretation alongside the original prompt in the existing Rules review panel. The disabled module stores its interpretation and structured plan in source comments, which survive save/reload. Existing Enable/Run controls still determine activation.

## Temporary attack policies (World API 1.16)

`protect_player(player_id, creature_kind)` prevents the named species from landing melee attacks on that player. It preserves creatures' underlying aggression, targeting and attacks against other creatures. `suppress_creature_attacks(creature_id)` prevents that creature from landing attacks on any target. Neither stops chasing or movement.

Both methods are for on_tick only. Each successful tick replaces that rule's policies with the policies requested by that callback. If a condition is false, the callback adds none and its previous policies are removed. Disable, deletion, failed callbacks and reactivation remove the old activation's policies. Independent rules compose: removing one cannot cancel another's protection. Policies are transient; saved enabled rules recompute them after load, avoiding stale saved aggression changes. Changes take effect through the existing host simulation and damage/animation replication; clients do not execute a second authoritative copy of the rules.

Rules already generated using set_aggressive/ignore retain their original persistent behavior. Disable and regenerate those rules to get temporary protection; manually undo persistent aggression changes if an older rule has already made them.

## Validation and evaluation

- Offline: `cargo test --features steam`.
- Live smoke: `cargo test --features steam live_natural_language_pipeline -- --ignored --nocapture`.
- Live 32-case evaluation: `cargo test --features steam live_intent_evaluation_suite -- --ignored --nocapture`.
- Dataset: `data/intent_eval.json`.
- Outputs: `target/intent-pipeline-live.json` and `target/intent-evaluation.json`.

The broader evaluation reports acceptance/errors rather than asserting perfect model accuracy. For typed policy cases it checks expected behavior against the authored reference plan, not merely successful generation. Custom cases check generation plus sandbox loading; inspect their Lua to assess intent. Model review is fallible and is not a substitute for the user's review or a general proof of correctness. Multiple local model calls add latency, especially on the custom-code path. Network failures, truncation, invalid plans and unresolved review errors fail explicitly without activation.

## Validation results from implementation

The regular Steam-feature suite passed 281 tests (13 opt-in tests ignored). A 32-case live evaluation during development accepted 30 cases, including 21/22 authored policy scenarios and 9/10 custom generation cases. The original omitted-target phrasing and a false rejection of healing prompted further scope/review corrections. The final focused live rerun passed all four protection prompts, including both original examples, checking their expected scope and conditions. Healing, inventory grants and time changes also passed live smoke checks during development. The full 32-case suite was not rerun after the final corrections; these figures are not a guarantee of perfect language understanding.

Structured interpretation/scope replies use greedy decoding (temperature 0); Lua generation retains temperature 0.2. When two supported scope readings disagree, player-only protection is preferred to avoid unexpectedly pacifying combat against other creatures. The resulting scope is shown explicitly in review. The model remains Qwen2.5-Coder 7B Q4_K_M.

## Preflight latency

The client retains its last successfully validated interpretation and independently checked target scope for five minutes. Corrective retries can reuse that interpretation. It also retains up to eight successfully reviewed code results for five minutes, keyed by the exact prompt and execution kind within one client/endpoint. Repeating an exact request reuses that code after fresh API lint and isolated sandbox/scenario checks. A cache miss follows the complete interpretation/generation/review pipeline. Failed generations are not cached, and corrective retries bypass the completed-code cache. These caches are in memory only and never activate a rule.

The bundled server now starts with `--ctx-size 32768 --parallel 1`. Only one host pipeline runs at a time, so reserving multiple large contexts is unnecessary. The previously running server reported four slots and `n_ctx=111616`. The new limits apply only when the game starts a server; an already-running server or a manually managed endpoint is left alone. Exit the game, stop its bundled `llama-server.exe`, then launch the game to apply these startup defaults. The model is unchanged.

Logs now record preflight elapsed time/cache hits and model-request time/token counts. Opt-in measurements:

```powershell
cargo test --features steam profile_preflight -- --ignored --nocapture
cargo test --features steam profile_preflight_cache -- --ignored --nocapture
```

On the existing server, the cache benchmark generated the same player-protection rule in 2.179 seconds initially and 0.006 seconds on repetition, including executable policy validation both times. This is a repeat-request result, not a claim about new prompts. Initial four-prompt measurements took 1.5–4.2 seconds for interpretation, with an additional roughly 0.7–0.8 seconds for policy scope extraction; server load and prompt-cache state affect timings. The reduced server allocation has not yet been benchmarked against that running instance. Shorter semantic plans were tried and rejected after live tests found interpretation regressions; the original interpretation instructions, schema, examples, and validation remain intact.

Final verification: 303 offline tests passed (19 opt-in tests ignored), all seven live pipeline prompts passed, the live cache benchmark passed, and the Steam build completed.

## Guest prompting and current latency work

Settings > Multiplayer > **When hosting: allow guests to prompt** is off by default.
Changing it while hosting applies to the current session; snapshots advertise the
host's setting. A guest's own hosting preference cannot grant permission in someone
else's session. Host prompting is always available.

With permission, guests use their own local AI and submit the original prompt plus
generated Lua to the host. Full Windows and macOS packages already include the model;
client-only packages need a separately configured local server to generate code.
Guests start inference on their first prompt, while hosts still prewarm it. Startup
waits for model readiness on a worker instead of immediately failing on a cold model.

The host checks membership, permission, input size (2 KiB prompt / 32 KiB code), a
10-second per-player submission interval, and one validation worker at a time.
Untrusted guest code receives fresh API lint and isolated sandbox checks on that
worker. Permission and identity are checked again before adding a disabled module.
Only the host can Enable/Run/Delete rules in the authoritative world. Guest copies
are read-only previews. The original prompt and escaped author/account attribution
persist with the saved source. Guest instant spells run with the requesting player's
caster identity when approved; they cannot be cast while that account is disconnected.
Direct identities retain the existing nickname-based limitations.

Protocol is now 12; all players need the updated build. Proposal delivery uses the
existing reliable channel, shared by Steam and Direct. Nothing executes merely from
receiving a proposal. Host-generated and guest-generated effects use the same existing
authoritative simulation and replication.

New-prompt checks were retained: most latency is inference, and earlier attempts to
shorten interpretation regressed accuracy. Requests explicitly enable llama prompt
caching. The largest improvement is exact-repeat custom prompts, which now avoid all
model calls while repeating sandbox validation. Generation-worker disconnection now
reports an error instead of leaving the UI waiting forever.

```powershell
cargo test --offline profile_reviewed_code_cache -- --ignored --nocapture
```

This benchmark compares a new `heal me` generation with validated reuse in the same
client. Cold model loading is additional latency; timings are machine-specific.

Measured on 2026-09-10 with the existing 7B Q4_K_M model: with the server already
loaded, a fresh client generated/reviewed `heal me` in **2.356 s**, and the identical
request reused reviewed code with fresh sandbox checks in **0.003 s**. An earlier
cold-start pass took **48.802 s** including startup, followed by **0.003 s** reuse.
This does not establish a first-time speedup versus the previous implementation.

Verification for this update: 312 tests passed in both Direct and Steam configurations
(19 and 20 opt-in tests ignored respectively), the live custom-cache benchmark passed,
and the Windows installer and Mac bundle-layout checks passed. A live multi-PC guest
proposal/approval session still needs testing.
