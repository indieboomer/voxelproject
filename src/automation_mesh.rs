//! Cheap replaceable workshop props. Geometry stays within each kind's occupied height.
use crate::automation::*;
use crate::voxel::{
    mesher::{push_cuboid, MeshData},
};
use glam::{Mat3, Vec3};
fn cube(mesh: &mut MeshData, a: [f32; 3], b: [f32; 3], color: [f32; 3]) {
    push_cuboid(
        &mut mesh.vertices,
        &mut mesh.indices,
        Vec3::from_array(a),
        Vec3::from_array(b),
        color,
        crate::voxel::atlas::uv_rect(crate::voxel::atlas::tile_for(crate::voxel::BlockType::Cobblestone, 0)),
    );
}
fn ring(mesh: &mut MeshData, y: f32, r: f32, color: [f32; 3], angle: f32) {
    for i in 0..8 {
        let first = mesh.vertices.len();
        let a = i as f32 * std::f32::consts::FRAC_PI_4 + angle;
        cube(mesh, [-0.045, -0.035, -0.13], [0.045, 0.035, 0.13], color);
        let rotation = Mat3::from_rotation_y(a);
        for v in &mut mesh.vertices[first..] {
            v.position = (Vec3::new(0.5 + a.cos() * r, y, 0.5 - a.sin() * r)
                + rotation * Vec3::from_array(v.position))
            .to_array();
            v.normal = (rotation * Vec3::from_array(v.normal)).to_array();
        }
    }
}
pub fn device(d: &Device, time: f32, preview: Option<bool>, state: &State) -> MeshData {
    let mut mesh = MeshData {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let wood = [0.33, 0.17, 0.08];
    let copper = [0.68, 0.32, 0.12];
    let stone = [0.35, 0.41, 0.44];
    let ceramic = [0.56, 0.24, 0.12];
    cube(&mut mesh, [0.08, 0.02, 0.08], [0.92, 0.14, 0.92], wood);
    match d.kind {
        Kind::Lantern => {
            let iron=[0.065,0.085,0.11];
            cube(&mut mesh,[0.18,0.05,0.18],[0.82,0.24,0.82],iron);
            cube(&mut mesh,[0.35,0.24,0.35],[0.65,0.46,0.65],iron);
            cube(&mut mesh,[0.45,0.46,0.45],[0.55,2.15,0.55],iron);
            for y in [0.5,1.9,2.12] {ring(&mut mesh,y,0.11,iron,0.0);}
            cube(&mut mesh,[0.21,2.13,0.21],[0.79,2.22,0.79],iron);
            let first=mesh.vertices.len();
            cube(&mut mesh,[0.3,2.25,0.3],[0.7,2.69,0.7],[0.7,0.86,1.0]);
            if d.config.enabled && d.activity==Activity::Working {
                for v in &mut mesh.vertices[first..] {v.emission=0.85;}
            }
            for x in [0.25,0.7] {for z in [0.25,0.7] {
                cube(&mut mesh,[x,2.21,z],[x+0.05,2.72,z+0.05],iron);
            }}
            cube(&mut mesh,[0.17,2.72,0.17],[0.83,2.8,0.83],iron);
            cube(&mut mesh,[0.27,2.8,0.27],[0.73,2.87,0.73],iron);
            cube(&mut mesh,[0.38,2.87,0.38],[0.62,2.93,0.62],iron);
            cube(&mut mesh,[0.47,2.93,0.47],[0.53,2.99,0.53],iron);
        }
        Kind::DarkAltar | Kind::Shrine => {
            let dark=d.kind==Kind::DarkAltar;
            let rock=if dark {[0.09,0.055,0.13]} else {[0.7,0.72,0.67]};
            let accent=if dark {[0.19,0.025,0.32]} else {[0.4,0.9,0.7]};
            cube(&mut mesh,[0.06,0.08,0.06],[0.94,0.3,0.94],rock);
            cube(&mut mesh,[0.2,0.3,0.2],[0.8,1.12,0.8],rock);
            cube(&mut mesh,[0.06,1.12,0.06],[0.94,1.3,0.94],rock);
            for x in [0.12,0.76] {cube(&mut mesh,[x,1.3,0.4],[x+0.12,1.92,0.6],rock);}
            let first=mesh.vertices.len();
            ring(&mut mesh,1.32,0.29,accent,0.0);
            cube(&mut mesh,[0.39,1.45,0.39],[0.61,1.8,0.61],accent);
            if d.config.enabled && d.activity==Activity::Working {
                for v in &mut mesh.vertices[first..] {v.emission=0.45;}
            }
        }
        Kind::Collector => {
            cube(&mut mesh, [0.43, 0.14, 0.43], [0.57, 0.8, 0.57], wood);
            ring(&mut mesh, 0.7, 0.26, copper, 0.0);
            cube(
                &mut mesh,
                [0.43, 0.82, 0.43],
                [0.57, 0.94, 0.57],
                [0.2, 0.7, 0.95],
            );
        }
        Kind::Vessel | Kind::Condenser | Kind::Dissipator => {
            cube(&mut mesh, [0.24, 0.14, 0.24], [0.76, 0.68, 0.76], ceramic);
            ring(&mut mesh, 0.64, 0.32, copper, 0.0);
            cube(
                &mut mesh,
                [0.31, 0.68, 0.31],
                [0.69, 0.71, 0.69],
                [0.1, 0.4, 0.65],
            );
            if d.kind != Kind::Vessel {
                cube(&mut mesh, [0.42, 0.71, 0.42], [0.58, 0.91, 0.58], stone);
            }
        }
        Kind::Chest => {
            let chest=crate::model::chest_mesh();
            mesh.vertices=chest.vertices.clone(); mesh.indices=chest.indices.clone();
        }
        Kind::Smelter => {
            cube(&mut mesh,[0.15,0.14,0.15],[0.85,0.68,0.85],stone);
            cube(&mut mesh,[0.25,0.68,0.27],[0.75,0.79,0.73],ceramic);
            cube(&mut mesh,[0.36,0.79,0.36],[0.64,0.96,0.64],stone);
            cube(&mut mesh,[0.39,0.94,0.39],[0.61,0.97,0.61],[0.08,0.06,0.05]);
            cube(&mut mesh,[0.25,0.24,0.12],[0.75,0.57,0.16],[0.07,0.045,0.03]);
            let first=mesh.vertices.len();
            cube(&mut mesh,[0.31,0.27,0.10],[0.69,0.36,0.13],[0.95,0.25,0.045]);
            if d.config.enabled && d.activity==Activity::Working {for v in &mut mesh.vertices[first..] {v.emission=0.3;}}
            for x in [0.32,0.47,0.62] {cube(&mut mesh,[x,0.22,0.09],[x+0.025,0.59,0.12],stone);}
        }
        Kind::Workshop => {
            for x in [0.15, 0.77] {
                for z in [0.15, 0.77] {
                    cube(&mut mesh, [x, 0.14, z], [x + 0.08, 0.82, z + 0.08], wood);
                }
            }
            cube(&mut mesh, [0.11, 0.82, 0.11], [0.89, 0.91, 0.89], wood);
            ring(
                &mut mesh,
                0.45,
                0.27,
                copper,
                if d.activity == Activity::Working {
                    time
                } else {
                    0.0
                },
            );
            cube(&mut mesh, [0.36, 0.15, 0.36], [0.64, 0.26, 0.64], stone);
        }
        Kind::Sensor => {
            cube(&mut mesh, [0.42, 0.14, 0.42], [0.58, 0.7, 0.58], wood);
            cube(&mut mesh, [0.26, 0.66, 0.3], [0.74, 0.9, 0.7], stone);
        }
        Kind::Valve => {
            cube(&mut mesh, [0.25, 0.2, 0.25], [0.75, 0.64, 0.75], copper);
            let y = if d.open() { 0.79 } else { 0.45 };
            cube(&mut mesh, [0.43, y, 0.13], [0.57, y + 0.07, 0.87], wood);
        }
        _ => {
            cube(
                &mut mesh,
                [0.36, 0.17, 0.36],
                [0.64, 0.35, 0.64],
                if d.kind == Kind::Signal { stone } else { wood },
            );
        }
    }
    // Rotate the prop once; face ports below are already in world orientation.
    let rotation = Mat3::from_rotation_y(-(d.rotation as f32) * std::f32::consts::FRAC_PI_2);
    for v in &mut mesh.vertices {
        v.position = (Vec3::splat(0.5)
            + rotation * (Vec3::from_array(v.position) - Vec3::splat(0.5)))
        .to_array();
        v.normal = (rotation * Vec3::from_array(v.normal)).to_array();
    }
    for (face, net, dir) in d.ports(balance()).into_iter().filter(|_|d.kind!=Kind::Chest) {
        let delta = face.delta();
        let normal = Vec3::new(delta.0 as f32, delta.1 as f32, delta.2 as f32);
        let mut anchor = Vec3::splat(0.5) + normal * 0.44;
        // A shared face has one composite socket with separate network marks.
        if d.ports(balance()).iter().filter(|p| p.0 == face).count() > 1 {
            let tangent = if normal.y.abs() > 0.5 {
                Vec3::X
            } else {
                Vec3::Y
            };
            anchor += tangent * if net == Network::Mana { -0.11 } else { 0.11 };
        }
        let c = match net {
            Network::Mana => [0.12, 0.55, 0.88],
            Network::Matter => [0.88, 0.57, 0.12],
            Network::Signal => [0.64, 0.3, 0.85],
        };
        if matches!(
            d.kind,
            Kind::Conduit | Kind::Channel | Kind::Splitter | Kind::Signal
        ) {
            cube(
                &mut mesh,
                (Vec3::splat(0.5).min(anchor) - Vec3::splat(0.035)).to_array(),
                (Vec3::splat(0.5).max(anchor) + Vec3::splat(0.035)).to_array(),
                c.map(|v| v * 0.65),
            );
        }
        let size = Vec3::splat(0.085);
        cube(
            &mut mesh,
            (anchor - size).max(Vec3::splat(0.01)).to_array(),
            (anchor + size).min(Vec3::splat(0.99)).to_array(),
            c,
        );
        // One/two/three marks identify network type without relying on color.
        let tangent = if normal.y.abs() > 0.5 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let marks = match net {
            Network::Mana => 1,
            Network::Matter => 2,
            Network::Signal => 3,
        };
        for i in 0..marks {
            let p =
                anchor + normal * 0.045 + tangent * ((i as f32 - (marks - 1) as f32 * 0.5) * 0.045);
            cube(
                &mut mesh,
                (p - Vec3::splat(0.014)).to_array(),
                (p + Vec3::splat(0.014)).to_array(),
                [0.95, 0.91, 0.7],
            );
        }
        let compatible = state
            .devices
            .get(&face.neighbor(d.cell))
            .is_some_and(|other| {
                other.ports(balance()).iter().any(|&(f, n, direction)| {
                    f == face.opposite()
                        && n == net
                        && !(dir == direction && dir != Direction::Both)
                })
            });
        if compatible {
            let p = anchor - normal * 0.13;
            cube(
                &mut mesh,
                (p - Vec3::splat(0.025)).to_array(),
                (p + Vec3::splat(0.025)).to_array(),
                [0.35, 0.95, 0.5],
            );
        }
    }
    if d.activity == Activity::Working || d.signal > 0 {
        let p = Vec3::new(
            0.5 + (time * 3.0).sin() * 0.18,
            0.94,
            0.5 + (time * 3.0).cos() * 0.18,
        );
        cube(
            &mut mesh,
            (p - Vec3::splat(0.025)).to_array(),
            (p + Vec3::splat(0.025)).to_array(),
            [0.7, 0.4, 0.95],
        );
    }
    if let Some(valid) = preview {
        let c = if valid {
            [0.2, 0.9, 0.45]
        } else {
            [0.95, 0.2, 0.15]
        };
        for level in 0..d.kind.height() {
        let first=mesh.vertices.len();
        for a in [0.01, 0.97] {
            for z in [0.01, 0.97] {
                cube(&mut mesh, [a, 0.01, z], [a + 0.02, 0.99, z + 0.02], c);
                cube(&mut mesh, [0.01, a, z], [0.99, a + 0.02, z + 0.02], c);
                cube(&mut mesh, [a, z, 0.01], [a + 0.02, z + 0.02, 0.99], c);
            }
        }
        for v in &mut mesh.vertices[first..] {v.position[1]+=level as f32;}
        }
    }
    let origin = Vec3::new(d.cell.0 as f32, d.cell.1 as f32, d.cell.2 as f32);
    for v in &mut mesh.vertices {
        v.position = (origin + Vec3::from_array(v.position)).to_array();
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automation_meshes_remain_inside_occupied_height_at_every_rotation() {
        for kind in Kind::ALL {
            for rotation in 0..4 {
                for time in [0.0, 0.5, 1.25] {
                    let mut d = Device::new(kind, (0, 0, 0), rotation);
                    d.activity = Activity::Working;
                    let m = device(&d, time, Some(true), &State::default());
                    assert!(!m.vertices.is_empty());
                    assert!(m.indices.iter().all(|i| (*i as usize) < m.vertices.len()));
                    for v in m.vertices {
                        assert!(
                            v.position
                                .into_iter().enumerate()
                                .all(|(axis,x)| x.is_finite() && x >= -0.00001 && x <= if axis==1 {kind.height() as f32+0.00001} else {1.00001}),
                            "{kind:?} {rotation}: {:?}",
                            v.position
                        );
                    }
                }
            }
        }
    }
}
