#!/usr/bin/env python3
"""Builds assets/textures/atlas.png from the Minecraft-format resource pack
in textures/assets/minecraft/textures/block/.

That pack is a curated subset (lots of wood species + ores, but no plain
dirt/stone/water, and both the grass edge and oak_leaves are grayscale
"biome tint" templates with no colormap included). This script derives what's
missing (dirt/water are procedural; sand comes from textures/sand.png, a
separate hand-picked tile not part of the resource pack) and bakes tints in
directly, so the game can just sample one flat atlas with no runtime tinting
logic. Re-run after changing anything under textures/ to regenerate
assets/textures/atlas.png.

Layout: 4 columns x 4 rows of 64x64 tiles (256x256 total), row-major:
  0 grass_top   1 grass_side   2 dirt        3 stone
  4 sand        5 wood_side    6 wood_top    7 leaves
  8 crystal     9 mud         10 redstone   11 water
 12 white       13-15 unused (filled white)

Tile 12 (white) is sampled by entities (creatures/players), which are
tinted purely through per-vertex color rather than a real texture -- keeps
world geometry and entities on one shared pipeline/atlas.
"""

from pathlib import Path
from PIL import Image
import random

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "textures" / "assets" / "minecraft" / "textures" / "block"
RAW = ROOT / "textures"
OUT_DIR = ROOT / "assets" / "textures"
OUT = OUT_DIR / "atlas.png"

TILE = 64
COLS, ROWS = 4, 4

LEAF_TINT = (86, 148, 64)
GRASS_SPLIT_ROW = 16  # measured: rows 0-15 are the green strip, 16-63 dirt


def load(name: str) -> Image.Image:
    return Image.open(SRC / name).convert("RGBA")


def tile_vertically(im: Image.Image, target_h: int) -> Image.Image:
    w, h = im.size
    out = Image.new("RGBA", (w, target_h))
    for y in range(target_h):
        row = im.crop((0, y % h, w, y % h + 1))
        out.paste(row, (0, y))
    return out


def tint(im: Image.Image, color) -> Image.Image:
    r, g, b = color
    px = im.load()
    w, h = im.size
    out = Image.new("RGBA", (w, h))
    opx = out.load()
    for y in range(h):
        for x in range(w):
            cr, cg, cb, ca = px[x, y]
            # Source is a grayscale luminance template; use it as a
            # brightness multiplier against the tint color.
            lum = (cr + cg + cb) / (3 * 255)
            opx[x, y] = (int(r * lum), int(g * lum), int(b * lum), ca)
    return out


def darken(im: Image.Image, factor: float, warm_shift=(0, 0, 0)) -> Image.Image:
    px = im.load()
    w, h = im.size
    out = Image.new("RGBA", (w, h))
    opx = out.load()
    for y in range(h):
        for x in range(w):
            r, g, b, a = px[x, y]
            opx[x, y] = (
                max(0, min(255, int(r * factor) + warm_shift[0])),
                max(0, min(255, int(g * factor) + warm_shift[1])),
                max(0, min(255, int(b * factor) + warm_shift[2])),
                a,
            )
    return out


def procedural(base, variance, seed) -> Image.Image:
    rng = random.Random(seed)
    im = Image.new("RGBA", (TILE, TILE))
    px = im.load()
    for y in range(TILE):
        for x in range(TILE):
            jitter = rng.randint(-variance, variance)
            px[x, y] = tuple(max(0, min(255, c + jitter)) for c in base) + (255,)
    return im


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)

    grass_side_src = load("grass_block_side.png")
    grass_top_crop = grass_side_src.crop((0, 0, TILE, GRASS_SPLIT_ROW))
    # A hard vertical tile of this 16px strip reads as an obvious repeating
    # stripe once it covers a whole ground plane; a blocky (nearest-filter)
    # stretch instead gives a much calmer, still-crisp result.
    grass_top = grass_top_crop.resize((TILE, TILE), Image.NEAREST)

    dirt_crop = grass_side_src.crop((0, GRASS_SPLIT_ROW, TILE, TILE))
    dirt = tile_vertically(dirt_crop, TILE)

    mud = darken(dirt, 0.55, warm_shift=(-6, -10, -4))
    stone = load("andesite.png")
    wood_side = load("oak_log.png")
    wood_top = load("oak_log_top.png")
    leaves = tint(load("oak_leaves.png"), LEAF_TINT)
    crystal = load("amethyst_block.png")
    redstone = load("redstone_ore.png")
    sand = Image.open(RAW / "sand.png").convert("RGBA").resize((TILE, TILE), Image.LANCZOS)
    water = procedural((45, 95, 150), 10, seed=2)
    white = Image.new("RGBA", (TILE, TILE), (255, 255, 255, 255))

    tiles = [
        grass_top, grass_side_src, dirt, stone,
        sand, wood_side, wood_top, leaves,
        crystal, mud, redstone, water,
        white, white, white, white,
    ]
    for t in tiles:
        assert t.size == (TILE, TILE), t.size

    atlas = Image.new("RGBA", (TILE * COLS, TILE * ROWS))
    for i, t in enumerate(tiles):
        col, row = i % COLS, i // COLS
        atlas.paste(t, (col * TILE, row * TILE))

    atlas.save(OUT)
    print(f"wrote {OUT} ({atlas.size[0]}x{atlas.size[1]})")


if __name__ == "__main__":
    main()
