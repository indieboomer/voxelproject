# Organic trees (experimental)

In shaped mode (**F7**), living trees render as round, tapered trunks and limbs
instead of cubes (`src/voxel/organic.rs`). This is the first tree pass from
`Voxel_Project_Material_Shaping_and_Trees.md`: straight trunks with mild taper,
angled limbs and forks. Foliage is unchanged in this pass, so trunks can be
judged on their own.

| | Cubic | Organic |
| --- | --- | --- |
| Grove | ![](trees/wide-cubic.jpg) | ![](trees/wide-organic.jpg) |
| Close | ![](trees/close-cubic.jpg) | ![](trees/close-organic.jpg) |

Render with `VOXEL_TREE_PREVIEW=off|on cargo test --release render_weather_previews -- --ignored`.
`VOXEL_TREE_EYE=x,y,z` moves the camera. The grove has four species and a
player-built oak post and beam touching the oak's trunk.

## Living wood versus construction wood

A wood cell is **living** if it is a branch block (these only come from tree
generation) or trunk wood that is not a recorded edit (`organic::is_living`).
Wood a player or rule places is always an edit, so it stays construction
material and keeps its cube, even when it touches a tree. The rule needs no new
save data and survives save/load, remeshing and network replication, because
edits are the saved and replicated state. This is the same rule that keeps
placed leaves from swaying.

World generation only places wood through its tree generators, so in real worlds
"generated" means "tree". Authored test scenes that write wood as raw chunk
data are the exception (the base render preview has one), and that wood renders
as trees.

## Shape

- **Pieces.** Each living cell is drawn as tube pieces from its centre to a ring
  on every face it shares with a connected living neighbour of the same
  species. Both neighbours build a shared ring from the same face key: same
  centre, radius and irregular cross-section (±9%). Tubes therefore join
  without gaps across cells and chunks.
- **Trunks.** 10-sided, with a radius per species: oak 0.44, spruce 0.33,
  birch 0.27, cherry 0.34. They flare at the ground (+35%, fading over the
  first cells), taper upward to 55%, and lean up to ±0.07 per cell for mild,
  lattice-keyed bends.
- **Limbs.** 6-sided, at 42% of the trunk radius. Where a limb leaves the trunk,
  it starts low inside the trunk and rises out through a collar (radius ×1.35).
  The result reads as a rising bough, not a perpendicular peg.
- **Elbows.** A stepped limb (one horizontal and one vertical link) is a single
  angled piece between the two face rings. Ring vertices are paired after
  rotating one ring's frame onto the other, so the tube doesn't twist.
- **Forks.** Three or more links meet in a rounded knot sized to the thickest piece.
- **Tips and cuts.** A cell with one link continues straight across, tapers, and
  closes with an end-grain disc (the wood's top texture). Removing a
  neighbouring piece turns it into exactly this, so fresh cuts are capped, never
  open.
- **Bounds.** Everything stays inside the owner cell.

Bark wraps once around each ring, using the wood's side texture. Lighting on
round wood is sampled outside the cell, so trunks under a canopy aren't
darkened as if buried. Cells beside a trunk (ground, leaves, placed blocks)
now draw the faces they used to hide against it.

## Interaction

- **Aiming.** A ray selects a living cell only where it meets the wood
  (`TreeCell::contains`), so a ray past a trunk's side misses it.
- **Collision.** The player collides with boxes around each piece. Trunks
  still fill most of their cell. Next to a thin limb, the player can stand in
  the cell's empty part, which the cube used to block.
- **Hover.** The outline frames the wood's bounds rather than the cube.
- **Unchanged.** Digging and placing remove or add exactly one cell with the
  existing yields. Tree support and decay rules are untouched.

## Validation

Tests in `voxel::organic::tests` and `player::tests`:
- trunks taper and flare, every ring stays inside its cell, and the top is capped;
- the ring on a shared face is identical from both cells;
- a limb leaves the trunk rising, with a collar, and its tip is capped;
- removing the limb leaves a straight trunk, and removing the trunk above a
  fork leaves a capped cut;
- placed wood stays a cube and doesn't connect to the tree, even when touching it;
- a ray past the trunk's side misses, while a ray at its middle hits;
- the player can stand beside a thin limb where the cube collided, but still
  can't walk into the limb.

655 tests pass.

## Performance

Measured on an RTX 3060 at 1280×720 with the release build.

| | Cubic | Shaped mode |
| --- | --- | --- |
| Meshing, 25 generated chunks (ms/chunk) | 1.67 | 8.04 (7.08 before trees) |
| Triangles, same chunks | 80,548 | 176,467 (154,331 before trees) |
| GPU frame, tree grove (median ms) | 0.66 | 1.53 |

The GPU figure covers the whole shaped look in that scene: dense grass tufts
over the entire lawn, natural foliage and the trees. It does not isolate the
trees.

## Limits and next steps

- **Trunk width.** Trunks are one cell wide at most (diameter up to about 0.9).
  The very thick trunks in the art reference would need multi-cell trunks from
  the generator, which changes world generation.
- **Collision corners.** Collision uses boxes around pieces, so trunks still
  block about their full cell.
- **Knots.** A knot at a three-way fork is a sphere. Dedicated fork templates
  with blended junction patches would read more naturally.
- **Foliage.** Crowns have not been reworked yet: clusters around limb ends,
  branch-to-canopy links and fewer leaf-face artifacts. Thin, stretched leaf
  faces are visible near some limbs.
- **Ink.** Ink outlines treat trunks like any other geometry; trunk rings
  produce no extra lines because the surface is smooth.
- **Next.** The post-and-beam timber junction, then material connections (brief §2 and §4).
