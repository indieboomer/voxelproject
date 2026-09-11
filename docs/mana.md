# Mana and decomposition

Mana is shown beside the five elements in inventory [I]. The host grants one
mana every five seconds of play, up to 100, to each connected player. Element
conversion in Crafting [C] can raise the balance above 100; recovery never lowers
it. Existing saves keep their balance, including zero, and recover normally.

| Action | Mana cost |
| --- | --- |
| Successfully create a persistent rule | 20, charged to its author |
| Successfully cast an instant | 5, charged to its caster each time |
| Decompose gathered resources | 1 per resource consumed |
| Create/decompose equipment | Recipe below |

Failed generation, rejected proposals, failed casts and failed crafting actions
cost nothing. Rule creation is charged when the validated module is added for
review, before enabling it. Disabling/enabling an existing rule has no further
charge; running rules have no upkeep. Instant generation is free. Busy script
queues reject a new paid cast without spending mana; the player may retry.
Resource decomposition uses the existing catalog's elemental compositions.

Select equipment in inventory to see both Create and Decompose actions, or select
a resource to decompose one unit. Each button shows its cost and the materials
returned or required. Crafting [C] also supports bulk resource extraction and
element-to-mana conversion. Transactions validate ownership, mana, quantities,
balance overflow and account revision on the host before committing any changes.

| Equipment | Create materials | Create mana | Decompose returns | Decompose mana |
| --- | --- | --- | --- | --- |
| Sword | 2 iron + 1 oak wood | 4 | 1 iron + 1 oak wood | 2 |
| Axe | 3 iron + 2 oak wood | 4 | 2 iron + 1 oak wood | 2 |
| Pickaxe | 3 iron + 2 oak wood | 6 | 2 iron + 1 oak wood | 3 |

Iron forms the blade/head and oak wood the handle. Decomposition loses some
metal and, for heavier tools, wood. It cannot return more of any material than
construction spent. The existing elemental crafting formulas for iron and wood
remain unchanged. Mana is an additional cost, never a substitute for materials.
