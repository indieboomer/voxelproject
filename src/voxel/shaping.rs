//! Experimental neighborhood-driven terrain shaping (docs/TERRAIN_SHAPING.md).
//!
//! Shapes are derived, never stored: `shape_with` reads only logical blocks,
//! so saves, networking, yields and costs are unchanged. One occupied cell is
//! still one gameplay unit; only its visible/collision volume changes, and
//! every shape stays inside its owner's cell bounds.
//!
//! Geometry contract: each terrain column has one continuous top surface,
//! a height grid over its footprint. Its rim heights come from `lattice_height`,
//! a function of the four column tops around a lattice point, so neighbors
//! agree on shared corners regardless of chunk or edit order. The surface is
//! then partitioned by layer: every cell of the column owns the part of the
//! surface inside its own slab, so a steep bank spanning several cells is one
//! landform rather than a stack of separately capped blocks. Side faces are
//! emitted only where a cell's profile rises above its neighbor's (see
//! `exposed_region`), which keeps mixed shape pairs watertight.

use super::block::BlockType;
use super::world::World;

/// Bump when rules change so screenshots/tests can name the rule set.
pub const SHAPING_VERSION: u32 = 3;
/// Rim height of the shoulder at the top of a drop taller than `BANK_DROP`.
pub const SHOULDER: f32 = 0.6;
/// Largest rise across one lattice point that forms a continuous bank; a
/// taller drop keeps a real cliff with a restrained shoulder.
pub const BANK_DROP: i32 = 3;
/// Rim height of an isolated or ridge cell (no fully supported corner).
pub const CAP_RIM: f32 = 0.7;
/// Top surface samples per side: a 4x4 height grid, 18 triangles.
pub const GRID: usize = 4;
const VARIANT_SALT: u32 = 0x5A9E_0001;
const JITTER_SALT: u32 = 0x5A9E_0100;
/// Interior grid points; the rim stays linear between corners so every
/// neighbor sees the same boundary regardless of either cell's variant.
const INTERIOR: [(usize, usize); 4] = [(1, 1), (2, 1), (1, 2), (2, 2)];

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
    /// A cell below its column's top whose slab the bank surface cuts through.
    Bank,
}

/// Cosmetic surface treatment, chosen per cell from its world position. Only
/// interior grid points move, so variants never affect neighbors or seams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// The plain slope (rock: broad deterministic facets).
    Smooth,
    /// Bulging, rounded shoulder.
    Rounded,
    /// Scooped, eroded hollow.
    Hollow,
    /// Non-smooth: a flat ledge between two short steep risers.
    Terraced,
    /// Non-smooth: lumpy broken ground.
    Rocky,
}

/// Top corner order, as (x, z) offsets from the cell's min corner.
pub const CORNERS: [(i32, i32); 4] = [(0, 0), (1, 0), (1, 1), (0, 1)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellShape {
    pub material: Material,
    pub kind: ShapeKind,
    /// Column surface heights above this cell's floor at `CORNERS`. Values
    /// above 1 belong to cells higher in the column, below 0 to lower ones.
    pub corners: [f32; 4],
    pub variant: Variant,
    /// Column surface heights above this cell's floor at `(i / 3, j / 3)`
    /// as `grid[i][j]`, unclamped like `corners`.
    pub grid: [[f32; GRID]; GRID],
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

/// Top of column `(x, z)` near `reference`: one above its highest supporting
/// cell within a window. Taller columns read as `reference + 4`, columns with
/// nothing in the window as `reference - 5`; both lie beyond `BANK_DROP`.
fn column_top(sample: &impl Fn(i32, i32, i32) -> BlockType, x: i32, z: i32, reference: i32) -> i32 {
    if supports(sample(x, reference + 3, z)) {
        return reference + 4;
    }
    (reference - 4..reference + 3)
        .rev()
        .find(|&y| supports(sample(x, y, z)))
        .map_or(reference - 5, |y| y + 1)
}

/// Surface height of lattice point `(x, z)` relative to `floor`, from the
/// tops of the four columns meeting there (`reference` is the asking column's
/// top). A one-cell rise always slopes down to the lowest top. Rises up to
/// `BANK_DROP` form a continuous bank only through soil: rock keeps a real
/// cliff. Taller drops keep the high side with a shoulder.
fn lattice_height(
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    top_of: &impl Fn(i32, i32, i32) -> i32,
    x: i32,
    z: i32,
    reference: i32,
    floor: i32,
) -> f32 {
    let columns = [(x - 1, z - 1), (x, z - 1), (x - 1, z), (x, z)];
    let tops = columns.map(|(cx, cz)| top_of(cx, cz, reference));
    let (low, high) = (*tops.iter().min().unwrap(), *tops.iter().max().unwrap());
    let earthen = || {
        columns.iter().zip(tops).all(|(&(cx, cz), top)| {
            (low..top).all(|y| material(sample(cx, y, cz)) == Some(Material::Soil))
        })
    };
    if high - low <= 1 || (high - low <= BANK_DROP && earthen()) {
        (low - floor) as f32
    } else {
        (high - floor - 1) as f32 + SHOULDER
    }
}

fn water_near(sample: &impl Fn(i32, i32, i32) -> BlockType, x: i32, y: i32, z: i32) -> bool {
    (-1..=1).any(|dx| (-1..=1).any(|dz| sample(x + dx, y, z + dz) == BlockType::Water))
}

/// The derived shape of the cell at `(x, y, z)`, or `None` when it renders and
/// collides as a plain cube (non-terrain, buried, full, or a fallback case).
pub fn shape_with(
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    x: i32,
    y: i32,
    z: i32,
) -> Option<CellShape> {
    shape_with_tops(sample, &|cx, cz, reference| column_top(sample, cx, cz, reference), x, y, z)
}

/// Memoized `shape_with` for one mesh build: every cell of a bank and each
/// neighbor face asks about the same columns, so shapes and column tops are
/// computed once per position. Results are identical to `shape_with`.
pub struct ShapeCache<'a, F: Fn(i32, i32, i32) -> BlockType> {
    sample: &'a F,
    shapes: std::cell::RefCell<std::collections::HashMap<(i32, i32, i32), Option<CellShape>>>,
    tops: std::cell::RefCell<std::collections::HashMap<(i32, i32, i32), i32>>,
    steepness: std::cell::RefCell<std::collections::HashMap<(i32, i32, i32), f32>>,
}

impl<'a, F: Fn(i32, i32, i32) -> BlockType> ShapeCache<'a, F> {
    pub fn new(sample: &'a F) -> Self {
        Self { sample, shapes: Default::default(), tops: Default::default(), steepness: Default::default() }
    }

    pub fn get(&self, x: i32, y: i32, z: i32) -> Option<CellShape> {
        // Cheap rejection for the common case: non-terrain or deeply buried.
        if material((self.sample)(x, y, z)).is_none()
            || (1..=BANK_DROP + 1).all(|k| (self.sample)(x, y + k, z).is_solid())
        {
            return None;
        }
        if let Some(shape) = self.shapes.borrow().get(&(x, y, z)) {
            return *shape;
        }
        let top_of = |cx: i32, cz: i32, reference: i32| {
            if let Some(&top) = self.tops.borrow().get(&(cx, cz, reference)) {
                return top;
            }
            let top = column_top(self.sample, cx, cz, reference);
            self.tops.borrow_mut().insert((cx, cz, reference), top);
            top
        };
        let shape = shape_with_tops(self.sample, &top_of, x, y, z);
        self.shapes.borrow_mut().insert((x, y, z), shape);
        shape
    }

    fn top(&self, x: i32, z: i32, reference: i32) -> i32 {
        if let Some(&top) = self.tops.borrow().get(&(x, z, reference)) {
            return top;
        }
        let top = column_top(self.sample, x, z, reference);
        self.tops.borrow_mut().insert((x, z, reference), top);
        top
    }

    /// How much grass covers world point `p`, 0..=1: one minus a smooth
    /// world-space steepness field. The field is the terrain gradient from
    /// neighboring column tops (central differences, rise per cell),
    /// bilinearly interpolated between column centers. It is continuous
    /// across cells, so grass and earth form coherent areas instead of a
    /// per-cell patchwork: a one-cell ledge inside a steep bank reads as bank,
    /// and wide shelves and gentle slopes stay green.
    pub fn grass_cover(&self, p: [f32; 3]) -> f32 {
        let reference = p[1].floor() as i32;
        let steepness = |x: i32, z: i32| {
            if let Some(&s) = self.steepness.borrow().get(&(x, z, reference)) {
                return s;
            }
            let t = |dx: i32, dz: i32| self.top(x + dx, z + dz, reference) as f32;
            let gx = (t(1, 0) - t(-1, 0)) * 0.5;
            let gz = (t(0, 1) - t(0, -1)) * 0.5;
            let s = (gx * gx + gz * gz).sqrt();
            self.steepness.borrow_mut().insert((x, z, reference), s);
            s
        };
        let (fx, fz) = (p[0] - 0.5, p[2] - 0.5);
        let (x0, z0) = (fx.floor() as i32, fz.floor() as i32);
        let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
        let low = steepness(x0, z0) + (steepness(x0 + 1, z0) - steepness(x0, z0)) * tx;
        let high = steepness(x0, z0 + 1) + (steepness(x0 + 1, z0 + 1) - steepness(x0, z0 + 1)) * tx;
        let s = low + (high - low) * tz;
        1.0 - ((s - 1.1) / 0.6).clamp(0.0, 1.0)
    }
}

fn shape_with_tops(
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    top_of: &impl Fn(i32, i32, i32) -> i32,
    x: i32,
    y: i32,
    z: i32,
) -> Option<CellShape> {
    let material = material(sample(x, y, z))?;
    // The column's exposed top cell: this cell or up to BANK_DROP above it,
    // through shapeable terrain only.
    let mut top = y;
    loop {
        let above = sample(x, top + 1, z);
        if !above.is_solid() {
            if above == BlockType::Water {
                return None;
            }
            break;
        }
        if top - y >= BANK_DROP || self::material(above).is_none() {
            return None;
        }
        top += 1;
    }
    // Shores stay cubic: water faces are culled against opaque neighbors.
    if water_near(sample, x, top, z) || water_near(sample, x, top + 1, z) {
        return None;
    }
    // Lowest layer the surface may cut into: shapeable, dry, and supported.
    let mut floor = top;
    while top - floor < BANK_DROP
        && self::material(sample(x, floor - 1, z)).is_some()
        && !water_near(sample, x, floor - 1, z)
    {
        floor -= 1;
    }
    while !supports(sample(x, floor - 1, z)) {
        floor += 1;
        if floor > top {
            return None; // overhang lip or narrow support: keep the cube
        }
    }
    if y < floor {
        return None;
    }
    // Corner heights relative to the top cell's floor.
    let mut corners = CORNERS.map(|(cx, cz)| lattice_height(sample, top_of, x + cx, z + cz, top + 1, top).min(1.0));
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
        if kind == ShapeKind::ConvexCorner { average.max(0.5) } else { average }
    };
    let lowest = (floor - top) as f32;
    corners = corners.map(|h| h.max(lowest));
    // Base surface: the fan from the corners to `center`, sampled on the grid.
    let mut grid: [[f32; GRID]; GRID] = std::array::from_fn(|i| {
        std::array::from_fn(|j| fan_height(corners, center, i as f32 / 3.0, j as f32 / 3.0))
    });
    // Variants are keyed to the column's top cell so every cell of a bank
    // partitions the same surface.
    let pick = super::noise::block_rand(x, top, z, SHAPING_VERSION, VARIANT_SALT);
    let variant = match material {
        Material::Soil => [Variant::Smooth, Variant::Rounded, Variant::Hollow, Variant::Terraced, Variant::Rocky]
            [((pick * 5.0) as usize).min(4)],
        Material::Rock => [Variant::Smooth, Variant::Terraced, Variant::Rocky][((pick * 3.0) as usize).min(2)],
    };
    let low_corner = corners.iter().copied().fold(f32::INFINITY, f32::min);
    let shelf = INTERIOR.iter().map(|&(i, j)| grid[i][j]).sum::<f32>() / 4.0;
    for (n, &(i, j)) in INTERIOR.iter().enumerate() {
        let jitter = super::noise::block_rand(x, top, z, SHAPING_VERSION, JITTER_SALT + n as u32) * 2.0 - 1.0;
        let base = grid[i][j];
        grid[i][j] = match variant {
            Variant::Smooth if material == Material::Rock => base + jitter * 0.12,
            Variant::Smooth => base,
            Variant::Rounded => base + 0.14,
            Variant::Hollow => base - 0.14,
            Variant::Terraced => shelf + (base - shelf) * 0.15,
            Variant::Rocky => base + jitter * 0.2,
        }
        .clamp(if low_corner >= 0.0 { 0.05 } else { low_corner.max(lowest) }, 1.0);
    }
    // Re-express everything relative to this cell's own floor.
    let lift = (top - y) as f32;
    let corners = corners.map(|h| h + lift);
    let grid = grid.map(|row| row.map(|h| h + lift));
    if grid.iter().flatten().all(|&h| h >= 1.0) {
        return None; // the surface never dips into this cell's slab
    }
    let kind = if y == top { kind } else { ShapeKind::Bank };
    let material = self::material(sample(x, y, z)).unwrap_or(material);
    Some(CellShape { material, kind, corners, variant, grid })
}

/// The original four-triangle fan from `corners` to an apex at the cell
/// center; linear along each rim edge.
fn fan_height(corners: [f32; 4], center: f32, u: f32, v: f32) -> f32 {
    let (du, dv) = (u - 0.5, v - 0.5);
    let edge = if dv.abs() >= du.abs() {
        if dv < 0.0 { 0 } else { 2 }
    } else if du > 0.0 {
        1
    } else {
        3
    };
    let point = |k: usize| {
        let (cx, cz) = CORNERS[k % 4];
        [cx as f32, corners[k % 4], cz as f32]
    };
    let (a, b, c) = ([0.5, center, 0.5], point(edge), point(edge + 1));
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    let wa = ((b[2] - c[2]) * (u - c[0]) + (c[0] - b[0]) * (v - c[2])) / det;
    let wb = ((c[2] - a[2]) * (u - c[0]) + (a[0] - c[0]) * (v - c[2])) / det;
    wa * a[1] + wb * b[1] + (1.0 - wa - wb) * c[1]
}

/// `shape_with` against loaded world blocks; `None` while shaping is off.
pub fn shape_at(world: &World, x: i32, y: i32, z: i32) -> Option<CellShape> {
    if !world.terrain_shaping {
        return None;
    }
    shape_with(&|a, b, c| world.get_block(a, b, c), x, y, z)
}

/// Display height for a body standing at `feet` on the voxel grid: lowered
/// onto shaped terrain under its footprint (center and four corners at
/// `radius`), never raised and never by more than one cell. Bodies that are
/// not standing on a cell top (flying, swimming, mid-jump) are unchanged.
pub fn rest_height(world: &World, feet: glam::Vec3, radius: f32) -> f32 {
    if !world.terrain_shaping || (feet.y - feet.y.round()).abs() > 0.02 {
        return feet.y;
    }
    let base = feet.y.round() as i32;
    let mut top = f32::NEG_INFINITY;
    for (dx, dz) in [(0.0, 0.0), (-radius, -radius), (radius, -radius), (-radius, radius), (radius, radius)] {
        let (x, z) = (feet.x + dx, feet.z + dz);
        let (cx, cz) = (x.floor() as i32, z.floor() as i32);
        // Two cells down covers bodies stepped up onto a ramp they overlap.
        if let Some(y) = (base - 2..base).rev().find(|&y| world.is_solid(cx, y, cz)) {
            let height = shape_at(world, cx, y, cz).map_or(1.0, |s| s.height(x - cx as f32, z - cz as f32));
            top = top.max(y as f32 + height);
        }
    }
    if top.is_finite() && top < feet.y {
        top.max(feet.y - 1.0)
    } else {
        feet.y
    }
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
/// and this cell's own profile `top`. Both are linear in face parameter `t`
/// but unclamped (column heights relative to this cell's floor), so the band
/// is clipped to the cell's slab 0..=1. Returns convex `(t, y)` polygons
/// wound like `FACE_VERTS`.
pub fn exposed_region(below: [f32; 2], top: [f32; 2]) -> Vec<Vec<[f32; 2]>> {
    const EPS: f32 = 1e-4;
    let at = |p: [f32; 2], t: f32| p[0] + (p[1] - p[0]) * t;
    let mut cuts = vec![0.0, 1.0];
    let mut crossing = |a: f32, b: f32, level: f32| {
        if (a - level) * (b - level) < 0.0 {
            cuts.push((level - a) / (b - a));
        }
    };
    for level in [0.0, 1.0] {
        crossing(top[0], top[1], level);
        crossing(below[0], below[1], level);
    }
    let gap = [top[0] - below[0], top[1] - below[1]];
    crossing(gap[0], gap[1], 0.0);
    cuts.sort_by(f32::total_cmp);
    let mut pieces = Vec::new();
    for w in cuts.windows(2) {
        let (t0, t1) = (w[0], w[1]);
        if t1 - t0 < 1e-6 {
            continue;
        }
        let lo = |t: f32| at(below, t).clamp(0.0, 1.0);
        let hi = |t: f32| at(top, t).clamp(0.0, 1.0);
        let mid = (t0 + t1) * 0.5;
        if hi(mid) - lo(mid) <= EPS {
            continue;
        }
        let mut polygon = vec![[t0, lo(t0)], [t1, lo(t1)]];
        if hi(t1) - lo(t1) > EPS {
            polygon.push([t1, hi(t1)]);
        }
        if hi(t0) - lo(t0) > EPS {
            polygon.push([t0, hi(t0)]);
        }
        pieces.push(polygon);
    }
    pieces
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Clips a 3D polygon to `y >= level` (`above`) or `y <= level`.
fn clip_y(polygon: &[[f32; 3]], level: f32, above: bool) -> Vec<[f32; 3]> {
    let side = |p: [f32; 3]| if above { p[1] - level } else { level - p[1] };
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for (i, &p) in polygon.iter().enumerate() {
        let q = polygon[(i + 1) % polygon.len()];
        let (dp, dq) = (side(p), side(q));
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dq >= 0.0) {
            let s = dp / (dp - dq);
            out.push(std::array::from_fn(|k| p[k] + (q[k] - p[k]) * s));
        }
    }
    if out.len() < 3 { Vec::new() } else { out }
}

/// Clips a `(t, y)` face polygon to the side of the linear profile `line`
/// (heights at t = 0 and t = 1) above it or below it, keeping its winding.
pub fn clip_band(polygon: &[[f32; 2]], line: [f32; 2], above: bool) -> Vec<[f32; 2]> {
    let side = |p: [f32; 2]| {
        let d = p[1] - (line[0] + (line[1] - line[0]) * p[0]);
        if above { d } else { -d }
    };
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for (i, &p) in polygon.iter().enumerate() {
        let q = polygon[(i + 1) % polygon.len()];
        let (dp, dq) = (side(p), side(q));
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dq >= 0.0) {
            let s = dp / (dp - dq);
            out.push([p[0] + (q[0] - p[0]) * s, p[1] + (q[1] - p[1]) * s]);
        }
    }
    if out.len() < 3 { Vec::new() } else { out }
}

impl CellShape {
    /// Height at top corner `(cx, cz)` in 0..=1 coordinates.
    pub fn corner(&self, cx: i32, cz: i32) -> f32 {
        let index = CORNERS.iter().position(|&c| c == (cx, cz)).expect("cell corner");
        self.corners[index]
    }

    /// Height at the cell's horizontal center, where plants root.
    pub fn center(&self) -> f32 {
        self.height(0.5, 0.5)
    }

    /// How much the mesher blends facet normals toward the cell's mean slope.
    pub fn smoothing(&self) -> f32 {
        match (self.material, self.variant) {
            (Material::Rock, _) | (_, Variant::Rocky) => 0.0,
            (_, Variant::Terraced) => 0.15,
            _ => 0.6,
        }
    }

    /// This cell's part of the column surface, in local coordinates: the 18
    /// grid triangles (each quad split along its (i, j)-(i + 1, j + 1)
    /// diagonal, wound like the mesher's +Y face) clipped to this cell's slab.
    /// A surface exactly on a layer plane belongs to the cell above it, so
    /// bank cells below the column top stop just short of their ceiling.
    pub fn top_triangles(&self) -> Vec<[[f32; 3]; 3]> {
        let point = |i: usize, j: usize| [i as f32 / 3.0, self.grid[i][j], j as f32 / 3.0];
        let mut triangles = Vec::with_capacity(18);
        for i in 0..GRID - 1 {
            for j in 0..GRID - 1 {
                let (a, b, c, d) = (point(i, j), point(i + 1, j), point(i + 1, j + 1), point(i, j + 1));
                for triangle in [[a, b, c], [a, c, d]] {
                    let ceiling = if self.kind == ShapeKind::Bank { 1.0 - 1e-4 } else { 1.0 };
                    let clipped = clip_y(&clip_y(&triangle, 0.0, true), ceiling, false);
                    for k in 1..clipped.len().saturating_sub(1) {
                        let t = [clipped[0], clipped[k], clipped[k + 1]];
                        let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
                        let area = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                        if area.iter().map(|a| a * a).sum::<f32>() > 1e-12 {
                            triangles.push(t); // skip clipping slivers
                        }
                    }
                }
            }
        }
        triangles
    }

    /// Solid height at local `(u, v)` (x, z) within this cell, 0..=1.
    pub fn height(&self, u: f32, v: f32) -> f32 {
        self.surface(u, v).clamp(0.0, 1.0)
    }

    /// Column surface height relative to this cell's floor, unclamped.
    pub fn surface(&self, u: f32, v: f32) -> f32 {
        let (u, v) = (u.clamp(0.0, 1.0) * 3.0, v.clamp(0.0, 1.0) * 3.0);
        let (i, j) = ((u as usize).min(2), (v as usize).min(2));
        let (fu, fv) = (u - i as f32, v - j as f32);
        let g = &self.grid;
        if fu >= fv {
            g[i][j] + fu * (g[i + 1][j] - g[i][j]) + fv * (g[i + 1][j + 1] - g[i + 1][j])
        } else {
            g[i][j] + fv * (g[i][j + 1] - g[i][j]) + fu * (g[i + 1][j + 1] - g[i][j + 1])
        }
    }

    /// Exact maximum of `height` over a local rectangle (clamped to the cell).
    /// The surface is linear on each grid triangle, so the maximum lies on a
    /// rectangle corner, a grid vertex inside it, or where the rectangle's
    /// edges cross grid lines or quad diagonals.
    pub fn max_height_in(&self, u0: f32, u1: f32, v0: f32, v1: f32) -> f32 {
        let (u0, u1) = (u0.clamp(0.0, 1.0), u1.clamp(0.0, 1.0));
        let (v0, v1) = (v0.clamp(0.0, 1.0), v1.clamp(0.0, 1.0));
        let mut best = f32::NEG_INFINITY;
        let mut consider = |u: f32, v: f32| {
            if (u0..=u1).contains(&u) && (v0..=v1).contains(&v) {
                best = best.max(self.surface(u, v));
            }
        };
        for (u, v) in [(u0, v0), (u1, v0), (u0, v1), (u1, v1)] {
            consider(u, v);
        }
        for k in 0..GRID {
            let line = k as f32 / 3.0;
            for other in 0..GRID {
                consider(line, other as f32 / 3.0);
            }
            for v in [v0, v1] {
                consider(line, v);
            }
            for u in [u0, u1] {
                consider(u, line);
            }
        }
        // Quad diagonals lie on lines u - v = d.
        for d in -2..=2 {
            let d = d as f32 / 3.0;
            for v in [v0, v1] {
                consider(v + d, v);
            }
            for u in [u0, u1] {
                consider(u, u - d);
            }
        }
        best.clamp(0.0, 1.0)
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
            assert_eq!((s.kind, s.corners), (ShapeKind::Ramp, corners), "{x},{z}");
            if s.variant == Variant::Smooth {
                // A plain 45-degree ramp: height rises linearly across the cell.
                assert!((s.height(0.5, 0.5) - 0.5).abs() < 1e-6);
            }
        }
        let corner = shape(5, 5);
        assert_eq!((corner.kind, corner.corners), (ShapeKind::ConvexCorner, [0.0, 0.0, 1.0, 0.0]));
        assert!(corner.center() >= 0.3, "convex corners keep substantial volume");
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
        assert_eq!(cap.kind, ShapeKind::Cap);
        assert!(cap.center() >= CAP_RIM);
        assert!(cap.corners.iter().all(|&h| h == CAP_RIM));
        assert_eq!(shape_at(&lone, 3, 2, 6).unwrap().kind, ShapeKind::Cap);

        // Opposite high corners have no ramp direction: block fallback.
        let raised = [(4, 4), (5, 4), (4, 5), (5, 5), (6, 5), (5, 6), (6, 6)];
        let saddle = terrain(|x, z| if raised.contains(&(x, z)) { 3 } else { 2 });
        assert_eq!(shape_at(&saddle, 5, 2, 5), None);

        // Drops taller than BANK_DROP get a restrained shoulder, not a ramp
        // into the missing cells.
        let cliff = terrain(|x, z| if (5..10).contains(&x) && (5..10).contains(&z) { 6 } else { 2 });
        assert_eq!(shape_at(&cliff, 5, 5, 7).unwrap().corners, [SHOULDER, 1.0, 1.0, SHOULDER]);
        assert_eq!(shape_at(&cliff, 5, 4, 7), None, "below a cliff shoulder stays a cube");

        // Rises up to BANK_DROP form one continuous bank, partitioned by layer:
        // the top cell and the cells below it share one surface.
        let bank = terrain(|x, z| if (5..10).contains(&x) && (5..10).contains(&z) { 5 } else { 2 });
        let top = shape_at(&bank, 5, 4, 7).unwrap();
        assert_eq!((top.kind, top.corners), (ShapeKind::Ramp, [-2.0, 1.0, 1.0, -2.0]));
        for y in [2, 3] {
            let cell = shape_at(&bank, 5, y, 7).unwrap();
            assert_eq!(cell.kind, ShapeKind::Bank);
            let lift = (4 - y) as f32;
            assert!((0..4).all(|k| (cell.corners[k] - top.corners[k] - lift).abs() < 1e-6));
        }
        assert_eq!(shape_at(&bank, 5, 1, 7), None, "the ground below the bank stays whole");

        // The same two-cell rise in rock keeps a cliff with a shoulder.
        let mut rock = terrain(|x, z| if (5..10).contains(&x) && (5..10).contains(&z) { 4 } else { 2 });
        for x in 5..10 {
            for z in 5..10 {
                rock.set_block(x, 2, z, BlockType::Stone);
            }
        }
        assert_eq!(shape_at(&rock, 5, 3, 7).unwrap().corners, [SHOULDER, 1.0, 1.0, SHOULDER]);
        assert_eq!(shape_at(&rock, 5, 2, 7), None);

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
                // Swaying decorations (grass tufts) are attachments, not terrain surface.
                if tri.iter().any(|&i| mesh.vertices[i as usize].wind > 0.0) {
                    continue;
                }
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
        assert_watertight_except(world, size, |_| false);
    }

    fn assert_watertight_except(world: &World, size: Vec3, skip: impl Fn(Vec3) -> bool) {
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
                        if skip(p) {
                            c += 0.4173;
                            continue;
                        }
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
    fn bevelled_blocks_stay_watertight_and_inside_their_cells() {
        let mut world = garden_world();
        world.terrain_shaping = false;
        world.bevel_blocks = true;
        // Points inside a chamfer (near two cell faces at once) are cut away
        // on purpose; everything else must still match the logical solid.
        let near = |c: f32| c.fract() < 0.09 || c.fract() > 0.91;
        assert_watertight_except(&world, Vec3::new(32.0, 13.0, 16.0), |p| {
            [p.x, p.y, p.z].iter().filter(|&&c| near(c)).count() >= 2
        });
        // A lone cube: 6 inset faces, 12 strips, 8 corner triangles.
        let mut lone = world_of(&[(0, 0)], [(5, 5, 5, BlockType::Stone)]);
        lone.terrain_shaping = false;
        lone.bevel_blocks = true;
        let mesh = build_chunk_mesh(&lone, &lone.chunks[&(0, 0)]);
        assert_eq!(mesh.vertices.len(), 6 * 4 + 12 * 4 + 8 * 3);
        assert!(mesh.vertices.iter().all(|v| v.position.iter().all(|&c| (5.0..=6.0).contains(&c))));
        lone.bevel_blocks = false;
        assert_eq!(build_chunk_mesh(&lone, &lone.chunks[&(0, 0)]).vertices.len(), 24);
        // On a floor, each vertical bevel ends in an upward cap on the floor
        // plane, closing the notch where the chamfer meets unbevelled ground.
        let mut cells = vec![(5, 1, 5, BlockType::Stone)];
        for x in 3..8 {
            for z in 3..8 {
                cells.push((x, 0, z, BlockType::Stone));
            }
        }
        let mut seated = world_of(&[(0, 0)], cells);
        seated.terrain_shaping = false;
        seated.bevel_blocks = true;
        let mesh = build_chunk_mesh(&seated, &seated.chunks[&(0, 0)]);
        let caps = mesh.indices.chunks(3).filter(|t| {
            let v = t.iter().map(|&i| mesh.vertices[i as usize]).collect::<Vec<_>>();
            let centroid = |k: usize| v.iter().map(|v| v.position[k]).sum::<f32>() / 3.0;
            v.iter().all(|v| v.normal == [0.0, 1.0, 0.0] && v.position[1] == 1.0)
                && centroid(0) > 5.0 && centroid(0) < 6.0 && centroid(2) > 5.0 && centroid(2) < 6.0
        });
        assert_eq!(caps.count(), 4, "one cap under each vertical bevel");
    }

    #[test]
    fn steep_bank_is_one_landform_and_edits_stay_exact() {
        let cells = crate::voxel::coast::steep_bank((0, 4, 0));
        let mut world = world_of(&[(0, 0), (1, 0), (0, 1), (1, 1)], cells);
        assert_watertight(&world, Vec3::new(24.0, 26.0, 20.0));
        // The two-per-column row (z 5..10): the bank surface is one plane
        // through the column tops, shared by the cells stacked under it.
        let banks = (0..24).flat_map(|x| (1..24).map(move |y| (x, y)))
            .filter(|&(x, y)| shape_at(&world, x, y, 7).is_some_and(|s| s.kind == ShapeKind::Bank))
            .count();
        assert!(banks >= 6, "{banks} bank cells");
        // Surface continuity: at a column boundary both sides agree.
        let (x, top) = (8, 4 + 2 * 4 - 1);
        let here = shape_at(&world, x, top, 7).unwrap();
        let next = shape_at(&world, x + 1, top + 2, 7).unwrap();
        assert!((here.surface(1.0, 0.5) - (next.surface(0.0, 0.5) + 2.0)).abs() < 1e-5);
        // Dig a bank cell: exactly one cell changes; refilling restores the mesh.
        let positions = |w: &World| triangles(w).iter().flatten().map(|v| v.to_array()).collect::<Vec<_>>();
        let before = positions(&world);
        let dug = (x, top - 1, 7);
        assert_eq!(shape_at(&world, dug.0, dug.1, dug.2).unwrap().kind, ShapeKind::Bank);
        let original = world.get_block(dug.0, dug.1, dug.2);
        world.set_block(dug.0, dug.1, dug.2, BlockType::Air);
        assert_eq!(world.get_block(dug.0, dug.1, dug.2), BlockType::Air);
        assert_eq!(world.get_block(dug.0, dug.1 + 1, dug.2), BlockType::Grass, "the cell above stays");
        assert_watertight(&world, Vec3::new(24.0, 26.0, 20.0));
        world.set_block(dug.0, dug.1, dug.2, original);
        assert_eq!(positions(&world), before);
    }

        #[test]
    fn grass_cover_follows_landform_steepness_continuously() {
        let world = world_of(&[(0, 0), (1, 0), (0, 1), (1, 1)], crate::voxel::coast::steep_bank((0, 4, 0)));
        let sample = |x, y, z| world.get_block(x, y, z);
        let cache = ShapeCache::new(&sample);
        // Flat ground before the hill, and the middle of the 45-degree row.
        assert_eq!(cache.grass_cover([1.5, 4.0, 2.5]), 1.0);
        assert_eq!(cache.grass_cover([7.5, 7.0, 2.5]), 1.0);
        // The 2:1 bank is earth; the two-column ledge breaking the last row
        // is a real shelf and stays green.
        assert_eq!(cache.grass_cover([7.5, 10.0, 7.5]), 0.0);
        assert!(cache.grass_cover([8.0, 10.0, 17.5]) > 0.5);
        // A single flat cell between two steep rises reads as bank.
        let mut notch = world_of(&[(0, 0)], crate::voxel::coast::steep_bank((0, 4, 0)).into_iter().filter(|c| c.0 < 16 && c.2 < 16));
        notch.set_block(8, 11, 7, BlockType::Air);
        notch.set_block(8, 10, 7, BlockType::Air);
        let sample = |x, y, z| notch.get_block(x, y, z);
        assert!(ShapeCache::new(&sample).grass_cover([8.5, 10.0, 7.5]) < 0.5);
        // Continuous: tiny moves never jump.
        for i in 0..200 {
            let p = [4.0 + i as f32 * 0.07, 9.0, 7.3];
            let q = [p[0] + 0.01, p[1], p[2]];
            assert!((cache.grass_cover(p) - cache.grass_cover(q)).abs() < 0.05);
        }
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
            // Winding matches FACE_VERTS, whose cross product points inward;
            // a sliver lying on a cell boundary may belong to either side.
            let centroid = (t[0] + t[1] + t[2]) / 3.0;
            let owner = [0.01f32, -0.01].iter().map(|d| (centroid + normal.normalize() * *d).floor()).find(|o| {
                world.get_block(o.x as i32, o.y as i32, o.z as i32).is_opaque()
                    && t.iter().all(|v| {
                        let local = *v - *o;
                        local.min_element() > -1e-4 && local.max_element() < 1.0 + 1e-4
                    })
            });
            let Some(owner) = owner else { panic!("triangle {t:?} has no occupied owner containing it") };
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
        assert_eq!(kinds.len(), 5, "{kinds:?}");
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
    fn standing_bodies_rest_on_shaped_surfaces_only() {
        let mut world = terrain(plateau);
        // Grid-standing on the -X rim ramp (top of cell layer 2 is y = 3).
        let ramp = shape_at(&world, 5, 2, 7).unwrap();
        let highest = |points: &[(f32, f32)]| {
            points.iter().map(|&(u, v)| 2.0 + ramp.height(u, v)).fold(f32::MIN, f32::max)
        };
        let on_ramp = rest_height(&world, Vec3::new(5.5, 3.0, 7.5), 0.2);
        let expected = highest(&[(0.5, 0.5), (0.3, 0.3), (0.7, 0.3), (0.3, 0.7), (0.7, 0.7)]);
        assert!((on_ramp - expected).abs() < 1e-5 && on_ramp < 3.0, "{on_ramp}");
        // Stepped up while overlapping the ramp, center still over low ground.
        let overlapping = rest_height(&world, Vec3::new(4.9, 3.0, 7.5), 0.2);
        assert!((overlapping - highest(&[(0.1, 0.3), (0.1, 0.7)]).max(2.0)).abs() < 1e-5, "{overlapping}");
        assert_eq!(rest_height(&world, Vec3::new(7.5, 3.0, 7.5), 0.2), 3.0, "flat plateau");
        assert_eq!(rest_height(&world, Vec3::new(2.5, 2.0, 7.5), 0.2), 2.0, "flat ground");
        assert_eq!(rest_height(&world, Vec3::new(5.5, 3.4, 7.5), 0.2), 3.4, "airborne");
        world.terrain_shaping = false;
        assert_eq!(rest_height(&world, Vec3::new(5.5, 3.0, 7.5), 0.2), 3.0);
    }

    /// A broad gentle field of ramps, so every variant appears many times.
    fn ramp_field() -> (World, Vec<CellShape>) {
        let world = terrain(|x, z| 2 + (x >= 8) as i32 + (z >= 8) as i32 * 0);
        let ramps = (0..16).filter_map(|z| shape_at(&world, 8, 2, z)).filter(|s| s.kind == ShapeKind::Ramp);
        let mut shapes: Vec<_> = ramps.collect();
        // More ramp rows: plateaus at several x positions across a second world.
        let wide = terrain(|x, _| 1 + x / 3);
        for x in 0..16 {
            for z in 1..15 {
                for y in 0..8 {
                    shapes.extend(shape_at(&wide, x, y, z).filter(|s| s.kind == ShapeKind::Ramp));
                }
            }
        }
        (world, shapes)
    }

    #[test]
    fn slope_variants_differ_inside_but_share_the_rim() {
        let (_, ramps) = ramp_field();
        let seen: std::collections::BTreeSet<_> = ramps.iter().map(|s| format!("{:?}", s.variant)).collect();
        assert_eq!(seen.len(), 5, "{seen:?}");
        for s in &ramps {
            // The rim stays linear between corners whatever the variant.
            for (i, &(x0, z0)) in CORNERS.iter().enumerate() {
                let (x1, z1) = CORNERS[(i + 1) % 4];
                for t in [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
                    let (u, v) = (x0 as f32 + (x1 - x0) as f32 * t, z0 as f32 + (z1 - z0) as f32 * t);
                    let linear = s.corners[i] + (s.corners[(i + 1) % 4] - s.corners[i]) * t;
                    assert!((s.height(u, v) - linear).abs() < 1e-5, "{:?} at {u},{v}", s.variant);
                }
            }
            assert!(s.grid.iter().flatten().all(|h| (0.0..=1.0).contains(h)));
            // Along-slope profile through the cell center row (ramp rising in +x).
            if s.corners == [0.0, 1.0, 1.0, 0.0] {
                let profile = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0].map(|u| s.height(u, 1.0 / 3.0));
                let rises = [profile[1] - profile[0], profile[2] - profile[1], profile[3] - profile[2]];
                match s.variant {
                    Variant::Smooth => assert!(rises.iter().all(|r| (r - 1.0 / 3.0).abs() < 1e-5)),
                    Variant::Rounded => assert!(rises[0] > rises[2], "bulges: steep low, gentle high"),
                    Variant::Hollow => assert!(rises[0] < rises[2], "scooped: gentle low, steep high"),
                    // Non-smooth: a near-flat ledge between two steep risers.
                    Variant::Terraced => assert!(rises[1].abs() < 0.06 && rises[0] > 0.4 && rises[2] > 0.4, "{rises:?}"),
                    Variant::Rocky => {}
                }
            }
        }
        // Deterministic: the same world gives the same variant everywhere.
        let (_, again) = ramp_field();
        assert_eq!(ramps, again);
    }

    #[test]
    fn leaves_vary_per_block_and_shared_corners_move_together() {
        let mut cells = vec![];
        for x in 2..10 {
            for z in 2..10 {
                cells.push((x, 5, z, BlockType::OakLeaves));
            }
        }
        let mut world = world_of(&[(0, 0)], cells);
        let mesh = build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
        // Every vertex is its lattice corner's shared placement, so faces
        // meeting at a corner never crack apart.
        let sample = |x, y, z| world.get_block(x, y, z);
        let mut tints = std::collections::BTreeSet::new();
        let mut moved = 0;
        for v in &mesh.vertices {
            let lattice = v.position.map(|c| c.round() as i32);
            let (expected, _) = crate::voxel::natural::leaf_corner(&sample, lattice);
            assert!((0..3).all(|k| (v.position[k] - expected[k]).abs() < 1e-4), "{:?} vs {expected:?}", v.position);
            moved += v.position.iter().any(|c| c.fract().abs() > 0.01) as usize;
            tints.insert(format!("{:.3}", v.color[0] / v.color[1]));
        }
        assert!(moved > 0);
        assert!(tints.len() >= 4, "{tints:?}");
        // Cubic mode leaves stay on the grid.
        world.terrain_shaping = false;
        let mesh = build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
        assert!(mesh.vertices.iter().all(|v| v.position.iter().all(|c| c.fract() == 0.0)));
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
        for (shaping, bevels) in [(false, false), (false, true), (true, false)] {
            world.terrain_shaping = shaping;
            world.bevel_blocks = bevels;
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
            println!("shaping={shaping} bevels={bevels}: {per_chunk:.2} ms/chunk, {tris} triangles, {shaped} shaped cells over {} chunks", keys.len());
        }
    }
}
