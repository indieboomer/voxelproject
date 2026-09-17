use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use super::atlas;
use super::block::BlockType;
use super::chunk::{Chunk, CHUNK_X, CHUNK_Z};
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

/// Appends a double-sided billboard cross for `only_on_top`+`cross` blocks
/// (short grass) at world position `(wx, ly, wz)`. Never culled, never
/// AO-darkened -- it's a thin decoration, not part of the solid cube grid.
/// Both this quad's triangle windings are emitted (rather than relying on
/// getting a single winding's handedness right against the pipeline's
/// front-face convention), so each plane renders from both sides off one
/// shared normal -- an intentional simplification for a paper-thin card.
/// Top corners get `wind = 1.0` (bottom corners `0.0`), which the vertex
/// shader uses to sway just the top of the card in the wind.
#[allow(clippy::too_many_arguments)]
fn push_cross(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    wx: i32,
    ly: i32,
    wz: i32,
    uv_rect: [f32; 4],
    emission: f32,
) {
    for plane in CROSS_PLANES.iter() {
        let normal = normalize3(cross3(sub3(plane[1], plane[0]), sub3(plane[3], plane[0])));
        let base_index = vertices.len() as u32;
        for (corner_idx, corner) in plane.iter().enumerate() {
            let [uc, vc] = FACE_UV_CORNERS[corner_idx];
            vertices.push(Vertex {
                position: [
                    wx as f32 + corner[0],
                    ly as f32 + corner[1],
                    wz as f32 + corner[2],
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
                wind: corner[1],
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
                    push_cross(
                        &mut vertices,
                        &mut indices,
                        wx,
                        ly,
                        wz,
                        uv_rect,
                        def.emission,
                    );
                    continue;
                }

                for (face_idx, normal) in FACE_NORMALS.iter().enumerate() {
                    let nx = wx + normal[0];
                    let ny = ly + normal[1];
                    let nz = wz + normal[2];
                    let neighbor = sample(nx, ny, nz);

                    let visible = if def.opacity < 1.0 {
                        neighbor == BlockType::Air
                    } else {
                        !neighbor.is_opaque() && neighbor != block
                    };
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
                    } else if def.cutout {
                        0.5
                    } else {
                        0.0
                    };
                    let base_index = vertices.len() as u32;

                    for (corner_idx, corner) in FACE_VERTS[face_idx].iter().enumerate() {
                        let (s1, s2, c) = AO_OFFSETS[face_idx][corner_idx];
                        let ao = if atlas::is_cutout(block) {
                            // Leaves shouldn't darken their own edges from
                            // neighboring leaves -- looks muddy fast.
                            1.0
                        } else {
                            ao_brightness(
                                sample(wx + s1.0, ly + s1.1, wz + s1.2).is_solid(),
                                sample(wx + s2.0, ly + s2.1, wz + s2.2).is_solid(),
                                sample(wx + c.0, ly + c.1, wz + c.2).is_solid(),
                            )
                        };
                        let [uc, vc] = FACE_UV_CORNERS[corner_idx];
                        vertices.push(Vertex {
                            position: [
                                wx as f32 + corner[0],
                                ly as f32 + corner[1],
                                wz as f32 + corner[2],
                            ],
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
                            tex_layer: 0.0,
                            glimmer: block.glimmer(),
                            skylight: 1.0,
                            wet: [crate::wetness::absorption(block), -1.],
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
