# Voxel Project — Natural Coast Visual Implementation Brief

## Task

Modify the existing Voxel Project so that its playable environment approaches the attached terrain-only coastal reference: grassy terraces, irregular connected rock formations, branching trees with asymmetrical foliage clusters, turquoise shallows, and animated shoreline foam.

Implement the changes in the existing project. Inspect its code and rendering capabilities first, then work incrementally. Preserve voxel editing and existing gameplay. This brief describes the desired result and design constraints; it does not assume a particular engine, repository layout, or existing implementation.

Use the latest reference image with NO buildings as the primary visual target. Attach that image alongside this brief when starting the implementation session. If it is unavailable in the project/session, report that limitation; do not claim to have compared against it.

Deliver a working, editable natural environment, not a static diorama or a screenshot-only scene. No new buildings, fences, docks, boats, or settlement features are required. Leave existing building systems intact where present, but hide them in the reference test scene.

## 1. Read the reference as an art direction

The important characteristics are:

- Broad grassy terraces separated by irregular rock faces and sloped grass shoulders.
- Coherent landforms with variation at three scales: island silhouette, terrain-sized chunks, and small surface details.
- Warm gray/beige stone, ochre soil, yellow-green grass, and cool turquoise water.
- Trees with visible branching trunks and irregular clusters of foliage; open gaps break up their silhouettes.
- Clear shallow water revealing the seabed, with broken white foam outlining the coast.
- Warm sunlight, soft-looking shadows, and readable contact shading.
- A stylized miniature appearance with usable open ground between vegetation groups.

Do not reproduce every incidental detail literally. In particular, the reference includes stair-like stone arrangements and a cut in the soil; they do not require authored stairs or a permanently excavated notch. Avoid exaggerated depth of field that hides playable terrain.

The visual quality comes from geometry, materials, vegetation, lighting, and water together. WFC alone will not reproduce the image.

## 2. Preserve the editing contract

Keep the existing voxel size and logical world as the source of truth.

- One occupied terrain cell remains one gameplay unit. Digging removes exactly one selected cell and yields its existing one-cell resource amount, where resources are implemented.
- Neighborhood-dependent shaping changes visible form, not occupancy or material ownership.
- Every terrain surface triangle must map to one occupied owner cell.
- Rendering, picking, and collision must agree. A ray through the empty portion of a wedge must not select its full cube bounds.
- Changing a cell updates dependent neighboring geometry and decorations locally, including across chunk boundaries.
- Preserve saves, undo, and existing controls where present. Do not add an economy or survival systems for this task.

Use actual-shape hover highlighting when editing, without requiring numeric HUD explanations. Do not draw a permanent grid or outline every terrain cell. A single cell need not be recognizable from every distant screenshot, but targeting and removal must be predictable during play.

## 3. Terrain: eliminate the cubic staircase silhouette

Implement or extend a compact shape vocabulary: flat caps, directional ramps, convex corners, concave transitions, cliff shoulders, isolated caps, and conservative fallbacks for overhangs or narrow supports.

Derive shape from logical occupancy and material in a documented bounded neighborhood. A cell directly below another cell is not enough to trigger a ramp. A ramp requires an exposed surface and an appropriate horizontal elevation transition.

Start with one-cell rises and directional wedges. A one-cell rise over one-cell horizontal distance is 45 degrees; use wider terrain composition and occasional intermediate terraces to achieve the reference's relaxed landscape. Do not describe every wedge as a gentle slope.

Keep initial terrain shapes inside their owning cell bounds. Adjacent cells must share identical boundary geometry where surfaces meet. Cull internal faces by actual occupied shape coverage, not merely by whether the neighboring cell exists. Mixed cube/wedge configurations may expose partial faces.

Avoid long rows of identical triangular ramps. Alternate coherent landform types: grassy shoulder, rocky break, short ramp, flat ledge, and concave hollow. Let underlying terrain layout establish those choices; do not randomly replace useful flat ground with clutter.

Rock should form connected masses. Use broad angular facets, clipped corners, and restrained variation inside compatible boundaries. Do not render every rock voxel as an independent rounded boulder. Grass transitions should be softer than rock, with intentional normal breaks at cliffs.

## 4. Materials and surface detail

Replace visibly tiled pixel-style block textures where present. Establish large coherent material areas first, then add restrained detail.

- Grass: related green tones with low-frequency variation, modest fine detail, and darker earth beneath exposed edges.
- Soil: warm earthy faces with subtle stones and layering, avoiding identical patterns on every cell.
- Rock: related warm gray tones, broad facet shading, and sparse cracks that reinforce the shape.
- Sand: small shoreline pockets and exposed shallow seabed, where the world material system supports them.

Use a consistent world-space scale for material variation. Do not assign independent strong colors to every cell. Use the renderer's existing suitable mapping technique; add new shader features only when they solve a visible problem.

Add small tufts, wildflowers, shrubs, and occasional pebbles after the base landforms read well. Distribute them in patches with open spaces. Bias placement by material, surface orientation, elevation relative to water, and stable world-space seeds.

Decorations must be attached to valid surfaces and regenerated or removed after edits. No floating grass or shrubs over freshly dug holes. Avoid placing terrestrial foliage below the waterline. Decorative visual detail need not have individual colliders.

## 5. Trees and shrubs: coherent growth before variation

Replace cubic crowns with a procedural hierarchy:

1. A trunk and branching skeleton establishes the tree's overall structure.
2. Primary branches define several asymmetric crown lobes.
3. Compatible irregular foliage clusters fill those lobes, with denser interiors and tapered ends.
4. Intentional gaps expose branches and sky.

Generate several recognizable silhouette families, such as spreading, upright, and young/small trees. Vary proportions within each family instead of scaling one tree uniformly. Use readable foliage masses and selective leaf detail; avoid both cube leaves and a dense uniform cloud of tiny particles.

If trees are already voxel-editable, retain that logical representation and ownership for editable parts. Shape local foliage clusters around it. If trees are currently objects, keep that representation for this pass rather than inventing tree-cell gameplay. In either case, do not allow terrain edits to leave newly placed vegetation floating; preserve or explicitly handle the existing support rules.

Keep foliage color variation coherent within a crown.

### Leaf animation: growing vegetation versus building material

Leaf movement is allowed only for leaves belonging to growing vegetation. Leaves placed by the player as building material must remain completely unanimated: no wind sway, flutter, vertex displacement, or idle motion, even when placed beside or against a living tree.

Track this distinction explicitly per cell or instance using the existing vegetation/building state or equivalent metadata. Do not enable animation solely because the material is classified as leaves. Preserve the distinction through remeshing, chunk reloads, save/load, and undo/redo. Reusing the same leaf material or mesh must not make player-built foliage animate.

Add subtle motion to eligible growing foliage only after its static silhouettes match the intended direction. Player-placed leaf shapes may still update discretely in response to neighborhood edits; this must not introduce ongoing animation.

## 6. Compatible variants and optional local WFC

First create good shape families. Variants should declare compatible boundary profiles or sockets, material family, orientation, and any semantic requirements.

Where several variants are freely interchangeable, use deterministic weighted selection. Use local WFC/constraint propagation only where relationships actually require coordination, such as connected rock fracture patterns or foliage cluster connections.

WFC must not decide gameplay occupancy, excavate terrain, or change resource quantities. It selects compatible visual variants of an established structure.

For local solving:

- Freeze the unaffected outer boundary.
- Prefer existing variants to avoid visual reshuffling after every edit.
- Use stable seeds, bounded solving budgets, and deterministic fallback variants.
- Handle contradictions by falling back locally, not by cascading changes across the island.
- Keep rendering and interaction on a coherent revision during asynchronous work.

Do not block the first visual milestone on a generic WFC framework. Branch structure, deliberate crown silhouettes, and a small good module library matter more than algorithm complexity.

## 7. Water and shoreline foam

Target calm coastal water with small moving surface ripples, readable shallow depth, and intermittent foam along the actual coast.

Maintain a clear distinction between a water-rendering effect and water occupancy/flooding. A sea plane or foam shader does not implement water flowing into a newly excavated basin. Reuse existing water connectivity if present. Otherwise establish and document a minimal ocean-connected water mask at the chosen sea level so isolated pits do not fill merely because they are low. No full fluid simulation is required.

Implement in stages:

1. Depth-dependent shallow/deep color and enough transparency to see shallow rocks and sand.
2. Small animated surface normals and restrained highlights; avoid mirror-like plastic water.
3. A shoreline proximity field or equivalent world-space shoreline representation derived from current terrain-water contact.
4. Broken foam bands moving toward the shore, appearing and fading at different phases along it.

Choose the shoreline implementation after inspecting available renderer data. A screen-space depth-intersection effect can be a quick starting point, but test its camera dependence and do not mistake shallow submerged ground for the coastline. Prefer a world-space contact representation if screen-space artifacts are visible.

Foam should be narrow and irregular at steep rocks, broader and softer on low slopes. Break it into patches; avoid a constant white outline or synchronized pulsing around the entire island. Keep foam on valid water and surface-contact regions, not across dry land, through cliffs, or under bridges/overhangs merely due to projected depth.

When the player edits the coast, update water contact and affected foam regions. Connectivity changes may require broader water-mask updates than local geometry edits; handle them explicitly. Limit the foam field to the necessary coastal band and rebuild affected regions rather than recomputing all shoreline data every frame.

## 8. Lighting and camera

Create a reproducible reference camera with an elevated three-quarter view and a foreground shoreline, middle-ground terrace, and background trees. Use the existing gameplay camera too; the result must work while moving and editing.

Aim for warm directional sunlight and cooler ambient fill. Add or tune shadows and contact shading using available renderer capabilities. Preserve detail in shadowed rock faces. Tune exposure and color before adding post-processing. Keep bloom restrained and gameplay depth of field off or very mild.

Judge the scene at normal gameplay distance as well as in close-up. Small vegetation details cannot compensate for weak terrain and tree silhouettes.

## 9. Implementation order and review gates

### A. Audit and baseline

Identify terrain storage, mesher, materials, picking, collision, vegetation, water, and renderer features. Capture the current scene from fixed camera positions. Establish a representative performance baseline and preserve a way to compare old/new terrain rendering if practical.

### B. One editable coastal patch

Build a reproducible scene with a grassy plateau, a slope, convex and concave corners, rock cliff, and shallow bay. Implement terrain shaping and reliable editing first. Verify matching collision and crack-free boundaries.

### C. Base visual treatment

Tune grass, earth, rock, water color, and lighting. Capture the same scene. Require coherent terrain masses without decorative cover before moving on.

### D. Trees and ground vegetation

Add a few strong tree silhouettes, grouped shrubs, and restrained surface plants. Test terrain edits underneath and beside them.

### E. Shoreline animation

Add water ripples, depth treatment, and animated foam that updates after coastal edits. Record a short clip, since a screenshot cannot demonstrate motion or updating.

### F. Variation and final comparison

Add compatible shape variants and local solving only where needed. Review matched screenshots against the reference, identify the three largest visual gaps, and address those before expanding scope. Profile the resulting scene and reduce density or geometry where it does not contribute at gameplay distance.

## 10. Acceptance tests

- Dig a grass wedge: exactly one logical cell disappears and nearby surfaces reconnect correctly.
- Add material back: the expected deterministic form returns without cracks or inventory anomalies.
- Edit a rock face and a chunk boundary: no stale geometry, invisible colliders, or targeting mismatch.
- Remove ground beneath decorations: no floating grass or detached shrubs remain unintentionally.
- Open a channel to the sea: the water mask/contact and foam reflect the new connected coastline; an isolated inland pit follows the documented water rule.
- Move the camera: foam remains anchored to the world and does not appear along unrelated screen-space overlaps.
- Walk across slopes: movement matches the displayed surface.
- Reload the world: geometry and variant placement remain stable.
- Compare growing foliage with player-placed leaf blocks: only the growing foliage may move. Built leaves stay static, including beside trees and after remeshing, save/load, and undo/redo.
- Compare at normal gameplay distance: no dominant cubic terrain stairs, cube crowns, repetitive ramp stripes, or noisy checkerboard coloring.

Record CPU/GPU frame times where available, terrain-edit latency, remesh cost, triangle/instance counts, and water/vegetation cost on the same scene before and after. Report the test hardware and settings. Do not invent a performance target or claim checks that were not run.

## Deliverables

Provide the working implementation, a reproducible natural-coast test scene, matched baseline/final screenshots, a short edit-and-shoreline video if capture is available, and a concise report of completed changes, validation, measured performance, and remaining visual gaps.

Proceed with reasonable reversible implementation decisions. Keep changes focused on this environment pass. If a major renderer limitation prevents an effect, implement the closest viable version and clearly state the limitation rather than silently replacing the project architecture.
