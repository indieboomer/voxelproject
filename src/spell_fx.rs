//! Short, bounded, emissive spell trails. The host emits these only after a successful cast.
use glam::Vec3;
use crate::voxel::{atlas, atlas_tiles::TILE_WHITE, mesher::{MeshData,push_cuboid}};

const LIFETIME: f32 = 0.8;
const MAX_BURSTS: usize = 16;
#[derive(Default)]
pub struct Effects { bursts: Vec<(Vec3,Vec3,f32)> }
impl Effects {
    pub fn cast(&mut self, origin: Vec3, target: Vec3) {
        if !origin.is_finite() || !target.is_finite() || origin.abs().max_element()>1_000_000.
            || target.abs().max_element()>1_000_000. || origin.distance(target)>24. {return;}
        if self.bursts.len()==MAX_BURSTS {self.bursts.remove(0);}
        self.bursts.push((origin,target,0.));
    }
    pub fn update(&mut self,dt:f32) {
        if !dt.is_finite() || dt<0. {return;}
        for (_,_,age) in &mut self.bursts {*age+=dt;}
        self.bursts.retain(|(_,_,age)|*age<LIFETIME);
    }
    pub fn mesh(&self, visible:impl Fn(Vec3)->bool)->MeshData {
        let mut mesh=MeshData {vertices:Vec::new(),indices:Vec::new()};
        for &(origin,target,age) in &self.bursts {
            let fade=1.-age/LIFETIME;
            let direction=(target-origin).try_normalize().unwrap_or(Vec3::Z);
            let axis=if direction.y.abs()>0.95 {Vec3::X}else{Vec3::Y};
            let side=direction.cross(axis).normalize_or_zero();
            let up=side.cross(direction).normalize_or_zero();
            for i in 0..40 {
                let phase=i as f32*2.39996;
                let radial=side*phase.cos()+up*phase.sin();
                let p=if i<20 {
                    origin.lerp(target,i as f32/19.)+radial*(0.08+age*0.3)
                } else {
                    target+radial*(0.12+age*1.3)+direction*((i%5) as f32-2.)*age*0.3
                };
                if !visible(p) {continue;}
                let size=Vec3::splat((0.025+(i%3) as f32*0.012)*fade);
                let color=if i%3==0 {[1.,0.8,0.35]}else{[0.65,0.35,1.]};
                let start=mesh.vertices.len();
                push_cuboid(&mut mesh.vertices,&mut mesh.indices,p-size,p+size,color,atlas::uv_rect(TILE_WHITE));
                for vertex in &mut mesh.vertices[start..] {vertex.emission=fade;}
            }
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_finite_visible_and_expires() {
        let mut fx=Effects::default();
        fx.cast(Vec3::NAN,Vec3::ZERO);
        fx.cast(Vec3::ZERO,Vec3::X*100.);
        assert!(fx.bursts.is_empty());
        for _ in 0..100 {fx.cast(Vec3::ZERO,Vec3::Z*10.);}
        assert_eq!(fx.bursts.len(),MAX_BURSTS);
        fx.update(0.2);
        let mesh=fx.mesh(|_|true);
        assert!(!mesh.indices.is_empty());
        assert!(mesh.vertices.iter().all(|v|v.emission>0.));
        assert!(fx.mesh(|_|false).indices.is_empty());
        fx.update(LIFETIME);
        assert!(fx.mesh(|_|true).vertices.is_empty());
    }
}
