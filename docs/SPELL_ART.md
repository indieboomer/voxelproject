# Live spell artwork and card collection

Every generated interpretation requests a bounded visual recipe alongside its
behavior plan: subject, effect, secondary symbol and palette. The existing local
coding model chooses the composition; no additional model, service, download or
image-generation request is required. Cosmetic metadata never changes gameplay.
Malformed cosmetic metadata falls back to a deterministic description-based icon.
Manual modules and old saves get that same offline fallback.

The renderer builds a 128 x 128 illustration on a single CPU worker. Work queues,
CPU images and per-egui-context texture caches are bounded to 128 entries each.
The UI shows a placeholder while work completes, uploads the image once and reuses
it. There is no per-frame image generation, shader compilation, GPU readback or
script execution. The vocabulary is intentionally finite; related descriptions
can produce the same composition. The images are symbolic illustrations rather
than arbitrary painted scenes.

## Saving and multiplayer

Use the normal world save (F5/menu) and load actions. Artwork is saved as its
lossless procedural source, not a reference to an external PNG. Remembered spells
store a versioned `artwork` recipe in the existing world envelope. Rules, instant
actions and attached rules retain the recipe in their saved source's `Intent plan`
metadata. Older module binary layouts are unchanged. Duplicating a remembered
spell preserves its artwork; renaming or changing targeting does not reroll it.
Missing artwork in old spellbooks migrates on revalidation. Renderer v1 should
remain stable; new drawing styles must use a new renderer version.

Loading reconstructs images locally without inference or internet access. There
are no sidecar files to copy, lose or regenerate through AI. Steam/Direct spell
catalogs carry the same small host-approved recipe, not image bytes or Lua source.
Protocol version is now 47; all session participants must update together.

## Card collection

K opens a searchable, responsive collection of original fantasy spell cards.
Cards display artwork, name, mana, target, description, range, cooldown and review
status. Click a card to inspect the complete description, cast, duplicate, delete,
rename, adjust targeting or change guest permission using the existing controls.
Clicking a card does not cast it. The inventory and hotbar reuse its illustration;
Rules shows artwork for persistent and attached rules as well as instant actions.
Remembering is still limited to instant spells, preserving existing cast semantics.

## Pixelized artwork and flavor quotes

Settings > Appearance > Spell artwork offers **Default** and **Pixelized**.
Default retains the original renderer. Pixelized applies a 32 x 32 block-average
pass and 16 levels per color channel, then displays with nearest-neighbor
filtering. Both modes are cached independently and processed on the existing
worker. Switching is immediate; it does not run inference or alter saved recipes.
The choice persists in local `appearance.spell_artwork` preferences and does not
affect other players. Older settings default to Default artwork.

New generations ask the existing local model for an original short poetic
`flavor_quote` in the same interpretation response. Cards show it in italics above
the mechanical description. Quotes are cosmetic, bounded to 96 UTF-8 bytes and
stored in world saves and guest catalogs. Duplicating a card preserves its quote.
Old worlds remain compatible; existing quotes are preserved on load without AI.

For older cards, hosts can select **Generate quote (local AI)** in the card details;
**Rewrite quote (local AI)** replaces an existing quote. This uses the configured
local model asynchronously, leaves spell code untouched, and keeps the current
quote if the request fails. Only one such request runs at once. A response for a
deleted or revised spell is rejected. Use the normal world save (F5) to keep the
result. Missing quotes in older saves remain empty until generated.

Set `UI_PREVIEW_ART=pixelized` alongside `UI_PREVIEW_PANEL=spellbook` for pixelized
UI screenshots (filenames end with `-pixelized.png`). The quote-generation smoke
test is `live_flavor_quote_for_existing_spell` (opt-in, local llama-server).

## Verification

Automated coverage includes image identity across world saves, old-save migration,
module persistence, guest catalog image identity and packet bounds, unsupported
recipe versions, malformed cosmetic output and image composition differences.

```
cargo test --offline --features steam
cargo test --offline --features steam preview_and_measure_artwork -- --ignored --nocapture
```

The latter writes `target/spell-art-sheet.png` and measures CPU rendering (not
inference latency or frame time). Initial optimized development measurement on
this machine: 12 images in 12.7 ms total. For GPU UI previews, set
`UI_PREVIEW_PANEL=spellbook` or `spellbook_guest` and run the ignored
`render_ui_previews` test. Images are written under `target/ui-*.png`.

Validation: 571 Steam-feature automated tests passed (27 opt-in checks ignored); Direct-only compilation passed. Offscreen host and guest card previews passed in both UI themes. A live local-model instant-spell run produced valid Lua and the requested artwork metadata in the same pipeline; the final add-stone example chose block/transform/none/gold. Separate-account Steam gameplay was not exercised for this UI change.

Pixelization/quote validation: 574 automated Steam-feature tests passed (28 opt-in tests ignored). Direct compilation, Default/Pixelized card previews, Settings previews, live quote generation for an existing spell, and live generation of spell code with artwork and flavor text passed. No separate-account Steam gameplay run was performed.

Spell deletion and ownership: each host card now has a Delete button. Explicit deletion from Rules removes matching remembered copies (including renamed duplicates); temporary module cleanup after a cast does not. Missing inventory entries and unavailable spells are removed from every hotbar slot, with authoritative guest reconciliation and save cleanup. See inventory.md. Validation: 578 automated Steam-feature tests passed; Direct compilation and offscreen card previews passed.
