# Voxel Project — Coherent Terrain, Organic Trees, and Material Connections

## Purpose

Implement the next geometry and material-connection pass in the existing Voxel Project. Coordinate it with the rendering features currently being added:

1. Thin painterly ink outlines at significant depth and surface-normal discontinuities.
2. Stepped sunlight with 3–4 softly separated lighting bands and cooler shadows.
3. Approximately 0.06 world-unit inward bevels on exposed edges of suitable constructed blocks, subject to the project's actual block scale.

Do not duplicate or replace ongoing work on these effects. Inspect the current implementation and integrate with it. Preserve voxel editing, inventory rules, saves, and multiplayer behavior where present. The screenshots and generated tree edit are visual references, not exact geometry specifications.

## 1. Correct the repeated grass-cap problem

The current steep hillside screenshot shows repeated green caps over tall vertical soil faces. Individual cells have slanted tops, but the hillside still resembles rows of small roofs or flowerpots. This is a surface-continuity problem, not merely a texture problem.

Generate coherent local landforms before assigning cosmetic variants. Derive a shared surface profile across a bounded neighborhood of cells, then partition the resulting surface into portions owned by individual occupied cells. Do not independently put a bevelled grass cap on every exposed block.

- Gentle transitions should form continuous grassy surfaces with compatible shared edges.
- Steep soil should form coherent exposed banks with occasional real ledges.
- Rock should form connected faceted masses with irregular breaks across several cells.
- Grass belongs primarily on upward-facing surfaces and real shelves; avoid a green stripe on every voxel row of a cliff.
- Round or chamfer the shoulder between a plateau and its steep face selectively.
- Preserve caves, overhangs, narrow supports, and deliberate player edits.

A one-cell rise over one-cell horizontal distance is still a 45-degree slope. A tall cliff does not need to become a ramp. Do not change soil into collectible stone merely to obtain a rock-like cliff appearance.

Start with shared boundary constraints and shapes inside occupied cell bounds. If this cannot remove the repeated-cap artifact in a particular configuration, use a coherent steep-face fallback. Do not silently add geometry across empty space or erase occupied gameplay cells to make smoothing easier.

Material coverage, surface normals, and texture coordinates must also be continuous where appropriate. Identical geometry can still look segmented if shading or grass coverage resets at every cell.

## 2. Separate material identity from its role

The same resource requires different geometry depending on how it is used. Represent this distinction explicitly rather than inferring it only from nearby material IDs.

| Material | Form or role | Visual behavior |
| --- | --- | --- |
| Oak wood | Living trunk/branch | Irregular cylindrical form, bark, taper, organic forks |
| Oak wood | Cut log | Bark-covered length, explicit axis, exposed end grain |
| Oak wood | Structural timber | Beams, posts, controlled bevels, carpentry joints |
| Oak wood | Plank surface | Grain direction, board spacing, finished edges |
| Stone | Natural formation | Joined irregular facets and fractures |
| Stone | Constructed masonry | Courses, joints, corners and coping |
| Leaves | Growing vegetation | Organic crown clusters; eligible for foliage movement |
| Leaves | Player-placed building material | Static constructed foliage; no wind or idle animation |

Use existing metadata where possible. Conceptually distinguish material/species, form/role, orientation, and relevant structural membership. Tree parts additionally need tree identity and branch connectivity or equivalent information.

Harvesting may convert a tree segment into the existing inventory resource. Placing that resource selects its construction role; it must not automatically reconnect to a living tree or inherit foliage animation. Respect existing recipes and controls; do not invent free processing or a new crafting economy.

If old saves lack role metadata, define a deterministic migration/fallback. Do not classify all connected wood as a growing tree, since buildings may touch trees.

## 3. Trees: irregular cylinders with coherent branching

Replace rectangular trunk prisms and right-angle branch elbows with low-poly irregular cylindrical segments.

### Shape

- Use a modest polygonal cross-section, initially around 8–12 sides where useful, adjustable after profiling.
- Cross-sections may be slightly oval and irregular, with restrained changes along the trunk.
- Taper toward the crown and branch tips.
- Introduce mild bends rather than making every tree strongly curved.
- Flare the trunk near the ground and add restrained root transitions where supported by existing occupied geometry.
- Replace L-shaped branch hooks with angled or gently curved boughs and organic Y-shaped forks.
- Vary silhouettes by species: some trees remain relatively straight, others spread more broadly.

The generated reference demonstrates the desired continuity and branching, but its pronounced bends should be moderated for the first implementation.

### Connections and ownership

Use the tree's branching structure to establish a continuous centerline and compatible cross-sections. Adjacent segments share boundary rings and bark orientation. Do not build trunks from disconnected cylinders, beads, or obvious intersecting tubes.

At forks, generate a dedicated junction patch or compatible fork template. Define ownership for junction triangles and regenerate exposed surfaces when a participating cell is removed. Cap freshly exposed cuts with end grain rather than leaving open meshes.

Logical tree voxels remain selectable and removable. Clip or partition the visual tree surface by owner-cell regions, retaining triangle-to-cell ownership. Keep the initial geometry inside the occupied tree-cell envelope. For newly generated trees, derive occupied cells from the desired branch path if needed; do not bend an old tree visually into empty gameplay space without handling occupancy, collision, and saves explicitly.

Use collision and picking consistent with the visible trunk. A rectangular collider around a narrow round trunk must not create conspicuous invisible barriers. Preserve the existing rules for unsupported branches, leaf decay, or falling trees; this pass does not require a new tree physics simulation.

### Foliage

Preserve the established distinction: only foliage belonging to growing vegetation may move. Leaves placed as building material remain unanimated, even next to a living tree. Preserve that state through edits, remeshing, save/load, undo, and multiplayer replication.

For the first tree comparison, keep the existing foliage mostly unchanged so that trunk geometry can be judged independently. Then improve crown clusters and branch-to-canopy relationships separately.

## 4. Material connection families

Build reusable connection rules by material family and role, with species-specific visual variants. Avoid a separate bespoke rule for every pair of wood species.

| Contact | Proposed treatment |
| --- | --- |
| Timber–timber, aligned | Continuous member or restrained longitudinal splice |
| Beam–post | Seated joint with a visible notch, tenon suggestion, or wooden peg |
| Crossing timbers | Controlled lap-joint treatment |
| Planks–planks | Coherent board direction and spacing |
| Timber–masonry | Beam pocket or supported contact, without creating free support blocks |
| Post–soil | Clean embedded contact; no automatic stone foundation |
| Masonry–masonry | Compatible courses, staggered joints, corner treatment |
| Natural rock–rock | Joined formation with compatible facets and sparse fractures |
| Soil–rock | Soil follows and partially covers exposed rock contours |
| Sand–rock | Sand fits lower contact regions without filling empty gameplay cells |
| Trunk–branch | Continuous organic fork |
| Foliage–foliage | Joined crown masses with irregular outer silhouettes |
| Roof–wall | Appropriate eave/end treatment using existing building roles |
| Unsupported material pair | Clean non-interpenetrating contact with no invented ornament |

Infer structural junctions from local arrangement and explicit orientations. Do not add a peg at every voxel boundary. Long continuous beams should read as long members, not chains of decorated cubes.

Small joint details are cosmetic and need not have separate colliders or yields. Large braces, foundations, or additional structural members must require actual occupied cells and existing resource costs. Do not disguise a cosmetic connection as newly functional structural support.

Use an explicit priority order: gameplay occupancy and role → valid contact geometry → connection treatment → optional cosmetic variant. A shared junction has one canonical owner/key to prevent duplicate pegs, overlapping patches, or asymmetric results across chunks.

## 5. Integrate with the new illustrated rendering

### Ink outlines

Outline meaningful silhouettes, deep creases, cuts, and structural joints. Avoid lines on every terrain-cell border, tree segment ring, mesh triangle, leaf card, or small bevel facet. Otherwise the outline pass will restore the grid and make foliage excessively noisy.

Where practical, use smooth semantic normals or feature masks for outline detection, independently of normals used for lighting. Test foliage coverage carefully: tiny alpha-cutout holes should not become dense black stippling. Tune screen-space width and distance behavior at normal gameplay resolution.

Sharing an existing depth-reading pass may reduce setup work, but measure the cost of extra samples and any normal input; do not assume the feature is free.

### Toon lighting

Apply the lighting bands to coherent surface normals. Continuous soil slopes and tree trunks should not acquire a separate band boundary at every cell seam. Preserve deliberate rock facets. Use softened transitions and a stable ambient contribution so shadowed surfaces remain readable.

### Bevels

Apply the proposed bevel primarily to appropriate built blocks, beams, plank edges, and masonry corners. Treat 0.06 as an initial world-space setting, not a universal value: clamp it relative to cell size and thin features.

Bevel the exposed boundary of a coherent constructed shape. Suppress unnecessary internal bevels where aligned blocks form a continuous member, unless a deliberate board or masonry joint belongs there. Do not apply generic cube-edge bevels on top of already shaped natural terrain or cylindrical tree segments.

Coordinate bevel normals, toon bands, and outlines so one small edge does not produce several competing dark stripes.

## 6. Variants and local WFC

Use deterministic variant selection for freely interchangeable pieces. Add local constraint propagation/WFC only where compatible joint or fracture patterns require it.

Freeze unaffected surroundings, prefer existing variants, bound retries, and provide safe fallbacks. WFC selects visual forms; it must not alter resources, tree identity, material role, or voxel occupancy. Local edits should not cause a whole hillside or forest to reshuffle.

## 7. Work sequence and validation

1. Inspect the current mesher, tree data, material roles, editing path, multiplayer serialization, and ongoing rendering work.
2. Fix one representative steep bank, removing repeated grass caps while preserving excavation.
3. Implement one tree with a straight trunk, mild taper, and a Y-shaped fork; test removing trunk and junction cells.
4. Implement a timber post-and-beam corner and compare it with living wood of the same species.
5. Extend the connection table across supported material families using clean defaults for other pairs.
6. Tune outlines, lighting bands, and bevels on all three examples together.
7. Expand species and variant diversity after continuity and editing are reliable.

Acceptance checks:

- The bank reads as one landform, not rows of green-topped blocks, with outlines both on and off.
- Tree segments join without gaps, exposed cylinder caps, or square elbows; removed segments expose valid cuts.
- Digging/removal still affects exactly the intended cell and existing resource yield.
- Living oak and constructed oak remain distinct when touching.
- Built leaves remain static through save/load and network synchronization.
- Timber junction detail appears once, at a meaningful joint, and does not create free structural elements.
- Chunk-border edits, reloads, and deterministic variants produce matching geometry on clients.
- Collision and picking follow visible geometry.
- Outline/lighting combinations do not expose hidden voxel seams or turn foliage into black noise.

Capture matched before/after views of the bank, trees, and timber junction. Profile geometry generation, edit latency, draw/instance counts, foliage overdraw, and frame time on the same scene and settings. The supplied screenshots show about 22 FPS, but that alone does not identify the bottleneck. Investigate before increasing mesh or foliage complexity broadly.

Deliver the working changes, a small reproducible test scene, visual comparisons, and a short report of measured performance, completed validation, and remaining limitations.
