# Targeted Magic and Persistent Spell System

> Stages 1A–1D are implemented: targeted casts, a saved Spellbook, host-permitted
> guest casting, inventory/hotbar integration, casting HUD and particles, plus
> bounded temporary statuses and creature pushes. Existing APIs cover the other
> Stage 1D action categories. Phase 1 still needs live multi-player acceptance.
> [Implementation notes](docs/SPELLCASTING.md) track checks and limitations.
> Stage 2A is implemented: saved creature/block/device attachments, target-loss
> handling, host review and shared targeting labels. See [enchantments](docs/ENCHANTMENTS.md).
> Next development phase: 2B, owned effects and composition. Live acceptance remains open.

## Goal

The next major development of the World API and natural-language prompting should transform the current rule console into a genuine in-game magic system.

The system should support two distinct but connected experiences:

1. **Reusable spells cast from the hotbar at the current target.**
2. **Persistent enchantments attached to a specific object, creature, device, block, or area of the world.**

These phases should remain separate in implementation and player experience. Phase 1 provides an immediate, repeatable and trailer-friendly magic loop. Phase 2 expands the same foundations into the more revolutionary idea of directly reprogramming individual parts of the world.

---

# Phase 1: Reusable Targeted Spells

## Player fantasy

The player describes a spell in natural language, reviews the generated result, saves it, assigns it to the hotbar, and then casts it repeatedly at whatever valid object is currently targeted.

The core loop is:

**describe a spell → generate and validate it → save it in the Spellbook → assign it to the hotbar → aim at a target → cast**

Examples:

* “Heal the creature I am aiming at.”
* “Set the targeted enemy on fire.”
* “Turn the targeted block into stone.”
* “Summon a wolf beside the targeted creature.”
* “Push the targeted creature away from me.”
* “Make the targeted machine stop working for ten seconds.”

The spell is generated only once. Casting it later must not invoke the LLM again.

## Separation between spell creation and spell casting

The implementation should treat these as two different operations.

### Spell creation

During creation, the LLM receives:

* the player’s natural-language request;
* the expected target category;
* the relevant subset of World API documentation;
* spell cost and safety constraints;
* representative test fixtures.

The result is a reusable spell definition containing:

* stable spell ID;
* name and description;
* original prompt;
* required target type;
* generated Lua source;
* API version;
* mana cost;
* cooldown;
* maximum range;
* validation results;
* icon or icon parameters;
* revision number.

### Spell casting

During casting, the game supplies the current authoritative target to the already generated spell:

```lua
function on_cast(api, event)
    local target = event.target

    if not target or target.kind ~= "creature" then
        error("Aim at a creature")
    end

    if not api.heal_creature(target.id, 20) then
        error("The creature can no longer be affected")
    end
end
```

The target ID must not be permanently embedded in the spell source. The same healing spell should work on different creatures each time it is cast.

## Target context

The client should collect a small targeting snapshot, but the host must validate it before execution.

The cast event should expose:

```text
event.player_id
event.cast_id
event.origin
event.facing
event.hit_position
event.hit_normal
event.target
event.selected_region
event.world_revision
```

Initial target types should include:

* creature;
* player;
* block;
* automation device;
* loot object;
* empty world position.

Each spell declares which target types it accepts. A creature-healing spell cannot be cast on a block, while a block-transformation spell cannot be cast on a creature.

The host verifies:

* that the target still exists;
* that its ID and type match;
* that it is within spell range;
* that terrain does not obstruct the cast where required;
* that the caster still has sufficient mana;
* that the cooldown has expired;
* that the target belongs to the current world revision.

If validation fails, the spell produces no effects and does not consume mana.

## Spellbook UI

Phase 1 requires a dedicated UI for remembered spells. The existing rules panel should not become the only place where players manage reusable magic.

The Spellbook should provide:

* a searchable list of saved spells;
* spell name and short description;
* icon;
* required target type;
* mana cost;
* cooldown and range;
* original prompt;
* current revision;
* validation status;
* assignment to a hotbar slot;
* test-cast or preview action;
* edit, duplicate and delete actions;
* access to generated code as an advanced option.

A selected spell should clearly show:

> **Healing Touch**
> Target: Creature
> Range: 18 m
> Cost: 5 mana
> Cooldown: 1.5 seconds
> Effect: Restores 20 health to the targeted creature.

The UI should prioritize the understandable gameplay effect. Lua source and technical validation logs should remain available but secondary.

## Hotbar integration

A saved spell should be assignable to any compatible hotbar slot in the same way as equipment.

When a spell is selected:

* the held-item display may show a wand, hand effect or magical focus;
* the crosshair indicates whether the current target is valid;
* the HUD shows the spell name, mana cost and cooldown;
* a valid target receives a clear outline;
* casting uses the spell without reopening the prompt console.

Suggested targeting feedback:

* gold: recognized object;
* green: valid target for the selected spell;
* red: invalid or obstructed target;
* grey: out of range;
* violet: object affected by persistent magic.

The hotbar stores a reference to a stable spell ID, not the position of the spell in a list.

## Saving and loading spells

Remembered spells must be part of normal persistence from the first Phase 1 implementation.

The save format should store:

* stable spell ID;
* owner or author identity;
* spell name and description;
* original prompt;
* Lua source;
* structured interpretation;
* required target schema;
* mana and cooldown configuration;
* API version;
* spell revision;
* hotbar assignments;
* validation metadata.

Loading a world should restore both the Spellbook and hotbar assignments.

If a spell was created against an older World API version, the game should:

1. attempt deterministic compatibility validation;
2. keep it available if still valid;
3. mark it as requiring review if incompatible;
4. never silently regenerate or modify it with the LLM;
5. preserve the original spell source and prompt.

Multiplayer worlds should keep spell definitions authoritative on the host. A joining player receives the spell metadata required for their Spellbook and hotbar, while execution remains server-authoritative.

## Phase 1 implementation stages

### Stage 1A — Targeted casting foundation

* Add raycast targeting for creatures and blocks.
* Introduce `TargetContext` and `event.target`.
* Validate range, visibility and target identity on the host.
* Implement three reference spells: heal creature, damage creature and replace targeted block.
* Ensure failed casts roll back and refund the base cast fee.

### Stage 1B — Persistent Spellbook

* Introduce stable spell IDs and revisions.
* Save and load generated spells.
* Add the Spellbook UI.
* Display target requirements, costs and validation status.
* Support rename, duplicate and delete.

### Stage 1C — Hotbar spell casting

* Assign remembered spells to hotbar slots.
* Display target validity and cooldown state.
* Cast without reopening the generation UI.
* Persist hotbar-to-spell assignments.
* Replicate casting and results to all connected players.

### Stage 1D — Expanded spell capabilities

Add a controlled set of composable actions:

* heal and damage;
* apply temporary status effects;
* push or teleport;
* transform a block;
* spawn near a target;
* manipulate supported devices;
* give, take, craft or decompose inventory items.

The API should prefer generic, bounded capabilities over many prompt-specific helper functions.

## Phase 1 completion criteria

Phase 1 is complete when:

* a player can generate a healing spell once and cast it on several different creatures;
* the spell can be saved, loaded and assigned to the hotbar;
* invalid targets do not consume mana;
* spells remain functional after save/load;
* hotbar assignments survive save/load;
* guests can cast permitted spells while the host validates every target and effect;
* loading an incompatible spell reports the problem without silently changing its meaning.

---

# Phase 2: Persistent Magic Attached to Specific Objects

## Player fantasy

The player points at a particular thing and describes how the world should behave around it from now on.

The core loop is:

**point at something → describe its new behavior → review the interpretation → attach the rule → observe and revise it**

Examples:

* “This sheep follows me whenever I carry a crystal.”
* “This wolf protects the camp at night.”
* “This tree glows during storms.”
* “This chest refills with one stone every morning.”
* “Creatures near this altar slowly recover health.”
* “This door opens when the nearby vessel contains at least twenty mana.”
* “Nothing hostile can enter this marked area.”

Unlike Phase 1 spells, these creations retain a specific target reference after activation.

## Persistent target references

A targeted rule needs a durable reference:

```text
CreatureTarget { entity_id, expected_species }
DeviceTarget   { device_id, expected_type }
BlockTarget    { position, expected_material }
RegionTarget   { region_id, bounds }
CreationTarget { creation_id, revision }
```

The reference should also record the world ID and creation-time world revision.

Different targets require different failure policies:

* if a creature dies, its rule may terminate, suspend or execute an `on_target_lost` behavior;
* if a block is replaced, the rule may detach or remain anchored to the position;
* if a device is packed and placed elsewhere, the rule should follow its durable device ID only if explicitly supported;
* if a selected region changes, the rule should retain its saved revision until reviewed.

These semantics must be shown before activation.

## Creation and review UI

The object-first creation flow is superseded by reusable Enchantment spells.
Workshop generation selects a spell type and needs no world target. Remember
saves the definition to Spellbook without activating a rule. Neither panel offers
Run for Enchantments. Assign the card to a hotbar slot, then aim and left-click;
the host validates the cast and creates a separate persistent instance.

The Workshop and Spellbook show target requirements. A concrete target card and
readiness feedback belong to aimed gameplay, not generation. Each instance keeps
its source spell/version, owner and durable target; template edits do not silently
revise earlier instances. See [current behavior and acceptance checks](docs/ENCHANTMENTS.md).

The player should be able to:

* cast the remembered template on a target from the hotbar;
* preview its expected effect;
* edit parameters;
* inspect the generated code;
* revise it using natural language;
* disable it;
* view its history;
* restore a previous revision;
* detach it from the object.

Looking at an enchanted object should expose a compact list of its active rules.

## Owned and leased effects

Persistent rules must not permanently overwrite creature or player state.

Instead of directly setting speed, aggression or damage values, a rule should add an effect owned by its creation ID:

```lua
api.add_modifier(target.id, "movement_speed", "multiply", 1.3, {
    key = "storm_speed",
    refresh_for = 2
})
```

The engine records:

* creation ID;
* creation revision;
* target ID;
* property;
* stacking operation;
* value;
* priority;
* expiry or refresh lease.

If the rule is disabled, deleted, replaced or crashes, all effects owned by that rule are removed automatically.

This is required for predictable interaction between multiple enchantments. Two rules affecting speed, aggression or targeting must follow documented stacking and priority rules rather than overwriting one another in arbitrary execution order.

## Natural-language revision

Phase 2 should support contextual follow-up commands:

* “Make it stronger.”
* “Only during rain.”
* “Make it affect nearby sheep as well.”
* “Change this rule to wolves.”
* “Stop it from attacking players.”
* “Move this enchantment to the selected altar.”

The prompt pipeline should receive:

* the selected creation ID and revision;
* its current structured plan;
* the current target;
* a compact summary of previous revisions;
* only the relevant World API capabilities.

The result should be shown as a structured diff before activation.

A revised rule is validated alongside the currently active version. The engine swaps revisions atomically at a simulation tick boundary. If validation fails, the previous version remains active.

## Areas and multi-target rules

After single-object enchantments are reliable, targeting can expand to regions.

Players should be able to:

* select two corners;
* define a radius around a targeted object;
* name an area;
* preview its bounds;
* attach rules to it.

Examples:

* “Heal all sheep inside this circle.”
* “Skeletons respawn in this cave after one minute.”
* “Rain turns exposed soil in this field into mud.”
* “This sanctuary prevents hostile creatures from entering.”

All queries must remain bounded. The UI should show the estimated area, maximum affected entities and expected runtime cost.

## Phase 2 implementation stages

### Stage 2A — Persistent single-object rules

* Add durable references for creatures, blocks and devices.
* Attach a rule to one target.
* Save and load the relationship.
* Display active enchantments on the targeted object.
* Define behavior when the target disappears.

### Stage 2B — Owned effects and composition

* Add leased modifiers and behavior overrides.
* Define stacking, priorities and clamps.
* Remove owned effects automatically on disable, crash or deletion.
* Report conflicts between rules.

### Stage 2C — Revision and history

* Add stable creation IDs and parent revisions.
* Show plan and code diffs.
* Validate replacements without disabling the current version.
* Support history and restoration after save/load.

### Stage 2D — Areas and named anchors

* Add point, radius and box selection.
* Support named regions and anchors.
* Preview affected bounds.
* Add enter, leave and interaction events.
* Introduce bounded multi-target queries.

### Stage 2E — Object-to-object relationships

Support rules connecting specific objects:

* a wolf guards a selected chest;
* a lever activates a selected device;
* an altar affects creatures in a selected region;
* a creature follows a selected player;
* one machine responds to the state of another.

These relationships should use durable references rather than coordinates embedded in generated Lua.

## Phase 2 completion criteria

Phase 2 is complete when:

* a rule can be attached to one specific creature and survive save/load;
* the object exposes its active enchantments in the UI;
* disabling the rule removes its temporary effects;
* two overlapping modifiers compose predictably;
* a destroyed or missing target produces a defined state rather than an error loop;
* a rule can be revised using “only during rain” and restored to its previous revision;
* a region rule can affect bounded targets and display its area before activation;
* all effects remain authoritative and consistent for up to four players.

---

# Recommended Product Order

Phase 1 should be completed before Phase 2.

Reusable hotbar spells provide an immediate and understandable player experience:

> I create my own spell with words, save it, equip it and cast it during normal gameplay.

Phase 2 delivers the deeper and more distinctive promise:

> I point at a specific part of the world and describe how it should behave from now on.

The two phases share targeting, structured interpretation, validation, persistence and host authority, but they should not be merged into one oversized implementation. Phase 1 proves that generated magic is enjoyable to use. Phase 2 turns the world itself into something the player can program through play.
