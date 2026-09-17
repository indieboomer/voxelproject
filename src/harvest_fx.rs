use glam::Vec3;
use crate::voxel::{atlas::white_uv, mesher::{push_cuboid, MeshData}};

#[derive(Default)]
pub struct Effects { particles: Vec<(Vec3, [f32; 3], f32, f32)> }
impl Effects {
    pub fn spawn(&mut self, origin: Vec3, color: [f32; 3]) {
        for i in 0..6 { let a = i as f32 * 1.047; self.particles.push((origin + Vec3::new(a.cos()*0.12, 0.25 + (i as f32)*0.04, a.sin()*0.12), color, i as f32 * 0.07, 0.)); }
        if self.particles.len() > 96 { self.particles.drain(..self.particles.len()-96); }
    }
    pub fn update(&mut self, dt: f32) { for (_,_,_,age) in &mut self.particles { *age += dt.max(0.); } self.particles.retain(|(_,_,_,age)| *age < 0.8); }
    pub fn mesh(&self) -> MeshData {
        let mut mesh = MeshData { vertices: vec![], indices: vec![] };
        for (origin, color, phase, age) in &self.particles { let t = (*age / 0.8).clamp(0.,1.); let a = *phase + *age * 3.; let pos = *origin + Vec3::new(a.cos()*t*0.35, t*0.8, a.sin()*t*0.35); let size = Vec3::splat(0.045 * (1.-t)); push_cuboid(&mut mesh.vertices, &mut mesh.indices, pos-size, pos+size, *color, white_uv()); }
        mesh
    }
}
