# agentic-test review, 2026-09-28

Evidence: `target/debug/saves/world-agentic-test.bin` (last written 17:52:58 local), its chat log, and `target/debug/saves/coplay.log`, especially request `request-1790610549015-26`.

The saved world contains 81 creatures, next creature ID 97, time 0.91628486, and only the disabled `create_chicken` spell. No night-horde rule was installed. The requested behavior was 10–20 zombies/skeletons plus 1–2 skeleton bosses in a 40-block diameter around the player, disappearing at dawn.

The agent twice called `upsert_rule` with the invented ID `night_horde_dawn_cleanup`. Nonempty IDs mean revision of an existing rule; new rules require an empty ID. The resulting “Rule missing or attached; inspect again” message led it to blame target validation and file CAP-0001. Inspections correctly reported the target available. The request ended blocked, with no committed change, after 17 steps and about 143 seconds. CAP-0001's original diagnosis is misleading; retain the record as evidence rather than marking it runtime-verified by this source change.

The proposed Lua had further lifecycle problems: `owned_ids` and the wave index existed only in VM memory, so reload would lose cleanup ownership and could create duplicates. `destroy` emits death events, potentially activating unrelated death rules. The spawn radius was silently clamped to 10 despite the requested radius of 20. Cleanup based on nearby queries would also miss creatures that wander away.

Earlier evidence: chicken-jump code first compared a Lua string kind with numeric `1`; a later revision corrected that. Several earlier requests committed changes but exhausted the model budget before verification. The saved chicken spell also queries around the corner of its intended search volume and treats the spawn result as two values even though the API returns one ID or nil. These are authoring/verification weaknesses, not evidence that a new native creature type is needed. `skeleton_sorcerer` already exists.

## Implementation plan and changes

1. Preserve revision conflict checks; explain empty-ID creation directly in errors and agent instructions, with a concrete tool example.
2. Add saved creature groups, transactional group assignment/query, and silent `despawn`. Group queries cover all live creatures, independent of player distance. Groups are labels, not security boundaries; use unique stable names.
3. Expose full group membership to Co-Play inspection/verification, including explicitly requested empty groups.
4. Allow land spawn radii up to 32; retain the 10-block limit for fish searches and spatial queries.
5. Provide `modules/night_horde.lua`: maintains 15 ordinary undead plus one sorcerer at night, at most four spawn attempts per tick, removes only its group in daylight, and reconciles saved members after reload. This example replenishes casualties; it does not promise a once-per-night quota. A durable once-per-night quota needs additional saved rule state. Disabling a rule stops its cleanup callbacks: explicitly clean the group when removing its effects.
6. Validate create/revise errors, callback rollback, label limits, silent removal versus death, save compatibility, night spawning, radius, VM/creature reload, dawn cleanup, and the next night.

The original save and backlog are not rewritten. In-game observation of the updated build remains separate from automated regression evidence.

## Validation

`cargo test --offline -- --test-threads=1`: 642 passed, 31 ignored, zero failures.
After strengthening the reload test to move group members 1,000 blocks away,
`cargo test --offline lifecycle_ -- --test-threads=1`: all four lifecycle tests passed.
The full suite includes the new Co-Play creation-error regression. Ignored tests
require external services, a GPU or manual benchmarking; no live model or graphical
multiplayer playtest was performed for this change.
