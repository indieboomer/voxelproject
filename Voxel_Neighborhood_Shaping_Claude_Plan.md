# Neighborhood-driven voxel shaping — implementation brief for Claude

## Goal and scope

Implement an experimental visual treatment for the existing voxel prototype. Keep discrete voxel construction, addition, excavation, materials, and resource accounting. Replace exposed cubes with deterministic, material-specific shapes derived from neighboring occupied cells.

The world should look like a stylized miniature landscape rather than a collection of Minecraft-like cubes. Players must still learn that one selectable terrain fragment represents one removable voxel, even when its visible shape is a ramp, rounded shoulder, or irregular rock.

This is a proposed design, not an audit of the current repository. Inspect the existing renderer, terrain representation, picking, collision, chunk updates, and save format before choosing integration points. Adapt naming and implementation to the project. Do not rewrite the engine or change voxel resolution for this experiment.

## 1. Core design contract

- One occupied logical cell is one gameplay unit, regardless of its rendered volume.
- Digging a selected cell removes exactly that cell and awards the existing one-cell yield. Shape does not alter yield.
- Adding material occupies exactly one empty logical cell and consumes the existing one-cell cost.
- Neighborhood changes may reshape nearby cells, but never silently add, remove, or change their material.
- Every visible terrain triangle has exactly one owning occupied cell.
- Start with geometry confined to its owner's cell bounds. No grass ramps extending into empty cells, unowned filler geometry, or decorative rock protrusions into another target's space.
- Each occupied cell retains a connected, nonzero solid shape. Exposed isolated cells must never disappear through smoothing.
- Shape selection is deterministic, local, and independent of edit history or chunk loading order.
- Unsupported configurations use a safe block-like fallback. A readable cliff is preferable to a broken smooth surface.

This deliberately treats voxels as discrete material portions, not conserved cubic meters. A wedge still costs and yields one unit. Validate whether players accept this convention; geometry alone cannot guarantee that they will.

## 2. Resolve the slope ambiguity explicitly

A grass cell directly below another grass cell is an interior/support relationship; it should not automatically become a slope. Require an exposed top and a horizontal neighboring column whose exposed top is one cell lower.

For an initial two-dimensional cross-section, let the lower column's top be at height H and the adjacent higher column's top at H+1. Shape the higher column's top cell into a wedge: its surface runs from H at the low-side boundary to H+1 at the high-side boundary. The lower column remains at H. The wedge belongs to the higher cell and excavating it yields one voxel.

A rise of one voxel over one voxel of horizontal distance is 45 degrees. It removes staircase corners but is not a genuinely gentle hillside. Do not promise gentle slopes from this case. First prove the one-cell ramp; later test wider transitions using several explicitly owned surface cells. Avoid broad smoothing that makes cells nearly invisible or generates surfaces in empty cells.

## 3. Initial rule vocabulary

Evaluate occupancy and material in a bounded neighborhood, initially the 3×3×3 cells around the owner. Use additional samples only when a documented rule needs them, and expand invalidation accordingly. Derive exposed faces and local height relationships from occupancy, not from previously selected neighboring shapes.

| Situation | Proposed visible result | Guardrails |
| --- | --- | --- |
| Flat grass/soil plateau | Broad flat top with restrained surface variation | Preserve clear usable ground; do not bevel every internal grid boundary. |
| Exposed grass top with one lower cardinal neighbor and supporting higher-side terrain | One-cell directional ramp | Start with pure two-level test arrangements; retain compatible boundary profiles. |
| Two adjacent low sides | Convex corner ramp | Use an explicit corner template; never overlap two independent wedges. |
| Concave terrain corner | Concave transition patch | Add after straight and convex ramps; ambiguous cases fall back. |
| Opposite low sides, isolated ridge, or conflicting height cues | Stable ridge/cap or block fallback | Do not select an arbitrary ramp direction. |
| Height difference greater than one cell | Cliff with a restrained top shoulder | No long diagonal bridge across missing cells. |
| Exposed soil below grass | Earth sidewall following the selected top profile | Grass is a surface treatment, not an extra collectible cell. |
| Isolated grass/soil cell | Substantial softened mound/block within the cell | Keep a readable one-cell footprint and substantial volume. |
| Rock next to rock at equal or stepped elevations | Joined faceted stone mass with irregular exposed surfaces | Shared boundaries must agree; no gaps between occupied solids. |
| Isolated rock | Chunky clipped rock shape | Keep a recognizable single removable chunk; avoid tiny shards. |
| Rock touching soil | Compatible contact with distinct material treatment | Use a fixed material-pair boundary policy, not independent deformation. |
| Cave ceiling, overhang, narrow support | Conservative faceted/block treatment | Do not apply top-surface grass slope rules to ceilings or erase supports. |
| Terrain touching existing architecture | Stable contact; preserve existing building shapes | Roofs, columns, and authored architecture remain outside terrain smoothing. |

Rule priority: protected architecture/support cases → unsupported/ambiguous fallback → material-specific top transitions → corners → cosmetic shaping. Resolve ties explicitly and consistently under rotations.

For the first playable milestone, implement flat soil/grass, four straight ramps, basic convex corners, cliffs, isolated cells, and faceted rock. Defer complex concave shapes until this set is correct.

## 4. Geometry architecture

Prefer a small explicit shape family with procedural vertex adjustment over unrestricted smoothing for this experiment. The useful property is direct control over ownership, recognizable shapes, and excavation results.

Suggested derived record:

    CellSurface {
        ownerCell,
        material,
        shapeKind,
        orientation,
        faceBoundaryProfiles,
        deterministicVariant
    }

This is conceptual pseudocode; fit it to the existing codebase.

Separate three operations:

1. Classify the owner using a snapshot of logical occupancy/material data.
2. Resolve shared boundary profiles using a canonical face key and a symmetric function of the incident cells and required raw samples.
3. Generate each cell's surface triangles, normals, collision data, and ownership mapping from those resolved constraints.

Do not make shape A depend recursively on the generated shape of B. Avoid repeated relaxation until convergence.

### Shared boundaries are mandatory

Adjacent shapes cannot simply cull a whole face because the neighboring cell is occupied. A wedge next to a full cell may expose part of the full cell's side. Compute the occupied cross-section of each incident shape at their shared face; emit only portions exposed to air, with correct ownership. Use a small tested interface table for the initial shape family if that is simpler than generic polygon clipping.

All shared surface edges must use identical endpoints and compatible subdivision. A canonical world-space boundary key avoids chunk-dependent results. Unsupported shape pairs must downgrade deterministically to compatible fallback shapes, using a bounded explicit policy rather than an unbounded propagation process.

For grass, use restrained normal smoothing on continuous top surfaces and preserve intentional cliff breaks. For rock, retain broad facets. Keep random detail away from shared boundaries unless it is generated from their shared key. Start with zero boundary displacement and modest interior facet variation.

Do not merge away cell ownership when batching triangles. A merged draw mesh may still retain per-triangle cell IDs or an equivalent CPU-side mapping.

## 5. Communicating one removable unit without HUD text

A perfectly smooth uniform hillside cannot reliably reveal its hidden logical subdivisions. Treat this as a design constraint to test, not a problem solved automatically by a mesher.

Use two complementary layers:

- At rest: subtle cell-scale variation in grass growth direction, soil texture, or rock fracture structure. Maintain a coherent large surface; avoid a checkerboard, dark seams around every cell, or one random color per voxel. Rock facets should suggest a chunky removable portion without turning every cell into a separate boulder.
- On hover/aim: tint or softly outline all visible surface fragments owned by the targeted cell. Follow the actual wedge/rock shape rather than drawing a cube. No numeric HUD label. This in-world targeting cue is part of the experiment.

When digging, briefly emphasize the selected portion before removing it; emit a compact material effect at that cell. Neighbor reshaping must not look like additional resources being removed. Initially update geometry immediately; add cosmetic interpolation only after selection and collision remain coherent.

An optional small world-space placement preview may show the resulting local shapes before confirmation, including neighboring reshaping. It must still make the single newly occupied cell identifiable.

If player testing fails without hover, strengthen material cues first. Do not restore a permanent black grid merely to make the accounting obvious.

## 6. Picking, placement, and collision

Raycast the displayed surface and recover the triangle's owner cell. Do not use the first full grid AABB hit as the final answer: rays can pass through the empty part of a wedge. Grid traversal can provide candidates, followed by actual shape intersection.

A sloped triangle normal is not a reliable logical placement direction. Associate surface patches with semantic placement directions: ramp top → above owner; exposed cardinal side → adjacent cell in that direction. At corners, use a documented deterministic rule and a placement ghost. Never place into an already occupied owner because of rounding a hit point.

Use collision that matches the generated terrain shape, or a demonstrably close simplified version. Full-cube colliders under wedge visuals are unacceptable. Keep collision, picking, and rendering on the same terrain revision. Check existing character slope limits: a 45-degree ramp is not automatically walkable in every controller.

Recalculate existing navigation/walkability locally if present. Do not introduce an unrelated navigation rewrite.

## 7. Local updates and persistence

On an edit, mark every cell whose classification or boundary resolver can read the edited cell. Derive this dependency radius from the complete sampling path, including shared-face resolution, rather than assuming six neighbors are enough. Remesh affected chunks and required border regions only.

Read a consistent occupancy snapshot. If meshing is asynchronous, reject stale jobs and publish matching render/picking/collision revisions together. Include chunk-border edits, chunk load/unload, undo, and bulk edits in invalidation handling.

Persist the existing logical world. Derive geometry after loading. Use world coordinates and a stable seed for cosmetic variants. Include a shaping-version/config identifier if needed for reproducible comparisons; do not serialize generated meshes as authoritative terrain.

## 8. Implementation sequence

### A. Inspect and establish a baseline

Identify current voxel ownership, meshing, selection, material storage, collision, resource accounting, and chunk invalidation. Record reproducible camera positions and baseline performance. Add an experimental renderer toggle so the same world can be compared in cubic and shaped modes.

### B. Prove one ramp end to end

Implement a straight grass/soil ramp in four orientations, shared-face exposure, exact owner picking, one-cell digging/placement, and matching collision. Make this playable before expanding the shape catalog.

### C. Add corners and safe fallbacks

Implement convex corners, cliff shoulders, isolated caps, and protected narrow supports. Add concave transitions only when interface handling is stable. Verify all supported shape pairs and rotations.

### D. Add rock and material contacts

Introduce deterministic broad facets, stepped rock transitions, and explicit rock/soil contacts. Begin with geometry variation; tune color and lighting after silhouettes work.

### E. Add readability and polish

Implement actual-shape hover highlighting, restrained material cues, coherent grass tops, and excavation feedback. Compare matched screenshots with highlighting disabled as well as enabled. Tune at normal gameplay camera distance.

### F. Validate edits and performance

Exercise chunk borders, reloads, repeated edits, and interaction from multiple directions. Report measured meshing cost, triangle counts, and edit latency versus the baseline on the same scenes. Optimize measured bottlenecks without sacrificing ownership or seam correctness.

## 9. Required test scenes and acceptance criteria

Create a compact terrain test garden: flat plateau; straight ramp in all four directions; convex and concave corners; ridge; one-cell pit; isolated cell; tall cliff; cave/overhang; rock mound; mixed rock/soil slope; architectural contact; and ramps crossing chunk boundaries.

Automated/invariant checks where practical:

- Every generated triangle has an occupied owner; shaping alone leaves inventory and occupancy unchanged.
- Digging a wedge, corner, or rock removes exactly one cell and yields exactly one unit.
- Dig/add reversibility restores the same logical state and deterministic geometry.
- Shared interfaces have no unintended holes, duplicated coplanar faces, or incompatible edges.
- Rays through a wedge's empty volume do not select the wedge's AABB.
- Rotated neighborhoods produce correspondingly rotated shapes.
- Reloading or changing chunk traversal order produces identical shapes.
- An edit on a chunk border updates all dependent render, collision, and picking data.

Manual checks:

- Walk up/down supported ramps and across corners; no invisible steps, hovering, or falling through seams.
- Hover identifies the exact portion that disappears; no neighboring cell unexpectedly vanishes.
- A player unfamiliar with the rules can predict the removed portion and one-unit yield after a short demonstration, without numeric HUD hints. Test several shape types; record misunderstandings rather than claiming success from screenshots alone.
- In matched screenshots, terrain reads as coherent slopes and rock formations, while individual editing units remain discoverable during interaction.

## 10. Deliverables and constraints

Deliver the working experimental mode, a reproducible test garden, concise rule documentation, matched before/after screenshots, and a short implementation report covering actual changes, validation, performance, unsupported cases, and next improvements.

Preserve existing saves and gameplay wherever possible. Keep the voxel size unchanged. Do not add resource fractions, hidden extra cells, broad terrain relaxation, or a new global terrain representation. Do not reshape buildings as soil. Prioritize a convincing small rule set over exhaustive combinations.

The first decision gate is a playable straight slope: does it look better, feel correct underfoot, and clearly behave as one removable cell? If that fails, adjust the shaping/readability contract before adding more shapes.
