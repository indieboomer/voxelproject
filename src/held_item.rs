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
        Some(Entry::Spell(_)) => {
            add([-0.12, 0.0, -0.12], [0.12, 0.42, 0.12], [0.70, 0.47, 0.30]);
            add([-0.07, 0.55, -0.07], [0.07, 0.69, 0.07], [0.65, 0.35, 1.0]);
        }
        Some(Entry::Resource(_)) => add([-0.30, 0.18, -0.30], [0.30, 0.78, 0.30], [1.0; 3]),
        Some(Entry::Gear(g)) => {
            let metal=if g.book().is_some(){g.definition().color}else{metal};
            if !matches!(g,Gear::Bow|Gear::Longbow) {
                add([-0.045, 0.02, -0.045], [0.045, 0.69, 0.045], wood);
            }
            match g {
                Gear::Torch => {add([-0.10,0.60,-0.10],[0.10,0.82,0.10],[1.,0.48,0.08]);add([-0.05,0.82,-0.05],[0.05,0.98,0.05],[1.,0.85,0.20]);}
                Gear::Axe | Gear::ForesterAxe => {
                    add([-0.29, 0.57, -0.06], [0.10, 0.84, 0.06], metal);
                    add([-0.34, 0.55, -0.04], [-0.25, 0.87, 0.04], metal);
                }
                Gear::Pickaxe | Gear::ProspectorPick => {
                    add([-0.35, 0.73, -0.055], [0.35, 0.84, 0.055], metal);
                    add([-0.38, 0.62, -0.04], [-0.29, 0.77, 0.04], metal);
                    add([0.29, 0.62, -0.04], [0.38, 0.77, 0.04], metal);
                }
                Gear::Sword => {
                    add([-0.20, 0.34, -0.06], [0.20, 0.40, 0.06], [0.8, 0.6, 0.20]);
                    add([-0.065, 0.40, -0.035], [0.065, 0.96, 0.035], metal);
                }
                Gear::Bow | Gear::Longbow => {
                    add([0.13, 0.20, -0.05], [0.20, 0.80, 0.05], wood);
                    add([-0.08, 0.08, -0.05], [0.16, 0.23, 0.05], wood);
                    add([-0.08, 0.77, -0.05], [0.16, 0.92, 0.05], wood);
                    add(
                        [-0.08, 0.08, -0.015],
                        [-0.06, 0.92, 0.015],
                        [0.9, 0.85, 0.72],
                    );
                }
                Gear::Spade => add([-0.18,0.55,-0.04],[0.18,0.9,0.04],metal),
                Gear::Sickle => {add([-0.25,0.70,-0.04],[0.05,0.80,0.04],metal);add([-0.28,0.45,-0.04],[-0.20,0.8,0.04],metal);}
                Gear::Spear => {add([-0.04,0.60,-0.04],[0.04,1.05,0.04],wood);add([-0.08,0.90,-0.03],[0.08,1.2,0.03],metal);}
                Gear::Dagger => {add([-0.13,0.32,-0.05],[0.13,0.38,0.05],metal);add([-0.055,0.38,-0.035],[0.055,0.72,0.035],metal);}
                Gear::Warhammer => add([-0.27,0.62,-0.15],[0.27,0.9,0.15],metal),
                Gear::SurveyLantern => {add([-0.18,0.35,-0.14],[0.18,0.72,0.14],g.definition().color);add([-0.23,0.72,-0.18],[0.23,0.80,0.18],metal);}
                _ => add([-0.13,0.65,-0.10],[0.13,0.9,0.10],g.definition().color),
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
