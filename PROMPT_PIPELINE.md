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

Both methods are for on_tick only. Each successful tick replaces that rule's policies with the policies requested by that callback. If a condition is false, the callback adds none and its previous policies are removed. Disable, deletion, failed callbacks and reactivation remove the old activation's policies. Independent rules compose: removing one cannot cancel another's protection. Policies are transient; saved enabled rules recompute them after load, avoiding stale saved aggression changes. Changes take effect through the existing host simulation and damage/animation replication; clients do not run a second AI.

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
