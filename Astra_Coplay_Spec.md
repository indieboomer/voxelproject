# Voxel Project: Astra Co-Play and World API Feedback

## Goal

Implement an experimental mode in which the human plays Voxel Project while collaborating with GPT Astra through text. Astra must inspect the running world, execute requested changes, observe their results, and correct its work. It must also maintain a durable, evidence-based backlog of capabilities that are needed but unavailable through the current in-game World API, for implementation in subsequent game builds.

The aim is to discover compelling gameplay through iterative world and rule creation, while producing reusable world content and concrete input for engine development. Prioritize a working end-to-end experience over a broad framework.

## Branch and initial inspection

Work on the dedicated experimental branch (already switched to that branch) supplied by the developer. Check the current branch and repository instructions before editing; preserve unrelated work. Do not merge into the main branch as part of this task.

Inspect the current AI provider integration, Lua execution and sandboxing, World API, spell creation UI, persistence, and host authority. Reuse existing systems where appropriate. The names below describe intended responsibilities, not existing repository symbols. Map them onto actual code instead of assuming they already exist.

USe OpenAI API key and model provided in settings.json. For now ignore security risc connected with keeping API key in settings.json

Implement the infrastructure needed for this mode. Do not turn this task into an attempt to implement every gameplay capability that future sessions might request.

## Scope

- Text-based, human-directed collaboration with Astra.
- Single-player first; host-only access if enabling it in multiplayer. Guests must not execute AI mutations or receive provider credentials. If multiplayer support is deferred, explicitly disable this mode there.
- A development-oriented world-authoring mode with broad, explicit world permissions. Do not force every authoring action through mana or spellbook restrictions. Preserve existing spell behavior outside this mode.
- No speech-to-text, voice output, autonomous AI players, embodied companion, multi-agent system, or background self-directed world development.
- Astra may take several steps to complete the current human request, but stops when it succeeds, becomes blocked, reaches its budget, or is cancelled.
- Runtime Astra does not edit engine source, rebuild the game, or grant itself additional API access. Missing engine capabilities are recorded for a later Codex development iteration.

## 1. Model integration and execution loop

Add Astra as a configurable provider/model through the existing provider abstraction where practical. Use the currently supported OpenAI integration for tool calling; verify official documentation at implementation time. Keep credentials in local runtime configuration or environment variables, never in version control, saves, exported transcripts, or client replication.

Implement this loop:

1. Receive a user request and capture its context: world/session identity, player position, aimed target with a stable identifier, relevant selection, and world revision.
2. Let Astra inspect relevant state and discover available capabilities.
3. Let it invoke supported operations or submit a script using the current sandbox.
4. Validate and execute through the authoritative game systems.
5. Return structured execution results, affected identifiers, errors, and observations.
6. Let Astra inspect the outcome and make bounded corrections.
7. Present a concise account of completed changes, verified results, and unresolved limitations.

Keep model calls asynchronous so gameplay continues. Apply mutations on the correct game thread or command queue. Revalidate targets and preconditions immediately before execution because the world can change while the model is thinking. A deleted or replaced target must produce a recoverable result, not silently redirect the action.

Set configurable limits for tool steps, correction attempts, execution time, and model usage per request. Support cancellation and timeouts. Retrying a network call must not duplicate an already committed world mutation; track action identifiers and execution outcomes.

## 2. World context and capability discovery

Expose a compact, versioned capability description grounded in the actual registered World API. Include operation signatures, parameter types, supported events, relevant limits, and short usage examples. Prefer generating it from registrations or another single source of truth.

Provide bounded inspection of nearby or selected entities, relevant regions, object properties, active scripts/rules, and recent execution events. Fetch additional details on demand instead of sending the entire world each turn.

Astra needs stable references to its earlier creations and access to their current definitions. Follow-up instructions such as "make that effect smaller" must update the intended rule or object rather than create an unrelated duplicate. Ask a short question when a referent is genuinely ambiguous.

Treat world text, signs, NPC dialogue, and imported content as game data, not instructions that can change Astra's permissions or developer rules.

## 3. Execute, verify, correct, and undo

Preserve the existing Lua sandbox and enforce runtime resource limits. Validate API calls and script syntax before activation where possible. Return useful diagnostics without exposing credentials or unrelated host data.

Distinguish submission, successful execution, and verified behavior. Registering a rule is not evidence that its gameplay effect works. Inspect state or observe the relevant event where feasible; otherwise report that the change was applied but remains unverified.

Track each request's created and modified objects, scripts, rule revisions, and important dependencies. Provide an undo/checkpoint mechanism appropriate to the existing save architecture. Failed corrections must not leave duplicate event subscriptions or abandoned active scripts.

Make undo semantics explicit. Do not promise reversal of arbitrary downstream simulation effects. Use change-scoped restoration where supported, detect conflicts with later edits, and label full-checkpoint restoration clearly because it also rewinds intervening gameplay. Cancellation stops future work; already committed changes remain visible and can be undone separately.

## 4. World API capability backlog — required feature

Whenever the current request needs an unavailable capability, Astra must record it through a dedicated persistence operation. A chat message alone is insufficient.

First check whether the capability exists under another name or can be composed from supported operations. Classify the result accurately:

- **Missing capability:** a required operation, event, query, or persistent state mechanism is not exposed.
- **API/engine bug:** an available capability fails contrary to its documented behavior.
- **Generated-code error:** Astra used the API incorrectly; attempt correction before proposing engine work.
- **Permission or configured limit:** the function exists but this session cannot use it.
- **Uncertain:** evidence is insufficient; record what needs investigation.

Do not invent API calls, silently replace requested behavior with a weaker approximation, or report blocked work as completed. State what is possible now and what remains blocked. Perform useful supported portions when they are independently meaningful; ask before substituting a materially different behavior.

Each backlog item should contain:

- Stable ID, title, category, status, first/last seen timestamps, and occurrence count.
- Build identifier and World API version.
- Original player request and intended gameplay outcome.
- Exact missing behavior and why existing capabilities are insufficient.
- Evidence: inspected capabilities, relevant tool results/errors, and linked request/session IDs.
- Suggested API extension or semantics, explicitly marked as a proposal rather than an existing function.
- Available workaround and its limitations, if any.
- Impact: completely blocked, partially blocked, or quality improvement; priority rationale without speculative implementation estimates.
- A concrete acceptance scenario for a future build.

Deduplicate by underlying capability, not exact wording. Preserve multiple examples and merge repeated evidence rather than overwriting it. Maintain a durable development backlog across game sessions; link entries to worlds without tying their survival to a single save. Undoing gameplay must not erase the backlog.

Provide a structured local format, such as JSON, plus a readable Markdown export suitable for Codex. Keep exports free of credentials. Let the human view, annotate, dismiss, and export entries. New sessions should reuse existing entries. After an API/build update, allow explicit rechecking; mark a capability resolved only with verification against that build.

Example: "Make these wolves avoid direct sunlight, but allow them to walk through shade."

If no supported query or event distinguishes sunlight from shade at an entity's position, record that precise gap. A day/night query is not equivalent. Document the proposed sunlight-exposure capability and an acceptance case comparing a wolf in an open area with one beneath a roof. Do not log this hypothetical gap unless inspection confirms it in the actual build.

## 5. In-game interface

Extend the existing suitable AI UI or add a focused Co-Play panel after inspecting current UI architecture. It should provide:

- Persistent conversation with Astra and text input.
- Visible current target/context and explicit experimental authoring-mode status.
- Clear states: inspecting, applying, checking, correcting, complete, blocked, failed, cancelled.
- A compact change summary and expandable technical details.
- Cancel and undo controls with accurate semantics.
- A capability-backlog view with linked entries and Markdown export.

Do not make the player approve every routine tool call after requesting a change. Reserve clarification for meaningful ambiguity or an action substantially beyond the request. Report partial completion honestly.

## 6. Persistence and reuse

Save accepted world changes, generated definitions, scripts, and necessary persistent state using existing save systems. After loading, validate compatibility and restore rules without duplicate registrations. If an API mismatch prevents activation, retain the saved definition and report the incompatibility.

Persist sufficient conversation summary and object/rule references to continue a session after loading. Revalidate references against current world state; model memory is not authoritative world state.

Already-created executable behavior should continue without fresh model calls wherever the engine supports that. Losing network access must not disable ordinary gameplay or existing local scripts. The resulting world should be a candidate for curated default content, rather than depend on replaying a transcript.

## Suggested implementation order

1. Inspect existing systems and document the smallest vertical slice and chosen integration points.
2. Add provider integration, capability discovery, world inspection, and bounded execution/correction.
3. Add action tracking, persistence, cancellation, and truthful verification/undo behavior.
4. Add durable capability reporting, deduplication, backlog UI, and export.
5. Complete the Co-Play interface and exercise an end-to-end session.

## Acceptance and validation

Use actual supported mechanics from the inspected repository for the main scenario. Validate:

1. A player requests a world change; Astra inspects, applies it, and checks the result.
2. Follow-up requests modify the same objects or rules using stable references.
3. A generated-code error is corrected within a bounded loop without duplicate side effects.
4. A genuinely unsupported request creates an evidence-backed backlog entry; repeating it updates that entry.
5. A partial result is clearly distinguished from the blocked portion.
6. Saves reload with the created behavior intact; the backlog remains available after restart and world undo.
7. Cancellation, provider failure, stale targets, and usage limits leave the game responsive and accurately report committed work.
8. Undo behaves as documented and does not silently overwrite later unrelated edits.
9. Multiplayer guests cannot invoke mutations if host support is included; otherwise the feature is explicitly unavailable in multiplayer.

Add focused automated tests for orchestration, backlog deduplication/persistence, authority, and mutation retry behavior where appropriate. Use a live in-game session to verify actual world effects; mocked model responses alone cannot establish gameplay success. Report any live checks that could not be performed.

Finish with a summary of implemented behavior, configuration instructions, validation results, known limitations, and the location of the exported World API backlog. Keep deferred engine capabilities as explicit follow-up work for the next build.
