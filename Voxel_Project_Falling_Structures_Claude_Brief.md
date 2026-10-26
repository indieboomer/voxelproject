# Voxel Project — Falling Structures and Resource Conversion

## Scheduling and intent

Implement this AFTER the organic tree-shaping pass is working: irregular cylindrical trunks, tapered branches, coherent forks, and reliable per-cell picking/removal. Do not interrupt that pass or duplicate it.

The gameplay reference is the behavior already prototyped in Hot Vampires: unsupported elements fall using physics, then settle as correctly oriented voxels. Reuse that implementation only if its source is actually available and compatible; otherwise implement the behavior in Voxel Project's existing architecture.

Begin with trees. Extend the same framework to constructed structures after the tree case works. Terrain cave-ins are outside the first milestone: applying support checks to an entire terrain mass could cause unintended widespread collapse.

## Required interaction

Cut through a tree trunk. The portion that loses structural support detaches and falls as a coherent physical object, preserving its shaped trunk, branches, and crown. Once it comes to rest, it is converted into static resource voxels aligned to the world's grid, without deleting, duplicating, or overwriting resources.

Do not make the player wait for the complete original trunk to disappear if the remaining structure is already disconnected. Conversely, do not detach a tree while a valid structural connection remains.

## 1. State and ownership

Use an explicit lifecycle, adapted to existing project code:

- Attached: cells belong to the static world/tree structure.
- Detached/dynamic: one physics object owns the detached logical cells and their material metadata.
- Settling: a candidate grid placement is being validated.
- Settled: cells belong to the static world again, with converted roles.
- Fallback resources: material is represented by collectible items if safe grid placement cannot be found.

At every point, each source cell/resource exists in exactly one state and has one authoritative owner. Remove static render/collision ownership when transferring to the dynamic object. Preserve recoverability if settling fails; never partially consume the dynamic object's contents without accounting for the remainder.

## 2. Support rules

Use explicit tree connectivity rather than a flood fill over all neighboring non-air cells.

- Roots/base anchor the tree according to existing support rules.
- Trunk and branch connections transmit structural support.
- Leaves are payload; they do not support trunks or branches.
- Touching crowns do not connect separate trees structurally.
- Associate foliage with its supporting branch/tree so overlapping crowns do not produce accidental attachment.
- Floating leaf remnants follow a documented deterministic rule, such as detaching with their associated branch or becoming collectible resources.

After an edit, find affected disconnected components. Each separate connected component may become its own dynamic body. Keep the first version's scope bounded and avoid scanning the entire world every frame.

For the later construction extension, use role-appropriate structural connections. Do not let arbitrary decorative contact count as a load-bearing connection. Full stress or engineering simulation is not required.

## 3. Physical fall

Keep a logical cell list and cell-local transforms in the dynamic object, alongside the visual mesh. Preserve the organic appearance while falling; do not instantly turn the crown into cubes in mid-air.

Use a suitable compound or simplified collision representation consistent enough with the visible mass. Do not create one independently simulated rigid body per leaf voxel. Derive mass from existing material conventions or documented tunable values.

Detached leaves immediately stop wind/idle animation. Their only movement comes from the falling object. Attached growing foliage keeps its existing permitted animation.

Update exposed cuts on the remaining stump and detached branches. Preserve the existing rules for growth; detached tree parts must not continue growing as living trees.

## 4. Safe grid settlement

Trigger settlement only after velocity and angular velocity stay below tunable thresholds for a stable interval, with valid support. Do not settle a momentarily slow object in mid-air. Use explicit water handling; floating or submerged bodies must not become static suspended voxels accidentally.

Choose among grid-preserving orientations (the 24 proper rotations of the cubic grid where compatible with the material). Use a consistent lattice pivot and integer translation. Prefer candidates close to the body's final position and orientation, then perform a bounded search of alternatives.

Transform the original logical cell coordinates exactly. Do not independently round each cell's continuous world position: doing so can merge cells, create gaps, or lose material. Verify the mapping is one-to-one and that all destinations are valid and empty or explicitly replaceable under existing rules.

Validate support using the converted resource roles. A settled log must not be held in mid-air by a static resource-leaf cell. If a full rigid arrangement cannot be supported, use the resource fallback or a clearly accounted component split rather than freezing an invalid structure.

Reserve destinations and commit placement atomically. If another edit occupies a destination before commitment, retry or retain the dynamic object. No overwriting terrain, buildings, other resources, or entities. Check the player's collision volume before final placement.

If bounded attempts fail, allow further physical movement or convert the contents into normal collectible resources using existing item systems. Preserve exact material totals. A small visual settle transition is acceptable only if it does not create mismatched collision or ownership.

## 5. Material-role conversion

Convert roles when settlement is committed; mark detached vegetation nonliving immediately upon detachment.

| Source role | Settled role |
| --- | --- |
| Living oak trunk or branch | Oak wood resource, preferably cut-log appearance |
| Other living wood species | Corresponding species wood resource |
| Growing tree leaves | Static leaf resource voxels |
| Constructed timber, in the later phase | Constructed timber with recomputed connections |

Retain species and existing inventory/yield rules. One logical source voxel contributes exactly its existing one-cell resource quantity, irrespective of rendered cylindrical volume. Decorative bark, twigs, and individual rendered leaves must not generate extra resources.

Resource wood must not automatically become a living tree or a finished carpentry joint. Later player placement in a construction role enables the relevant beam/plank connection rules. Resource leaves stay unanimated even when touching growing foliage.

## 6. Persistence, multiplayer, and performance

Use the game's existing authoritative simulation model. In hosted multiplayer, the host/server determines detachment, resource ownership, physics outcome, and settlement. Clients must not independently choose different snap positions. Transfer stable IDs and sufficient metadata for late join, save/load, and deterministic settled geometry.

Persist dynamic contents and lifecycle state if the game supports saving during falls. Never reload both the original static tree and its detached resource body. Preserve existing undo behavior where supported and explicitly handle references to moving objects.

Set measured budgets for simultaneous dynamic components and collider complexity. Queue excess simulation work without losing contents. Profile before increasing scope: the earlier screenshots showed roughly 22 FPS, which identifies a need to measure but not a specific bottleneck.

## 7. Implementation sequence

1. Inspect the completed tree representation and existing physics/item/network systems.
2. Implement support detection and debug visualization on one tree.
3. Transfer one detached tree component into a coherent dynamic object.
4. Implement exact lattice placement and role conversion.
5. Add collision-safe fallback, persistence, and authoritative multiplayer behavior.
6. Validate several tree shapes and chained edits before extending to constructed blocks.

## Acceptance scenarios

- Cutting a trunk releases the unsupported crown while leaving the stump intact.
- Cutting a branch releases only the affected component and associated foliage.
- Two trees with touching crowns do not hold each other up.
- A fallen tree settles at a valid lattice orientation without overlapping existing world cells or the player.
- Insufficient landing space produces a safe fallback with exactly conserved material quantities.
- Detached foliage stops wind animation; settled or rebuilt resource leaves remain static.
- Resource logs remain resources when touching a live tree.
- Save/load and a second multiplayer client observe one consistent set of objects/resources.
- Cutting or removing another support during a fall does not duplicate or orphan components.
- An attempted leaf-supported settlement is rejected or safely split/fallback-converted.

Deliver a working tree-first milestone, a reproducible test scene, a short clip showing cutting → falling → grid settlement → resource reuse if recording is available, and a concise report of validation, performance, and limitations. Do not claim access to the Hot Vampires implementation unless it was actually inspected.
