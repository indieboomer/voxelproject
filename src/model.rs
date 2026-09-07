//! Minimal glTF/GLB loader for the game's creature models (see `models/`).
//!
//! Each model was exported from Blockbench as a GLB with: no skinning (every
//! moving part is its own node, animated by keyframing the node's own
//! translation/rotation/scale -- a "rigid rig" rather than a skinned mesh),
//! no images (materials are flat `baseColorFactor` colors, rendered through
//! the same solid-white texel every other flat-shaded engine vertex uses),
//! and LINEAR-only sampler interpolation. This loader only supports that
//! shape -- it is not a general-purpose glTF importer.
//!
//! Kept dependency-free (hand-parsed against the already-present
//! `serde_json`, mirroring `creature.rs`'s own `SimpleRng` -- this project
//! avoids adding a crate for something this self-contained) rather than
//! pulling in the `gltf` crate.

use std::collections::HashMap;

use glam::{Mat3, Mat4, Quat, Vec3};
use serde_json::Value;

use crate::creature::CreatureKind;
use crate::voxel::atlas::white_uv;
use crate::voxel::mesher::Vertex;

struct Primitive {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    indices: Vec<u32>,
    color: [f32; 3],
}

struct ModelNode {
    children: Vec<usize>,
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
    mesh: Vec<Primitive>,
}

struct Channel<T> {
    times: Vec<f32>,
    values: Vec<T>,
}

#[derive(Default)]
struct NodeAnim {
    translation: Option<Channel<Vec3>>,
    rotation: Option<Channel<Quat>>,
    scale: Option<Channel<Vec3>>,
}

struct AnimationClip {
    /// Largest keyframe time across every channel -- playback loops at this
    /// point (see `push_model`'s `clip_time.rem_euclid(duration)`).
    duration: f32,
    nodes: HashMap<usize, NodeAnim>,
}

/// One loaded creature model: a node hierarchy (translated into world space
/// each frame by `push_model`) plus its named animation clips ("idle",
/// "walk", ...). Not every model has every clip -- see each `.glb`'s own
/// export (chicken/sheep: idle+walk; stone_golem/stinger: idle+walk+attack;
/// wolf/goblin: idle+walk+run+attack; cow: idle+walk+run).
pub struct AnimatedModel {
    nodes: Vec<ModelNode>,
    roots: Vec<usize>,
    animations: HashMap<String, AnimationClip>,
}

/// The seven creature models, loaded once at startup and shared by every
/// spawned creature of that kind (see `App::new`).
pub struct Models {
    sheep: AnimatedModel,
    chicken: AnimatedModel,
    stone_golem: AnimatedModel,
    wolf: AnimatedModel,
    stinger: AnimatedModel,
    cow: AnimatedModel,
    goblin: AnimatedModel,
}

impl Models {
    pub fn load() -> Self {
        Self {
            sheep: load_glb(include_bytes!("../models/sheep.glb")),
            chicken: load_glb(include_bytes!("../models/chicken.glb")),
            stone_golem: load_glb(include_bytes!("../models/stone_golem.glb")),
            wolf: load_glb(include_bytes!("../models/wolf.glb")),
            stinger: load_glb(include_bytes!("../models/stinger.glb")),
            cow: load_glb(include_bytes!("../models/cow.glb")),
            goblin: load_glb(include_bytes!("../models/goblin.glb")),
        }
    }

    pub fn for_kind(&self, kind: CreatureKind) -> &AnimatedModel {
        match kind {
            CreatureKind::Sheep => &self.sheep,
            CreatureKind::Chicken => &self.chicken,
            CreatureKind::StoneGolem => &self.stone_golem,
            CreatureKind::Wolf => &self.wolf,
            CreatureKind::Stinger => &self.stinger,
            CreatureKind::Cow => &self.cow,
            CreatureKind::Goblin => &self.goblin,
        }
    }
}

// ---------------------------------------------------------------------------
// GLB container + accessor parsing
// ---------------------------------------------------------------------------

/// Splits a `.glb`'s two chunks apart. Returns the parsed JSON chunk and a
/// slice over the binary chunk (borrowed straight from `bytes`, which is
/// always a `'static` `include_bytes!` array in practice).
fn parse_glb(bytes: &[u8]) -> (Value, &[u8]) {
    assert_eq!(&bytes[0..4], b"glTF", "not a glb file");
    let total_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut offset = 12usize;
    let mut json_val = None;
    let mut bin: &[u8] = &[];
    while offset < total_len {
        let chunk_len = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let chunk_type = &bytes[offset + 4..offset + 8];
        let data = &bytes[offset + 8..offset + 8 + chunk_len];
        if chunk_type == b"JSON" {
            json_val = Some(serde_json::from_slice(data).expect("invalid glTF JSON chunk"));
        } else if chunk_type == b"BIN\0" {
            bin = data;
        }
        offset += 8 + chunk_len;
    }
    (json_val.expect("glb missing JSON chunk"), bin)
}

fn buffer_view_offset_stride(json: &Value, bv_idx: usize, item_size: usize) -> (usize, usize) {
    let bv = &json["bufferViews"][bv_idx];
    let bv_offset = bv.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let stride = bv
        .get("byteStride")
        .and_then(Value::as_u64)
        .map(|s| s as usize)
        .unwrap_or(item_size);
    (bv_offset, stride)
}

/// Reads a FLOAT accessor (componentType 5126) with `components` floats per
/// item (1 for SCALAR, 3 for VEC3, 4 for VEC4) into a flat `Vec<f32>`.
fn accessor_floats(json: &Value, bin: &[u8], accessor_idx: usize, components: usize) -> Vec<f32> {
    let acc = &json["accessors"][accessor_idx];
    let count = acc["count"].as_u64().unwrap() as usize;
    let acc_offset = acc.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let bv_idx = acc["bufferView"].as_u64().unwrap() as usize;
    let (bv_offset, stride) = buffer_view_offset_stride(json, bv_idx, components * 4);
    let start = bv_offset + acc_offset;
    let mut out = Vec::with_capacity(count * components);
    for i in 0..count {
        let base = start + i * stride;
        for c in 0..components {
            let o = base + c * 4;
            out.push(f32::from_le_bytes(bin[o..o + 4].try_into().unwrap()));
        }
    }
    out
}

/// Reads an index accessor (unsigned byte/short/int) into `u32`s.
fn accessor_indices(json: &Value, bin: &[u8], accessor_idx: usize) -> Vec<u32> {
    let acc = &json["accessors"][accessor_idx];
    let count = acc["count"].as_u64().unwrap() as usize;
    let component_type = acc["componentType"].as_u64().unwrap();
    let acc_offset = acc.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let bv_idx = acc["bufferView"].as_u64().unwrap() as usize;
    let item_size = match component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        other => panic!("unsupported index component type {other}"),
    };
    let (bv_offset, stride) = buffer_view_offset_stride(json, bv_idx, item_size);
    let start = bv_offset + acc_offset;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = start + i * stride;
        let v = match component_type {
            5121 => bin[base] as u32,
            5123 => u16::from_le_bytes(bin[base..base + 2].try_into().unwrap()) as u32,
            5125 => u32::from_le_bytes(bin[base..base + 4].try_into().unwrap()),
            _ => unreachable!(),
        };
        out.push(v);
    }
    out
}

fn read_vec3(v: Option<&Value>, default: Vec3) -> Vec3 {
    match v.and_then(Value::as_array) {
        Some(a) if a.len() == 3 => Vec3::new(
            a[0].as_f64().unwrap_or(0.0) as f32,
            a[1].as_f64().unwrap_or(0.0) as f32,
            a[2].as_f64().unwrap_or(0.0) as f32,
        ),
        _ => default,
    }
}

fn read_quat(v: Option<&Value>) -> Quat {
    match v.and_then(Value::as_array) {
        Some(a) if a.len() == 4 => Quat::from_xyzw(
            a[0].as_f64().unwrap_or(0.0) as f32,
            a[1].as_f64().unwrap_or(0.0) as f32,
            a[2].as_f64().unwrap_or(0.0) as f32,
            a[3].as_f64().unwrap_or(1.0) as f32,
        ),
        _ => Quat::IDENTITY,
    }
}

fn material_color(json: &Value, idx: Option<usize>) -> [f32; 3] {
    let Some(mat) = idx.and_then(|i| json["materials"].get(i)) else {
        return [1.0, 1.0, 1.0];
    };
    let factor = mat
        .get("pbrMetallicRoughness")
        .and_then(|p| p.get("baseColorFactor"))
        .and_then(Value::as_array);
    match factor {
        Some(a) if a.len() >= 3 => [
            a[0].as_f64().unwrap_or(1.0) as f32,
            a[1].as_f64().unwrap_or(1.0) as f32,
            a[2].as_f64().unwrap_or(1.0) as f32,
        ],
        _ => [1.0, 1.0, 1.0],
    }
}

fn load_glb(bytes: &[u8]) -> AnimatedModel {
    let (json, bin) = parse_glb(bytes);
    let empty = Vec::new();
    let nodes_json = json["nodes"].as_array().unwrap_or(&empty);
    let meshes_json = json["meshes"].as_array().cloned().unwrap_or_default();

    let mut nodes = Vec::with_capacity(nodes_json.len());
    for n in nodes_json {
        let translation = read_vec3(n.get("translation"), Vec3::ZERO);
        let rotation = read_quat(n.get("rotation"));
        let scale = read_vec3(n.get("scale"), Vec3::ONE);
        let children: Vec<usize> = n
            .get("children")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect())
            .unwrap_or_default();

        let mut mesh = Vec::new();
        if let Some(mesh_idx) = n.get("mesh").and_then(Value::as_u64) {
            for prim in meshes_json[mesh_idx as usize]["primitives"]
                .as_array()
                .unwrap_or(&empty)
            {
                let attrs = &prim["attributes"];
                let Some(pos_idx) = attrs.get("POSITION").and_then(Value::as_u64) else {
                    continue;
                };
                let positions: Vec<Vec3> = accessor_floats(&json, bin, pos_idx as usize, 3)
                    .chunks_exact(3)
                    .map(|c| Vec3::new(c[0], c[1], c[2]))
                    .collect();
                let normals: Vec<Vec3> = match attrs.get("NORMAL").and_then(Value::as_u64) {
                    Some(idx) => accessor_floats(&json, bin, idx as usize, 3)
                        .chunks_exact(3)
                        .map(|c| Vec3::new(c[0], c[1], c[2]))
                        .collect(),
                    None => vec![Vec3::Y; positions.len()],
                };
                let mut indices = match prim.get("indices").and_then(Value::as_u64) {
                    Some(idx) => accessor_indices(&json, bin, idx as usize),
                    None => (0..positions.len() as u32).collect(),
                };
                // glTF's front-face winding is counter-clockwise (the spec
                // convention every exporter, Blockbench included, follows),
                // but this engine's main render pipeline uses clockwise as
                // front-face with back-face culling on (see
                // `app.rs`'s `render_pipeline`, matching the voxel mesher's
                // own winding). Loaded as-is, every triangle here would be
                // culled on its visible side and show its inside instead --
                // with these models' many closely-stacked decorative parts,
                // that reads as flicker/z-fighting everywhere, not just
                // "inverted". Flip each triangle once, here, to match.
                for tri in indices.chunks_exact_mut(3) {
                    tri.swap(1, 2);
                }
                let material_idx = prim.get("material").and_then(Value::as_u64).map(|v| v as usize);
                mesh.push(Primitive {
                    positions,
                    normals,
                    indices,
                    color: material_color(&json, material_idx),
                });
            }
        }

        nodes.push(ModelNode {
            children,
            translation,
            rotation,
            scale,
            mesh,
        });
    }

    let mut has_parent = vec![false; nodes.len()];
    for n in &nodes {
        for &c in &n.children {
            has_parent[c] = true;
        }
    }
    let roots: Vec<usize> = (0..nodes.len()).filter(|&i| !has_parent[i]).collect();

    let mut animations = HashMap::new();
    if let Some(anims) = json["animations"].as_array() {
        for (anim_idx, a) in anims.iter().enumerate() {
            let name = a
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("anim_{anim_idx}"));
            let samplers = a["samplers"].as_array().cloned().unwrap_or_default();
            let mut per_node: HashMap<usize, NodeAnim> = HashMap::new();
            let mut duration = 0.0f32;
            for ch in a["channels"].as_array().unwrap_or(&empty) {
                let sampler_idx = ch["sampler"].as_u64().unwrap() as usize;
                let sampler = &samplers[sampler_idx];
                let target = &ch["target"];
                let Some(node_idx) = target.get("node").and_then(Value::as_u64) else {
                    continue;
                };
                let node_idx = node_idx as usize;
                let path = target["path"].as_str().unwrap_or("");
                let input_idx = sampler["input"].as_u64().unwrap() as usize;
                let output_idx = sampler["output"].as_u64().unwrap() as usize;
                let times = accessor_floats(&json, bin, input_idx, 1);
                if let Some(&last) = times.last() {
                    duration = duration.max(last);
                }
                let entry = per_node.entry(node_idx).or_default();
                match path {
                    "translation" => {
                        let values = accessor_floats(&json, bin, output_idx, 3)
                            .chunks_exact(3)
                            .map(|c| Vec3::new(c[0], c[1], c[2]))
                            .collect();
                        entry.translation = Some(Channel { times, values });
                    }
                    "scale" => {
                        let values = accessor_floats(&json, bin, output_idx, 3)
                            .chunks_exact(3)
                            .map(|c| Vec3::new(c[0], c[1], c[2]))
                            .collect();
                        entry.scale = Some(Channel { times, values });
                    }
                    "rotation" => {
                        let values = accessor_floats(&json, bin, output_idx, 4)
                            .chunks_exact(4)
                            .map(|c| Quat::from_xyzw(c[0], c[1], c[2], c[3]))
                            .collect();
                        entry.rotation = Some(Channel { times, values });
                    }
                    // Morph target weights aren't used by any of these rigs.
                    _ => {}
                }
            }
            animations.insert(name, AnimationClip { duration, nodes: per_node });
        }
    }

    AnimatedModel { nodes, roots, animations }
}

// ---------------------------------------------------------------------------
// Sampling + pose mesh construction
// ---------------------------------------------------------------------------

/// Every keyframe track observed in these exports uses LINEAR interpolation
/// (see `model.rs`'s module doc), so that's the only kind implemented here.
fn sample<T: Copy>(times: &[f32], values: &[T], t: f32, lerp: impl Fn(T, T, f32) -> T) -> T {
    debug_assert!(!times.is_empty() && times.len() == values.len());
    if t <= times[0] {
        return values[0];
    }
    let last = times.len() - 1;
    if t >= times[last] {
        return values[last];
    }
    let mut i = 0;
    while i + 1 < times.len() && times[i + 1] < t {
        i += 1;
    }
    let (t0, t1) = (times[i], times[i + 1]);
    let alpha = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
    lerp(values[i], values[i + 1], alpha)
}

fn local_trs(node: &ModelNode, clip: Option<&AnimationClip>, node_idx: usize, t: f32) -> (Vec3, Quat, Vec3) {
    let Some(anim) = clip.and_then(|c| c.nodes.get(&node_idx)) else {
        return (node.translation, node.rotation, node.scale);
    };
    let translation = anim
        .translation
        .as_ref()
        .map(|c| sample(&c.times, &c.values, t, Vec3::lerp))
        .unwrap_or(node.translation);
    let rotation = anim
        .rotation
        .as_ref()
        .map(|c| sample(&c.times, &c.values, t, Quat::slerp))
        .unwrap_or(node.rotation);
    let scale = anim
        .scale
        .as_ref()
        .map(|c| sample(&c.times, &c.values, t, Vec3::lerp))
        .unwrap_or(node.scale);
    (translation, rotation, scale)
}

/// Small per-node outward-normal nudge, scaled by the node's own index in
/// the model's flat node array. These rigs have two sources of essentially
/// zero-gap coincident geometry: thin decorative parts layered flush
/// against a sibling's surface (moss patches, eye glints, cracks, combs,
/// ...), and joints deliberately overlapping their neighbor by design (a
/// forearm modeled to overlap the upper arm slightly, so no gap opens up
/// mid-rotation) -- both leave the depth test with a near-exact tie. Which
/// surface wins such a tie isn't stable frame to frame (tiny floating-point
/// differences from animation or camera movement flip it), which reads as
/// flicker on every such part. Nudging every vertex a hair outward along
/// its own normal, by an amount that differs (but is always identical run
/// to run) per node, breaks that tie deterministically -- a smaller-
/// magnitude coincidence than any real intended gap, but enough to give the
/// depth test a consistent winner every frame.
///
/// Kept deliberately tiny: a version of this tried a much larger step to
/// more decisively clear a few observed multi-millimeter gaps, but the same
/// nudge also applies to *every* node uniformly, including nodes that are
/// meant to sit flush against their neighbor (a genuine touching seam, not
/// a z-fight) -- at that larger magnitude it visibly pulled those seams
/// apart into real gaps, trading one visible defect for a worse one. This
/// only needs to be big enough to break an exact floating-point tie, not to
/// out-run every real-but-small authored gap.
const ANTI_ZFIGHT_NUDGE: f32 = 0.00006;

/// How far a node's vertices get nudged outward along their own normals --
/// see `ANTI_ZFIGHT_NUDGE`. Factored out to a small pure function so its
/// scaling is directly testable without needing to pose a whole model.
fn anti_zfight_nudge_amount(node_idx: usize) -> f32 {
    node_idx as f32 * ANTI_ZFIGHT_NUDGE
}

#[allow(clippy::too_many_arguments)]
fn visit(
    model: &AnimatedModel,
    clip: Option<&AnimationClip>,
    t: f32,
    node_idx: usize,
    parent_world: Mat4,
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    origin: Vec3,
    yaw_rotate: &dyn Fn(f32, f32) -> (f32, f32),
    uv: [f32; 4],
) {
    let node = &model.nodes[node_idx];
    let (translation, rotation, scale) = local_trs(node, clip, node_idx, t);
    let local = Mat4::from_scale_rotation_translation(scale, rotation, translation);
    let world = parent_world * local;
    // Non-uniform per-part scale shows up in a couple of these rigs (e.g.
    // squash/stretch-y bits), so normals need the proper inverse-transpose
    // rather than just the rotation part.
    let normal_mat = Mat3::from_mat4(world).inverse().transpose();
    let nudge_amount = anti_zfight_nudge_amount(node_idx);

    for prim in &node.mesh {
        let base_index = vertices.len() as u32;
        for (i, &local_pos) in prim.positions.iter().enumerate() {
            let local_normal = prim.normals.get(i).copied().unwrap_or(Vec3::Y);
            let world_normal = (normal_mat * local_normal).normalize_or_zero();
            let world_pos = world.transform_point3(local_pos) + world_normal * nudge_amount;
            let (wx, wz) = yaw_rotate(world_pos.x, world_pos.z);
            let (nx, nz) = yaw_rotate(world_normal.x, world_normal.z);
            vertices.push(Vertex {
                position: [origin.x + wx, origin.y + world_pos.y, origin.z + wz],
                color: prim.color,
                normal: [nx, world_normal.y, nz],
                uv: [uv[0], uv[1]],
                ao: 1.0,
                reflectivity: 0.0,
                emission: 0.0,
                wind: 0.0,
            });
        }
        for &idx in &prim.indices {
            indices.push(base_index + idx);
        }
    }

    for &child in &node.children {
        visit(
            model, clip, t, child, world, vertices, indices, origin, yaw_rotate, uv,
        );
    }
}

/// Appends one creature's currently-posed mesh (body parts transformed by
/// `clip_name`'s animation sampled at `clip_time`, then rotated to `facing`
/// and translated to `origin`) to a mesh being built. `origin` is the
/// creature's feet position, matching every model's own rig origin. Falls
/// back to the model's bind pose if `clip_name` doesn't exist on this model
/// (shouldn't happen -- callers only ever request clips the kind is known to
/// have) rather than panicking, since a missing clip is a rendering
/// imperfection, not a reason to crash the game.
pub fn push_model(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    model: &AnimatedModel,
    clip_name: &str,
    clip_time: f32,
    origin: Vec3,
    facing: f32,
) {
    let clip = model.animations.get(clip_name);
    let t = match clip {
        Some(c) if c.duration > 0.0 => clip_time.rem_euclid(c.duration),
        _ => 0.0,
    };

    // Local +Z is "forward"; `facing` rotates it around Y the same way
    // `Camera::forward` does (0 faces +X, increasing turns toward +Z) --
    // matches every model's own rig, which was confirmed to face +Z (nose/
    // beak/muzzle at positive local Z, tail at negative Z) in each `.glb`.
    let (s, c) = facing.sin_cos();
    let yaw_rotate = move |x: f32, z: f32| (x * s + z * c, -x * c + z * s);
    let uv = white_uv();

    for &root in &model.roots {
        visit(
            model,
            clip,
            t,
            root,
            Mat4::IDENTITY,
            vertices,
            indices,
            origin,
            &yaw_rotate,
            uv,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creature_faces_sample_white_away_from_atlas_boundaries() {
        let atlas = image::load_from_memory(crate::voxel::atlas::ATLAS_BYTES)
            .unwrap()
            .to_rgba8();
        let models = Models::load();
        for kind in 0..=6 {
            let model = models.for_kind(CreatureKind::from_u8(kind));
            for clip in model.animations.keys() {
                let mut vertices = Vec::new();
                let mut indices = Vec::new();
                push_model(
                    &mut vertices, &mut indices, model, clip, 0.37,
                    Vec3::new(37.0, 5.0, -12.0), 0.73,
                );
                assert!(!vertices.is_empty());
                for vertex in vertices {
                    // Model nearest sampling on both sides of the UV to
                    // catch a constant coordinate placed on a tile edge.
                    for dx in [-0.25, 0.0, 0.25] {
                        for dy in [-0.25, 0.0, 0.25] {
                            let x = (vertex.uv[0] * atlas.width() as f32 + dx).floor() as u32;
                            let y = (vertex.uv[1] * atlas.height() as f32 + dy).floor() as u32;
                            assert_eq!(
                                atlas.get_pixel(x, y).0, [255, 255, 255, 255],
                                "creature {kind}, clip {clip}: UV {:?} samples outside white swatch",
                                vertex.uv,
                            );
                        }
                    }
                }
            }
        }
    }

    fn extent(vertices: &[Vertex], axis: usize) -> f32 {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for v in vertices {
            lo = lo.min(v.position[axis]);
            hi = hi.max(v.position[axis]);
        }
        hi - lo
    }

    /// The main render pipeline uses `front_face: Cw` with back-face
    /// culling (`app.rs`'s `render_pipeline`), the opposite of glTF's own
    /// CCW convention -- loading indices unflipped culled the visible side
    /// of every triangle and showed the inside instead, which with these
    /// models' many closely-stacked decorative parts read as flicker on
    /// every part of every creature. For a triangle wound correctly for
    /// this engine (CW front-face), the standard right-hand-rule face
    /// normal of its first two edges points *into* the surface, i.e.
    /// opposite its vertex normals -- this pins that down against the real
    /// files so a regression (e.g. someone "fixing" the winding back to
    /// glTF's native order) fails loudly instead of only visually.
    #[test]
    fn triangle_winding_matches_the_engines_clockwise_front_face_convention() {
        let models = Models::load();
        for (name, model) in [
            ("sheep", &models.sheep),
            ("chicken", &models.chicken),
            ("stone_golem", &models.stone_golem),
            ("wolf", &models.wolf),
            ("stinger", &models.stinger),
            ("cow", &models.cow),
            ("goblin", &models.goblin),
        ] {
            let mut checked = 0;
            let mut wrong = 0;
            for node in &model.nodes {
                for prim in &node.mesh {
                    for tri in prim.indices.chunks_exact(3) {
                        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                        let (v0, v1, v2) = (prim.positions[i0], prim.positions[i1], prim.positions[i2]);
                        let face_normal = (v1 - v0).cross(v2 - v0);
                        if face_normal.length_squared() < 1e-10 {
                            continue; // degenerate/near-zero-area triangle
                        }
                        let vertex_normal = (prim.normals[i0] + prim.normals[i1] + prim.normals[i2]) / 3.0;
                        checked += 1;
                        if face_normal.dot(vertex_normal) > 0.0 {
                            wrong += 1;
                        }
                    }
                }
            }
            assert!(checked > 0, "expected {name} to have triangles to check");
            assert_eq!(
                wrong, 0,
                "{name}: {wrong}/{checked} triangles are wound backwards for this engine's clockwise front-face convention"
            );
        }
    }

    /// Every model is imported at full fidelity -- no decorative geometry
    /// (wool locks, feathers, hackles, spines, moss/cracks/hoof splits, ...)
    /// is filtered out. This pins the sheep's posed triangle count to its
    /// full raw file total (1904 -- see `models/sheep.glb`), so a future
    /// reintroduction of any such filtering fails loudly here instead of
    /// only visually.
    #[test]
    fn every_model_renders_at_full_unfiltered_geometry() {
        let models = Models::load();
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        push_model(
            &mut vertices,
            &mut indices,
            &models.sheep,
            "idle",
            0.0,
            Vec3::ZERO,
            0.0,
        );
        let triangle_count = indices.len() / 3;
        assert_eq!(
            triangle_count, 1904,
            "expected the sheep's full raw geometry (including its wool-lock clusters) to render \
             unfiltered; got {triangle_count} triangles"
        );
    }

    /// Every real `.glb` under `models/` should parse without panicking and
    /// yield a non-trivial mesh for each clip the model is documented to
    /// have (see each kind's animation list in `model.rs`'s module doc) --
    /// this is the loader's own hand-parsed byte offsets/strides being
    /// exercised against the actual shipped files, not synthetic data.
    #[test]
    fn every_bundled_model_loads_and_poses_its_documented_clips() {
        let models = Models::load();
        let cases: &[(&AnimatedModel, &[&str])] = &[
            (&models.sheep, &["idle", "walk"]),
            (&models.chicken, &["idle", "walk"]),
            (&models.stone_golem, &["idle", "walk", "attack"]),
            (&models.wolf, &["idle", "walk", "run", "attack"]),
            (&models.stinger, &["idle", "walk", "attack"]),
            (&models.cow, &["idle", "walk", "run"]),
            (&models.goblin, &["idle", "walk", "run", "attack"]),
        ];
        for (model, clips) in cases {
            assert!(
                !model.nodes.is_empty(),
                "expected the loaded model to have at least one node"
            );
            for &clip in *clips {
                let mut vertices = Vec::new();
                let mut indices = Vec::new();
                push_model(
                    &mut vertices,
                    &mut indices,
                    model,
                    clip,
                    0.0,
                    Vec3::ZERO,
                    0.0,
                );
                assert!(
                    !vertices.is_empty() && !indices.is_empty(),
                    "expected clip \"{clip}\" to produce a non-empty mesh"
                );
            }
        }
    }

    /// A missing/unknown clip name must fall back to the bind pose rather
    /// than panicking -- `push_model`'s documented behavior for a clip name
    /// that doesn't exist on this particular model.
    #[test]
    fn an_unknown_clip_name_falls_back_to_the_bind_pose_instead_of_panicking() {
        let models = Models::load();
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        push_model(
            &mut vertices,
            &mut indices,
            &models.sheep,
            "does_not_exist",
            1.23,
            Vec3::ZERO,
            0.0,
        );
        assert!(!vertices.is_empty());
    }

    /// Proves the animation sampler is actually being evaluated over time --
    /// not just always returning the bind pose -- by checking the walk
    /// clip's pose differs between two points in time.
    #[test]
    fn a_walk_clip_pose_actually_changes_over_time() {
        let models = Models::load();
        let mut early = Vec::new();
        let mut early_i = Vec::new();
        push_model(
            &mut early,
            &mut early_i,
            &models.wolf,
            "walk",
            0.0,
            Vec3::ZERO,
            0.0,
        );
        let mut later = Vec::new();
        let mut later_i = Vec::new();
        push_model(
            &mut later,
            &mut later_i,
            &models.wolf,
            "walk",
            0.35,
            Vec3::ZERO,
            0.0,
        );

        assert_eq!(early.len(), later.len(), "same clip should yield the same vertex count");
        let moved = early
            .iter()
            .zip(later.iter())
            .any(|(a, b)| Vec3::from_array(a.position).distance(Vec3::from_array(b.position)) > 1e-4);
        assert!(moved, "expected at least one vertex to move between two different times in the walk cycle");
    }

    /// End-to-end sanity check of the whole node-hierarchy + facing-rotation
    /// pipeline: the wolf model is much longer (nose to tail) than it is
    /// wide, so at facing 0 (forward = world +X, see `push_model`'s doc
    /// comment) its bounding box should be wider along X than along Z, and
    /// after a 90-degree turn that relationship should flip. A sign error
    /// in the yaw rotation or a broken node transform would fail this.
    #[test]
    fn facing_rotates_the_whole_posed_model_not_just_its_root() {
        let models = Models::load();

        let mut facing_plus_x = Vec::new();
        let mut indices = Vec::new();
        push_model(
            &mut facing_plus_x,
            &mut indices,
            &models.wolf,
            "idle",
            0.0,
            Vec3::ZERO,
            0.0,
        );
        let x_extent = extent(&facing_plus_x, 0);
        let z_extent = extent(&facing_plus_x, 2);
        assert!(
            x_extent > z_extent,
            "facing 0 (+X) should read as the wolf's long axis: x_extent={x_extent}, z_extent={z_extent}"
        );

        let mut facing_plus_z = Vec::new();
        indices.clear();
        push_model(
            &mut facing_plus_z,
            &mut indices,
            &models.wolf,
            "idle",
            0.0,
            Vec3::ZERO,
            std::f32::consts::FRAC_PI_2,
        );
        let x_extent_2 = extent(&facing_plus_z, 0);
        let z_extent_2 = extent(&facing_plus_z, 2);
        assert!(
            z_extent_2 > x_extent_2,
            "after a 90-degree turn the long axis should have swapped to Z: x_extent={x_extent_2}, z_extent={z_extent_2}"
        );
    }

    /// The posed mesh should sit at (and around) `origin`, not at the
    /// model's own local-space bind coordinates -- a forgotten `+= origin`
    /// would leave every creature rendering at the world's actual (0,0,0)
    /// regardless of its real position.
    #[test]
    fn origin_translates_the_posed_model() {
        let models = Models::load();
        let origin = Vec3::new(37.0, 5.0, -12.0);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        push_model(
            &mut vertices,
            &mut indices,
            &models.sheep,
            "idle",
            0.0,
            origin,
            0.0,
        );
        let centroid = vertices
            .iter()
            .fold(Vec3::ZERO, |acc, v| acc + Vec3::from_array(v.position))
            / vertices.len() as f32;
        assert!(
            centroid.distance(origin) < 5.0,
            "expected the posed sheep's centroid to be near its origin {origin}, got {centroid}"
        );
    }

    /// Checks `push_model`'s actual generated output (not just the source
    /// `.glb` data) for two concrete z-fight signatures: (a) any pair of
    /// triangles whose 3 vertex positions (rounded to 0.1mm) exactly
    /// coincide -- true duplicate geometry, which should never happen --
    /// and (b) any pair of *different* triangles whose centroids land
    /// within 1mm of each other with nearly-parallel face normals -- close
    /// enough that `ANTI_ZFIGHT_NUDGE` should have already pushed them
    /// apart. A model can legitimately have two *distinct* decorative parts
    /// a few millimeters apart by design (several are, e.g. a cluster of
    /// moss patches) -- that's not what this checks for; the 1mm bar is
    /// well under any such intentional gap found in these files.
    #[test]
    fn posed_output_has_no_duplicate_or_truly_coincident_triangles() {
        use std::collections::HashMap;

        let models = Models::load();
        let cases: &[(&str, &AnimatedModel, &str, f32)] = &[
            ("sheep", &models.sheep, "idle", 0.0),
            ("sheep", &models.sheep, "walk", 0.3),
            ("wolf", &models.wolf, "idle", 0.0),
            ("wolf", &models.wolf, "walk", 0.3),
            ("wolf", &models.wolf, "run", 0.3),
            ("stone_golem", &models.stone_golem, "idle", 0.0),
            ("stinger", &models.stinger, "idle", 0.0),
            ("cow", &models.cow, "idle", 0.0),
            ("goblin", &models.goblin, "idle", 0.0),
            ("chicken", &models.chicken, "idle", 0.0),
        ];

        for (name, model, clip, time) in cases {
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            push_model(&mut vertices, &mut indices, model, clip, *time, Vec3::ZERO, 0.0);

            let mut seen: HashMap<[[i32; 3]; 3], usize> = HashMap::new();
            for tri in indices.chunks_exact(3) {
                let mut key = [[0i32; 3]; 3];
                for (k, &vi) in tri.iter().enumerate() {
                    let p = vertices[vi as usize].position;
                    key[k] = [
                        (p[0] * 10000.0).round() as i32,
                        (p[1] * 10000.0).round() as i32,
                        (p[2] * 10000.0).round() as i32,
                    ];
                }
                key.sort();
                *seen.entry(key).or_insert(0) += 1;
            }
            let exact_dupes: usize = seen.values().filter(|&&c| c > 1).map(|&c| c - 1).sum();
            assert_eq!(
                exact_dupes, 0,
                "[{name}/{clip}] found {exact_dupes} exactly-duplicate triangles in the posed output"
            );

            let mut centroids = Vec::with_capacity(indices.len() / 3);
            let mut normals = Vec::with_capacity(indices.len() / 3);
            for tri in indices.chunks_exact(3) {
                let p0 = Vec3::from_array(vertices[tri[0] as usize].position);
                let p1 = Vec3::from_array(vertices[tri[1] as usize].position);
                let p2 = Vec3::from_array(vertices[tri[2] as usize].position);
                centroids.push((p0 + p1 + p2) / 3.0);
                normals.push((p1 - p0).cross(p2 - p0).normalize_or_zero());
            }
            let mut sub_mm_pairs = 0;
            for i in 0..centroids.len() {
                for j in (i + 1)..centroids.len() {
                    if centroids[i].distance(centroids[j]) < 0.001 && normals[i].dot(normals[j]).abs() > 0.95 {
                        sub_mm_pairs += 1;
                    }
                }
            }
            assert_eq!(
                sub_mm_pairs, 0,
                "[{name}/{clip}] found {sub_mm_pairs} sub-millimeter near-parallel triangle pairs -- \
                 ANTI_ZFIGHT_NUDGE should have separated these"
            );
        }
    }

    /// The reported "z-buffer fight" flicker's actual cause was frame-to-
    /// frame instability: whichever of two near/exactly-coincident surfaces
    /// (several of these rigs layer decorative parts flush against a
    /// sibling's surface -- moss patches, eye glints, cracks, ...) wins the
    /// depth test isn't stable unless a static pose produces the exact same
    /// vertex positions every single call. This pins `push_model` down as a
    /// pure function of its inputs -- byte-identical output for identical
    /// arguments, not just "visually the same" -- which the `ANTI_ZFIGHT_NUDGE`
    /// tie-break in `visit` depends on to actually fix anything.
    #[test]
    fn posed_mesh_is_bit_for_bit_deterministic_across_repeated_calls() {
        let models = Models::load();
        for model in [&models.sheep, &models.wolf, &models.stone_golem, &models.goblin] {
            let mut a_vertices = Vec::new();
            let mut a_indices = Vec::new();
            push_model(
                &mut a_vertices,
                &mut a_indices,
                model,
                "idle",
                1.7,
                Vec3::new(3.0, 4.0, 5.0),
                0.9,
            );
            let mut b_vertices = Vec::new();
            let mut b_indices = Vec::new();
            push_model(
                &mut b_vertices,
                &mut b_indices,
                model,
                "idle",
                1.7,
                Vec3::new(3.0, 4.0, 5.0),
                0.9,
            );

            assert_eq!(a_indices, b_indices);
            assert_eq!(a_vertices.len(), b_vertices.len());
            for (va, vb) in a_vertices.iter().zip(b_vertices.iter()) {
                assert_eq!(va.position, vb.position);
                assert_eq!(va.normal, vb.normal);
            }
        }
    }

    /// `anti_zfight_nudge_amount` is the whole mechanism behind the tie-
    /// break: zero for the root (so nothing shifts for no reason), strictly
    /// increasing with node index (so no two distinct nodes ever tie), and
    /// small enough at realistic node counts (rigs here top out around 100
    /// nodes) to be visually imperceptible -- a regression that zeroed it
    /// out, made it non-monotonic, or let it grow large would either bring
    /// the flicker back or visibly separate geometry that should look
    /// seamless (a larger version of this nudge did exactly that -- see
    /// `ANTI_ZFIGHT_NUDGE`'s doc comment).
    #[test]
    fn anti_zfight_nudge_amount_is_zero_at_root_and_grows_small_and_monotonically() {
        assert_eq!(anti_zfight_nudge_amount(0), 0.0);
        let mut previous = anti_zfight_nudge_amount(0);
        for idx in 1..=150 {
            let nudge = anti_zfight_nudge_amount(idx);
            assert!(
                nudge > previous,
                "expected a strictly larger nudge at node index {idx} than the previous node"
            );
            previous = nudge;
        }
        assert!(
            anti_zfight_nudge_amount(150) < 0.05,
            "expected even a very deep node's nudge to stay visually imperceptible"
        );
    }
}
