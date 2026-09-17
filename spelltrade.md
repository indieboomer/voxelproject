# Spell Trading Mechanics

Saved spells are represented by physical spell cards. A card stores only the
immutable numeric ID of a saved spell; it never stores or transfers executable
Lua source.

## Card ownership

- The host receives a card when an instant spell is remembered.
- Duplicating a spell also creates a card for the host.
- Cards are persisted inside each player's saved account.
- Each spell ID appears at most once per account.
- An account can hold up to `MAX_SPELLS` cards.
- Deleting a spell removes cards referencing that spell from the host and
  guest accounts.
- Older saves are migrated by giving the host cards for validated saved spells.

## Direct player transfer

The Spellbook panel has a **Trade card to** field and a **Give card** button.
The recipient can be a connected player's nickname/display name, or `host`
when a guest is returning a card to the host.

The host validates every transfer:

1. The sender is authenticated from the network connection.
2. The referenced spell exists in the host spellbook and is validated.
3. The sender owns that physical card.
4. The recipient is connected and has room for the card.
5. The card is removed and added atomically; failures roll back the removal.

The network message contains only the spell ID and recipient name. Both
accounts receive an authoritative `CraftState` update after a successful
transfer.

## Chest trading

Automation chests treat cards as special matter IDs in the form
`spell:<id>`.

- Open a chest through the Automation UI.
- Select the card from **Transfer matter**.
- Use **Deposit** to place the card in the chest.
- Another player can use **Withdraw** to claim it.
- If chest content ejection is enabled, the card is released as a physical
  loot bag that another player can pick up.
- Cards can also move through supported automation matter networks.

Chest deposits, withdrawals, ejections, and loot pickups all run through the
host account transaction path. Unknown, deleted, or unvalidated spell IDs are
rejected even if a client submits a forged item name.

When a guest picks up a card from a chest or loot bag, the host sends the
updated account state so the card appears in the guest's inventory.

## Casting requirement

Guests must own the physical card to cast a saved spell. The host still checks
the spell's guest-casting permission, validation status, target, cooldown, and
mana cost before execution.

The host's spellbook remains authoritative. Trading changes ownership of the
card only; it does not copy, edit, or transfer the saved spell definition.
