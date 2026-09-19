# Hunger and eating

Hunger is a gentle reminder to bring food on expeditions. It runs automatically;
no generated rule or Lua module is needed. The fullness meter stays hidden, and
the HUD shows a status only when you need food.

## Player guide

From full, without eating:

| Time in the running simulation | Effect |
| --- | --- |
| About 16 minutes 15 seconds | **HUNGRY** appears; no hunger damage yet. |
| About 23 minutes | **HUNGRY - eat to stop health loss** appears. Hunger drains approximately 1 health every 30 seconds. |
| At 50 health or below | Hunger stops dealing damage. It cannot kill you or heal existing injuries. |

Other damage, including poison, creatures and drowning, still applies normally.
Hunger does not slow movement. Any successfully eaten food immediately clears
hunger and stops its damage, even if that food has no fullness bonus. You do not
have to fill the meter or eat several items to stop the effect.

Press **I**, select food, and use **Eat 1** for resource stacks or **Eat** for
harvested foods and dishes. Aim at a campfire and press **F** to cook. Cooked meat,
pumpkin and mushrooms provide more fullness than their raw counterparts, so they
postpone the next hunger warning. Raw eggs must be cooked before eating.

Eating does not generally cure poison. Glowcaps and toxic dishes still poison
you while satisfying hunger; herbal purifying stew cures poison.

**Current UI limitation:** resource **Eat 1** is disabled at 100 health, even if
you are hungry. Harvested-food and dish **Eat** buttons remain available. The
host already accepts resource meals at full health when fullness is below maximum;
the resource button still needs to use that condition.

## Implementation and multiplayer

- `src/hunger.rs` shares the same calculation between the host player and guests.
  Fullness starts at 100, drains at `100 / 1500` per simulation second, and uses
  thresholds of 35 for hungry and 8 for damage. Damage is continuous at `1 / 30`
  health per second, with a floor of 50; it is not a discrete poison-timer tick.
- A successful meal sets fullness to
  `min(100, max(current, 45) + max(nutrition, 0))`. Even a zero-bonus meal gives
  about 2 minutes 30 seconds before the next hungry warning. Food-specific
  bonuses are defined in `src/food.rs`.
- The host consumes food and updates health/fullness. Snapshots now carry
  `satiety`, so guest HUDs receive authoritative hunger state. Both eating paths
  clear hunger only after successful consumption.
- Host fullness is captured in `PlayerSave`; older saves without the field
  default to full. Guest fullness is session state, not part of saved guest
  crafting accounts. Hunger does not advance while the game is closed.
- This change raises the multiplayer protocol from **47 to 48**. Host and guests
  must use the updated build. The Lua API is unchanged.

## Verification

The hunger, food, network and save test filters passed (29 tests total):

```powershell
cargo test --offline hunger
cargo test --offline food::tests
cargo test --offline net::tests
cargo test --offline save::tests
```

New tests cover both grace periods, damage pacing, the health floor, already
injured/dead players, meal relief and nutrition differences. The network snapshot
round-trip includes low fullness. Live host/guest gameplay and HUD checks remain
manual validation work.
