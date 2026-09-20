# Spell feedback and diagnosis

Use **T** to comment while testing spells, then **F5** to save. The host exports
the transcript to `saves/<worldname>_chat.log` in the game's data directory.
The matching world save retains spell definitions, modules, and the transcript.
Save before leaving: this is a save-time export, not a crash-safe live recorder.

For precise attribution, click **Comment** beside a spell's Run / Remember /
View Code / Delete controls in Spell Workshop. Write up to 360 characters and
click **Post comment** (or Ctrl+Enter). The message includes that spell's name
and exact code identifier in session chat; guest posts also reach the host's
saved transcript. The local diagnostic record retains its prompt, source, and
matching saved-spell IDs/revisions. Cancel or Escape discards the draft.

Useful comments identify the spell, expected effect, observed effect, target,
and whether a repeat cast behaved differently. For example:

> Heal sheep, first cast: expected the wounded sheep to recover 20 health.
> The blue effect appeared, but its health stayed at 30. Second cast did the same.

Comment on generation too: whether the result matches the prompt, whether its
instant/enchantment/rule type is correct, and whether it requires unexpected
targeting or setup. Use the spell name when discussing an older spell; the
selected spell or hotbar item is only context, not proof of which spell you mean.

The original chat was insufficient for detailed diagnosis. It saved ordinary
messages and many notifications, but omitted timestamps, some panel-only cast
errors, validation retries, generated code that was rejected/deleted, and target
and inventory context. It could report a requested cast without establishing
that the intended gameplay effect occurred.

New records include:

- Timestamped chat/notifications and structured `player_comment` records.
- Generation prompts, API version, requested type, returned Lua, validation and
  corrective retry reasons. Each distinct code snapshot has a stable `code_id`.
- Saved-spell IDs/revisions, cast targets, position, mana before/after, immediate
  feedback, and authoritative guest results linked to their request sequence.
- Instant cast block-edit counts and samples (first 32), player effects,
  warnings, errors, and broadcasts. Saved-spell cast records also include
  creatures changed/spawned/removed during execution; creature tuples are
  `[id, kind, position, health, max_health]`.

Detailed `[spell-diagnostic]` records are single-line JSON in the saved transcript;
they do not fill the on-screen chat or get broadcast to other players. Code is
captured once per distinct source per session. Guest-side generation details
are local to that guest; the host log includes accepted guest cast execution.

This is diagnostic evidence, not a deterministic replay or automatic verdict.
Successful execution means the runtime accepted the call, not that the player's
intent was satisfied. Ongoing enchantment/rule effects, movement, animation,
rendering, latency, and all intervening world state are not recorded frame by
frame. Internal AI planning/review calls, exact model identity, and complete
model responses are not recorded. Screenshots/video and player observations
remain useful for visual issues or delayed behavior.

For analysis, provide the chat export and matching world save. Group the events
by spell ID/code ID and cast attempt, then report: intended behavior, generation
and validation result, observed runtime effects, player observation, likely
cause, confidence, and a concrete fix with a regression test. Clearly distinguish
facts in the log from hypotheses; record missing evidence before changing code.
