# Voxel Rune

`voxel-rune.ttf` is an original project font made from 5 x 7 pixel glyphs.
It includes uppercase and lowercase ASCII, punctuation, and common UI arrows,
dashes, multiplication and ellipsis. Other characters use egui's existing
fallback fonts. Lua source keeps its standard monospace font.

The editable glyph source is `tools/build_ui_font.py`. Rebuild with
`python tools/build_ui_font.py` in a Python environment with FontTools installed.
Neither Python nor FontTools is required to build or run the game: Rust embeds
the generated TTF. No Minecraft font, textures, or other external art is used.
