//! Experimental neighborhood-driven terrain shaping (docs/TERRAIN_SHAPING.md).
//!
//! Shapes are derived, never stored: `shape_with` reads only logical blocks,
//! so saves, networking, yields and costs are unchanged. One occupied cell is
//! still one gameplay unit; only its visible/collision volume changes, and
//! every shape stays inside its owner's cell bounds.
//!
//! Geometry contract: a shaped cell keeps a full square floor and replaces its
//! flat top with a four-triangle fan (`corners` + `center`). Corner heights
//! come from `corner_height`, a function of the lattice point alone, so
//! neighbors that share a corner agree on it regardless of chunk or edit
//! order. Side faces are emitted only where a cell's top edge rises above its
//! neighbor's (see `exposed_region`), which keeps mixed shape pairs watertight.

use super::block::BlockType;
use super::world::World;

/// Bump when rules change so screenshots/tests can name the rule set.
pub const SHAPING_VERSION: u32 = 1;
/// Lowered corner height at the top of a drop of more than one cell.
pub const SHOULDER: f32 = 0.6;
/// Rim height of an isolated or ridge cell (no fully supported corner).
pub const CAP_RIM: f32 = 0.7;
/// Maximum interior facet offset of a rock fan center.
const ROCK_FACET: f32 = 0.12;
const FACET_SALT: u32 = 0x5A9E_0001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Soil,
    Rock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    /// Two adjacent high corners: a one-cell directional ramp.
    Ramp,
    /// One high corner: the outside corner of a plateau.
    ConvexCorner,
    /// Three high corners: the inside elbow of a plateau.
    ConcaveCorner,
    /// No fully supported corner (isolated cell, ridge, pillar top).
    Cap,
}

/// Top corner order, as (x, z) offsets from the cell's min corner.
pub const CORNERS: [(i32, i32); 4] = [(0, 0), (1, 0), (1, 1), (0, 1)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellShape {
    pub material: Material,
    pub kind: ShapeKind,
    /// Top heights above the cell floor at `CORNERS`, each in 0..=1.
    pub corners: [f32; 4],
    /// Fan apex height at the cell's horizontal center.
    pub center: f32,
}

pub fn material(block: BlockType) -> Option<Material> {
    use BlockType::*;
    match block {
        Grass | Soil | Sand | Mud | Clay | Peat | Snow => Some(Material::Soil),
        Stone | Basalt | Granite | Limestone | Marble => Some(Material::Rock),
        _ => None,
    }
}

/// Whether a cell fully supports a terrain corner beside it. Architecture,
/// devices and other opaque solids count, so terrain keeps a stable contact
/// with buildings instead of ramping away from them.
fn supports(block: BlockType) -> bool {
    block.is_solid()
        && (block.is_opaque()
            || matches!(block, BlockType::AutomationDevice | BlockType::Campfire))
}

/// Height of the lattice point `(x, y + 1, z)` for cells on layer `y`.
/// Depends only on the four cells around the point on layers `y` and `y - 1`.
fn corner_height(sample: &impl Fn(i32, i32, i32) -> BlockType, x: i32, y: i32, z: i32) -> f32 {
    let all = |layer: i32| {
        [(x - 1, z - 1), (x, z - 1), (x - 1, z), (x, z)]
            .iter()
            .all(|&(cx, cz)| supports(sample(cx, layer, cz)))
    };
    if all(y) {
        1.0
    } else if all(y - 1) {
        0.0
    } else {
        SHOULDER
    }
}

/// The derived shape of the cell at `(x, y, z)`, or `None` when it renders and
/// collides as a plain cube (non-terrain, buried, full, or a fallback case).
pub fn shape_with(
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    x: i32,
    y: i32,
    z: i32,
) -> Option<CellShape> {
    let block = sample(x, y, z);
    let material = material(block)?;
    // Only exposed tops are shaped; ceilings and supports stay conservative.
    let above = sample(x, y + 1, z);
    if above.is_solid() || above == BlockType::Water || !supports(sample(x, y - 1, z)) {
        return None;
    }
    // Shores stay cubic: water faces are culled against opaque neighbors.
    for dy in 0..=1 {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if sample(x + dx, y + dy, z + dz) == BlockType::Water {
                    return None;
                }
            }
        }
    }
    let mut corners = CORNERS.map(|(cx, cz)| corner_height(sample, x + cx, y, z + cz));
    let high = corners.map(|h| h >= 1.0);
    let kind = match high.iter().filter(|&&h| h).count() {
        4 => return None,
        3 => ShapeKind::ConcaveCorner,
        // Opposite high corners are a saddle: no single ramp direction.
        2 if high[0] == high[2] => return None,
        2 => ShapeKind::Ramp,
        1 => ShapeKind::ConvexCorner,
        _ => ShapeKind::Cap,
    };
    let center = if kind == ShapeKind::Cap {
        corners = corners.map(|h| h.max(CAP_RIM));
        1.0
    } else {
        let average = corners.iter().sum::<f32>() / 4.0;
        let base = if kind == ShapeKind::ConvexCorner { average.max(0.5) } else { average };
        if material == Material::Rock {
            let r = super::noise::block_rand(x, y, z, SHAPING_VERSION, FACET_SALT);
            (base + (r * 2.0 - 1.0) * ROCK_FACET).clamp(0.2, 1.0)
        } else {
            base
        }
    };
    Some(CellShape { material, kind, corners, center })
}

/// `shape_with` against loaded world blocks; `None` while shaping is off.
pub fn shape_at(world: &World, x: i32, y: i32, z: i32) -> Option<CellShape> {
    if !world.terrain_shaping {
        return None;
    }
    shape_with(&|a, b, c| world.get_block(a, b, c), x, y, z)
}

/// Whether world point `p` lies inside the visible solid of the targetable
/// cell `voxel` (always true for unshaped cells).
pub fn cell_contains(shape: Option<&CellShape>, voxel: (i32, i32, i32), p: glam::Vec3) -> bool {
    shape.map_or(true, |s| {
        p.y - voxel.1 as f32 <= s.height(p.x - voxel.0 as f32, p.z - voxel.2 as f32)
    })
}

/// Top-edge heights of `shape` along cube face `face` (mesher order) at that
/// face's first and second bottom corner. `offset` shifts the corner lookup,
/// so passing the face normal reads the neighbor's heights on the shared face.
pub fn face_edge_heights(shape: Option<&CellShape>, face: usize, offset: [i32; 3]) -> [f32; 2] {
    let Some(shape) = shape else { return [1.0, 1.0] };
    let verts = super::mesher::FACE_VERTS[face];
    [0, 1].map(|i| {
        shape.corner(verts[i][0] as i32 - offset[0], verts[i][2] as i32 - offset[2])
    })
}

/// The part of a vertical face between a neighbor's covering profile `below`
/// and this cell's top profile `top`, both linear in face parameter `t`.
/// Returns a polygon of `(t, y)` points wound like `FACE_VERTS`.
pub fn exposed_region(below: [f32; 2], top: [f32; 2]) -> Vec<[f32; 2]> {
    const EPS: f32 = 1e-4;
    let d0 = top[0] - below[0];
    let d1 = top[1] - below[1];
    let mut points = Vec::with_capacity(4);
    if d0 <= EPS && d1 <= EPS {
        return points;
    }
    if d0 > EPS && d1 < -EPS {
        let t = d0 / (d0 - d1);
        points.extend([[0.0, below[0]], [t, below[0] + (below[1] - below[0]) * t], [0.0, top[0]]]);
    } else if d0 < -EPS && d1 > EPS {
        let t = -d0 / (d1 - d0);
        points.extend([[t, below[0] + (below[1] - below[0]) * t], [1.0, below[1]], [1.0, top[1]]]);
    } else {
        points.extend([[0.0, below[0]], [1.0, below[1]]]);
        if d1 > EPS {
            points.push([1.0, top[1]]);
        }
        if d0 > EPS {
            points.push([0.0, top[0]]);
        }
    }
    points
}

impl CellShape {
    /// Height at top corner `(cx, cz)` in 0..=1 coordinates.
    pub fn corner(&self, cx: i32, cz: i32) -> f32 {
        let index = CORNERS.iter().position(|&c| c == (cx, cz)).expect("cell corner");
        self.corners[index]
    }

    /// The four top triangles in local cell coordinates, each `[center,
    /// corner i, corner i + 1]`, wound like the mesher's +Y face.
    pub fn top_triangles(&self) -> [[[f32; 3]; 3]; 4] {
        let apex = [0.5, self.center, 0.5];
        std::array::from_fn(|i| {
            let point = |k: usize| {
                let (cx, cz) = CORNERS[k % 4];
                [cx as f32, self.corners[k % 4], cz as f32]
            };
            [apex, point(i), point(i + 1)]
        })
    }

    /// Surface height at local `(u, v)` (x, z) in 0..=1.
    pub fn height(&self, u: f32, v: f32) -> f32 {
        let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let (du, dv) = (u - 0.5, v - 0.5);
        let triangle = if dv.abs() >= du.abs() {
            if dv < 0.0 { 0 } else { 2 }
        } else if du > 0.0 {
            1
        } else {
            3
        };
        let [a, b, c] = self.top_triangles()[triangle];
        // Barycentric interpolation in the xz plane.
        let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
        let wa = ((b[2] - c[2]) * (u - c[0]) + (c[0] - b[0]) * (v - c[2])) / det;
        let wb = ((c[2] - a[2]) * (u - c[0]) + (a[0] - c[0]) * (v - c[2])) / det;
        wa * a[1] + wb * b[1] + (1.0 - wa - wb) * c[1]
    }

    /// Exact maximum of `height` over a local rectangle (clamped to the cell).
    /// The surface is linear on triangles split by the two diagonals, so the
    /// maximum lies on a rectangle corner, a diagonal crossing, or the apex.
    pub fn max_height_in(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> f32 {
        let (u0, u1) = (u0.clamp(0.0, 1.0), u1.clamp(0.0, 1.0));
        let (v0, v1) = (v0.clamp(0.0, 1.0), v1.clamp(0.0, 1.0));
        let mut best = [(u0, v0), (u1, v0), (u0, v1), (u1, v1)]
            .iter()
            .map(|&(u, v)| self.height(u, v))
            .fold(0.0f32, f32::max);
        let mut consider = |u: f32, v: f32| {
            if (u0..=u1).contains(&u) && (v0..=v1).contains(&v) {
                best = best.max(self.height(u, v));
            }
        };
        consider(0.5, 0.5);
        for u in [u0, u1] {
            consider(u, u);
            consider(u, 1.0 - u);
        }
        for v in [v0, v1] {
            consider(v, v);
            consider(1.0 - v, v);
        }
        best
    }
}

/// Builds the compact terrain test garden on flat ground whose top layer is
/// `origin.1 - 1`, inside a 32 x 16 footprint starting at `origin` (x, z).
/// Returns every edit so a host can replicate it like any other block change.
pub fn test_garden(origin: (i32, i32, i32)) -> Vec<(i32, i32, i32, BlockType)> {
    let (ox, y, oz) = origin;
    let mut cells = std::collections::BTreeMap::new();
    let mut fill = |x: i32, z: i32, from: i32, to: i32, block: BlockType| {
        for yy in from..to {
            cells.insert((ox + x, y + yy, oz + z), block);
        }
    };
    // Clear the footprint above ground and lay a stone/soil/grass floor.
    for x in 0..32 {
        for z in 0..16 {
            fill(x, z, 0, 8, BlockType::Air);
            fill(x, z, -3, -1, BlockType::Soil);
            fill(x, z, -1, 0, BlockType::Grass);
        }
    }
    fn column(fill: &mut impl FnMut(i32, i32, i32, i32, BlockType), x: i32, z: i32, h: i32, top: BlockType) {
        fill(x, z, 0, h - 1, BlockType::Soil);
        fill(x, z, h - 1, h, top);
    }
    // 1: flat 4x4 plateau, one cell high -> ramps on all four sides and
    // convex corners at its four corners.
    for x in 1..5 {
        for z in 1..5 {
            column(&mut fill, x, z, 1, BlockType::Grass);
        }
    }
    // 2: thick L-shaped plateau -> concave elbow at its inside corner.
    for x in 6..10 {
        for z in 1..5 {
            if x < 8 || z < 3 {
                column(&mut fill, x, z, 1, BlockType::Grass);
            }
        }
    }
    // 3: one-cell ridge and one isolated cell -> caps.
    for z in 7..12 {
        column(&mut fill, 2, z, 1, BlockType::Grass);
    }
    column(&mut fill, 5, 9, 1, BlockType::Grass);
    // 4: one-cell pit in a plateau.
    for x in 11..16 {
        for z in 1..6 {
            if (x, z) != (13, 3) {
                column(&mut fill, x, z, 1, BlockType::Grass);
            }
        }
    }
    // 5: tall cliff (three cells) -> restrained shoulders.
    for x in 18..22 {
        for z in 1..5 {
            column(&mut fill, x, z, 3, BlockType::Grass);
        }
    }
    // 6: two-step hill, 2 cells high.
    for x in 7..14 {
        for z in 7..14 {
            let h = if (9..12).contains(&x) && (9..12).contains(&z) { 2 } else { 1 };
            column(&mut fill, x, z, h, BlockType::Grass);
        }
    }
    // 7: rock mound and mixed rock/soil slope.
    for x in 17..22 {
        for z in 8..13 {
            let h = if (18..21).contains(&x) && (9..12).contains(&z) { 2 } else { 1 };
            column(&mut fill, x, z, h, BlockType::Stone);
        }
    }
    for z in 8..13 {
        column(&mut fill, 22, z, 1, BlockType::Grass);
    }
    // 8: overhang (cave lip) and architectural contact.
    for x in 24..28 {
        for z in 1..4 {
            column(&mut fill, x, z, 2, BlockType::Grass);
        }
    }
    fill(25, 1, 0, 1, BlockType::Air);
    fill(25, 2, 0, 1, BlockType::Air);
    for z in 5..9 {
        fill(24, z, 0, 3, BlockType::Bricks);
        column(&mut fill, 25, z, 1, BlockType::Grass);
    }
    // 9: a ramp line crossing a chunk border, wherever that falls.
    let border = (ox + 31).div_euclid(16) * 16 - ox;
    for x in (border - 3)..(border + 3) {
        if (0..32).contains(&x) {
            column(&mut fill, x, 15, 1, BlockType::Grass);
        }
    }
    cells.into_iter().map(|((x, y, z), b)| (x, y, z, b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::chunk::Chunk;
    use crate::voxel::mesher::build_chunk_mesh;
    use glam::Vec3;

    /// Empty loaded chunks plus `cells`, with shaping on.
    fn world_of(chunks: &[(i32, i32)], cells: impl IntoIterator<Item = (i32, i32, i32, BlockType)>) -> World {
        let mut world = World::new(7);
        for &(cx, cz) in chunks {
            world.chunks.insert((cx, cz), Chunk::new(cx, cz));
        }
        for (x, y, z, block) in cells {
            world.set_block(x, y, z, block);
        }
        world.terrain_shaping = true;
        world
    }

    /// Grass-topped columns over x, z in 0..16 with heights from `height`.
    fn terrain(height: impl Fn(i32, i32) -> i32) -> World {
        let mut cells = vec![];
        for x in 0..16 {
            for z in 0..16 {
                let h = height(x, z);
                for y in 0..h {
                    cells.push((x, y, z, if y == h - 1 { BlockType::Grass } else { BlockType::Soil }));
                }
            }
        }
        world_of(&[(0, 0)], cells)
    }

    fn plateau(x: i32, z: i32) -> i32 {
        if (5..10).contains(&x) && (5..10).contains(&z) { 3 } else { 2 }
    }

    #[test]
    fn straight_ramps_face_each_way_and_corners_are_convex() {
        let world = terrain(plateau);
        let shape = |x, z| shape_at(&world, x, 2, z).expect("plateau rim is shaped");
        for (x, z, corners) in [
            (5, 7, [0.0, 1.0, 1.0, 0.0]),
            (9, 7, [1.0, 0.0, 0.0, 1.0]),
            (7, 5, [0.0, 0.0, 1.0, 1.0]),
            (7, 9, [1.0, 1.0, 0.0, 0.0]),
        ] {
            let s = shape(x, z);
            assert_eq!((s.kind, s.corners, s.center), (ShapeKind::Ramp, corners, 0.5), "{x},{z}");
            // A 45-degree ramp: height rises linearly across the cell.
            assert!((s.height(0.5, 0.5) - 0.5).abs() < 1e-6);
        }
        let corner = shape(5, 5);
        assert_eq!((corner.kind, corner.corners), (ShapeKind::ConvexCorner, [0.0, 0.0, 1.0, 0.0]));
        assert!(corner.center >= 0.5, "convex corners keep substantial volume");
        // Interior and lower ground stay plain cubes.
        assert_eq!(shape_at(&world, 7, 2, 7), None);
        assert_eq!(shape_at(&world, 2, 1, 7), None);
        assert_eq!(shape_at(&world, 7, 1, 7), None, "buried cells are never shaped");
        // Shaping off: nothing is shaped.
        let mut cubic = terrain(plateau);
        cubic.terrain_shaping = false;
        assert_eq!(shape_at(&cubic, 5, 2, 7), None);
    }

    #[test]
    fn rotated_neighborhoods_give_rotated_shapes() {
        let world = terrain(plateau);
        // Rotating the plateau a quarter turn about its center (7, 7) maps
        // cell (x, z) to (14 - z, x) and corner i to corner i + 1.
        for x in 4..11 {
            for z in 4..11 {
                let a = shape_at(&world, x, 2, z);
                let b = shape_at(&world, 14 - z, 2, x);
                assert_eq!(a.map(|s| s.kind), b.map(|s| s.kind), "{x},{z}");
                if let (Some(a), Some(b)) = (a, b) {
                    let rotated: [f32; 4] = std::array::from_fn(|i| a.corners[(i + 3) % 4]);
                    assert_eq!(rotated, b.corners, "{x},{z}");
                }
            }
        }
    }

    #[test]
    fn concave_cap_saddle_cliff_and_conservative_cases() {
        // A one-cell pit: diagonal cells become concave, edge cells face in.
        let pit = terrain(|x, z| if (5..12).contains(&x) && (5..12).contains(&z) && (x, z) != (8, 8) { 3 } else { 2 });
        let concave = shape_at(&pit, 9, 2, 9).unwrap();
        assert_eq!((concave.kind, concave.corners), (ShapeKind::ConcaveCorner, [0.0, 1.0, 1.0, 1.0]));
        assert_eq!(shape_at(&pit, 9, 2, 8).unwrap().corners, [0.0, 1.0, 1.0, 0.0]);

        // An isolated cell keeps a substantial cap; so does a one-cell ridge.
        let lone = terrain(|x, z| if (x, z) == (7, 7) || (x == 3 && (4..10).contains(&z)) { 3 } else { 2 });
        let cap = shape_at(&lone, 7, 2, 7).unwrap();
        assert_eq!((cap.kind, cap.center), (ShapeKind::Cap, 1.0));
        assert!(cap.corners.iter().all(|&h| h == CAP_RIM));
        assert_eq!(shape_at(&lone, 3, 2, 6).unwrap().kind, ShapeKind::Cap);

        // Opposite high corners have no ramp direction: block fallback.
        let raised = [(4, 4), (5, 4), (4, 5), (5, 5), (6, 5), (5, 6), (6, 6)];
        let saddle = terrain(|x, z| if raised.contains(&(x, z)) { 3 } else { 2 });
        assert_eq!(shape_at(&saddle, 5, 2, 5), None);

        // Drops of more than one cell get a restrained shoulder, not a ramp
        // into the missing cells.
        let cliff = terrain(|x, z| if (5..10).contains(&x) && (5..10).contains(&z) { 5 } else { 2 });
        assert_eq!(shape_at(&cliff, 5, 4, 7).unwrap().corners, [SHOULDER, 1.0, 1.0, SHOULDER]);

        // Overhangs, water shores and architecture contacts stay conservative.
        let mut world = terrain(plateau);
        world.set_block(9, 1, 7, BlockType::Air);
        assert_eq!(shape_at(&world, 9, 2, 7), None, "no support below");
        world.set_block(7, 2, 4, BlockType::Water);
        assert_eq!(shape_at(&world, 7, 2, 5), None, "water nearby");
        for z in 5..10 {
            world.set_block(10, 2, z, BlockType::Bricks);
        }
        assert_eq!(shape_at(&world, 9, 2, 6), None, "flush with a wall");
        assert_eq!(shape_at(&world, 10, 2, 6), None, "architecture is never shaped");
    }

    #[test]
    fn max_height_in_matches_dense_sampling() {
        let world = terrain(plateau);
        for (x, z) in [(5, 5), (5, 7), (9, 9)] {
            let s = shape_at(&world, x, 2, z).unwrap();
            for (u0, u1, v0, v1) in [(0.0, 1.0, 0.0, 1.0), (0.1, 0.4, 0.6, 0.9), (0.7, 1.0, 0.0, 0.2), (0.3, 0.8, 0.2, 0.45)] {
                let mut dense = 0.0f32;
                for i in 0..=50 {
                    for j in 0..=50 {
                        let u = u0 + (u1 - u0) * i as f32 / 50.0;
                        let v = v0 + (v1 - v0) * j as f32 / 50.0;
                        dense = dense.max(s.height(u, v));
                    }
                }
                let exact = s.max_height_in(u0, u1, v0, v1);
                assert!(exact >= dense - 1e-5 && exact <= dense + 0.03, "{x},{z} {exact} vs {dense}");
            }
        }
    }

    type Triangle = [Vec3; 3];

    fn triangles(world: &World) -> Vec<Triangle> {
        let mut keys: Vec<_> = world.chunks.keys().copied().collect();
        keys.sort();
        let mut out = vec![];
        for key in keys {
            let mesh = build_chunk_mesh(world, &world.chunks[&key]);
            for tri in mesh.indices.chunks(3) {
                out.push([tri[0], tri[1], tri[2]].map(|i| Vec3::from(mesh.vertices[i as usize].position)));
            }
        }
        out
    }

    fn garden_world() -> World {
        world_of(&[(0, 0), (1, 0)], test_garden((0, 4, 0)))
    }

    fn inside(world: &World, p: Vec3) -> bool {
        let v = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        world.get_block(v.0, v.1, v.2).is_opaque()
            && cell_contains(shape_at(world, v.0, v.1, v.2).as_ref(), v, p)
    }

    /// Crossing coordinates of an axis-aligned line through `(a, b)` on the
    /// two other axes; `axis` is the line's direction.
    fn crossings(tris: &[Triangle], axis: usize, a: f32, b: f32) -> Vec<f32> {
        let (i, j) = match axis { 0 => (1, 2), 1 => (0, 2), _ => (0, 1) };
        let mut hits = vec![];
        for t in tris {
            let (p, q, r) = (t[0], t[1], t[2]);
            let det = (q[j] - r[j]) * (p[i] - r[i]) + (r[i] - q[i]) * (p[j] - r[j]);
            if det.abs() < 1e-9 {
                continue;
            }
            let w0 = ((q[j] - r[j]) * (a - r[i]) + (r[i] - q[i]) * (b - r[j])) / det;
            let w1 = ((r[j] - p[j]) * (a - r[i]) + (p[i] - r[i]) * (b - r[j])) / det;
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                hits.push(w0 * p[axis] + w1 * q[axis] + w2 * r[axis]);
            }
        }
        hits
    }

    /// Parity along many axis lines must match the logical solid: catches
    /// holes, missing partial sides and duplicated coplanar faces at once.
    fn assert_watertight(world: &World, size: Vec3) {
        let tris = triangles(world);
        let mut checked = 0;
        for axis in 0..3 {
            let (i, j) = match axis { 0 => (1, 2), 1 => (0, 2), _ => (0, 1) };
            let mut a = 0.1371;
            while a < size[i] {
                let mut b = 0.2913;
                while b < size[j] {
                    let hits = crossings(&tris, axis, a, b);
                    let mut c = 0.0731;
                    while c < size[axis] {
                        let mut p = Vec3::ZERO;
                        p[axis] = c;
                        p[i] = a;
                        p[j] = b;
                        let parity = hits.iter().filter(|&&h| h > c).count() % 2 == 1;
                        assert_eq!(parity, inside(world, p), "axis {axis} at {p}");
                        checked += 1;
                        c += 0.4173;
                    }
                    b += 0.6127;
                }
                a += 0.5309;
            }
        }
        assert!(checked > 10_000);
    }

    #[test]
    fn test_garden_mesh_is_watertight_in_both_modes() {
        let mut world = garden_world();
        let size = Vec3::new(32.0, 13.0, 16.0);
        assert_watertight(&world, size);
        world.terrain_shaping = false;
        assert_watertight(&world, size);
    }

    #[test]
    fn every_triangle_lies_inside_one_occupied_owner() {
        let world = garden_world();
        let mut shaped = 0;
        for t in triangles(&world) {
            let normal = (t[1] - t[0]).cross(t[2] - t[0]);
            if normal.length() < 1e-7 {
                continue;
            }
            // Winding matches FACE_VERTS, whose cross product points inward.
            let centroid = (t[0] + t[1] + t[2]) / 3.0;
            let probe = centroid + normal.normalize() * 0.01;
            let owner = probe.floor();
            let block = world.get_block(owner.x as i32, owner.y as i32, owner.z as i32);
            assert!(block.is_opaque(), "triangle {t:?} has no occupied owner");
            for v in t {
                let local = v - owner;
                assert!(local.min_element() > -1e-4 && local.max_element() < 1.0 + 1e-4, "{t:?} leaves {owner}");
            }
            shaped += shape_at(&world, owner.x as i32, owner.y as i32, owner.z as i32).is_some() as usize;
        }
        assert!(shaped > 100, "the garden should exercise many shaped cells");
    }

    #[test]
    fn every_documented_shape_appears_in_the_garden() {
        let world = garden_world();
        let mut kinds = std::collections::BTreeSet::new();
        let mut rock = false;
        let mut shoulder = false;
        for x in 0..32 {
            for z in 0..16 {
                for y in 3..12 {
                    if let Some(s) = shape_at(&world, x, y, z) {
                        kinds.insert(format!("{:?}", s.kind));
                        rock |= s.material == Material::Rock;
                        shoulder |= s.corners.contains(&SHOULDER);
                    }
                }
            }
        }
        assert_eq!(kinds.len(), 4, "{kinds:?}");
        assert!(rock && shoulder);
    }

    #[test]
    fn shapes_ignore_chunk_order_and_edits_are_reversible() {
        let cells = test_garden((0, 4, 0));
        let forward = world_of(&[(0, 0), (1, 0)], cells.clone());
        let mut backward = world_of(&[(1, 0), (0, 0)], cells.iter().rev().copied());
        let positions = |w: &World| triangles(w).iter().flatten().map(|v| v.to_array()).collect::<Vec<_>>();
        let before = positions(&forward);
        assert_eq!(before, positions(&backward));

        // Dig one ramp cell: exactly that cell changes, neighbors only reshape.
        let ramp = (1, 4, 2);
        assert_eq!(shape_at(&backward, ramp.0, ramp.1, ramp.2).unwrap().kind, ShapeKind::Ramp);
        let snapshot = |w: &World| {
            (0..32).flat_map(|x| (0..16).flat_map(move |z| (0..12).map(move |y| (x, y, z))))
                .map(|(x, y, z)| w.get_block(x, y, z)).collect::<Vec<_>>()
        };
        let solid = |w: &World| snapshot(w).iter().filter(|b| b.is_solid()).count();
        let (blocks, count) = (snapshot(&backward), solid(&backward));
        let original = backward.get_block(ramp.0, ramp.1, ramp.2);
        backward.set_block(ramp.0, ramp.1, ramp.2, BlockType::Air);
        assert_eq!(solid(&backward), count - 1);
        let after = snapshot(&backward);
        assert_eq!(blocks.iter().zip(&after).filter(|(a, b)| a != b).count(), 1);
        backward.set_block(ramp.0, ramp.1, ramp.2, original);
        assert_eq!(snapshot(&backward), blocks);
        assert_eq!(positions(&backward), before);
    }

    #[test]
    fn chunk_border_edits_remesh_the_neighbor() {
        let mut world = world_of(&[(0, 0), (1, 0)], test_garden((0, 4, 0)));
        // The z = 15 ramp line crosses x = 16: digging (15, 4, 15) in chunk 0
        // exposes the side of the cap at (16, 4, 15) in chunk 1.
        let mesh = |w: &World| {
            build_chunk_mesh(w, &w.chunks[&(1, 0)]).vertices.iter().map(|v| v.position).collect::<Vec<_>>()
        };
        let before = mesh(&world);
        for chunk in world.chunks.values_mut() {
            chunk.dirty = false;
        }
        world.set_block(15, 4, 15, BlockType::Air);
        assert!(world.chunks[&(1, 0)].dirty, "neighbor chunk must remesh");
        assert_ne!(mesh(&world), before);
    }

    #[test]
    fn rays_pass_through_a_wedges_empty_part_and_place_above_it() {
        let mut world = terrain(plateau);
        // Along the -X rim (local u = 0.1 surface height 0.1) at height 0.5.
        let along = crate::raycast::raycast(&world, Vec3::new(5.1, 2.5, 6.2), Vec3::Z, 2.5);
        assert!(along.is_none(), "should not pick the ramp's empty AABB");
        // Straight down onto the low side of the ramp: hit its real surface.
        let down = crate::raycast::raycast(&world, Vec3::new(5.2, 4.0, 7.5), -Vec3::Y, 3.0).unwrap();
        assert_eq!((down.target, down.place), ((5, 2, 7), (5, 3, 7)));
        // From the side, toward the high part: the neighbor in front is the place.
        let side = crate::raycast::raycast(&world, Vec3::new(3.5, 2.9, 7.5), Vec3::X, 4.0).unwrap();
        assert_eq!(side.target, (5, 2, 7));
        assert_ne!(world.get_block(side.place.0, side.place.1, side.place.2), BlockType::Grass);
        world.terrain_shaping = false;
        let cubic = crate::raycast::raycast(&world, Vec3::new(5.1, 2.5, 6.2), Vec3::Z, 2.5);
        assert_eq!(cubic.map(|h| h.target), Some((5, 2, 6)));
    }

    /// `cargo test --release mesh_cost -- --ignored --nocapture`
    #[test]
    #[ignore = "timing report, run manually"]
    fn mesh_cost_report_on_generated_terrain() {
        let mut world = World::new(12345);
        for cx in -3..=3 {
            for cz in -3..=3 {
                world.ensure_chunk_loaded(cx, cz);
            }
        }
        for shaping in [false, true] {
            world.terrain_shaping = shaping;
            let mut keys: Vec<_> = world.chunks.keys().copied().filter(|(x, z)| x.abs() <= 2 && z.abs() <= 2).collect();
            keys.sort();
            let (mut tris, mut shaped) = (0, 0);
            let start = std::time::Instant::now();
            for _ in 0..3 {
                tris = 0;
                for key in &keys {
                    tris += build_chunk_mesh(&world, &world.chunks[key]).indices.len() / 3;
                }
            }
            let per_chunk = start.elapsed().as_secs_f64() * 1000.0 / (3 * keys.len()) as f64;
            for key in &keys {
                let (ox, oz) = world.chunks[key].world_origin();
                for x in ox..ox + 16 {
                    for z in oz..oz + 16 {
                        for y in 0..crate::voxel::chunk::CHUNK_Y {
                            shaped += shape_at(&world, x, y, z).is_some() as usize;
                        }
                    }
                }
            }
            println!("shaping={shaping}: {per_chunk:.2} ms/chunk, {tris} triangles, {shaped} shaped cells over {} chunks", keys.len());
        }
    }
}
