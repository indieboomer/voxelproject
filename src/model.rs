//! glTF/GLB loader for the game's creature models (see `models/`).
//!
//! Two rig shapes are supported, chosen automatically per file based on
//! whether it has a `skin`:
//!
//! - **Rigid** (sheep, chicken): no skinning -- every moving part is its own
//!   node, animated by keyframing that node's own translation/rotation/
//!   scale, with flat `baseColorFactor` materials (no images).
//! - **Skinned** (every other creature): the glTF-standard shape -- one
//!   mesh, one skin, per-vertex `JOINTS_0`/`WEIGHTS_0` blending against an
//!   animated joint hierarchy, with a real `baseColorTexture` sampled at
//!   draw time. Every skinned model's decoded texture is uploaded once (at
//!   startup, see `Models::creature_texture_layers` and `app.rs`'s
//!   `create_atlas_bind_group`) into one shared `texture_2d_array`, one
//!   layer per `CreatureKind`; a posed vertex just carries its real glTF UV
//!   plus a `tex_layer` selecting that layer (see `emit_skinned_mesh`), so
//!   the posed mesh still renders through the same shared `entity_mesh`
//!   buffer/draw call and pipeline every other creature does -- only the
//!   fragment shader's texture lookup branches on `tex_layer`.
//!
//! Both shapes share the same animation-channel/keyframe-sampling code
//! (`local_trs`/`sample`) and the same clip-driven node-hierarchy walk
//! (`compute_world_matrices`) -- a skin's joints are just nodes in that same
//! hierarchy, so a skinned model's joint world matrices come from exactly
//! the code path a rigid model's part world matrices always have.
//!
//! Only LINEAR sampler interpolation, non-negative/relative accessor
//! indices, and (for a skinned model) a single mesh/primitive/skin/material/
//! image are supported -- this loader targets exactly the shape these
//! bundled files use, not general-purpose glTF import.
//!
//! Kept dependency-free: hand-parsed against the already-present
//! `serde_json` (mirroring `creature.rs`'s own `SimpleRng` -- this project
//! avoids adding a crate for something this self-contained) rather than
//! pulling in the `gltf` crate; texture decoding reuses the already-present
//! `image` crate (see `voxel::atlas::ATLAS_BYTES`'s use of it).

use std::collections::HashMap;

use glam::{Mat3, Mat4, Quat, Vec3};
use serde_json::Value;

use crate::creature::CreatureKind;
use crate::voxel::atlas::white_uv;
use crate::voxel::mesher::Vertex;

struct Primitive {
    absorption: f32,
    emission: f32,
    colors: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
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
    /// Rigid-rig geometry attached directly to this node -- empty for every
    /// joint node in a skinned model, and for a skinned model's own mesh
    /// node (that mesh is reached through `AnimatedModel::skin` instead).
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

/// One mesh skinned against a joint hierarchy -- see this module's doc
/// comment. The joint hierarchy itself lives in the owning `AnimatedModel`'s
/// `nodes`; this only holds the skin's own joint list/inverse-bind matrices
/// and its one mesh.
struct Skin {
    /// Node index (into the owning `AnimatedModel::nodes`) for each joint
    /// slot, in the same order as `inverse_bind`. `SkinnedMesh::joint_indices`
    /// indexes into *this* array, not directly into `nodes`.
    joints: Vec<usize>,
    /// One inverse bind matrix per joint, same order as `joints`.
    inverse_bind: Vec<Mat4>,
    mesh: SkinnedMesh,
}

struct SkinnedMesh {
    absorption: f32,
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    /// Up to 4 joint slot indices (into `Skin::joints`) per vertex, as
    /// glTF's `JOINTS_0` attribute stores them.
    joint_indices: Vec<[u16; 4]>,
    /// Blend weights matching `joint_indices`, summing to ~1.0 per vertex.
    joint_weights: Vec<[f32; 4]>,
    /// glTF `TEXCOORD_0`, sampled at draw time against this model's own
    /// layer of the shared `creature_texture` array (see `emit_skinned_mesh`
    /// and `shader.wgsl`) rather than baked into a per-vertex color.
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

/// One loaded creature model: a node hierarchy (translated into world space
/// each frame by `push_model`) plus its named animation clips ("idle",
/// "walk", ...), and -- for every model but sheep/chicken -- a `Skin`.
/// Not every model has every clip; see each `.glb`'s own export.
pub struct AnimatedModel {
    /// Optional texture slot for visual variants, after the player/hat slots.
    texture_layer: Option<f32>,
    hat_socket: Option<usize>,
    right_grip: Option<usize>,
    nodes: Vec<ModelNode>,
    roots: Vec<usize>,
    animations: HashMap<String, AnimationClip>,
    skin: Option<Skin>,
    /// The skin's decoded `baseColorTexture`, if it has one -- `None` for a
    /// rigid model (sheep/chicken). Kept around (rather than only used at
    /// load time) so `Models::creature_texture_layers` can hand every
    /// model's texture to `app.rs` for the shared GPU texture array.
    texture: Option<image::RgbaImage>,
}

/// Creature models and their variants, loaded once at startup and shared by every
/// spawned creature of that kind (see `App::new`).
pub struct Models {
    npcs: [AnimatedModel; 6],
    players: [AnimatedModel; 4],
    hats: [AnimatedModel; 4],
    sheep: AnimatedModel,
    chicken: AnimatedModel,
    stone_golem: AnimatedModel,
    wolf: AnimatedModel,
    stinger: AnimatedModel,
    cow: AnimatedModel,
    goblin: AnimatedModel,
    sunscorch: AnimatedModel,
    zombies: [AnimatedModel; 4],
    skeletons: [AnimatedModel; 4],
    skeleton_sorcerer: [AnimatedModel; 1],
    dragons: [AnimatedModel; 2],
    fish: [AnimatedModel; 1],
}

impl Models {
    pub fn push_npc(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        npc: &crate::npc::Npc,
    ) {
        if let Some(model) = self.npcs.get(npc.kind as usize) {
            push_model(
                vertices,
                indices,
                model,
                CreatureKind::Sheep,
                if npc.walking { "walk" } else { "idle" },
                npc.phase,
                Vec3::from_array(npc.position),
                npc.facing,
            );
        }
    }
    pub fn load() -> Self {
        Self {
            npcs: load_variants(
                [
                    include_bytes!("../models/npc/sage/sage.glb"),
                    include_bytes!("../models/npc/elf_ranger/elf_ranger.glb"),
                    include_bytes!("../models/npc/warrior/warrior.glb"),
                    include_bytes!("../models/npc/merchant/merchant.glb"),
                    include_bytes!("../models/npc/fire_sorceress/fire_sorceress.glb"),
                    include_bytes!("../models/npc/necromancer/necromancer.glb"),
                ],
                29,
            ),
            players: [
                load_glb(include_bytes!("../models/player/player1.glb")),
                load_glb(include_bytes!("../models/player/player2.glb")),
                load_glb(include_bytes!("../models/player/player3.glb")),
                load_glb(include_bytes!("../models/player/player4.glb")),
            ],
            hats: [
                load_glb(include_bytes!("../models/player/hat1.glb")),
                load_glb(include_bytes!("../models/player/hat2.glb")),
                load_glb(include_bytes!("../models/player/hat3.glb")),
                load_glb(include_bytes!("../models/player/hat4.glb")),
            ],
            sheep: load_glb(include_bytes!("../models/sheep.glb")),
            chicken: load_glb(include_bytes!("../models/chicken.glb")),
            stone_golem: load_glb(include_bytes!("../models/stone_golem.glb")),
            wolf: load_glb(include_bytes!("../models/wolf.glb")),
            stinger: load_glb(include_bytes!("../models/stinger.glb")),
            cow: load_glb(include_bytes!("../models/cow.glb")),
            goblin: load_glb(include_bytes!("../models/goblin.glb")),
            sunscorch: load_glb(include_bytes!("../models/sunscorch.glb")),
            zombies: load_variants(
                [
                    include_bytes!("../models/zombie_01.glb"),
                    include_bytes!("../models/zombie_02.glb"),
                    include_bytes!("../models/zombie_03.glb"),
                    include_bytes!("../models/zombie_04.glb"),
                ],
                17,
            ),
            skeletons: load_variants(
                [
                    include_bytes!("../models/skeleton_01.glb"),
                    include_bytes!("../models/skeleton_02.glb"),
                    include_bytes!("../models/skeleton_03.glb"),
                    include_bytes!("../models/skeleton_04.glb"),
                ],
                21,
            ),
            dragons: load_variants(
                [
                    include_bytes!("../models/dragon_green.glb"),
                    include_bytes!("../models/dragon_red.glb"),
                ],
                25,
            ),
            skeleton_sorcerer: load_variants(
                [include_bytes!("../models/skeleton_sorcerer.glb")],
                37,
            ),
            fish: load_variants([include_bytes!("../models/fish.glb")], 27),
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
            CreatureKind::Sunscorch => &self.sunscorch,
            CreatureKind::Zombie => &self.zombies[0],
            CreatureKind::Skeleton => &self.skeletons[0],
            CreatureKind::SkeletonSorcerer => &self.skeleton_sorcerer[0],
            CreatureKind::DragonGreen => &self.dragons[0],
            CreatureKind::DragonRed => &self.dragons[1],
            CreatureKind::Fish => &self.fish[0],
        }
    }

    /// Each kind's decoded `baseColorTexture`, indexed by `CreatureKind::
    /// to_u8()` -- `None` for Sheep/Chicken, which have no real texture
    /// (see this module's doc comment). `app.rs`'s `create_atlas_bind_group`
    /// uploads these once at startup into the `tex_layer`-indexed
    /// `creature_texture` array `emit_skinned_mesh`'s vertices sample from.
    pub fn creature_texture_layers(&self) -> [Option<&image::RgbaImage>; 37] {
        [
            self.sheep.texture.as_ref(),
            self.chicken.texture.as_ref(),
            self.stone_golem.texture.as_ref(),
            self.wolf.texture.as_ref(),
            self.stinger.texture.as_ref(),
            self.cow.texture.as_ref(),
            self.goblin.texture.as_ref(),
            self.sunscorch.texture.as_ref(),
            self.players[0].texture.as_ref(),
            self.players[1].texture.as_ref(),
            self.players[2].texture.as_ref(),
            self.players[3].texture.as_ref(),
            self.hats[0].texture.as_ref(),
            self.hats[1].texture.as_ref(),
            self.hats[2].texture.as_ref(),
            self.hats[3].texture.as_ref(),
            self.zombies[0].texture.as_ref(),
            self.zombies[1].texture.as_ref(),
            self.zombies[2].texture.as_ref(),
            self.zombies[3].texture.as_ref(),
            self.skeletons[0].texture.as_ref(),
            self.skeletons[1].texture.as_ref(),
            self.skeletons[2].texture.as_ref(),
            self.skeletons[3].texture.as_ref(),
            self.dragons[0].texture.as_ref(),
            self.dragons[1].texture.as_ref(),
            self.fish[0].texture.as_ref(),
            chest_model().texture.as_ref(),
            self.npcs[0].texture.as_ref(),
            self.npcs[1].texture.as_ref(),
            self.npcs[2].texture.as_ref(),
            self.npcs[3].texture.as_ref(),
            self.npcs[4].texture.as_ref(),
            self.npcs[5].texture.as_ref(),
            lore_book_model().texture.as_ref(),
            torch_model().texture.as_ref(),
            self.skeleton_sorcerer[0].texture.as_ref(),
        ]
    }

    pub fn for_variant(&self, kind: CreatureKind, variant: u8) -> &AnimatedModel {
        match kind {
            CreatureKind::Zombie => &self.zombies[(variant % 4) as usize],
            CreatureKind::Skeleton => &self.skeletons[(variant % 4) as usize],
            _ => self.for_kind(kind),
        }
    }
}

fn load_variants<const N: usize>(bytes: [&[u8]; N], first_layer: u8) -> [AnimatedModel; N] {
    std::array::from_fn(|i| {
        let mut model = load_glb(bytes[i]);
        model.texture_layer = Some((first_layer as usize + i) as f32);
        model
    })
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
/// item (1 for SCALAR, 2 for VEC2, 3 for VEC3, 4 for VEC4, 16 for MAT4) into
/// a flat `Vec<f32>`.
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

/// Reads a VEC4 accessor of small unsigned integers -- glTF's `JOINTS_0`
/// attribute is always componentType UNSIGNED_BYTE or UNSIGNED_SHORT -- into
/// `[u16; 4]`s.
fn accessor_u16_vec4(json: &Value, bin: &[u8], accessor_idx: usize) -> Vec<[u16; 4]> {
    let acc = &json["accessors"][accessor_idx];
    let count = acc["count"].as_u64().unwrap() as usize;
    let component_type = acc["componentType"].as_u64().unwrap();
    let acc_offset = acc.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let bv_idx = acc["bufferView"].as_u64().unwrap() as usize;
    let item_size = match component_type {
        5121 => 4, // 4 x u8
        5123 => 8, // 4 x u16
        other => panic!("unsupported JOINTS_0 component type {other}"),
    };
    let (bv_offset, stride) = buffer_view_offset_stride(json, bv_idx, item_size);
    let start = bv_offset + acc_offset;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = start + i * stride;
        let mut quad = [0u16; 4];
        for (c, slot) in quad.iter_mut().enumerate() {
            *slot = match component_type {
                5121 => bin[base + c] as u16,
                5123 => u16::from_le_bytes(bin[base + c * 2..base + c * 2 + 2].try_into().unwrap()),
                _ => unreachable!(),
            };
        }
        out.push(quad);
    }
    out
}

/// Reads a MAT4 accessor (componentType FLOAT, as `inverseBindMatrices`
/// always is) into `glam::Mat4`s -- glTF stores matrices column-major, same
/// as `Mat4::from_cols_array`.
fn accessor_mat4s(json: &Value, bin: &[u8], accessor_idx: usize) -> Vec<Mat4> {
    accessor_floats(json, bin, accessor_idx, 16)
        .chunks_exact(16)
        .map(|c| Mat4::from_cols_array(c.try_into().unwrap()))
        .collect()
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

fn material_absorption(json: &Value, idx: Option<usize>) -> f32 {
    let Some(mat) = idx.and_then(|i| json["materials"].get(i)) else {
        return 0.65;
    };
    if let Some(value) = mat["extras"]["wetAbsorption"].as_f64() {
        return (value as f32).clamp(0., 1.);
    }
    let pbr = &mat["pbrMetallicRoughness"];
    let metal = pbr["metallicFactor"].as_f64().unwrap_or(0.) as f32;
    let rough = pbr["roughnessFactor"].as_f64().unwrap_or(0.8) as f32;
    (0.15 + rough.clamp(0., 1.) * 0.65) * (1. - metal.clamp(0., 1.) * 0.9)
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

/// Decodes a material's `baseColorTexture`, if it has one -- every bundled
/// skinned model's image is embedded directly in the GLB's binary chunk via
/// a `bufferView` (no external URIs), so this never touches the filesystem.
fn material_base_color_image(
    json: &Value,
    bin: &[u8],
    material_idx: Option<usize>,
) -> Option<image::RgbaImage> {
    let mat = material_idx.and_then(|i| json["materials"].get(i))?;
    let tex_idx = mat
        .get("pbrMetallicRoughness")?
        .get("baseColorTexture")?
        .get("index")?
        .as_u64()? as usize;
    let image_idx = json["textures"].get(tex_idx)?.get("source")?.as_u64()? as usize;
    let image_json = json["images"].get(image_idx)?;
    let bv_idx = image_json.get("bufferView")?.as_u64()? as usize;
    let bv = &json["bufferViews"][bv_idx];
    let offset = bv.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let length = bv["byteLength"].as_u64().unwrap() as usize;
    Some(
        image::load_from_memory(&bin[offset..offset + length])
            .expect("embedded creature texture should decode")
            .to_rgba8(),
    )
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
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64())
                    .map(|v| v as usize)
                    .collect()
            })
            .unwrap_or_default();

        // A node with a "skin" is a skinned model's one mesh-holding node --
        // its geometry is parsed separately below (into `AnimatedModel::skin`,
        // with proper per-vertex joint blending) rather than as a rigid part
        // attached to this single node.
        let mut mesh = Vec::new();
        if n.get("skin").is_none() {
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
                    // convention every exporter follows), but this engine's
                    // main render pipeline uses clockwise as front-face with
                    // back-face culling on (see `app.rs`'s `render_pipeline`,
                    // matching the voxel mesher's own winding). Flip each
                    // triangle once, here, to match.
                    for tri in indices.chunks_exact_mut(3) {
                        tri.swap(1, 2);
                    }
                    let material_idx = prim
                        .get("material")
                        .and_then(Value::as_u64)
                        .map(|v| v as usize);
                    mesh.push(Primitive {
                        absorption: material_absorption(&json, material_idx),
                        emission: json["materials"][material_idx.unwrap_or(usize::MAX)]
                            ["emissiveFactor"]
                            .as_array()
                            .map_or(0., |v| {
                                v.iter().filter_map(Value::as_f64).fold(0f64, f64::max) as f32
                            }),
                        colors: attrs
                            .get("COLOR_0")
                            .and_then(Value::as_u64)
                            .map(|idx| {
                                let components =
                                    if json["accessors"][idx as usize]["type"] == "VEC4" {
                                        4
                                    } else {
                                        3
                                    };
                                accessor_floats(&json, bin, idx as usize, components)
                                    .chunks_exact(components)
                                    .map(|c| [c[0], c[1], c[2]])
                                    .collect()
                            })
                            .unwrap_or_default(),
                        uvs: attrs
                            .get("TEXCOORD_0")
                            .and_then(Value::as_u64)
                            .map(|idx| {
                                accessor_floats(&json, bin, idx as usize, 2)
                                    .chunks_exact(2)
                                    .map(|uv| [uv[0], uv[1]])
                                    .collect()
                            })
                            .unwrap_or_default(),
                        positions,
                        normals,
                        indices,
                        color: material_color(&json, material_idx),
                    });
                }
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
            animations.insert(
                name,
                AnimationClip {
                    duration,
                    nodes: per_node,
                },
            );
        }
    }

    // A skinned model has exactly one skin, referenced by exactly one node
    // (its mesh-holding node) -- see this module's doc comment.
    let skin_and_texture = json["skins"]
        .as_array()
        .and_then(|skins| skins.first())
        .map(|skin_json| {
            let joints: Vec<usize> = skin_json["joints"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect();
            let inverse_bind = accessor_mat4s(
                &json,
                bin,
                skin_json["inverseBindMatrices"].as_u64().unwrap() as usize,
            );

            let mesh_node = nodes_json
                .iter()
                .find(|n| n.get("skin").and_then(Value::as_u64) == Some(0))
                .expect("a model with a skin should have exactly one node referencing it");
            let mesh_idx = mesh_node["mesh"].as_u64().unwrap() as usize;
            let prim = &meshes_json[mesh_idx]["primitives"][0];
            let attrs = &prim["attributes"];

            let positions: Vec<Vec3> =
                accessor_floats(&json, bin, attrs["POSITION"].as_u64().unwrap() as usize, 3)
                    .chunks_exact(3)
                    .map(|c| Vec3::new(c[0], c[1], c[2]))
                    .collect();
            let normals: Vec<Vec3> =
                accessor_floats(&json, bin, attrs["NORMAL"].as_u64().unwrap() as usize, 3)
                    .chunks_exact(3)
                    .map(|c| Vec3::new(c[0], c[1], c[2]))
                    .collect();
            let uvs: Vec<[f32; 2]> = accessor_floats(
                &json,
                bin,
                attrs["TEXCOORD_0"].as_u64().unwrap() as usize,
                2,
            )
            .chunks_exact(2)
            .map(|c| [c[0], c[1]])
            .collect();
            let joint_indices =
                accessor_u16_vec4(&json, bin, attrs["JOINTS_0"].as_u64().unwrap() as usize);
            let joint_weights: Vec<[f32; 4]> =
                accessor_floats(&json, bin, attrs["WEIGHTS_0"].as_u64().unwrap() as usize, 4)
                    .chunks_exact(4)
                    .map(|c| [c[0], c[1], c[2], c[3]])
                    .collect();

            let mut mesh_indices =
                accessor_indices(&json, bin, prim["indices"].as_u64().unwrap() as usize);
            // Same CCW (glTF-standard) -> CW (this engine) winding flip as the
            // rigid path above.
            for tri in mesh_indices.chunks_exact_mut(3) {
                tri.swap(1, 2);
            }

            let material_idx = prim
                .get("material")
                .and_then(Value::as_u64)
                .map(|v| v as usize);
            let texture = material_base_color_image(&json, bin, material_idx);

            let skin = Skin {
                joints,
                inverse_bind,
                mesh: SkinnedMesh {
                    absorption: material_absorption(&json, material_idx),
                    positions,
                    normals,
                    joint_indices,
                    joint_weights,
                    uvs,
                    indices: mesh_indices,
                },
            };
            (skin, texture)
        });
    let (skin, texture) = match skin_and_texture {
        Some((skin, texture)) => (Some(skin), texture),
        None => (None, None),
    };

    let texture = texture.or_else(|| material_base_color_image(&json, bin, Some(0)));
    let hat_socket = nodes_json.iter().position(|n| n["name"] == "hat_socket");
    let right_grip = nodes_json
        .iter()
        .position(|n| n["name"] == "grip_R")
        .or_else(|| nodes_json.iter().position(|n| n["name"] == "hand_R"));
    AnimatedModel {
        nodes,
        roots,
        animations,
        skin,
        texture,
        hat_socket,
        right_grip,
        texture_layer: None,
    }
}

impl Models {
    /// Player assets are authored in meters, facing +Z, with a named hat socket.
    #[cfg(test)]
    pub fn push_player(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        appearance: crate::remote_player::Appearance,
        origin: Vec3,
        yaw: f32,
        speed: f32,
        time: f32,
    ) {
        use crate::player_animation::Clip;
        self.push_player_animated(
            vertices,
            indices,
            appearance,
            origin,
            yaw,
            if speed > 5.0 {
                Clip::Run
            } else if speed > 0.2 {
                Clip::Walk
            } else {
                Clip::Idle
            },
            time,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_player_animated(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        appearance: crate::remote_player::Appearance,
        origin: Vec3,
        yaw: f32,
        animation: crate::player_animation::Clip,
        time: f32,
        held: Option<crate::equipment::Entry>,
    ) {
        let model_index = usize::from(appearance.model).min(3);
        let model = &self.players[model_index];
        let clip = model.animations.get(animation.name());
        let t = clip.filter(|c| c.duration > 0.0).map_or(0.0, |c| {
            if animation.duration().is_some() {
                time.clamp(0.0, c.duration)
            } else {
                time.rem_euclid(c.duration)
            }
        });
        let matrices = compute_world_matrices(model, clip, t);
        let (s, c) = yaw.sin_cos();
        let rotate = |x: f32, z: f32| (x * s + z * c, -x * c + z * s);
        if let Some((entry, grip)) = held.zip(model.right_grip) {
            // Use the exact same animated pose and yaw as the skinned hand.
            let rotation = Mat3::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
            let socket = matrices[grip];
            let basis = rotation * Mat3::from_mat4(socket);
            let grip_pos = origin + rotation * socket.transform_point3(Vec3::ZERO);
            let scale = 0.55;
            // Mesh handles are authored along +Y with their grip above the origin.
            let grip_height = if matches!(entry, crate::equipment::Entry::Resource(_)) {
                0.48
            } else {
                0.22
            };
            let item = crate::held_item::mesh(
                Some(entry),
                grip_pos - basis * Vec3::Y * grip_height * scale,
                basis,
                scale,
            );
            let base = vertices.len() as u32;
            vertices.extend(item.vertices);
            indices.extend(item.indices.into_iter().map(|i| i + base));
        }
        if let Some(skin) = &model.skin {
            emit_skinned_mesh(
                skin,
                &matrices,
                vertices,
                indices,
                origin,
                &rotate,
                9.0 + model_index as f32,
            );
        }
        if let Some((hat_index, socket)) = appearance.hat.filter(|&h| h < 4).zip(model.hat_socket) {
            let hat = &self.hats[hat_index as usize];
            let hat_matrices: Vec<_> = compute_world_matrices(hat, None, 0.0)
                .into_iter()
                .map(|m| matrices[socket] * m)
                .collect();
            let start = vertices.len();
            emit_rigid_parts(
                hat,
                &hat_matrices,
                vertices,
                indices,
                origin,
                &rotate,
                white_uv(),
            );
            let uvs = hat.nodes.iter().flat_map(|n| &n.mesh).flat_map(|p| &p.uvs);
            for (vertex, uv) in vertices[start..].iter_mut().zip(uvs) {
                vertex.uv = *uv;
                vertex.tex_layer = 13.0 + hat_index as f32;
            }
        }
    }
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

fn local_trs(
    node: &ModelNode,
    clip: Option<&AnimationClip>,
    node_idx: usize,
    t: f32,
) -> (Vec3, Quat, Vec3) {
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
/// the model's flat node array -- only relevant to a *rigid* model
/// (sheep/chicken; a skinned model is one continuous mesh with nothing to
/// nudge apart). Those rigs have two sources of essentially zero-gap
/// coincident geometry: thin decorative parts layered flush against a
/// sibling's surface (moss patches, eye glints, cracks, combs, ...), and
/// joints deliberately overlapping their neighbor by design (a forearm
/// modeled to overlap the upper arm slightly, so no gap opens up mid-
/// rotation) -- both leave the depth test with a near-exact tie. Which
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

/// Computes every node's current world matrix (parent-relative TRS
/// composed all the way from the model's roots down), sampling `clip` at
/// `t` for any node it animates and falling back to that node's bind pose
/// otherwise. Shared by rigid-part emission (a part's own world matrix)
/// and skinned-mesh emission (a joint's world matrix is exactly the same
/// kind of node world matrix -- a skin's joints are just nodes in this same
/// hierarchy).
fn compute_world_matrices(
    model: &AnimatedModel,
    clip: Option<&AnimationClip>,
    t: f32,
) -> Vec<Mat4> {
    fn visit(
        model: &AnimatedModel,
        clip: Option<&AnimationClip>,
        t: f32,
        node_idx: usize,
        parent_world: Mat4,
        out: &mut [Mat4],
    ) {
        let node = &model.nodes[node_idx];
        let (translation, rotation, scale) = local_trs(node, clip, node_idx, t);
        let world =
            parent_world * Mat4::from_scale_rotation_translation(scale, rotation, translation);
        out[node_idx] = world;
        for &child in &node.children {
            visit(model, clip, t, child, world, out);
        }
    }
    let mut out = vec![Mat4::IDENTITY; model.nodes.len()];
    for &root in &model.roots {
        visit(model, clip, t, root, Mat4::IDENTITY, &mut out);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn emit_rigid_parts(
    model: &AnimatedModel,
    world_matrices: &[Mat4],
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    origin: Vec3,
    yaw_rotate: &dyn Fn(f32, f32) -> (f32, f32),
    uv: [f32; 4],
) {
    for (node_idx, node) in model.nodes.iter().enumerate() {
        if node.mesh.is_empty() {
            continue;
        }
        let world = world_matrices[node_idx];
        // Non-uniform per-part scale shows up in a couple of these rigs
        // (e.g. squash/stretch-y bits), so normals need the proper
        // inverse-transpose rather than just the rotation part.
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
                    color: (Vec3::from_array(prim.color)
                        * Vec3::from_array(prim.colors.get(i).copied().unwrap_or([1.0; 3])))
                    .to_array(),
                    normal: [nx, world_normal.y, nz],
                    uv: if model.texture_layer.is_some() {
                        prim.uvs.get(i).copied().unwrap_or([0.0; 2])
                    } else {
                        [uv[0], uv[1]]
                    },
                    ao: 1.0,
                    reflectivity: 0.0,
                    emission: prim.emission,
                    wind: 0.0,
                    tex_layer: model.texture_layer.unwrap_or(0.0),
                    glimmer: 0.0,
                    skylight: 1.0,
                    wet: [prim.absorption, 0.],
                });
            }
            for &idx in &prim.indices {
                indices.push(base_index + idx);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_skinned_mesh(
    skin: &Skin,
    world_matrices: &[Mat4],
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
    origin: Vec3,
    yaw_rotate: &dyn Fn(f32, f32) -> (f32, f32),
    tex_layer: f32,
) {
    // Per glTF's skinning formula, simplified for the case (true for every
    // bundled skinned model) where the mesh-holding node itself has an
    // identity transform: a joint's contribution to a vertex is its own
    // animated world matrix composed with its inverse bind matrix, computed
    // once per joint per frame rather than per vertex.
    let skin_matrices: Vec<Mat4> = skin
        .joints
        .iter()
        .zip(&skin.inverse_bind)
        .map(|(&joint_node, inverse_bind)| world_matrices[joint_node] * *inverse_bind)
        .collect();

    let mesh = &skin.mesh;
    let base_index = vertices.len() as u32;
    for i in 0..mesh.positions.len() {
        let ji = mesh.joint_indices[i];
        let w = mesh.joint_weights[i];
        // These rigs only ever use a single dominant joint per vertex in
        // practice (weight [1,0,0,0]), but the general 4-way weighted blend
        // costs nothing extra and is what glTF's JOINTS_0/WEIGHTS_0 actually
        // promise to support.
        let blended = skin_matrices[ji[0] as usize] * w[0]
            + skin_matrices[ji[1] as usize] * w[1]
            + skin_matrices[ji[2] as usize] * w[2]
            + skin_matrices[ji[3] as usize] * w[3];
        let normal_mat = Mat3::from_mat4(blended).inverse().transpose();
        let world_pos = blended.transform_point3(mesh.positions[i]);
        let world_normal = (normal_mat * mesh.normals[i]).normalize_or_zero();
        let (wx, wz) = yaw_rotate(world_pos.x, world_pos.z);
        let (nx, nz) = yaw_rotate(world_normal.x, world_normal.z);
        vertices.push(Vertex {
            position: [origin.x + wx, origin.y + world_pos.y, origin.z + wz],
            color: [1.0, 1.0, 1.0],
            normal: [nx, world_normal.y, nz],
            uv: mesh.uvs[i],
            ao: 1.0,
            reflectivity: 0.0,
            emission: 0.0,
            wind: 0.0,
            tex_layer,
            glimmer: 0.0,
            skylight: 1.0,
            wet: [mesh.absorption, 0.],
        });
    }
    for &idx in &mesh.indices {
        indices.push(base_index + idx);
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
    kind: CreatureKind,
    clip_name: &str,
    clip_time: f32,
    origin: Vec3,
    facing: f32,
) {
    let clip = model.animations.get(clip_name);
    let first_vertex = vertices.len();
    let t = match clip {
        Some(c) if c.duration > 0.0 => clip_time.rem_euclid(c.duration),
        _ => 0.0,
    };

    // Local +Z is "forward"; `facing` rotates it around Y the same way
    // `Camera::forward` does (0 faces +X, increasing turns toward +Z) --
    // matches every model's own rig, which was confirmed to face +Z (nose/
    // beak/muzzle/head at positive local Z, tail at negative Z) in each
    // `.glb`.
    let (s, c) = facing.sin_cos();
    let yaw_rotate = move |x: f32, z: f32| (x * s + z * c, -x * c + z * s);
    let uv = white_uv();

    let world_matrices = compute_world_matrices(model, clip, t);
    emit_rigid_parts(
        model,
        &world_matrices,
        vertices,
        indices,
        origin,
        &yaw_rotate,
        uv,
    );
    if let Some(skin) = &model.skin {
        // See `shader.wgsl`'s `fs_main` and `voxel::mesher::Vertex::tex_layer`
        // -- 0.0 is reserved for "sample the terrain atlas", so a real
        // creature layer is offset by one.
        let tex_layer = model.texture_layer.unwrap_or(kind.to_u8() as f32 + 1.0);
        emit_skinned_mesh(
            skin,
            &world_matrices,
            vertices,
            indices,
            origin,
            &yaw_rotate,
            tex_layer,
        );
    }
    let scale = kind.model_scale();
    if scale != 1.0 {
        for vertex in &mut vertices[first_vertex..] {
            vertex.position =
                (origin + (Vec3::from_array(vertex.position) - origin) * scale).to_array();
        }
    }
}

fn chest_model() -> &'static AnimatedModel {
    static MODEL: std::sync::OnceLock<AnimatedModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        let mut model = load_glb(include_bytes!("../models/chest.glb"));
        model.texture_layer = Some(28.0);
        model
    })
}

/// Bundled chest, normalized without distortion into one occupied voxel.
pub fn chest_mesh() -> &'static crate::voxel::mesher::MeshData {
    static MESH: std::sync::OnceLock<crate::voxel::mesher::MeshData> = std::sync::OnceLock::new();
    MESH.get_or_init(|| {
        let model = chest_model();
        let mut mesh = crate::voxel::mesher::MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let matrices = compute_world_matrices(&model, None, 0.0);
        emit_rigid_parts(
            &model,
            &matrices,
            &mut mesh.vertices,
            &mut mesh.indices,
            Vec3::ZERO,
            &|x, z| (x, z),
            white_uv(),
        );
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for v in &mesh.vertices {
            let p = Vec3::from_array(v.position);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let scale = 0.9 / (hi - lo).max_element().max(0.001);
        let origin = Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5);
        for v in &mut mesh.vertices {
            v.position = ((Vec3::from_array(v.position) - origin) * scale
                + Vec3::new(0.5, 0.02, 0.5))
            .to_array();
        }
        mesh
    })
}

/// Cache the static bag geometry once; instances only translate these vertices.
fn lore_book_model() -> &'static AnimatedModel {
    static MODEL: std::sync::OnceLock<AnimatedModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        let mut model = load_glb(include_bytes!("../models/lore_book.glb"));
        model.texture_layer = Some(35.0);
        model
    })
}
fn torch_model() -> &'static AnimatedModel {
    static MODEL: std::sync::OnceLock<AnimatedModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| {
        let mut m = load_glb(include_bytes!("../models/torch.glb"));
        m.texture_layer = Some(36.);
        m
    })
}
fn beehive_model() -> &'static AnimatedModel {
    static MODEL: std::sync::OnceLock<AnimatedModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(|| load_glb(include_bytes!("../models/beehive.glb")))
}
pub fn beehive_mesh(positions: &[glam::Vec3]) -> crate::voxel::mesher::MeshData {
    let mut mesh = crate::voxel::mesher::MeshData { vertices: vec![], indices: vec![] };
    for &pos in positions {
        let start = mesh.vertices.len();
        push_model(&mut mesh.vertices, &mut mesh.indices, beehive_model(), crate::creature::CreatureKind::Sheep, "", 0., pos, 0.);
        for vertex in &mut mesh.vertices[start..] {
            let local = glam::Vec3::from_array(vertex.position) - pos;
            let rotated = glam::Vec3::new(local.x, -local.z, local.y);
            vertex.position = (pos + rotated).to_array();
            let normal = glam::Vec3::from_array(vertex.normal);
            vertex.normal = glam::Vec3::new(normal.x, -normal.z, normal.y).to_array();
            vertex.color = [0.72, 0.38, 0.10];
        }
    }
    mesh
}
/// Supplied torch's looping flame animation, cached at 16 frames per cycle.
pub fn torch_mesh(time: f32) -> &'static crate::voxel::mesher::MeshData {
    static FRAMES: std::sync::OnceLock<Vec<crate::voxel::mesher::MeshData>> =
        std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| {
        let m = torch_model();
        (0..16)
            .map(|frame| {
                let mut mesh = crate::voxel::mesher::MeshData {
                    vertices: vec![],
                    indices: vec![],
                };
                let matrices =
                    compute_world_matrices(m, m.animations.get("on"), frame as f32 * 1.2 / 16.);
                emit_rigid_parts(
                    m,
                    &matrices,
                    &mut mesh.vertices,
                    &mut mesh.indices,
                    Vec3::ZERO,
                    &|x, z| (x, z),
                    white_uv(),
                );
                mesh
            })
            .collect()
    });
    &frames[((time.max(0.) / 1.2 * 16.) as usize) % 16]
}
pub fn lore_book_mesh() -> &'static crate::voxel::mesher::MeshData {
    static MESH: std::sync::OnceLock<crate::voxel::mesher::MeshData> = std::sync::OnceLock::new();
    MESH.get_or_init(|| {
        let model = lore_book_model();
        let mut mesh = crate::voxel::mesher::MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let matrices = compute_world_matrices(model, None, 0.0);
        emit_rigid_parts(
            model,
            &matrices,
            &mut mesh.vertices,
            &mut mesh.indices,
            Vec3::ZERO,
            &|x, z| (x, z),
            white_uv(),
        );
        // The supplied book lies flat. Stand it on its bottom edge, keeping lighting correct.
        let upright = glam::Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2);
        for v in &mut mesh.vertices {
            v.position = (upright * Vec3::from_array(v.position)).to_array();
            v.normal = (upright * Vec3::from_array(v.normal)).to_array();
        }
        let min = mesh
            .vertices
            .iter()
            .fold(Vec3::splat(f32::INFINITY), |a, v| {
                a.min(Vec3::from_array(v.position))
            });
        let max = mesh
            .vertices
            .iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), |a, v| {
                a.max(Vec3::from_array(v.position))
            });
        let scale = 0.85 / (max - min).max_element().max(0.001);
        let origin = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
        for v in &mut mesh.vertices {
            v.position = ((Vec3::from_array(v.position) - origin) * scale).to_array();
            v.emission = 0.;
        }
        mesh
    })
}
pub fn loot_bag_mesh() -> &'static crate::voxel::mesher::MeshData {
    static MESH: std::sync::OnceLock<crate::voxel::mesher::MeshData> = std::sync::OnceLock::new();
    MESH.get_or_init(|| {
        let model = load_glb(include_bytes!("../models/loot_bag.glb"));
        let mut mesh = crate::voxel::mesher::MeshData {
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let matrices = compute_world_matrices(&model, None, 0.0);
        emit_rigid_parts(
            &model,
            &matrices,
            &mut mesh.vertices,
            &mut mesh.indices,
            Vec3::ZERO,
            &|x, z| (x, z),
            white_uv(),
        );
        for v in &mut mesh.vertices {
            // Keep the bag readable at pickup size and center it on the bobbing origin.
            v.position = (Vec3::from_array(v.position) * 1.5 - Vec3::Y * 0.2).to_array();
            // The white atlas sample makes emission add white over the vertex
            // colors. Keep the bag normally lit so its colors stay saturated.
            v.emission = 0.0;
        }
        mesh
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn wet_materials_respect_absorption_overrides_and_metal_content() {
        let json = serde_json::json!({"materials":[
            {"pbrMetallicRoughness":{"metallicFactor":0,"roughnessFactor":1}},
            {"pbrMetallicRoughness":{"metallicFactor":1,"roughnessFactor":1}},
            {"extras":{"wetAbsorption":0.42}}
        ]});
        assert!(
            super::material_absorption(&json, Some(0)) > super::material_absorption(&json, Some(1))
        );
        assert_eq!(super::material_absorption(&json, Some(2)), 0.42);
    }
    #[test]
    fn chest_has_its_embedded_texture_and_fits_one_voxel() {
        let mesh = super::chest_mesh();
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.indices.is_empty());
        assert!(super::chest_model().texture.is_some());
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.tex_layer == 28.0 && v.position.iter().all(|n| *n >= 0.0 && *n <= 1.0)));
        assert!(mesh.vertices.windows(2).any(|v| v[0].uv != v[1].uv));
    }
    #[test]
    fn player_actions_keep_resources_on_the_animated_right_grip() {
        use super::*;
        use crate::player_animation::Clip;
        let models = Models::load();
        for index in 0..4 {
            let model = &models.players[index];
            let grip = model
                .right_grip
                .expect("player requires a right grip socket");
            for animation in [
                Clip::Idle,
                Clip::Walk,
                Clip::Run,
                Clip::Attack,
                Clip::Work,
                Clip::Jump,
                Clip::Hello,
                Clip::Dance,
                Clip::Angry,
            ] {
                let clip = model
                    .animations
                    .get(animation.name())
                    .expect("required player animation");
                if let Some(duration) = animation.duration() {
                    assert!((clip.duration - duration).abs() < 0.001);
                }
                let mut positions = Vec::new();
                for time in [0.0, clip.duration * 0.3, clip.duration * 0.6] {
                    let matrices = compute_world_matrices(model, Some(clip), time);
                    for yaw in [0.0, 1.1, -2.0] {
                        let origin = Vec3::new(17.0, 26.0, -11.0);
                        let rotation = Mat3::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
                        let expected =
                            origin + rotation * matrices[grip].transform_point3(Vec3::ZERO);
                        let mut vertices = Vec::new();
                        let mut indices = Vec::new();
                        models.push_player_animated(
                            &mut vertices,
                            &mut indices,
                            crate::remote_player::Appearance {
                                model: index as u8,
                                hat: Some(1),
                            },
                            origin,
                            yaw,
                            animation,
                            time,
                            Some(crate::equipment::Entry::Resource(
                                crate::voxel::BlockType::Stone,
                            )),
                        );
                        let held: Vec<_> = vertices.iter().filter(|v| v.tex_layer == 0.0).collect();
                        assert!(!held.is_empty());
                        let center = held
                            .iter()
                            .map(|v| Vec3::from_array(v.position))
                            .sum::<Vec3>()
                            / held.len() as f32;
                        assert!(
                            center.distance(expected) < 0.0001,
                            "{index} {animation:?}: {center:?} != {expected:?}"
                        );
                        assert!(vertices
                            .iter()
                            .all(|v| Vec3::from_array(v.position).is_finite()));
                        assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
                    }
                    positions.push(matrices[grip].transform_point3(Vec3::ZERO));
                }
                if matches!(animation, Clip::Work | Clip::Attack | Clip::Hello | Clip::Dance) {
                    assert!(
                        positions[0].distance(positions[1]) > 0.01,
                        "hand must move during {animation:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn loot_bag_preserves_vertex_colors_and_valid_geometry() {
        let mesh = super::loot_bag_mesh();
        assert_eq!(mesh.vertices.len(), 6480);
        assert_eq!(mesh.indices.len(), 6480);
        assert!(mesh
            .indices
            .iter()
            .all(|i| (*i as usize) < mesh.vertices.len()));
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.position.iter().all(|p| p.is_finite())));
        assert!(mesh
            .vertices
            .iter()
            .any(|v| v.color != mesh.vertices[0].color));
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.position[1] >= -0.21 && v.position[1] < 0.3));
    }
    #[test]
    fn fish_model_animates_and_fits_its_water_clearance() {
        let models = Models::load();
        let model = models.for_kind(CreatureKind::Fish);
        assert!(model.texture.is_some());
        assert!(model.animations.contains_key("idle"));
        let mut first = Vec::new();
        for time in [0.0, 0.2, 0.6, 1.0] {
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            push_model(
                &mut vertices,
                &mut indices,
                model,
                CreatureKind::Fish,
                "idle",
                time,
                Vec3::ZERO,
                0.0,
            );
            assert!(!indices.is_empty());
            assert!(vertices.iter().all(|v| v.tex_layer == 27.0));
            assert!(
                vertices.iter().all(|v| v.position[0].abs() <= 0.7
                    && v.position[2].abs() <= 0.7
                    && v.position[1].abs() <= 0.35),
                "fish mesh must fit inside the checked water volume"
            );
            if first.is_empty() {
                first = vertices.iter().map(|v| v.position).collect();
            } else {
                assert!(vertices.iter().zip(&first).any(|(v, p)| v.position != *p));
            }
        }
    }
    #[test]
    fn dragon_models_have_all_ground_and_flight_clips_at_seven_human_heights() {
        let models = Models::load();
        let mut human = Vec::new();
        let mut indices = Vec::new();
        models.push_player(
            &mut human,
            &mut indices,
            crate::remote_player::Appearance {
                model: 0,
                hat: None,
            },
            Vec3::ZERO,
            0.0,
            0.0,
            0.0,
        );
        let height = |vertices: &[Vertex]| {
            vertices
                .iter()
                .map(|v| v.position[1])
                .fold(f32::NEG_INFINITY, f32::max)
                - vertices
                    .iter()
                    .map(|v| v.position[1])
                    .fold(f32::INFINITY, f32::min)
        };
        let human_height = height(&human);
        for kind in [CreatureKind::DragonGreen, CreatureKind::DragonRed] {
            let model = models.for_kind(kind);
            assert!(model.texture.is_some());
            for clip in ["idle", "walk", "fly", "attack_walk", "attack_fly", "die"] {
                assert!(model.animations.contains_key(clip), "missing {clip}");
                let mut vertices = Vec::new();
                let mut indices = Vec::new();
                push_model(
                    &mut vertices,
                    &mut indices,
                    model,
                    kind,
                    clip,
                    0.3,
                    Vec3::ZERO,
                    0.0,
                );
                assert!(!indices.is_empty());
                assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
                assert!(vertices
                    .iter()
                    .all(|v| Vec3::from_array(v.position).is_finite()));
                assert!(vertices
                    .iter()
                    .all(|v| v.tex_layer == model.texture_layer.unwrap()));
                if clip == "idle" {
                    let ratio = height(&vertices) / human_height;
                    assert!(
                        (6.0..=8.0).contains(&ratio),
                        "dragon/human height ratio = {ratio}"
                    );
                }
            }
        }
    }
    #[test]
    fn undead_variants_have_textures_and_render_all_required_animations() {
        let models = Models::load();
        for kind in [
            CreatureKind::Zombie,
            CreatureKind::Skeleton,
            CreatureKind::SkeletonSorcerer,
        ] {
            for variant in 0..4 {
                let model = models.for_variant(kind, variant);
                assert!(model.texture.is_some());
                for clip in ["idle", "walk", "attack", "die"] {
                    assert!(model.animations.contains_key(clip));
                    let mut vertices = Vec::new();
                    let mut indices = Vec::new();
                    push_model(
                        &mut vertices,
                        &mut indices,
                        model,
                        kind,
                        clip,
                        0.3,
                        Vec3::ZERO,
                        0.0,
                    );
                    assert!(!indices.is_empty());
                    assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
                    assert!(vertices
                        .iter()
                        .all(|v| Vec3::from_array(v.position).is_finite()));
                    assert!(vertices
                        .iter()
                        .all(|v| v.tex_layer == model.texture_layer.unwrap()));
                }
            }
        }
    }
    #[test]
    fn all_six_npcs_render_textured_animated_human_sized_models() {
        let models = Models::load();
        for (kind, model) in models.npcs.iter().enumerate() {
            assert!(model.texture.is_some());
            for clip in ["idle", "walk"] {
                assert!(model.animations.contains_key(clip));
                let mut vertices = Vec::new();
                let mut indices = Vec::new();
                push_model(
                    &mut vertices,
                    &mut indices,
                    model,
                    CreatureKind::Sheep,
                    clip,
                    0.3,
                    Vec3::ZERO,
                    0.,
                );
                assert!(!indices.is_empty());
                assert!(vertices.iter().all(|v| v.tex_layer == 29. + kind as f32
                    && Vec3::from_array(v.position).is_finite()));
                assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
                let top = vertices
                    .iter()
                    .map(|v| v.position[1])
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!((1.4..2.8).contains(&top), "NPC {kind} height {top}");
            }
        }
    }
    #[test]
    fn player_models_and_all_hats_render_with_textures_and_animated_sockets() {
        use super::*;
        let models = Models::load();
        assert!(models.creature_texture_layers()[8..]
            .iter()
            .all(|t| t.is_some()));
        for model in 0..4 {
            assert!(models.players[model as usize].hat_socket.is_some());
            for hat in [None, Some(0), Some(1), Some(2), Some(3)] {
                for speed in [0.0, 3.0, 7.0] {
                    let mut vertices = Vec::new();
                    let mut indices = Vec::new();
                    models.push_player(
                        &mut vertices,
                        &mut indices,
                        crate::remote_player::Appearance { model, hat },
                        Vec3::ZERO,
                        0.0,
                        speed,
                        0.3,
                    );
                    assert!(!indices.is_empty());
                    assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
                    assert!(vertices
                        .iter()
                        .all(|v| Vec3::from_array(v.position).is_finite()));
                    assert!(vertices.iter().any(|v| v.tex_layer == 9.0 + model as f32));
                    if let Some(hat) = hat {
                        let hat_vertices: Vec<_> = vertices
                            .iter()
                            .filter(|v| v.tex_layer == 13.0 + hat as f32)
                            .collect();
                        assert!(!hat_vertices.is_empty());
                        assert!(hat_vertices
                            .iter()
                            .all(|v| v.position[1] > 1.4 && v.position[1] < 2.3));
                        assert!(hat_vertices.iter().any(|v| v.uv != hat_vertices[0].uv));
                    }
                }
            }
        }
    }
    use super::*;

    #[test]
    fn creature_rigid_faces_sample_white_away_from_atlas_boundaries() {
        let atlas = image::load_from_memory(crate::voxel::atlas::ATLAS_BYTES)
            .unwrap()
            .to_rgba8();
        let models = Models::load();
        for kind in 0..=7 {
            let kind = CreatureKind::from_u8(kind);
            let model = models.for_kind(kind);
            for clip in model.animations.keys() {
                let mut vertices = Vec::new();
                let mut indices = Vec::new();
                push_model(
                    &mut vertices,
                    &mut indices,
                    model,
                    kind,
                    clip,
                    0.37,
                    Vec3::new(37.0, 5.0, -12.0),
                    0.73,
                );
                assert!(!vertices.is_empty());
                // Only a rigid part's vertices (tex_layer == 0.0) sample the
                // terrain atlas -- a skinned mesh's vertices carry a real
                // glTF UV into its own `creature_texture` layer instead (see
                // `emit_skinned_mesh`), which isn't atlas-space at all.
                for vertex in vertices.iter().filter(|v| v.tex_layer == 0.0) {
                    // Model nearest sampling on both sides of the UV to
                    // catch a constant coordinate placed on a tile edge.
                    for dx in [-0.25, 0.0, 0.25] {
                        for dy in [-0.25, 0.0, 0.25] {
                            let x = (vertex.uv[0] * atlas.width() as f32 + dx).floor() as u32;
                            let y = (vertex.uv[1] * atlas.height() as f32 + dy).floor() as u32;
                            assert_eq!(
                                atlas.get_pixel(x, y).0, [255, 255, 255, 255],
                                "creature {kind:?}, clip {clip}: UV {:?} samples outside white swatch",
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
    /// of every triangle and showed the inside instead. For a triangle
    /// wound correctly for this engine (CW front-face), the standard
    /// right-hand-rule face normal of its first two edges points *into*
    /// the surface, i.e. opposite its vertex normals -- this pins that down
    /// against the real files (both the rigid sheep/chicken and every
    /// skinned model's own mesh) so a regression fails loudly instead of
    /// only visually.
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
            ("sunscorch", &models.sunscorch),
        ] {
            let mut checked = 0;
            let mut wrong = 0;
            let mut check_triangle =
                |i0: usize, i1: usize, i2: usize, positions: &[Vec3], normals: &[Vec3]| {
                    let (v0, v1, v2) = (positions[i0], positions[i1], positions[i2]);
                    let face_normal = (v1 - v0).cross(v2 - v0);
                    if face_normal.length_squared() < 1e-10 {
                        return; // degenerate/near-zero-area triangle
                    }
                    let vertex_normal = (normals[i0] + normals[i1] + normals[i2]) / 3.0;
                    checked += 1;
                    if face_normal.dot(vertex_normal) > 0.0 {
                        wrong += 1;
                    }
                };
            for node in &model.nodes {
                for prim in &node.mesh {
                    for tri in prim.indices.chunks_exact(3) {
                        check_triangle(
                            tri[0] as usize,
                            tri[1] as usize,
                            tri[2] as usize,
                            &prim.positions,
                            &prim.normals,
                        );
                    }
                }
            }
            if let Some(skin) = &model.skin {
                for tri in skin.mesh.indices.chunks_exact(3) {
                    check_triangle(
                        tri[0] as usize,
                        tri[1] as usize,
                        tri[2] as usize,
                        &skin.mesh.positions,
                        &skin.mesh.normals,
                    );
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
            CreatureKind::Sheep,
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
    /// have -- this is the loader's own hand-parsed byte offsets/strides
    /// being exercised against the actual shipped files, not synthetic
    /// data, for both the rigid and skinned code paths.
    #[test]
    fn every_bundled_model_loads_and_poses_its_documented_clips() {
        let models = Models::load();
        let cases: &[(&AnimatedModel, CreatureKind, &[&str])] = &[
            (&models.sheep, CreatureKind::Sheep, &["idle", "walk"]),
            (&models.chicken, CreatureKind::Chicken, &["idle", "walk"]),
            (
                &models.stone_golem,
                CreatureKind::StoneGolem,
                &["idle", "walk", "attack", "die"],
            ),
            (
                &models.wolf,
                CreatureKind::Wolf,
                &["idle", "walk", "run", "attack", "die"],
            ),
            (
                &models.stinger,
                CreatureKind::Stinger,
                &["idle", "walk", "run", "attack", "die"],
            ),
            (&models.cow, CreatureKind::Cow, &["idle", "walk", "die"]),
            (
                &models.goblin,
                CreatureKind::Goblin,
                &["idle", "walk", "run", "attack", "die"],
            ),
            (
                &models.sunscorch,
                CreatureKind::Sunscorch,
                &["idle", "walk", "attack", "die"],
            ),
        ];
        for (model, kind, clips) in cases {
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
                    *kind,
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
            CreatureKind::Sheep,
            "does_not_exist",
            1.23,
            Vec3::ZERO,
            0.0,
        );
        assert!(!vertices.is_empty());
    }

    /// Proves the animation sampler is actually being evaluated over time --
    /// not just always returning the bind pose -- by checking the walk
    /// clip's pose differs between two points in time. Checked for both a
    /// rigid model (sheep) and a skinned one (wolf), since skinning has its
    /// own separate pose-computation path (`emit_skinned_mesh`).
    #[test]
    fn a_walk_clip_pose_actually_changes_over_time() {
        let models = Models::load();
        for (model, kind) in [
            (&models.sheep, CreatureKind::Sheep),
            (&models.wolf, CreatureKind::Wolf),
        ] {
            let mut early = Vec::new();
            let mut early_i = Vec::new();
            push_model(
                &mut early,
                &mut early_i,
                model,
                kind,
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
                model,
                kind,
                "walk",
                0.35,
                Vec3::ZERO,
                0.0,
            );

            assert_eq!(
                early.len(),
                later.len(),
                "same clip should yield the same vertex count"
            );
            let moved = early.iter().zip(later.iter()).any(|(a, b)| {
                Vec3::from_array(a.position).distance(Vec3::from_array(b.position)) > 1e-4
            });
            assert!(moved, "expected at least one vertex to move between two different times in the walk cycle");
        }
    }

    /// End-to-end sanity check of the whole node-hierarchy + facing-rotation
    /// pipeline: the wolf model is much longer (nose to tail) than it is
    /// wide, so at facing 0 (forward = world +X, see `push_model`'s doc
    /// comment) its bounding box should be wider along X than along Z, and
    /// after a 90-degree turn that relationship should flip. A sign error
    /// in the yaw rotation or a broken skinning matrix would fail this.
    #[test]
    fn facing_rotates_the_whole_posed_model_not_just_its_root() {
        let models = Models::load();

        let mut facing_plus_x = Vec::new();
        let mut indices = Vec::new();
        push_model(
            &mut facing_plus_x,
            &mut indices,
            &models.wolf,
            CreatureKind::Wolf,
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
            CreatureKind::Wolf,
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
            CreatureKind::Sheep,
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
    /// within 1mm of each other with nearly-parallel face normals. A model
    /// can legitimately have two *distinct* decorative parts a few
    /// millimeters apart by design -- that's not what this checks for; the
    /// 1mm bar is well under any such intentional gap found in these files.
    #[test]
    fn posed_output_has_no_unexpected_duplicate_or_coincident_triangles() {
        use std::collections::HashMap;

        let models = Models::load();
        // (name, model, clip, time, exact duplicate triangles expected,
        // additional near-but-not-identical sub-mm pairs expected). Every
        // case is (0, 0) except sunscorch, which has 64 *genuinely*
        // duplicate triangles plus 12 more near-coincident ones baked into
        // its own raw source mesh (its 64 exact dupes were independently
        // verified offline against the bind-pose local-space geometry
        // directly, so this isn't a loader bug -- likely a duplicated
        // glow/aura shell, common for a ghost-type creature) -- reimporting
        // without simplifying geometry means keeping it, not silently
        // dropping it, so this pins the known counts down instead of
        // asserting a blanket zero.
        let cases: &[(&str, &AnimatedModel, CreatureKind, &str, f32, usize, usize)] = &[
            (
                "sheep",
                &models.sheep,
                CreatureKind::Sheep,
                "idle",
                0.0,
                0,
                0,
            ),
            (
                "sheep",
                &models.sheep,
                CreatureKind::Sheep,
                "walk",
                0.3,
                0,
                0,
            ),
            (
                "chicken",
                &models.chicken,
                CreatureKind::Chicken,
                "idle",
                0.0,
                0,
                0,
            ),
            ("wolf", &models.wolf, CreatureKind::Wolf, "idle", 0.0, 0, 0),
            ("wolf", &models.wolf, CreatureKind::Wolf, "walk", 0.3, 0, 0),
            ("wolf", &models.wolf, CreatureKind::Wolf, "run", 0.3, 0, 0),
            (
                "stone_golem",
                &models.stone_golem,
                CreatureKind::StoneGolem,
                "idle",
                0.0,
                0,
                0,
            ),
            (
                "stinger",
                &models.stinger,
                CreatureKind::Stinger,
                "idle",
                0.0,
                0,
                0,
            ),
            ("cow", &models.cow, CreatureKind::Cow, "idle", 0.0, 0, 0),
            (
                "goblin",
                &models.goblin,
                CreatureKind::Goblin,
                "idle",
                0.0,
                0,
                0,
            ),
            (
                "sunscorch",
                &models.sunscorch,
                CreatureKind::Sunscorch,
                "idle",
                0.0,
                64,
                12,
            ),
        ];

        for (name, model, kind, clip, time, expected_exact_dupes, expected_sub_mm_pairs) in cases {
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            push_model(
                &mut vertices,
                &mut indices,
                model,
                *kind,
                clip,
                *time,
                Vec3::ZERO,
                0.0,
            );

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
                exact_dupes, *expected_exact_dupes,
                "[{name}/{clip}] found {exact_dupes} exactly-duplicate triangles in the posed output, expected {expected_exact_dupes}"
            );

            // One representative triangle per exact-duplicate group (not
            // every copy) -- known, expected duplicates like sunscorch's
            // shouldn't also trip the *separate* near-but-not-identical
            // z-fight check below.
            let mut centroids = Vec::with_capacity(seen.len());
            let mut normals = Vec::with_capacity(seen.len());
            let mut representative = std::collections::HashSet::new();
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
                if !representative.insert(key) {
                    continue;
                }
                let p0 = Vec3::from_array(vertices[tri[0] as usize].position);
                let p1 = Vec3::from_array(vertices[tri[1] as usize].position);
                let p2 = Vec3::from_array(vertices[tri[2] as usize].position);
                centroids.push((p0 + p1 + p2) / 3.0);
                normals.push((p1 - p0).cross(p2 - p0).normalize_or_zero());
            }
            let mut sub_mm_pairs = 0;
            for i in 0..centroids.len() {
                for j in (i + 1)..centroids.len() {
                    if centroids[i].distance(centroids[j]) < 0.001
                        && normals[i].dot(normals[j]).abs() > 0.95
                    {
                        sub_mm_pairs += 1;
                    }
                }
            }
            assert_eq!(
                sub_mm_pairs, *expected_sub_mm_pairs,
                "[{name}/{clip}] found {sub_mm_pairs} sub-millimeter near-parallel triangle pairs, expected {expected_sub_mm_pairs}"
            );
        }
    }

    /// `push_model` must be a pure function of its inputs -- byte-identical
    /// output for identical arguments, not just "visually the same" --
    /// which the `ANTI_ZFIGHT_NUDGE` tie-break (for rigid models) and
    /// per-frame re-skinning (for skinned models) both depend on to render
    /// stably rather than flickering.
    #[test]
    fn posed_mesh_is_bit_for_bit_deterministic_across_repeated_calls() {
        let models = Models::load();
        let cases = [
            (&models.sheep, CreatureKind::Sheep),
            (&models.wolf, CreatureKind::Wolf),
            (&models.stone_golem, CreatureKind::StoneGolem),
            (&models.goblin, CreatureKind::Goblin),
            (&models.sunscorch, CreatureKind::Sunscorch),
        ];
        for (model, kind) in cases {
            let mut a_vertices = Vec::new();
            let mut a_indices = Vec::new();
            push_model(
                &mut a_vertices,
                &mut a_indices,
                model,
                kind,
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
                kind,
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
    /// small enough at realistic node counts to be visually imperceptible.
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

    /// A skinned model's real `baseColorTexture` should decode to real,
    /// varied pixel colors -- not a single uniform value a broken decode
    /// (e.g. one that silently fell back to a blank image) would produce.
    /// The bar is deliberately low (>5, not some larger number): these are
    /// small, deliberately palette-limited hand-painted textures
    /// (stone_golem's whole 256x256 texture has only 63 distinct colors in
    /// it), so "many" distinct colors isn't the right signal -- "more than
    /// a small handful" is enough to rule out a flat fallback while still
    /// passing for a genuinely limited palette.
    #[test]
    fn skinned_models_decode_a_real_varied_texture_not_a_flat_fallback() {
        let models = Models::load();
        for (name, model) in [
            ("stone_golem", &models.stone_golem),
            ("wolf", &models.wolf),
            ("sunscorch", &models.sunscorch),
        ] {
            let texture = model
                .texture
                .as_ref()
                .unwrap_or_else(|| panic!("expected {name} to have a decoded baseColorTexture"));
            let mut distinct = std::collections::HashSet::new();
            for pixel in texture.pixels() {
                distinct.insert(pixel.0);
            }
            assert!(
                distinct.len() > 5,
                "{name}: expected several distinct colors in the decoded texture, got only {}",
                distinct.len()
            );
        }
    }

    /// A skinned mesh's vertices must carry their *real* glTF UV (varied
    /// across the mesh, since the model actually unwraps to many distinct
    /// swatches of its texture -- see `emit_skinned_mesh`) rather than a
    /// degenerate constant value, and every one must point at this kind's
    /// own layer of the shared `creature_texture` array (`CreatureKind::
    /// to_u8() + 1.0`, see `push_model`) so it never accidentally samples a
    /// different creature's texture.
    #[test]
    fn skinned_mesh_vertices_carry_real_varied_uvs_and_the_right_texture_layer() {
        let models = Models::load();
        for (name, model, kind) in [
            ("stone_golem", &models.stone_golem, CreatureKind::StoneGolem),
            ("wolf", &models.wolf, CreatureKind::Wolf),
            ("sunscorch", &models.sunscorch, CreatureKind::Sunscorch),
        ] {
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            push_model(
                &mut vertices,
                &mut indices,
                model,
                kind,
                "idle",
                0.0,
                Vec3::ZERO,
                0.0,
            );
            let skin_vertices: Vec<_> = vertices.iter().filter(|v| v.tex_layer != 0.0).collect();
            assert!(
                !skin_vertices.is_empty(),
                "expected {name} to have skinned-mesh vertices"
            );

            let expected_layer = kind.to_u8() as f32 + 1.0;
            assert!(
                skin_vertices.iter().all(|v| v.tex_layer == expected_layer),
                "{name}: expected every skinned vertex to use layer {expected_layer}"
            );

            let mut distinct_uvs = std::collections::HashSet::new();
            for v in &skin_vertices {
                distinct_uvs.insert(v.uv.map(|c| (c * 4096.0).round() as i32));
            }
            assert!(
                distinct_uvs.len() > 5,
                "{name}: expected several distinct real UVs, got only {}",
                distinct_uvs.len()
            );
        }
    }
}
