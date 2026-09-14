use crate::{crafting::Account,equipment::Gear,voxel::mesher::MeshData};
use glam::{Vec3,Mat3};
pub fn equipped(a:&Account)->bool {a.torch_equipped && a.gear[Gear::Torch as usize]>0}
/// Negative radii below -100 distinguish warm portable flames from cool lanterns.
pub fn light(pos:Vec3)->[f32;4] {pos.extend(-106.).to_array()}
pub fn mesh(origin:Vec3,basis:Mat3,scale:f32,time:f32)->MeshData {
    let source=crate::model::torch_mesh(time);
    let mut mesh=MeshData{vertices:source.vertices.clone(),indices:source.indices.clone()};
    for v in &mut mesh.vertices {v.position=(origin+basis*(Vec3::from_array(v.position)*scale)).to_array();v.normal=(basis*Vec3::from_array(v.normal)).to_array();}
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recipe_and_left_hand_keep_tool_and_inventory_independent() {
        use crate::crafting::{Action,Registry};
        let r=Registry::load().unwrap();let mut a=Account::default();a.resources.fill(10);a.mana=20;
        let before=a.clone();assert!(r.prepare(&mut a,&Action::CraftGear(Gear::Torch)).is_err());assert_eq!(a,before);
        a.adventure.recipe_books=1<<3;
        r.prepare(&mut a,&Action::CraftGear(Gear::Torch)).unwrap();
        assert_eq!(a.gear[Gear::Torch as usize],1);assert_eq!(a.mana,18);
        assert!(!equipped(&a));let hotbar=a.hotbar.clone();let stock=a.resources;
        r.prepare(&mut a,&Action::EquipTorch(true)).unwrap();assert!(equipped(&a));assert_eq!(a.hotbar,hotbar);
        a.hotbar.active=2;assert_eq!(a.hotbar.entry(),Some(crate::equipment::Entry::Gear(Gear::Sword)));assert!(equipped(&a));
        let saved:Account=serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();assert!(equipped(&saved));assert_eq!(a.resources,stock);
        a.gear[Gear::Torch as usize]=0;assert!(!equipped(&a));assert!(r.prepare(&mut a,&Action::EquipTorch(true)).is_err());
        let old:Account=serde_json::from_value(serde_json::json!({"gear":vec![0;20]})).unwrap();assert_eq!(old.gear.len(),21);assert!(!old.torch_equipped);
    }
    #[test]
    fn supplied_torch_has_animated_emissive_flames_and_texture() {
        let a=crate::model::torch_mesh(0.);let b=crate::model::torch_mesh(0.45);
        assert!(!a.indices.is_empty());assert!(a.vertices.iter().all(|v|v.tex_layer==36. && Vec3::from_array(v.position).is_finite()));
        assert!(a.vertices.iter().any(|v|v.emission>0.));
        assert!(a.vertices.iter().zip(&b.vertices).any(|(a,b)|a.position!=b.position));
        let mut audio=crate::audio::AudioEngine::new();audio.update_torches(&[]);
    }
}
