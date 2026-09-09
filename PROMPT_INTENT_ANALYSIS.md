# Natural-language rule interpretation analysis

## Finding

Less technical requests are feasible with a local model, but the current direct-to-Lua pipeline does not reliably preserve meaning. Better wording alone did not fix the tested case. Production code was not changed during this analysis.

## Live setup and evidence

Confirmed running model: Qwen2.5-Coder-7B-Instruct Q4_K_M, served by llama.cpp on localhost:8090. Tests used temperature 0.2, max_tokens 800 (400 for a constrained plan), fixed seed 42. Current full system prompt plus request used about 8,700 input tokens. The server reported 111,616 context tokens per slot, so these requests were not exceeding its context window. This is a small qualitative probe, not an accuracy benchmark. Outputs were inspected; generated rules were not enabled in the game.

Raw requests' resulting outputs and timings: [target/prompt-intent-analysis.json](target/prompt-intent-analysis.json).

| Experiment | Finding |
|---|---|
| Current pipeline: if it rains then wolf doesnt attack | Disabled wolf aggression in rain, but never restored it afterward. |
| Current pipeline: players are protected during rain from wolf attack | Invented a nighttime requirement and 10-block radius; never restored behavior. |
| Added general intent guidance to full prompt | Still invented nighttime and a 16-block radius. |
| Separate interpretation, then focused code generation | Understood rain protection; generated restoration branch for creature kind player, which cannot match a creature. |
| Same two stages with rain makes wolves leave adventurers alone | Invented weather value rainy instead of rain; mishandled saved false values. |
| Corrective review with behavioral requirements | Retained unreachable player-kind restoration branch. |
| Schema-constrained capability plan | Correct rain/wolf/set_aggressive=false/restoration fields, but incorrectly selected instant execution alongside an ongoing condition. |

Both original user examples classify as Rule. The earlier instant-versus-rule classifier issue does not explain their difference. API validation currently checks names/contracts/sandbox behavior, not whether a correct-looking rule implements the request. Existing examples include night-conditioned wolf-adjacent behavior; copying their conditions is a plausible explanation for the invented night restriction, not a proven causal finding.

## Recommended implementation

1. Add an interpretation step that separates affected actors, beneficiary, condition, desired effect, scope, and restoration. Preserve original request. Show a short interpretation in the existing review UI, such as: While it rains, wolves stop attacking players; afterward their previous aggression returns.
2. Validate a typed capability plan against the API schema, including exact enum values, supported actions, and consistency of execution type with ongoing conditions. JSON structure alone does not validate meaning. The constrained experiment manually limited the capability family, so it is not evidence of general-purpose planning accuracy.
3. Provide only the relevant API slice and a small number of varied examples to generation. Test this independently from introducing a planner; the two-stage probe changed both and cannot attribute improvement to one alone.
4. For common temporary effects, use reusable engine-managed behavior overrides or compile validated primitives to Lua. Handle saved false values, cleanup, newly spawned creatures, rule disable/reload, and competing rules centrally. Compile generic conditions/actions, not a list of hard-coded natural-language prompts. Keep sandboxed Lua for requests requiring richer logic.
5. Run scenario checks before activation: dry/day, rain/day, rain/night, rain ends, initially peaceful wolf, unrelated species, and multiple players. Confirm target selection, actual damage and restoration, not just successful compilation. Retry with concrete failures; reject or request clarification if meaning remains unsupported.
6. Benchmark 30-50 paraphrases and negative controls across inventory, creatures, weather and time. Track semantic correctness, restoration, accidental restrictions, p50/p95 latency and memory. Include genuinely ambiguous requests and concurrent rules. Do not infer general reliability from this single family of prompts.

## API semantics to address

Making all wolves peaceful is a practical approximation of protecting all players. It also cancels wolves' explicit creature targets. Exact protection of selected players while wolves continue hunting creatures needs a target-specific damage/attack policy. Do not silently interpret that narrower request as global pacification. Current persistent aggression overrides also outlive rule disable; temporary overrides need lifecycle ownership for reliable cleanup and composition.

## Model options

Start by improving and evaluating the current 7B pipeline. Benchmark Qwen3-8B as a similar-size general instruction/reasoning candidate; do not assume a newer model solves the problem. Its thinking/non-thinking modes and llama.cpp support are documented by the model authors. Thinking output requires an explicit token budget and parsing separate from Lua; the current 800-token response budget is not a sufficient default for unrestricted reasoning. Larger models are another experiment if memory and game rendering headroom allow; no alternative was downloaded or measured here.

Sources: [Qwen2.5-Coder model card](https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct), [Qwen3 author documentation](https://qwenlm.github.io/blog/qwen3/), [llama.cpp server documentation, including schema-constrained JSON](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).
