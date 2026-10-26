use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use super::atlas;
use super::block::BlockType;
use super::chunk::{Chunk, CHUNK_X, CHUNK_Z};
use super::natural::{self, Palette};
use super::shaping::{self, CellShape, Material};
use super::world::World;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Contact ambient occlusion, independent of skylight.
    pub ao: f32,
    /// Base reflectivity (0..1), plus 2 when the face is exposed to rain.
    /// Packing the flag here keeps the vertex layout/bandwidth unchanged.
    pub reflectivity: f32,
    /// Self-illumination strength, 0..1 -- see `BlockType::emission`. A
    /// purely visual glow on the block's own surface, not a light source
    /// that affects neighboring geometry.
    pub emission: f32,
    /// Wind sway strength, 0..1 -- only nonzero for cross-billboard
    /// decorations (see `push_cross`), where the top corners get 1.0 and
    /// the bottom corners 0.0 so the vertex shader bends just the top.
    pub wind: f32,
    /// Which texture a fragment samples from: 0.0 means the shared terrain
    /// `atlas_texture` (every voxel face, plus a creature's flat-colored
    /// rigid parts); a skinned creature's real per-pixel texture instead
    /// sets this to `CreatureKind::to_u8() + 1.0`, selecting that layer of
    /// the `creature_texture` array (see `model.rs`'s `emit_skinned_mesh`
    /// and `shader.wgsl`'s `fs_main`).
    pub tex_layer: f32,
    /// Resource sparkle strength, independent of water reflectivity.
    pub glimmer: f32,
    pub skylight: f32,
    /// Absorption and moisture. Negative moisture uses exposed terrain weather.
    pub wet: [f32; 2],
}

impl Vertex {
    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem::size_of;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, wet) as u64,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, skylight) as u64,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 16]>() as wgpu::BufferAddress,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 6]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 9]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 11]>() as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 13]>() as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 14]>() as wgpu::BufferAddress,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 15]>() as wgpu::BufferAddress,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn extend(&mut self, other: MeshData) {
        let offset = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + offset));
    }
}

// Face order: +X, -X, +Y, -Y, +Z, -Z
pub(crate) const FACE_NORMALS: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// Vertex offsets (relative to block min corner) for each face, wound CCW
/// when viewed from outside the block along the face normal.
pub(crate) const FACE_VERTS: [[[f32; 3]; 4]; 6] = [
    // +X
    [
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ],
    // -X
    [
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
    ],
    // +Y
    [
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ],
    // -Y
    [
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
    ],
    // +Z
    [
        [1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 1.0],
    ],
    // -Z
    [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ],
];

/// UV corners matching `FACE_VERTS`' winding, mapped into whatever tile
/// rect the caller picks.
const FACE_UV_CORNERS: [[f32; 2]; 4] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

/// Per-face, per-corner ambient occlusion sample offsets: (side1, side2,
/// corner), each a world-space offset from the block's own position.
/// Derived by hand from `FACE_VERTS`: for each corner, the two tangent axes
/// (the ones not equal to the face normal) each contribute a +/-1 offset
/// depending on which side of the face that corner is on, and every sample
/// also steps one block out along the face normal, since AO looks at
/// what's beside the neighboring (exposed) voxel, not the block itself.
/// See the classic "ambient occlusion for minecraft-like worlds" algorithm.
type Offset = (i32, i32, i32);
const AO_OFFSETS: [[(Offset, Offset, Offset); 4]; 6] = [
    // +X
    [
        ((1, -1, 0), (1, 0, -1), (1, -1, -1)),
        ((1, -1, 0), (1, 0, 1), (1, -1, 1)),
        ((1, 1, 0), (1, 0, 1), (1, 1, 1)),
        ((1, 1, 0), (1, 0, -1), (1, 1, -1)),
    ],
    // -X
    [
        ((-1, -1, 0), (-1, 0, 1), (-1, -1, 1)),
        ((-1, -1, 0), (-1, 0, -1), (-1, -1, -1)),
        ((-1, 1, 0), (-1, 0, -1), (-1, 1, -1)),
        ((-1, 1, 0), (-1, 0, 1), (-1, 1, 1)),
    ],
    // +Y
    [
        ((-1, 1, 0), (0, 1, -1), (-1, 1, -1)),
        ((1, 1, 0), (0, 1, -1), (1, 1, -1)),
        ((1, 1, 0), (0, 1, 1), (1, 1, 1)),
        ((-1, 1, 0), (0, 1, 1), (-1, 1, 1)),
    ],
    // -Y
    [
        ((-1, -1, 0), (0, -1, 1), (-1, -1, 1)),
        ((1, -1, 0), (0, -1, 1), (1, -1, 1)),
        ((1, -1, 0), (0, -1, -1), (1, -1, -1)),
        ((-1, -1, 0), (0, -1, -1), (-1, -1, -1)),
    ],
    // +Z
    [
        ((1, 0, 1), (0, -1, 1), (1, -1, 1)),
        ((-1, 0, 1), (0, -1, 1), (-1, -1, 1)),
        ((-1, 0, 1), (0, 1, 1), (-1, 1, 1)),
        ((1, 0, 1), (0, 1, 1), (1, 1, 1)),
    ],
    // -Z
    [
        ((-1, 0, -1), (0, -1, -1), (-1, -1, -1)),
        ((1, 0, -1), (0, -1, -1), (1, -1, -1)),
        ((1, 0, -1), (0, 1, -1), (1, 1, -1)),
        ((-1, 0, -1), (0, 1, -1), (-1, 1, -1)),
    ],
];

/// Two diagonal planes (corner-to-corner across the block) used for
/// `only_on_top`+`cross` decorations like short grass -- the classic
/// "billboard cross" shape, each spanning the full block from one bottom
/// edge to the opposite top edge so the two planes cross through the
/// block's center.
const CROSS_PLANES: [[[f32; 3]; 4]; 2] = [
    [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
    ],
    [
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
    ],
];

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-6 {
        [0.0, 1.0, 0.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}

fn plant_size(seed: u32, block: BlockType, x: i32, y: i32, z: i32) -> f32 {
    // Nominal 0.8-block size leaves space for the +20% specimens.
    0.8 * (0.8 + 0.4 * super::noise::block_rand(x, y, z, seed, 0x51AE_9100 ^ block as u32))
}

/// Appends a double-sided billboard cross for `only_on_top`+`cross` blocks
/// (short grass) at world position `(wx, ly, wz)`. Never culled, never
/// AO-darkened -- it's a thin decoration, not part of the solid cube grid.
/// Both this quad's triangle windings are emitted (rather than relying on
/// getting a single winding's handedness right against the pipeline's
/// front-face convention), so each plane renders from both sides off one
/// shared normal -- an intentional simplification for a paper-thin card.
/// Top corners get bounded wind motion (bottom corners stay anchored).
#[allow(clippy::too_many_arguments)]
fn push_cross(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    wx: i32,
    ly: i32,
    wz: i32,
    uv_rect: [f32; 4],
    emission: f32,
    size: f32,
) {
    // Both material and shadow shaders have a maximum sway of
    // (0.09 + 0.05) * Storm.wind_strength(). Keep even storms in the cell.
    let margin = (1.0 - size) * 0.5;
    let wind = (margin / (0.14 * crate::weather::Weather::Storm.wind_strength())).min(1.0);
    for plane in CROSS_PLANES.iter() {
        let normal = normalize3(cross3(sub3(plane[1], plane[0]), sub3(plane[3], plane[0])));
        let base_index = vertices.len() as u32;
        for (corner_idx, corner) in plane.iter().enumerate() {
            let [uc, vc] = FACE_UV_CORNERS[corner_idx];
            vertices.push(Vertex {
                position: [
                    wx as f32 + margin + corner[0] * size,
                    ly as f32 + corner[1] * size,
                    wz as f32 + margin + corner[2] * size,
                ],
                color: [1.0, 1.0, 1.0],
                normal,
                uv: [
                    uv_rect[0] + uc * (uv_rect[2] - uv_rect[0]),
                    uv_rect[1] + vc * (uv_rect[3] - uv_rect[1]),
                ],
                ao: 1.0,
                reflectivity: 0.0,
                emission,
                wind: corner[1] * wind,
                tex_layer: 0.0,
                glimmer: 0.0,
                skylight: 1.0,
                wet: [0., 0.],
            });
        }
        indices.extend_from_slice(&[
            base_index,
            base_index + 1,
            base_index + 2,
            base_index,
            base_index + 2,
            base_index + 3,
            // Reversed winding of the same quad, so it's visible (and lit)
            // from both sides regardless of which winding this pipeline
            // treats as front-facing.
            base_index,
            base_index + 2,
            base_index + 1,
            base_index,
            base_index + 3,
            base_index + 2,
        ]);
    }
}

pub(crate) fn face_shade(face: usize) -> f32 {
    match face {
        2 => 1.0,  // +Y top
        3 => 0.45, // -Y bottom
        0 | 1 => 0.8,
        _ => 0.7,
    }
}

/// Classic voxel AO: 0 (fully occluded) to 3 (fully lit), mapped to a
/// brightness multiplier. Two solid edge-neighbors always fully occlude a
/// corner regardless of the diagonal, matching real-world light behavior.
fn ao_brightness(side1: bool, side2: bool, corner: bool) -> f32 {
    let occlusion = if side1 && side2 {
        0
    } else {
        3 - (side1 as i32 + side2 as i32 + corner as i32)
    };
    match occlusion {
        3 => 1.0,
        2 => 0.8,
        1 => 0.6,
        _ => 0.45,
    }
}

/// Builds a mesh for one chunk. Uses simple per-face culling against
/// neighboring blocks (queried through `world`, so cross-chunk faces are
/// culled correctly too), textured from the shared atlas, and shaded with
/// both a fixed per-face factor and real per-vertex ambient occlusion from
/// neighboring geometry.
pub fn build_chunk_mesh(world: &World, chunk: &Chunk) -> MeshData {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let (ox, oz) = chunk.world_origin();
    let side_roof: [[f32; 18]; 18] = std::array::from_fn(|x| {
        std::array::from_fn(|z| {
            let wx = ox + x as i32 - 1;
            let wz = oz + z as i32 - 1;
            world
                .chunks
                .get(&super::chunk::world_to_chunk(wx, wz))
                .map_or(super::chunk::CHUNK_Y as f32, |c| {
                    c.roof_height(wx.rem_euclid(16), wz.rem_euclid(16)) as f32
                })
        })
    });
    let sample = |x: i32, y: i32, z: i32| {
        if world.automation.devices.is_empty()
            && x >= ox
            && x < ox + CHUNK_X
            && z >= oz
            && z < oz + CHUNK_Z
        {
            chunk.get_local(x - ox, y, z - oz)
        } else {
            world.get_block(x, y, z)
        }
    };
    let shapes = shaping::ShapeCache::new(&sample);
    // One short column scan per mesh build, never per weather change/frame.
    // A roof, leaves, or water above a face shields it from surface rain.
    let rain_height: [[i32; CHUNK_Z as usize]; CHUNK_X as usize] = std::array::from_fn(|x| {
        std::array::from_fn(|z| {
            (0..chunk.stored_height())
                .rev()
                .find(|&y| {
                    let block = chunk.get_local(x as i32, y, z as i32);
                    block.is_solid() || block == BlockType::Water
                })
                .unwrap_or(-1)
        })
    });

    for lx in 0..CHUNK_X {
        for ly in 0..chunk.stored_height() {
            for lz in 0..CHUNK_Z {
                let block = chunk.get_local(lx, ly, lz);
                if block == BlockType::Air {
                    continue;
                }
                let wx = ox + lx;
                let wz = oz + lz;
                let def = block.def();

                // Living trees: organic tapered trunks and limbs (shaped mode).
                if world.terrain_shaping && block.is_wood() {
                    let living = |p: (i32, i32, i32), b: BlockType| super::organic::is_living(world, b, p);
                    if let Some(tree) = super::organic::cell_with(&sample, &living, wx, ly, wz) {
                        push_tree(&mut vertices, &mut indices, &tree, block, [wx, ly, wz]);
                        continue;
                    }
                }

                if block.is_branch() {
                    let connected = FACE_NORMALS.map(|n| {
                        let neighbor = sample(wx + n[0], ly + n[1], wz + n[2]);
                        neighbor.is_wood() && neighbor.wood_resource() == block.wood_resource()
                    });
                    push_branch(&mut vertices, &mut indices,
                        Vec3::new(wx as f32, ly as f32, wz as f32), connected,
                        atlas::uv_rect(def.tile_side));
                    continue;
                }

                if block == BlockType::Campfire {
                    crate::campfire::base_mesh(
                        &mut vertices,
                        &mut indices,
                        Vec3::new(wx as f32, ly as f32, wz as f32),
                    );
                    continue;
                }

                if def.cross {
                    // Billboard decoration (short grass): not part of the
                    // cube grid, never culled against neighbors.
                    let uv_rect = atlas::uv_rect(atlas::tile_for(block, 2));
                    let first = vertices.len();
                    push_cross(
                        &mut vertices,
                        &mut indices,
                        wx,
                        ly,
                        wz,
                        uv_rect,
                        def.emission,
                        plant_size(world.seed, block, wx, ly, wz),
                    );
                    // A plant on shaped ground roots at the fan apex.
                    if world.terrain_shaping {
                        if let Some(below) = shapes.get(wx, ly - 1, wz) {
                            for v in &mut vertices[first..] {
                                v.position[1] += below.center() - 1.0;
                            }
                        }
                    }
                    continue;
                }

                let shape = if world.terrain_shaping {
                    shapes.get(wx, ly, wz)
                } else {
                    None
                };
                if let Some(shape) = &shape {
                    let rain_exposed = !def.cutout && ly >= rain_height[lx as usize][lz as usize];
                    let reflectivity = block.reflectivity() + if rain_exposed { 2.0 } else { 0.0 };
                    push_shaped_top(
                        &mut vertices,
                        &mut indices,
                        &sample,
                        block,
                        shape,
                        (wx, ly, wz),
                        reflectivity,
                    );
                }
                let natural = if world.terrain_shaping { natural::palette(block) } else { None };
                let bevel = world.bevel_blocks && bevel_eligible(world, block);
                if world.terrain_shaping && block == BlockType::Grass && !sample(wx, ly + 1, wz).is_solid()
                    && sample(wx, ly + 1, wz) != BlockType::Water
                {
                    // No tufts on steep banks (their surface dips below this cell).
                    if shape.as_ref().map_or(true, |s| s.corners.iter().all(|&h| h >= -0.01)) {
                        push_tufts(&mut vertices, &mut indices, shape.as_ref(), (wx, ly, wz));
                    }
                }

                let leaf = (world.terrain_shaping && def.cutout).then(|| LeafVariant::at(wx, ly, wz));
                // Only growing (generated) foliage sways; leaves a player or
                // rule placed are recorded edits and stay completely still.
                let placed = def.cutout && world.edits.get(&(wx, ly, wz)) == Some(&block);
                // Growing crowns in shaped mode: foliage clumps instead of
                // cube faces. Placed leaves keep their static block form.
                if world.terrain_shaping && def.cutout && !placed {
                    push_foliage(&mut vertices, &mut indices, &sample, block, [wx, ly, wz]);
                    continue;
                }
                for (face_idx, normal) in FACE_NORMALS.iter().enumerate() {
                    if shape.is_some() && face_idx == 2 {
                        continue; // replaced by the shaped top fan
                    }
                    let nx = wx + normal[0];
                    let ny = ly + normal[1];
                    let nz = wz + normal[2];
                    let neighbor = sample(nx, ny, nz);

                    let mut visible = if def.opacity < 1.0 {
                        neighbor == BlockType::Air
                    } else {
                        !neighbor.is_opaque() && neighbor != block
                    };
                    // A living trunk no longer fills its cell: show what it touches.
                    if !visible
                        && world.terrain_shaping
                        && neighbor.is_wood()
                        && super::organic::is_living(world, neighbor, (nx, ny, nz))
                    {
                        visible = true;
                    }
                    // With shaping, a side may be only partly covered: draw the
                    // (below, top) band between the neighbor's profile and ours.
                    let mut band = None;
                    if world.terrain_shaping && normal[1] == 0 {
                        let top = shaping::face_edge_heights(shape.as_ref(), face_idx, [0; 3]);
                        if visible {
                            if shape.is_some() || natural.is_some() {
                                band = Some(([0.0; 2], top));
                            }
                        } else if neighbor.is_opaque() {
                            if let Some(other) = shapes.get(nx, ny, nz) {
                                visible = true;
                                // Non-opaque blocks (leaves) draw their whole face.
                                if block.is_opaque() {
                                    let below = shaping::face_edge_heights(Some(&other), face_idx, *normal);
                                    band = Some((below, top));
                                }
                            }
                        }
                    }
                    if !visible {
                        continue;
                    }

                    let shade = 0.85 + 0.15 * face_shade(face_idx);
                    let color = [shade, shade, shade];
                    let uv_rect = atlas::uv_rect(atlas::tile_for(block, face_idx));
                    let rain_exposed = !def.cutout
                        && (normal[1] > 0 && ly >= rain_height[lx as usize][lz as usize]
                            || normal[1] == 0
                                && side_roof[(nx - ox + 1) as usize][(nz - oz + 1) as usize]
                                    <= ly as f32);
                    let reflectivity = block.reflectivity() + if rain_exposed { 2.0 } else { 0.0 };
                    let emission = def.emission;
                    // Leaves aren't anchored to anything solid, so (unlike
                    // short grass) the whole block sways rather than just
                    // its top -- toned down from the grass card's 1.0 since
                    // it's a full cube, not a thin billboard.
                    // Water reuses the otherwise unused wind scalar: negative
                    // values encode current heading without growing vertices.
                    let wind = if block == BlockType::Water
                        && normal[1] > 0
                        && world.generation.shape == crate::worldgen::Shape::Mainland
                    {
                        let flow = super::terrain::current_with_lakes(
                            wx,
                            wz,
                            world.seed,
                            ly > super::world::SEA_LEVEL,
                            world.generation.landscape_version >= 3,
                        );
                        if flow == [0.0; 2] {
                            0.0
                        } else {
                            -(flow[1].atan2(flow[0]) + std::f32::consts::PI + 1.0)
                        }
                    } else if def.cutout && !placed {
                        0.5
                    } else {
                        0.0
                    };
                    let base_index = vertices.len() as u32;
                    let corner_ao: [f32; 4] = std::array::from_fn(|corner_idx| {
                        let (s1, s2, c) = AO_OFFSETS[face_idx][corner_idx];
                        if atlas::is_cutout(block) {
                            // Leaves shouldn't darken their own edges from
                            // neighboring leaves -- looks muddy fast.
                            1.0
                        } else {
                            ao_brightness(
                                ao_occluder(sample(wx + s1.0, ly + s1.1, wz + s1.2)),
                                ao_occluder(sample(wx + s2.0, ly + s2.1, wz + s2.2)),
                                ao_occluder(sample(wx + c.0, ly + c.1, wz + c.2)),
                            )
                        }
                    });

                    if let Some((below, top)) = band {
                        let side = Side {
                            face: face_idx,
                            origin: (wx, ly, wz),
                            corner_ao,
                            color,
                            reflectivity,
                            emission,
                            wind,
                            glimmer: block.glimmer(),
                            absorption: crate::wetness::absorption(block),
                        };
                        match natural {
                            // Earth side under a grass lip that follows the top edge.
                            // Only a near-level shelf edge gets the lip; steep
                            // bank edges stay earth rather than a green stripe.
                            Some(Palette::Grass) => {
                                let shelf = top.iter().all(|&h| (0.4..=1.0001).contains(&h))
                                    && (top[1] - top[0]).abs() <= 1.0;
                                let lip = top.map(|h| if shelf { (h - natural::GRASS_LIP).max(0.0) } else { 2.0 });
                                let soil = atlas::uv_rect(atlas::tile_for(BlockType::Soil, face_idx));
                                let grass = atlas::uv_rect(atlas::tile_for(BlockType::Grass, 2));
                                for band in shaping::exposed_region(below, top) {
                                    let earth = shaping::clip_band(&band, lip, false);
                                    let turf = shaping::clip_band(&band, lip, true);
                                    side.push_polygon(&mut vertices, &mut indices, &earth, top, soil, Some(Palette::Soil));
                                    side.push_polygon(&mut vertices, &mut indices, &turf, top, grass, Some(Palette::Grass));
                                }
                            }
                            Some(Palette::Rock) => {
                                side.push(&mut vertices, &mut indices, below, top, uv_rect, natural, true)
                            }
                            _ => side.push(&mut vertices, &mut indices, below, top, uv_rect, natural, false),
                        }
                        continue;
                    }

                    if bevel && leaf.is_none() {
                        let bevel = Bevel { world, sample: &sample, cell: [wx, ly, wz] };
                        bevel.push_face(&mut vertices, &mut indices, face_idx, uv_rect, corner_ao, Side {
                            face: face_idx,
                            origin: (wx, ly, wz),
                            corner_ao,
                            color,
                            reflectivity,
                            emission,
                            wind,
                            glimmer: block.glimmer(),
                            absorption: crate::wetness::absorption(block),
                        });
                        continue;
                    }

                    for (corner_idx, corner) in FACE_VERTS[face_idx].iter().enumerate() {
                        let ao = corner_ao[corner_idx];
                        let [mut uc, mut vc] = FACE_UV_CORNERS[corner_idx];
                        let mut position = [
                            wx as f32 + corner[0],
                            ly as f32 + corner[1],
                            wz as f32 + corner[2],
                        ];
                        let mut color = color;
                        // Cutout foliage is tagged so ink outlines skip its holes.
                        let mut tex_layer = if def.cutout { LEAF_MARK } else { 0.0 };
                        let mut wet = [crate::wetness::absorption(block), -1.];
                        let lattice = [wx + corner[0] as i32, ly + corner[1] as i32, wz + corner[2] as i32];
                        if let Some(leaf) = &leaf {
                            [uc, vc] = leaf.uv([uc, vc]);
                            let (corner, depth_shade) = natural::leaf_corner(&sample, lattice);
                            position = corner;
                            let albedo = natural::foliage(block, position);
                            color = std::array::from_fn(|k| {
                                color[k] * albedo[k] * depth_shade * (1.0 + (leaf.tint[k] - 1.0) * 0.25)
                            });
                            tex_layer = natural::NATURAL_FOLIAGE;
                        } else if let Some(palette) = natural {
                            let albedo = natural::tint(palette, position);
                            color = std::array::from_fn(|k| color[k] * albedo[k]);
                            tex_layer = natural::NATURAL_TEX;
                        } else if world.terrain_shaping && block == BlockType::Water && face_idx == 2 {
                            // Shallows: depth, shore proximity and seabed color
                            // for the water shader's tint and foam.
                            let (depth, shore, floor) = natural::water_corner(&sample, lattice[0], ly, lattice[2]);
                            color = std::array::from_fn(|k| color[k] * floor[k]);
                            wet = [natural::WATER_DATA + depth, shore];
                        }
                        vertices.push(Vertex {
                            position,
                            color,
                            normal: [normal[0] as f32, normal[1] as f32, normal[2] as f32],
                            uv: [
                                uv_rect[0] + uc * (uv_rect[2] - uv_rect[0]),
                                uv_rect[1] + vc * (uv_rect[3] - uv_rect[1]),
                            ],
                            ao,
                            reflectivity,
                            emission,
                            wind,
                            tex_layer,
                            glimmer: block.glimmer(),
                            skylight: 1.0,
                            wet,
                        });
                    }
                    indices.extend_from_slice(&[
                        base_index,
                        base_index + 1,
                        base_index + 2,
                        base_index,
                        base_index + 2,
                        base_index + 3,
                    ]);
                }
            }
        }
    }

    let mut mesh = MeshData { vertices, indices };
    crate::shelter::Roofs::default().shade(world, &mut mesh);
    mesh
}

fn ao_occluder(block: BlockType) -> bool {
    block.is_solid() && !block.is_branch()
}

/// Brightness for a sloped normal, blending the fixed per-face factors.
fn slope_shade(n: [f32; 3]) -> f32 {
    let side = if n[0].abs() >= n[2].abs() { face_shade(0) } else { face_shade(4) };
    0.85 + 0.15 * (side + (face_shade(2) - side) * n[1].max(0.0))
}

/// Emits a shaped cell's top grid. Smooth soil variants blend facet normals
/// toward the cell's mean slope (restrained smoothing); rock, terraced and
/// rocky variants keep crisp facets. A small per-cell brightness variation
/// hints at the editable unit without drawing seams.
fn push_shaped_top(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    block: BlockType,
    shape: &CellShape,
    (wx, ly, wz): (i32, i32, i32),
    reflectivity: f32,
) {
    let def = block.def();
    let uv_rect = atlas::uv_rect(atlas::tile_for(block, 2));
    // Bank cells lie under their column's top cell: take contact AO at the
    // column's open surface layer rather than from the solid cell above.
    let surface = (ly..ly + 4).find(|&y| !sample(wx, y + 1, wz).is_solid()).unwrap_or(ly);
    let corner_ao: [f32; 4] = std::array::from_fn(|i| {
        let (s1, s2, c) = AO_OFFSETS[2][i];
        ao_brightness(
            ao_occluder(sample(wx + s1.0, surface + s1.1, wz + s1.2)),
            ao_occluder(sample(wx + s2.0, surface + s2.1, wz + s2.2)),
            ao_occluder(sample(wx + c.0, surface + c.1, wz + c.2)),
        )
    });
    // Corner order (0,0), (1,0), (1,1), (0,1), interpolated across the cell.
    let ao_at = |u: f32, v: f32| {
        let low = corner_ao[0] + (corner_ao[1] - corner_ao[0]) * u;
        let high = corner_ao[3] + (corner_ao[2] - corner_ao[3]) * u;
        low + (high - low) * v
    };
    let triangles = shape.top_triangles();
    let facets: Vec<[f32; 3]> =
        triangles.iter().map(|&[a, b, c]| normalize3(cross3(sub3(c, a), sub3(b, a)))).collect();
    let mean = normalize3(facets.iter().fold([0.0; 3], |s, n| [s[0] + n[0], s[1] + n[1], s[2] + n[2]]));
    let smoothing = shape.smoothing();
    let spread = match shape.material {
        Material::Soil => 0.06,
        Material::Rock => 0.10,
    };
    let variation = 1.0
        + spread * (super::noise::block_rand(wx, ly, wz, shaping::SHAPING_VERSION, 0x7E11_0002) - 0.5);
    let natural = natural::palette(block);
    let soil_rect = atlas::uv_rect(atlas::tile_for(BlockType::Soil, 2));
    for (triangle, f) in triangles.iter().zip(&facets) {
        let normal = normalize3(std::array::from_fn(|k| f[k] + (mean[k] - f[k]) * smoothing));
        // Grass grows on shelves and gentle slopes; steep bank faces are earth.
        // Judged mostly by the cell's overall slope so bumpy variants don't speckle.
        let steep = block == BlockType::Grass && f[1] * 0.3 + mean[1] * 0.7 < 0.6;
        let palette = if steep { natural.map(|_| Palette::Soil) } else { natural };
        let uv_rect = if steep { soil_rect } else { uv_rect };
        let shade = slope_shade(normal) * variation;
        let base_index = vertices.len() as u32;
        for p in triangle {
            let position = [wx as f32 + p[0], ly as f32 + p[1], wz as f32 + p[2]];
            let albedo = palette.map_or([1.0; 3], |palette| natural::tint(palette, position));
            vertices.push(Vertex {
                position,
                color: albedo.map(|a| a * shade),
                normal,
                // Same orientation as the cube's +Y face: u = x, v = 1 - z.
                uv: [
                    uv_rect[0] + p[0] * (uv_rect[2] - uv_rect[0]),
                    uv_rect[1] + (1.0 - p[2]) * (uv_rect[3] - uv_rect[1]),
                ],
                ao: ao_at(p[0], p[2]),
                reflectivity,
                emission: def.emission,
                wind: 0.0,
                tex_layer: if palette.is_some() { natural::NATURAL_TEX } else { 0.0 },
                glimmer: block.glimmer(),
                skylight: 1.0,
                wet: [crate::wetness::absorption(block), -1.],
            });
        }
        indices.extend_from_slice(&[base_index, base_index + 1, base_index + 2]);
    }
}

/// Icosahedron directions and faces for foliage clumps.
fn icosahedron() -> ([Vec3; 12], [[usize; 3]; 20]) {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let v = [
        [-1.0, t, 0.0], [1.0, t, 0.0], [-1.0, -t, 0.0], [1.0, -t, 0.0],
        [0.0, -1.0, t], [0.0, 1.0, t], [0.0, -1.0, -t], [0.0, 1.0, -t],
        [t, 0.0, -1.0], [t, 0.0, 1.0], [-t, 0.0, -1.0], [-t, 0.0, 1.0],
    ]
    .map(|p| Vec3::from(p).normalize());
    let f = [
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    (v, f)
}

/// One irregular foliage clump for an exposed living leaf cell (shaped mode).
/// Buried cells emit nothing. Clumps are fuller where the crown is dense and
/// smaller at its sparse edges, which opens gaps onto limbs and sky; color is
/// the crown palette, darker inside, with leaf-texture cutout edges. Picking
/// and collision keep the cell's cube.
fn push_foliage(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    sample: &impl Fn(i32, i32, i32) -> BlockType,
    block: BlockType,
    [x, y, z]: [i32; 3],
) {
    let leafy = |b: BlockType| b.def().cutout || b.is_wood();
    let exposed = FACE_NORMALS.iter().any(|n| !leafy(sample(x + n[0], y + n[1], z + n[2])));
    if !exposed {
        return;
    }
    let mut dense = 0;
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                dense += ((dx, dy, dz) != (0, 0, 0) && sample(x + dx, y + dy, z + dz).def().cutout) as i32;
            }
        }
    }
    let density = dense as f32 / 26.0;
    let r = |salt: u32| super::noise::block_rand(x, y, z, shaping::SHAPING_VERSION, 0xF011_0000 + salt);
    let center = Vec3::new(
        x as f32 + 0.5 + (r(1) - 0.5) * 0.24,
        y as f32 + 0.5 + (r(2) - 0.5) * 0.2,
        z as f32 + 0.5 + (r(3) - 0.5) * 0.24,
    );
    // Sparse edges shrink to small sprigs; dense interiors swell into masses.
    let radius = 0.42 + 0.36 * density.sqrt();
    // Flattened slightly, like a leaf cluster rather than a ball.
    let squash = Vec3::new(1.0 + (r(4) - 0.5) * 0.3, 0.78 + r(5) * 0.18, 1.0 + (r(6) - 0.5) * 0.3);
    let (dirs, faces) = icosahedron();
    let points: Vec<Vec3> = dirs
        .iter()
        .enumerate()
        .map(|(i, d)| center + *d * squash * radius * (0.8 + 0.4 * r(10 + i as u32)))
        .collect();
    let uv_rect = atlas::uv_rect(atlas::tile_for(block, 2));
    let shade = 0.7 + 0.3 * (1.0 - density); // deeper inside the crown
    for face in faces {
        let [a, b, c] = face.map(|i| points[i]);
        let outward = (a + b + c) / 3.0 - center;
        let mut tri = [a, b, c];
        if (b - a).cross(c - a).dot(outward) > 0.0 {
            tri = [a, c, b]; // mesher winding is inward
        }
        let normal = outward.normalize_or_zero();
        // Planar leaf texture across the triangle's dominant plane.
        let (u_axis, v_axis) = if normal.y.abs() > normal.x.abs().max(normal.z.abs()) {
            (0, 2)
        } else if normal.x.abs() > normal.z.abs() {
            (2, 1)
        } else {
            (0, 1)
        };
        let base_index = vertices.len() as u32;
        for p in tri {
            let local = p - Vec3::new(x as f32, y as f32, z as f32);
            let (u, v) = (local[u_axis].rem_euclid(1.0), 1.0 - local[v_axis].rem_euclid(1.0));
            let smooth = ((p - center).normalize_or_zero() * 0.7 + normal * 0.3 + Vec3::Y * 0.15).normalize_or_zero();
            let albedo = natural::foliage(block, p.to_array());
            vertices.push(Vertex {
                position: p.to_array(),
                color: albedo.map(|c| c * shade * (0.9 + 0.2 * smooth.y.max(0.0))),
                normal: smooth.to_array(),
                uv: [uv_rect[0] + u * (uv_rect[2] - uv_rect[0]), uv_rect[1] + v * (uv_rect[3] - uv_rect[1])],
                ao: 1.0,
                reflectivity: 0.0, // matte foliage: no sky sheen on clumps
                emission: 0.0,
                wind: 0.5,
                tex_layer: natural::NATURAL_FOLIAGE,
                glimmer: 0.0,
                skylight: 1.0,
                wet: [crate::wetness::absorption(block), -1.],
            });
        }
        indices.extend_from_slice(&[base_index, base_index + 1, base_index + 2]);
    }
}

/// Emits a living wood cell's tube pieces, knot and end-grain caps with
/// smooth radial normals and bark wrapped once around each ring.
fn push_tree(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, tree: &super::organic::TreeCell, block: BlockType, cell: [i32; 3]) {
    use super::organic::ring_point;
    let wood = block.wood_resource();
    let bark = atlas::uv_rect(atlas::tile_for(wood, 0));
    let grain = atlas::uv_rect(atlas::tile_for(wood, 2));
    let origin = Vec3::new(cell[0] as f32, cell[1] as f32, cell[2] as f32);
    let vertex = |position: Vec3, normal: Vec3, uv: [f32; 2]| Vertex {
        position: position.to_array(),
        color: [0.95; 3],
        normal: normal.to_array(),
        uv,
        ao: 1.0,
        reflectivity: block.reflectivity(),
        emission: 0.0,
        wind: 0.0,
        tex_layer: 0.0,
        glimmer: 0.0,
        skylight: 1.0,
        wet: [crate::wetness::absorption(block), -1.],
    };
    let at = |rect: [f32; 4], u: f32, v: f32| {
        [rect[0] + u.clamp(0.0, 1.0) * (rect[2] - rect[0]), rect[1] + v.clamp(0.0, 1.0) * (rect[3] - rect[1])]
    };
    // Emits a quad/triangle facing `outward` (mesher winding is inward).
    let mut emit = |vertices: &mut Vec<Vertex>, polygon: [u32; 3], outward: Vec3| {
        let [a, b, c] = polygon.map(|i| Vec3::from(vertices[i as usize].position));
        if (b - a).cross(c - a).dot(outward) > 0.0 {
            indices.extend_from_slice(&[polygon[0], polygon[2], polygon[1]]);
        } else {
            indices.extend_from_slice(&polygon);
        }
    };
    for piece in &tree.pieces {
        let n = piece.sides;
        let base = vertices.len() as u32;
        // Pair each ring-a vertex with the ring-b vertex it faces after
        // turning ring a's axis onto ring b's, so angled pieces don't twist.
        let along = piece.b - piece.a;
        let unit = |axis: usize| {
            let mut e = Vec3::ZERO;
            e[axis] = if along[axis] < 0.0 { -1.0 } else { 1.0 };
            e
        };
        let turn = glam::Quat::from_rotation_arc(unit(piece.axis_a), unit(piece.axis_b));
        let order: Vec<usize> = (0..n)
            .map(|k| {
                let (_, out_a) = ring_point(piece.a, piece.axis_a, piece.ra, piece.key_a, k, n);
                let target = turn * out_a;
                (0..n)
                    .max_by(|&i, &j| {
                        let d = |m| ring_point(piece.b, piece.axis_b, piece.rb, piece.key_b, m, n).1.dot(target);
                        d(i).total_cmp(&d(j))
                    })
                    .unwrap()
            })
            .collect();
        for (end, axis, radius, key, is_b) in [(piece.a, piece.axis_a, piece.ra, piece.key_a, false), (piece.b, piece.axis_b, piece.rb, piece.key_b, true)] {
            // Bark runs along the piece; its texture row follows the cell height.
            let v = (end - origin)[axis].clamp(0.0, 1.0);
            for k in 0..=n {
                let index = if is_b { order[k % n] } else { k % n };
                let (p, out) = ring_point(end, axis, radius, key, index, n);
                vertices.push(vertex(p, out, at(bark, k as f32 / n as f32, 1.0 - v)));
            }
        }
        for k in 0..n as u32 {
            let (a0, a1, b0, b1) = (base + k, base + k + 1, base + n as u32 + 1 + k, base + n as u32 + 2 + k);
            let out = Vec3::from(vertices[a0 as usize].normal) + Vec3::from(vertices[b0 as usize].normal);
            emit(vertices, [a0, a1, b1], out);
            emit(vertices, [a0, b1, b0], out);
        }
        if piece.cap_b {
            // End grain on a tip or a fresh cut.
            let normal = along.normalize_or_zero();
            let hub = vertices.len() as u32;
            vertices.push(vertex(piece.b, normal, at(grain, 0.5, 0.5)));
            for k in 0..n {
                let (p, out) = ring_point(piece.b, piece.axis_b, piece.rb, piece.key_b, k, n);
                let uv = at(grain, 0.5 + out.x * 0.45 + out.z * 0.0, 0.5 + out.y * 0.45 + out.z * 0.45);
                vertices.push(vertex(p, normal, uv));
            }
            for k in 0..n as u32 {
                emit(vertices, [hub, hub + 1 + k, hub + 1 + (k + 1) % n as u32], normal);
            }
        }
    }
    if let Some((center, radius)) = tree.knot {
        // A low-poly rounded knot closing elbows and forks.
        let (rings, segments) = (5u32, 8u32);
        let base = vertices.len() as u32;
        for i in 0..=rings {
            let phi = std::f32::consts::PI * i as f32 / rings as f32;
            for j in 0..=segments {
                let theta = std::f32::consts::TAU * j as f32 / segments as f32;
                let out = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
                vertices.push(vertex(center + out * radius, out, at(bark, j as f32 / segments as f32, i as f32 / rings as f32)));
            }
        }
        for i in 0..rings {
            for j in 0..segments {
                let a = base + i * (segments + 1) + j;
                let b = a + segments + 1;
                let out = Vec3::from(vertices[a as usize].normal);
                emit(vertices, [a, b, b + 1], out);
                emit(vertices, [a, b + 1, a + 1], out);
            }
        }
    }
}

/// `Vertex::tex_layer` tag on cutout leaves (samples the atlas like 0.0).
pub(crate) const LEAF_MARK: f32 = 0.05;

/// Chamfer width of `World::bevel_blocks` edges, in cells.
pub(crate) const BEVEL: f32 = 0.07;

/// Plain opaque cubes take bevels; shaped-mode terrain keeps its own forms.
fn bevel_eligible(world: &World, block: BlockType) -> bool {
    block.is_opaque() && !(world.terrain_shaping && shaping::material(block).is_some())
}

/// Bevelled cube faces. A convex edge (both faces exposed, diagonal cell
/// open) is chamfered: each face is inset along it, a strip joins the two
/// insets, corner triangles close three-way corners, and a cap closes the
/// notch where a bevel meets a neighbor that does not continue it. All
/// geometry stays inside the owner cell; picking and collision are unchanged.
struct Bevel<'a, F: Fn(i32, i32, i32) -> BlockType> {
    world: &'a World,
    sample: &'a F,
    cell: [i32; 3],
}

fn face_of(normal: [i32; 3]) -> usize {
    FACE_NORMALS.iter().position(|n| *n == normal).expect("axis normal")
}

impl<F: Fn(i32, i32, i32) -> BlockType> Bevel<'_, F> {
    fn at(&self, c: [i32; 3], d: [i32; 3]) -> BlockType {
        (self.sample)(c[0] + d[0], c[1] + d[1], c[2] + d[2])
    }

    fn convex_at(&self, c: [i32; 3], a: [i32; 3], b: [i32; 3]) -> bool {
        bevel_eligible(self.world, self.at(c, [0; 3]))
            && !self.at(c, a).is_opaque()
            && !self.at(c, b).is_opaque()
            && !self.at(c, [a[0] + b[0], a[1] + b[1], a[2] + b[2]]).is_opaque()
    }

    fn convex(&self, a: [i32; 3], b: [i32; 3]) -> bool {
        self.convex_at(self.cell, a, b)
    }

    /// Outward axis directions at local cube corner `v`, other than `normal`.
    fn sides(v: [f32; 3], normal: [i32; 3]) -> Vec<[i32; 3]> {
        (0..3)
            .filter(|&a| normal[a] == 0)
            .map(|a| {
                let mut d = [0; 3];
                d[a] = if v[a] > 0.5 { 1 } else { -1 };
                d
            })
            .collect()
    }

    /// Corner `v` of the face with `normal`, pulled in along its convex edges.
    fn inset(&self, normal: [i32; 3], v: [f32; 3]) -> [f32; 3] {
        let mut p = v;
        for d in Self::sides(v, normal) {
            if self.convex(normal, d) {
                for a in 0..3 {
                    p[a] -= d[a] as f32 * BEVEL;
                }
            }
        }
        std::array::from_fn(|a| self.cell[a] as f32 + p[a])
    }

    fn corner_ao(&self, face: usize, v: [f32; 3]) -> f32 {
        let index = FACE_VERTS[face].iter().position(|c| *c == v).expect("face corner");
        let (s1, s2, c) = AO_OFFSETS[face][index];
        let occluder = |o: (i32, i32, i32)| ao_occluder(self.at(self.cell, [o.0, o.1, o.2]));
        ao_brightness(occluder(s1), occluder(s2), occluder(c))
    }

    #[allow(clippy::too_many_arguments)]
    fn push_face(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        face: usize,
        uv_rect: [f32; 4],
        corner_ao: [f32; 4],
        side: Side,
    ) {
        let n = FACE_NORMALS[face];
        let corners = FACE_VERTS[face];
        let uv = |k: usize| {
            let [u, v] = FACE_UV_CORNERS[k];
            [uv_rect[0] + u * (uv_rect[2] - uv_rect[0]), uv_rect[1] + v * (uv_rect[3] - uv_rect[1])]
        };
        let vertex = |position: [f32; 3], normal: [f32; 3], shade: f32, uv: [f32; 2], ao: f32| Vertex {
            position,
            color: side.color.map(|c| c * shade),
            normal,
            uv,
            ao,
            reflectivity: side.reflectivity,
            emission: side.emission,
            wind: side.wind,
            tex_layer: 0.0,
            glimmer: side.glimmer,
            skylight: 1.0,
            wet: [side.absorption, -1.],
        };
        // Emits a convex polygon facing `outward` (mesher winding is inward).
        let mut emit = |mut polygon: Vec<Vertex>, outward: [i32; 3]| {
            let [a, b, c] = [polygon[0].position, polygon[1].position, polygon[2].position];
            let cross = cross3(sub3(b, a), sub3(c, a));
            if cross[0] * outward[0] as f32 + cross[1] * outward[1] as f32 + cross[2] * outward[2] as f32 > 0.0 {
                polygon.reverse();
            }
            let base = vertices.len() as u32;
            let count = polygon.len() as u32;
            vertices.extend(polygon);
            for i in 1..count - 1 {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        };
        let normal_of = |d: [i32; 3]| normalize3(d.map(|c| c as f32));
        let add = |a: [i32; 3], b: [i32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let flat = normal_of(n);
        // The inset face itself.
        emit(
            (0..4).map(|k| vertex(self.inset(n, corners[k]), flat, 1.0, uv(k), corner_ao[k])).collect(),
            n,
        );
        for k in 0..4 {
            let (v0, v1) = (corners[k], corners[(k + 1) % 4]);
            // This edge's outward direction: the axis along which both ends agree.
            let mut d = [0; 3];
            for a in 0..3 {
                if n[a] == 0 && (v0[a] - v1[a]).abs() < 0.5 {
                    d[a] = if v0[a] > 0.5 { 1 } else { -1 };
                }
            }
            let other = face_of(d);
            if !self.convex(n, d) || other < face {
                continue;
            }
            // Chamfer strip, lit a touch brighter so edges catch the light.
            let strip_normal = normal_of(add(n, d));
            let shade = 1.06;
            emit(
                vec![
                    vertex(self.inset(n, v0), strip_normal, shade, uv(k), corner_ao[k]),
                    vertex(self.inset(n, v1), strip_normal, shade, uv((k + 1) % 4), corner_ao[(k + 1) % 4]),
                    vertex(self.inset(d, v1), strip_normal, shade, uv((k + 1) % 4), self.corner_ao(other, v1)),
                    vertex(self.inset(d, v0), strip_normal, shade, uv(k), self.corner_ao(other, v0)),
                ],
                add(n, d),
            );
            // Close the strip end against a neighbor whose edge is not bevelled.
            for (v, kk) in [(v0, k), (v1, (k + 1) % 4)] {
                let along = Self::sides(v, n).into_iter().find(|e| *e != d).expect("edge axis");
                let next = add(self.cell, along);
                if self.at(self.cell, along).is_opaque() && !self.convex_at(next, n, d) {
                    let corner = std::array::from_fn(|a| self.cell[a] as f32 + v[a]);
                    let back = along.map(|c| -c);
                    emit(
                        vec![
                            vertex(corner, normal_of(back), 0.9, uv(kk), corner_ao[kk]),
                            vertex(self.inset(n, v), normal_of(back), 0.9, uv(kk), corner_ao[kk]),
                            vertex(self.inset(d, v), normal_of(back), 0.9, uv(kk), corner_ao[kk]),
                        ],
                        back,
                    );
                }
            }
        }
        // Three-way corners, emitted by the lowest-indexed face.
        for (k, &v) in corners.iter().enumerate() {
            let [d1, d2] = <[[i32; 3]; 2]>::try_from(Self::sides(v, n)).expect("two sides");
            if self.convex(n, d1) && self.convex(n, d2) && self.convex(d1, d2)
                && face < face_of(d1) && face < face_of(d2)
            {
                let corner_normal = normal_of(add(add(n, d1), d2));
                emit(
                    vec![
                        vertex(self.inset(n, v), corner_normal, 1.08, uv(k), corner_ao[k]),
                        vertex(self.inset(d1, v), corner_normal, 1.08, uv(k), self.corner_ao(face_of(d1), v)),
                        vertex(self.inset(d2, v), corner_normal, 1.08, uv(k), self.corner_ao(face_of(d2), v)),
                    ],
                    add(add(n, d1), d2),
                );
            }
        }
    }
}

/// Shared attributes of one vertical cube face, emitted as polygon bands.
struct Side {
    face: usize,
    origin: (i32, i32, i32),
    corner_ao: [f32; 4],
    color: [f32; 3],
    reflectivity: f32,
    emission: f32,
    wind: f32,
    glimmer: f32,
    absorption: f32,
}

impl Side {
    /// Emits the band between profiles `below` and `top` (see
    /// `shaping::exposed_region`). With a palette, colors follow the natural
    /// world-space tint; `facet` breaks a full rock face into angular facets
    /// whose interior points sink up to 0.16 into the owner cell.
    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        below: [f32; 2],
        top: [f32; 2],
        uv_rect: [f32; 4],
        palette: Option<Palette>,
        facet: bool,
    ) {
        let (wx, ly, wz) = self.origin;
        let [c0, c1, ..] = FACE_VERTS[self.face];
        let n = FACE_NORMALS[self.face].map(|c| c as f32);
        let point = |t: f32, y: f32| {
            [wx as f32 + c0[0] + (c1[0] - c0[0]) * t, ly as f32 + y, wz as f32 + c0[2] + (c1[2] - c0[2]) * t]
        };
        let full = below.iter().all(|&h| h <= 0.0) && top.iter().all(|&h| h >= 1.0);
        let push = |vertices: &mut Vec<Vertex>, t: f32, y: f32, position: [f32; 3], normal: [f32; 3]| {
            let lerp = |a: f32, b: f32| a + (b - a) * t;
            let (ao_low, ao_high) = (lerp(self.corner_ao[0], self.corner_ao[1]), lerp(self.corner_ao[3], self.corner_ao[2]));
            // The texture's top row follows this cell's own top edge.
            let v = (lerp(top[0], top[1]) - y).clamp(0.0, 1.0);
            let albedo = palette.map_or([1.0; 3], |palette| natural::tint(palette, position));
            vertices.push(Vertex {
                position,
                color: std::array::from_fn(|k| self.color[k] * albedo[k]),
                normal,
                uv: [uv_rect[0] + t * (uv_rect[2] - uv_rect[0]), uv_rect[1] + v * (uv_rect[3] - uv_rect[1])],
                ao: ao_low + (ao_high - ao_low) * y,
                reflectivity: self.reflectivity,
                emission: self.emission,
                wind: self.wind,
                tex_layer: if palette.is_some() { natural::NATURAL_TEX } else { 0.0 },
                glimmer: self.glimmer,
                skylight: 1.0,
                wet: [self.absorption, -1.],
            });
        };
        if facet && full {
            // Angular stone: four flat triangles around an off-center apex,
            // each lit with its own tilted normal. Positions stay on the cube
            // face, so neighbors, picking and collision are unaffected.
            let r = |salt: u32| super::noise::block_rand(wx * 7 + self.face as i32, ly, wz, shaping::SHAPING_VERSION, 0xFAC7 + salt);
            let apex = [0.3 + 0.4 * r(0), 0.3 + 0.4 * r(1)];
            let rim = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            let tangent = sub3(point(1.0, 0.0), point(0.0, 0.0));
            for i in 0..4 {
                let (q, w) = (rim[i], rim[(i + 1) % 4]);
                // Tilt toward this triangle's rim: mostly raised bosses, some hollows.
                let out = [(q[0] + w[0]) * 0.5 - apex[0], (q[1] + w[1]) * 0.5 - apex[1]];
                let tilt = -0.25 + 0.75 * r(2 + i as u32);
                let normal = normalize3(std::array::from_fn(|k| {
                    n[k] + (tangent[k] * out[0] + [0.0, 1.0, 0.0][k] * out[1]) * tilt * 1.4
                }));
                let first = vertices.len() as u32;
                let first_vertex = vertices.len();
                for [t, y] in [apex, q, w] {
                    push(vertices, t, y, point(t, y), normal);
                }
                // Slightly darker rims read as joints between stone blocks.
                for v in &mut vertices[first_vertex + 1..] {
                    v.color = v.color.map(|c| c * 0.86);
                }
                indices.extend_from_slice(&[first, first + 1, first + 2]);
            }
            return;
        }
        for band in shaping::exposed_region(below, top) {
            self.push_polygon(vertices, indices, &band, top, uv_rect, palette);
        }
    }

    /// Emits one flat `(t, y)` polygon of this face as a triangle fan.
    fn push_polygon(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        polygon: &[[f32; 2]],
        top: [f32; 2],
        uv_rect: [f32; 4],
        palette: Option<Palette>,
    ) {
        let (wx, ly, wz) = self.origin;
        let [c0, c1, ..] = FACE_VERTS[self.face];
        let normal = FACE_NORMALS[self.face].map(|c| c as f32);
        let base_index = vertices.len() as u32;
        for &[t, y] in polygon {
            let lerp = |a: f32, b: f32| a + (b - a) * t;
            let position = [wx as f32 + lerp(c0[0], c1[0]), ly as f32 + y, wz as f32 + lerp(c0[2], c1[2])];
            let (ao_low, ao_high) = (lerp(self.corner_ao[0], self.corner_ao[1]), lerp(self.corner_ao[3], self.corner_ao[2]));
            // The texture's top row follows this cell's own top edge.
            let v = (lerp(top[0], top[1]) - y).clamp(0.0, 1.0);
            let albedo = palette.map_or([1.0; 3], |palette| natural::tint(palette, position));
            vertices.push(Vertex {
                position,
                color: std::array::from_fn(|k| self.color[k] * albedo[k]),
                normal,
                uv: [uv_rect[0] + t * (uv_rect[2] - uv_rect[0]), uv_rect[1] + v * (uv_rect[3] - uv_rect[1])],
                ao: ao_low + (ao_high - ao_low) * y,
                reflectivity: self.reflectivity,
                emission: self.emission,
                wind: self.wind,
                tex_layer: if palette.is_some() { natural::NATURAL_TEX } else { 0.0 },
                glimmer: self.glimmer,
                skylight: 1.0,
                wet: [self.absorption, -1.],
            });
        }
        for i in 1..polygon.len().saturating_sub(1) as u32 {
            indices.extend_from_slice(&[base_index, base_index + i, base_index + i + 1]);
        }
    }
}

/// Grass tufts in world-space patches on an exposed grass top. Derived from
/// the cell itself, so digging or covering it removes them with the remesh.
fn push_tufts(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, shape: Option<&CellShape>, (wx, ly, wz): (i32, i32, i32)) {
    let patch = super::noise::fbm(wx as f32 * 0.17, wz as f32 * 0.17, 0x7F75_0001, 2, 2.0, 0.5);
    if patch < 0.5 {
        return;
    }
    let uv = atlas::uv_rect(atlas::tile_for(BlockType::ShortGrass, 2));
    let count = 1 + ((patch - 0.5) * 8.0) as u32;
    for i in 0..count.min(3) {
        let r = |salt: u32| super::noise::block_rand(wx, ly, wz, 0x7F75_0002, salt * 7 + i);
        let (u, v, size) = (0.2 + 0.6 * r(1), 0.2 + 0.6 * r(2), 0.35 + 0.25 * r(3));
        let ground = shape.map_or(1.0, |s| s.height(u, v));
        let first = vertices.len();
        push_cross(vertices, indices, wx, ly + 1, wz, uv, 0.0, size);
        for vertex in &mut vertices[first..] {
            vertex.position[0] += u - 0.5;
            vertex.position[1] += ground - 1.0;
            vertex.position[2] += v - 0.5;
            vertex.color = natural::tint(Palette::Grass, vertex.position).map(|c| c * 1.6);
        }
    }
}

/// Shaped-mode leaf look, picked per block from its world position: texture
/// orientation and a restrained tint, so canopies stop repeating one tile.
pub(crate) struct LeafVariant {
    turns: u8,
    mirror: bool,
    pub tint: [f32; 3],
}

const LEAF_VARIANTS: [(u8, bool, [f32; 3]); 5] = [
    (0, false, [1.0, 1.0, 1.0]),
    (1, false, [0.9, 0.95, 0.88]),   // shaded, deeper green
    (2, true, [1.07, 1.04, 0.9]),    // sun-warmed, yellowish
    (3, false, [0.94, 1.02, 1.02]),  // cool, blue-green
    (1, true, [1.04, 1.07, 0.98]),   // fresh, bright
];

/// Maximum per-axis displacement of a leaf lattice corner.
pub(crate) const LEAF_JITTER: f32 = 0.12;

impl LeafVariant {
    pub(crate) fn at(x: i32, y: i32, z: i32) -> Self {
        let pick = super::noise::block_rand(x, y, z, shaping::SHAPING_VERSION, 0x1EAF_0001);
        let (turns, mirror, tint) = LEAF_VARIANTS[((pick * 5.0) as usize).min(4)];
        Self { turns, mirror, tint }
    }

    /// Rotates by quarter turns, then optionally mirrors, a 0..1 tile UV.
    fn uv(&self, [mut u, mut v]: [f32; 2]) -> [f32; 2] {
        for _ in 0..self.turns {
            [u, v] = [1.0 - v, u];
        }
        if self.mirror {
            u = 1.0 - u;
        }
        [u, v]
    }
}

/// Displacement of a leaf corner, keyed by its world lattice point so every
/// leaf face sharing that corner moves together: a ragged canopy outline
/// without cracks.
pub(crate) fn leaf_offset([x, y, z]: [i32; 3]) -> [f32; 3] {
    std::array::from_fn(|axis| {
        (super::noise::block_rand(x, y, z, shaping::SHAPING_VERSION, 0x1EAF_0010 + axis as u32) * 2.0 - 1.0)
            * LEAF_JITTER
    })
}

/// Straight beams occupy exactly 1 x 0.5 x 0.5, rotated along their neighbors.
/// Non-overlapping center/arm boxes keep bends and rises visually connected.
fn push_branch(vertices: &mut Vec<Vertex>, indices: &mut Vec<u32>, origin: Vec3,
    connected: [bool; 6], uv: [f32; 4]) {
    let axes = [connected[0] || connected[1], connected[2] || connected[3], connected[4] || connected[5]];
    if axes.iter().filter(|&&v| v).count() <= 1 {
        let axis = axes.iter().position(|&v| v).unwrap_or(0);
        let mut min = Vec3::splat(0.25);
        let mut max = Vec3::splat(0.75);
        min[axis] = 0.0; max[axis] = 1.0;
        push_cuboid(vertices, indices, origin+min, origin+max, [1.;3], uv);
        return;
    }
    push_cuboid(vertices, indices, origin+Vec3::splat(0.25), origin+Vec3::splat(0.75), [1.;3], uv);
    for (face, link) in connected.iter().enumerate() {
        if !*link { continue; }
        let axis = face / 2;
        let mut min = Vec3::splat(0.25);
        let mut max = Vec3::splat(0.75);
        if face % 2 == 0 { min[axis] = 0.75; max[axis] = 1.0; }
        else { min[axis] = 0.0; max[axis] = 0.25; }
        push_cuboid(vertices, indices, origin+min, origin+max, [1.;3], uv);
    }
}

/// Appends an axis-aligned box (all 6 faces, no culling, no AO) to a mesh
/// being built. Used for entities: small, standalone, not part of the
/// voxel grid. `uv_rect` is normally the atlas's white swatch so the box
/// reads as flat-colored via `color`, same as before texturing existed.
pub fn push_cuboid(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    min: Vec3,
    max: Vec3,
    color: [f32; 3],
    uv_rect: [f32; 4],
) {
    let size = max - min;
    for (face_idx, normal) in FACE_NORMALS.iter().enumerate() {
        let shade = face_shade(face_idx);
        let c = [color[0] * shade, color[1] * shade, color[2] * shade];
        let base_index = vertices.len() as u32;
        for (corner_idx, corner) in FACE_VERTS[face_idx].iter().enumerate() {
            let [uc, vc] = FACE_UV_CORNERS[corner_idx];
            vertices.push(Vertex {
                position: [
                    min.x + corner[0] * size.x,
                    min.y + corner[1] * size.y,
                    min.z + corner[2] * size.z,
                ],
                color: c,
                normal: [normal[0] as f32, normal[1] as f32, normal[2] as f32],
                uv: [
                    uv_rect[0] + uc * (uv_rect[2] - uv_rect[0]),
                    uv_rect[1] + vc * (uv_rect[3] - uv_rect[1]),
                ],
                ao: 1.0,
                reflectivity: 0.0,
                emission: 0.0,
                wind: 0.0,
                tex_layer: 0.0,
                glimmer: 0.0,
                skylight: 1.0,
                wet: [0.55, 0.],
            });
        }
        indices.extend_from_slice(&[
            base_index,
            base_index + 1,
            base_index + 2,
            base_index,
            base_index + 2,
            base_index + 3,
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plant_variation_is_seeded_rooted_and_stays_inside_the_cell_in_storms() {
        let mut smallest = 1.0f32;
        let mut largest = 0.0f32;
        for block in crate::voxel::COLLECTIBLE_BLOCKS.into_iter().filter(|b| b.def().cross) {
            for x in -32..32 {
                let size = plant_size(42, block, x, 8, -17);
                assert_eq!(size, plant_size(42, block, x, 8, -17));
                assert!((0.64..=0.960001).contains(&size));
                smallest = smallest.min(size); largest = largest.max(size);
                let (mut vertices, mut indices) = (vec![], vec![]);
                push_cross(&mut vertices, &mut indices, 0, 0, 0, [0.,0.,1.,1.], 0., size);
                assert_eq!((vertices.len(), indices.len()), (8,24));
                assert!(vertices.iter().any(|v| v.position[1] == 0.0 && v.wind == 0.0));
                for v in vertices {
                    assert!((0.0..=1.0).contains(&v.position[1]));
                    let max_sway = 0.14 * crate::weather::Weather::Storm.wind_strength() * v.wind;
                    for axis in [0,2] {
                        assert!(v.position[axis] - max_sway >= -0.000001);
                        assert!(v.position[axis] + max_sway <= 1.000001);
                    }
                }
            }
        }
        assert!(smallest < 0.66 && largest > 0.94);
        assert_ne!(plant_size(42, BlockType::Fern, 0, 8, 0), plant_size(43, BlockType::Fern, 0, 8, 0));
    }
    #[test]
    fn branches_have_centered_half_thickness_and_connected_elbows() {
        for axis in 0..3 {
            let mut connected = [false; 6];
            connected[axis*2] = true;
            let (mut vertices, mut indices) = (vec![], vec![]);
            push_branch(&mut vertices, &mut indices, Vec3::ZERO, connected, [0.,0.,1.,1.]);
            assert_eq!(vertices.len(), 24);
            for a in 0..3 {
                let min = vertices.iter().map(|v| v.position[a]).fold(f32::INFINITY, f32::min);
                let max = vertices.iter().map(|v| v.position[a]).fold(f32::NEG_INFINITY, f32::max);
                assert_eq!((min,max), if a == axis { (0.,1.) } else { (0.25,0.75) });
            }
        }
        let (mut vertices, mut indices) = (vec![], vec![]);
        push_branch(&mut vertices, &mut indices, Vec3::ZERO, [true,false,true,false,false,true], [0.,0.,1.,1.]);
        for point in [[1.,0.25,0.25],[0.25,1.,0.25],[0.25,0.25,0.]] {
            assert!(vertices.iter().any(|v| v.position == point));
        }
    }
    use std::collections::HashMap;

    #[test]
    fn shiny_material_reaches_all_faces_without_affecting_matte_or_water() {
        for (block, shiny) in [
            (BlockType::Crystal, true),
            (BlockType::IronOre, true),
            (BlockType::Gold, true),
            (BlockType::Grass, false),
            (BlockType::Water, false),
        ] {
            let mut world = World::new(1);
            let mut chunk = Chunk::new(0, 0);
            chunk.set_local(5, 10, 5, block);
            world.chunks.insert((0, 0), chunk);
            let mesh = build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
            assert_eq!(mesh.vertices.len(), 24);
            assert!(mesh.vertices.iter().all(|v| (v.glimmer > 0.0) == shiny));
        }
        let attribute = Vertex::layout()
            .attributes
            .iter()
            .find(|a| a.shader_location == 9)
            .unwrap();
        assert_eq!(
            attribute.offset as usize,
            std::mem::offset_of!(Vertex, glimmer)
        );
    }

    /// The AO_OFFSETS table was derived by hand from FACE_VERTS -- exactly
    /// the kind of thing that's easy to get subtly wrong (a flipped sign
    /// looks fine at a glance but darkens the wrong corner). This pins down
    /// one concrete, checkable case: a block with a taller neighbor on one
    /// side should have its top face dimmed only at the two corners next to
    /// that neighbor, not the two corners on the far side.
    #[test]
    fn top_face_ao_darkens_only_the_corner_next_to_a_taller_neighbor() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::Stone); // the block under test
        chunk.set_local(6, 11, 5, BlockType::Stone); // steps up beside its top face
        world.chunks.insert((0, 0), chunk);

        let chunk_ref = world.chunks.get(&(0, 0)).unwrap();
        let mesh = build_chunk_mesh(&world, chunk_ref);

        let mut top_face_ao: HashMap<(i32, i32, i32), f32> = HashMap::new();
        for v in &mesh.vertices {
            if v.normal != [0.0, 1.0, 0.0] {
                continue;
            }
            let pos = (
                v.position[0].round() as i32,
                v.position[1].round() as i32,
                v.position[2].round() as i32,
            );
            if pos.0 >= 5 && pos.0 <= 6 && pos.1 == 11 && pos.2 >= 5 && pos.2 <= 6 {
                top_face_ao.insert(pos, v.ao.abs());
            }
        }

        assert_eq!(
            top_face_ao.len(),
            4,
            "expected exactly the 4 corners of one top face, got {top_face_ao:?}"
        );
        assert_eq!(
            top_face_ao[&(5, 11, 5)],
            1.0,
            "far corner should be fully lit"
        );
        assert_eq!(
            top_face_ao[&(5, 11, 6)],
            1.0,
            "far corner should be fully lit"
        );
        assert_eq!(
            top_face_ao[&(6, 11, 5)],
            0.8,
            "corner beside the taller neighbor should be dimmed"
        );
        assert_eq!(
            top_face_ao[&(6, 11, 6)],
            0.8,
            "corner beside the taller neighbor should be dimmed"
        );
    }

    #[test]
    fn side_face_ao_darkens_only_the_corner_diagonally_behind_an_occluder() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::Stone); // the block under test
        chunk.set_local(6, 9, 6, BlockType::Stone); // occludes only one corner's diagonal
        world.chunks.insert((0, 0), chunk);

        let chunk_ref = world.chunks.get(&(0, 0)).unwrap();
        let mesh = build_chunk_mesh(&world, chunk_ref);

        let mut plus_x_ao: HashMap<(i32, i32, i32), f32> = HashMap::new();
        for v in &mesh.vertices {
            if v.normal != [1.0, 0.0, 0.0] {
                continue;
            }
            let pos = (
                v.position[0].round() as i32,
                v.position[1].round() as i32,
                v.position[2].round() as i32,
            );
            if pos.0 == 6 && (pos.1 == 10 || pos.1 == 11) && (pos.2 == 5 || pos.2 == 6) {
                plus_x_ao.insert(pos, v.ao.abs());
            }
        }

        assert_eq!(
            plus_x_ao.len(),
            4,
            "expected exactly the 4 corners of one +X face, got {plus_x_ao:?}"
        );
        assert_eq!(
            plus_x_ao[&(6, 10, 6)],
            0.8,
            "corner diagonally behind the occluder should be dimmed"
        );
        assert_eq!(
            plus_x_ao[&(6, 10, 5)],
            1.0,
            "other corners should be untouched"
        );
        assert_eq!(
            plus_x_ao[&(6, 11, 5)],
            1.0,
            "other corners should be untouched"
        );
        assert_eq!(
            plus_x_ao[&(6, 11, 6)],
            1.0,
            "other corners should be untouched"
        );
    }

    #[test]
    fn leaves_skip_ao_entirely() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::OakLeaves);
        chunk.set_local(6, 11, 5, BlockType::OakLeaves);
        world.chunks.insert((0, 0), chunk);

        let chunk_ref = world.chunks.get(&(0, 0)).unwrap();
        let mesh = build_chunk_mesh(&world, chunk_ref);

        assert!(
            mesh.vertices.iter().all(|v| v.ao.abs() == 1.0),
            "leaves should never be AO-darkened"
        );
    }

    #[test]
    fn rain_exposure_respects_roofs_and_updates_when_removed() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 5, 5, BlockType::Stone);
        chunk.set_local(5, 9, 5, BlockType::Stone);
        world.chunks.insert((0, 0), chunk);
        let mesh = build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
        let top = |y: f32| {
            mesh.vertices
                .iter()
                .filter(move |v| v.normal == [0.0, 1.0, 0.0] && v.position[1] == y)
        };
        assert_eq!(top(6.0).count(), 4);
        assert!(top(6.0).all(|v| v.reflectivity == BlockType::Stone.reflectivity()));
        assert!(top(10.0).all(|v| v.reflectivity == BlockType::Stone.reflectivity() + 2.0));
        assert!(mesh
            .vertices
            .iter()
            .filter(|v| v.normal[1] < 0.0)
            .all(|v| v.reflectivity < 1.0));
        world
            .chunks
            .get_mut(&(0, 0))
            .unwrap()
            .set_local(5, 9, 5, BlockType::Air);
        let mesh = build_chunk_mesh(&world, &world.chunks[&(0, 0)]);
        assert!(mesh
            .vertices
            .iter()
            .filter(|v| v.normal[1] > 0.0)
            .all(|v| v.reflectivity >= 2.0));
    }
    #[test]
    fn vertical_walls_receive_rain_but_overhangs_shelter_them() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        for y in 1..5 {
            chunk.set_local(5, y, 5, BlockType::Stone);
        }
        world.chunks.insert((0, 0), chunk);
        let exposed = |mesh: &MeshData| {
            mesh.vertices
                .iter()
                .filter(|v| v.normal == [1., 0., 0.] && v.position[0] == 6. && v.position[1] < 4.)
                .all(|v| v.reflectivity >= 2.)
        };
        assert!(exposed(&build_chunk_mesh(&world, &world.chunks[&(0, 0)])));
        world.set_block(6, 5, 5, BlockType::Stone);
        assert!(!exposed(&build_chunk_mesh(&world, &world.chunks[&(0, 0)])));
    }
}
