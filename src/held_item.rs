//! Small procedural cuboid models shared by first-person and remote hands.
use crate::{
    equipment::{Entry, Gear},
    voxel::mesher::{push_cuboid, MeshData},
};
use glam::{Mat3, Vec3};
pub fn parts(entry: Option<Entry>) -> Vec<(Vec3, Vec3, [f32; 3])> {
    let wood = [0.42, 0.24, 0.10];
    let metal = [0.75, 0.83, 0.87];
    let mut p = Vec::new();
    let mut add =
        |a: [f32; 3], b: [f32; 3], c| p.push((Vec3::from_array(a), Vec3::from_array(b), c));
    match entry {
        None => add([-0.12, 0.0, -0.12], [0.12, 0.42, 0.12], [0.70, 0.47, 0.30]),
        Some(Entry::Resource(_)) => add([-0.30, 0.18, -0.30], [0.30, 0.78, 0.30], [1.0; 3]),
        Some(Entry::Gear(g)) => {
            if g != Gear::Bow {
                add([-0.045, 0.02, -0.045], [0.045, 0.69, 0.045], wood);
            }
            match g {
                Gear::Axe => {
                    add([-0.29, 0.57, -0.06], [0.10, 0.84, 0.06], metal);
                    add([-0.34, 0.55, -0.04], [-0.25, 0.87, 0.04], metal);
                }
                Gear::Pickaxe => {
                    add([-0.35, 0.73, -0.055], [0.35, 0.84, 0.055], metal);
                    add([-0.38, 0.62, -0.04], [-0.29, 0.77, 0.04], metal);
                    add([0.29, 0.62, -0.04], [0.38, 0.77, 0.04], metal);
                }
                Gear::Sword => {
                    add([-0.20, 0.34, -0.06], [0.20, 0.40, 0.06], [0.8, 0.6, 0.20]);
                    add([-0.065, 0.40, -0.035], [0.065, 0.96, 0.035], metal);
                }
                Gear::Bow => {
                    add([0.13, 0.20, -0.05], [0.20, 0.80, 0.05], wood);
                    add([-0.08, 0.08, -0.05], [0.16, 0.23, 0.05], wood);
                    add([-0.08, 0.77, -0.05], [0.16, 0.92, 0.05], wood);
                    add(
                        [-0.08, 0.08, -0.015],
                        [-0.06, 0.92, 0.015],
                        [0.9, 0.85, 0.72],
                    );
                }
            }
        }
    }
    p
}
pub fn mesh(entry: Option<Entry>, origin: Vec3, basis: Mat3, scale: f32) -> MeshData {
    // Axe handle is aligned with local Y; twist around its own long axis,
    // before the shared held-pose and animation transforms.
    let basis = if entry == Some(Entry::Gear(Gear::Axe)) {
        basis * Mat3::from_rotation_y((1270.0_f32 % 360.0).to_radians())
    } else {
        basis
    };
    let mut mesh = MeshData {
        vertices: vec![],
        indices: vec![],
    };
    let uv = match entry {
        Some(Entry::Resource(b)) => {
            crate::voxel::atlas::uv_rect(crate::voxel::atlas::tile_for(b, 2))
        }
        _ => crate::voxel::atlas::white_uv(),
    };
    for (min, max, color) in parts(entry) {
        push_cuboid(&mut mesh.vertices, &mut mesh.indices, min, max, color, uv);
    }
    for v in &mut mesh.vertices {
        v.position = (origin + basis * (Vec3::from_array(v.position) * scale)).to_array();
        v.normal = (basis * Vec3::from_array(v.normal)).to_array();
    }
    mesh
}
