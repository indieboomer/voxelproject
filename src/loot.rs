//! Bounded transient loot and pixel death puffs. Loot ownership stays on the host.
use glam::Vec3;
use crate::{creature::CreatureKind, voxel::{World,BlockType,atlas,mesher::{MeshData,push_cuboid}}};
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize)]
pub struct Drop {
    pub pos:[f32;3],pub contents:Vec<(BlockType,u32)>,pub age:f32,
    #[serde(default)] pub cargo:crate::automation::Inventory,
    #[serde(default)] pub machine:bool,
    #[serde(default)] pub launch:Option<[f32;3]>,
}
impl Drop {
    pub fn valid_saved(&self)->bool {
        if self.machine {return self.valid_machine();}
        self.pos.iter().all(|v|v.is_finite()&&v.abs()<1_000_000.0)
            && self.age.is_finite() && self.age>=0.0 && self.cargo.is_empty()
            && self.launch.is_none() && !self.contents.is_empty() && self.contents.len()<=64
            && self.contents.iter().all(|(b,n)|crate::voxel::COLLECTIBLE_BLOCKS.contains(b)&&*n>0)
    }
    pub fn display_position(&self)->Vec3 {
        let end=Vec3::from_array(self.pos);
        if let Some(start)=self.launch {
            let t=(self.age/0.85).clamp(0.0,1.0);
            Vec3::from_array(start).lerp(end,t)+Vec3::Y*(4.0*t*(1.0-t)*1.2)
        } else {end+Vec3::Y*(0.08*(self.age*3.0).sin()+0.6*(1.0-self.age/0.6).max(0.0))}
    }
    pub fn valid_machine(&self)->bool {
        self.machine && self.pos.iter().all(|v|v.is_finite()&&v.abs()<1_000_000.0)
            && self.launch.is_some_and(|p|p.iter().all(|v|v.is_finite()&&v.abs()<1_000_000.0))
            && self.age.is_finite() && self.age>=0.0 && self.contents.len()+self.cargo.len()>0
            && self.contents.len()<=64 && self.cargo.len()<=64
            && self.contents.iter().all(|(b,n)|crate::voxel::COLLECTIBLE_BLOCKS.contains(b)&&*n>0&&*n<=256)
            && self.cargo.iter().all(|(id,n)|crate::automation::valid_item(id)&&!id.starts_with("creature:")&&*n>0&&*n<=256)
    }
}
#[derive(Default)]
pub struct Effects {pub drops:Vec<Drop>,puffs:Vec<(Vec3,f32)>}
const PICKUP_DELAY: f32 = 2.0;

// Try several nearby columns, then the death position itself. A single
// scattered column can intersect a hillside or cross an unloaded chunk edge.
fn landing_position(world: &World, pos: Vec3, angle: f32) -> Option<[f32; 3]> {
    if !pos.is_finite() { return None; }
    for attempt in 0..5 {
        let offset = if attempt == 4 { Vec3::ZERO } else {
            let a = angle + attempt as f32 * std::f32::consts::FRAC_PI_2;
            Vec3::new(a.cos(), 0.0, a.sin()) * 1.4
        };
        let x = (pos.x + offset.x).floor() as i32;
        let z = (pos.z + offset.z).floor() as i32;
        if !world.chunks.contains_key(&crate::voxel::chunk::world_to_chunk(x,z)) { continue; }
        let top = (pos.y.ceil() as i32).saturating_add(3).clamp(1, crate::voxel::chunk::CHUNK_Y-1);
        for y in (0..=top).rev() {
            let support = world.get_block(x,y,z);
            if (support.is_solid() || support == BlockType::Water)
                && !world.get_block(x,y+1,z).is_solid()
                && world.get_block(x,y+1,z) != BlockType::Water {
                return Some([x as f32+0.5,y as f32+1.3,z as f32+0.5]);
            }
        }
    }
    None
}
pub fn rewards(kind:CreatureKind)->Vec<(BlockType,u32)> {
    use BlockType::*;
    use CreatureKind as C;
    match kind {
        C::Sheep=>vec![(PlantFiber,2),(Meat,2)], C::Cow=>vec![(PlantFiber,2),(Meat,4)],
        C::Chicken=>vec![(PlantFiber,1),(Meat,1)],
        C::Wolf=>vec![(PlantFiber,1),(Resin,1)], C::Stinger=>vec![(Resin,1)],
        C::Goblin=>vec![(IronOre,1),(Cloth,1)], C::StoneGolem=>vec![(Stone,3),(IronOre,1)],
        C::Sunscorch=>vec![(Sulfur,2),(Ash,1)], C::Zombie=>vec![(Ash,1),(Cloth,1)],
        C::Skeleton=>vec![(Ash,2)], C::Fish=>vec![(PlantFiber,1),(Meat,1)],
        C::DragonGreen=>vec![(Resin,3),(CrystalDust,2)], C::DragonRed=>vec![(Sulfur,3),(CrystalDust,2)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn friendly_creatures_supply_species_sized_meat_rewards() {
        for (kind,n) in [(CreatureKind::Chicken,1),(CreatureKind::Sheep,2),(CreatureKind::Cow,4),(CreatureKind::Fish,1)] {
            let loot=rewards(kind);
            assert_eq!(loot.iter().filter(|(b,_)|*b==BlockType::Meat).map(|(_,n)|*n).sum::<u32>(),n);
            assert!(loot.iter().any(|(b,_)|*b==BlockType::PlantFiber));
        }
    }
    #[test]
    fn bundle_collection_is_atomic_and_only_rewards_one_player() {
        let mut world=World::new(1);
        let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
        chunk.set_local(8,24,8,BlockType::Stone);
        world.chunks.insert((0,0),chunk);
        let mut fx=Effects::default();
        fx.spawn(&world,CreatureKind::StoneGolem,Vec3::new(8.0,25.0,8.0));
        fx.update(PICKUP_DELAY,true);
        let feet=Vec3::from_array(fx.drops[0].pos);
        let mut account=crate::crafting::Account::default();
        let iron=crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b|*b==BlockType::IronOre).unwrap();
        account.resources[iron]=u32::MAX;
        let before=account.clone();
        assert!(fx.collect(&world,feet,&mut account).is_empty());
        assert_eq!(account,before);
        assert_eq!(fx.drops.len(),1);
        let mesh = fx.mesh(|_|true);
        assert!(mesh.vertices.iter().any(|v|v.emission==0.0)); // Normally lit bags.
        assert!(mesh.vertices.iter().any(|v|v.emission>0.0)); // Pickup sparkles.
        account.resources[iron]=0;
        assert_eq!(fx.collect(&world,feet,&mut account),rewards(CreatureKind::StoneGolem));
        let mut other=crate::crafting::Account::default();
        let before=other.clone();
        assert!(fx.collect(&world,feet,&mut other).is_empty());
        assert_eq!(other,before);
    }
    #[test]
    fn drops_find_sloping_ground_and_fall_back_at_loaded_edges() {
        let mut world=World::new(1);
        let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
        // The scatter column is three blocks above the creature's feet.
        for y in 0..=27 { chunk.set_local(9,y,8,BlockType::Stone); }
        chunk.set_local(8,24,8,BlockType::Stone);
        world.chunks.insert((0,0),chunk);
        assert_eq!(landing_position(&world,Vec3::new(8.0,25.0,8.0),0.0),Some([9.5,28.3,8.5]));
        let mut edge=crate::voxel::chunk::Chunk::new(1,0);
        edge.set_local(0,24,0,BlockType::Stone);
        world.chunks.insert((1,0),edge);
        assert_eq!(landing_position(&world,Vec3::new(16.0,25.0,0.0),std::f32::consts::PI),Some([16.5,25.3,0.5]));
    }
    #[test]
    fn nearby_killer_can_see_drops_before_automatic_pickup() {
        let mut world=World::new(1);
        let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
        for x in 0..16 { for z in 0..16 { chunk.set_local(x,24,z,BlockType::Stone); } }
        world.chunks.insert((0,0),chunk);
        let mut fx=Effects::default();
        fx.spawn(&world,CreatureKind::Zombie,Vec3::new(8.0,25.0,8.0));
        let feet=Vec3::from_array(fx.drops[0].pos);
        let mut account=crate::crafting::Account::default();
        let before=account.clone();
        fx.update(1.0,true);
        fx.collect(&world,feet,&mut account);
        assert_eq!(fx.drops.len(),1);
        assert_eq!(account,before);
        assert!(!fx.mesh(|_|true).indices.is_empty());
        fx.update(1.0,true);
        fx.collect(&world,feet,&mut account);
        assert!(fx.drops.is_empty());
        assert_ne!(account,before);
    }
    #[test]
    fn every_drop_has_an_inventory_stack_texture_and_elemental_composition() {
        let registry=crate::crafting::Registry::parse(include_str!("../data/crafting.json")).unwrap();
        for kind in 0..=12 {
            for (block,amount) in rewards(CreatureKind::from_u8(kind)) {
                assert!(crate::voxel::COLLECTIBLE_BLOCKS.contains(&block));assert!(amount>0);
                assert_ne!(registry.composition(crate::crafting::ObjectKind::Resource,block.id()),[0;5]);
                let uv=atlas::uv_rect(atlas::tile_for(block,0));assert!(uv.iter().all(|v|v.is_finite()));
            }
        }
    }
    #[test]
    fn scattered_loot_awards_once_preserves_full_inventory_and_expires() {
        let mut world=World::new(1);let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
        for x in 0..16 {for z in 0..16 {chunk.set_local(x,24,z,BlockType::Stone);}}
        world.chunks.insert((0,0),chunk);
        let mut fx=Effects::default();fx.spawn(&world,CreatureKind::StoneGolem,Vec3::new(8.0,25.0,8.0));
        assert_eq!(fx.drops.len(),1);assert_eq!(fx.drops[0].contents.len(),2);
        fx.update(PICKUP_DELAY,true);let mut account=crate::crafting::Account::default();
        for drop in fx.drops.clone() {fx.collect(&world,Vec3::from_array(drop.pos),&mut account);}
        assert!(fx.drops.is_empty());let before=account.clone();fx.collect(&world,Vec3::new(8.0,25.0,8.0),&mut account);assert_eq!(before,account);
        fx.spawn(&world,CreatureKind::Sheep,Vec3::new(8.0,25.0,8.0));fx.update(PICKUP_DELAY,true);account.resources.fill(u32::MAX);
        let pos=Vec3::from_array(fx.drops[0].pos);fx.collect(&world,pos,&mut account);assert!(!fx.drops.is_empty());
        for _ in 0..100 {fx.spawn(&world,CreatureKind::Sheep,pos);fx.puff(pos);}
        assert!(fx.drops.len()<=48 && fx.puffs.len()<=16);
        assert!(!fx.mesh(|_|true).vertices.is_empty());fx.update(121.0,true);assert!(fx.drops.is_empty() && fx.puffs.is_empty());
    }
}
impl Effects {
    pub fn puff(&mut self,pos:Vec3) {if self.puffs.len()>=16 {self.puffs.remove(0);}self.puffs.push((pos,0.0));}
    pub fn spawn(&mut self,world:&World,kind:CreatureKind,pos:Vec3) {
        if self.drops.len()>=48 {return;}
        let angle=pos.x*1.7+pos.z*0.8;
        let Some(pos)=landing_position(world,pos,angle) else {return;};
        self.drops.push(Drop{pos,contents:rewards(kind),age:0.0,cargo:Default::default(),machine:false,launch:None});
    }
    pub fn restore_machine(drops:Vec<Drop>)->Self {Self{drops,puffs:Vec::new()}}
    /// Return false without mutation if there is no room or clear throw path.
    pub fn eject(&mut self,world:&World,cell:crate::automation::Cell,item:&str,amount:u32,seed:u64)->bool {
        if self.drops.len()>=48 || amount==0 || amount>256 || !crate::automation::valid_item(item) || item.starts_with("creature:") {return false;}
        let start=crate::automation::center(cell)+Vec3::Y*0.75;
        for attempt in 0..8 {
            let angle=(seed%997) as f32*0.37+attempt as f32*std::f32::consts::FRAC_PI_4;
            let Some(pos)=landing_position(world,start,angle) else {continue;};
            let end=Vec3::from_array(pos);
            if glam::Vec2::new(end.x-start.x,end.z-start.z).length()<0.8 || end.distance(start)>5.0 {continue;}
            let mut drop=Drop{pos,contents:Vec::new(),age:0.0,cargo:Default::default(),machine:true,launch:Some(start.to_array())};
            if let Some(block)=item.strip_prefix("resource:").and_then(BlockType::from_name) {drop.contents.push((block,amount));}
            else {drop.cargo.insert(item.into(),amount);}
            let clear=(0..=30).all(|i| {
                drop.age=i as f32/30.0*0.85;let p=drop.display_position();
                [Vec3::ZERO,Vec3::X*0.18,-Vec3::X*0.18,Vec3::Z*0.18,-Vec3::Z*0.18,Vec3::Y*0.25].iter()
                    .all(|offset|{let p=p+*offset;!world.is_solid(p.x.floor() as i32,p.y.floor() as i32,p.z.floor() as i32)})
            });
            if clear {drop.age=0.0;self.drops.push(drop);return true;}
        }
        false
    }
    pub fn update(&mut self,dt:f32,host:bool) {
        for (_,age) in &mut self.puffs {*age+=dt;}
        self.puffs.retain(|(_,age)|*age<0.6);
        for d in &mut self.drops {d.age=(d.age+dt.max(0.0)).min(3600.0);}
        if host {self.drops.retain(|d|d.machine || d.age<120.0);}
    }
    pub fn collect(&mut self,world:&World,feet:Vec3,account:&mut crate::crafting::Account) -> Vec<(BlockType,u32)> {
        let mut acquired=Vec::new();
        self.drops.retain(|d| {
            let pos=Vec3::from_array(d.pos);let eye=feet+Vec3::Y*1.4;
            if d.age<PICKUP_DELAY || pos.distance(feet)>1.5 || crate::raycast::raycast(world,eye,pos-eye,(pos-eye).length()-0.25).is_some() {return true;}
            // Award the entire bundle atomically; overflow leaves it on the ground.
            let mut next=account.clone();
            for &(block,amount) in &d.contents {
                let Some(i)=crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b|*b==block) else {return true;};
                let Some(count)=next.resources[i].checked_add(amount) else {return true;};
                next.resources[i]=count;
            }
            for (item,n) in &d.cargo {if crate::automation::account_add(&mut next,item,*n).is_err(){return true;}}
            let Some(revision)=next.revision.checked_add(1)else{return true;};next.revision=revision;
            *account=next;
            acquired.extend_from_slice(&d.contents);
            false
        });
        acquired
    }
    pub fn mesh(&self,visible:impl Fn(Vec3)->bool)->MeshData {
        let mut mesh=MeshData{vertices:vec![],indices:vec![]};
        for d in &self.drops {
            let p=d.display_position();if !visible(p) {continue;}
            let bag=crate::model::loot_bag_mesh();
            let base=mesh.vertices.len() as u32;
            mesh.vertices.extend(bag.vertices.iter().map(|v| {
                let mut v=*v;v.position=(Vec3::from_array(v.position)+p).to_array();v
            }));
            mesh.indices.extend(bag.indices.iter().map(|i|base+i));
            let uv=atlas::uv_rect(crate::voxel::atlas_tiles::TILE_WHITE);
            // Only three tiny pixels per bag, staggered through an upward drift.
            let start=mesh.vertices.len();
            for i in 0..3 {
                let phase=(d.age*0.55+i as f32/3.0+p.x*0.13+p.z*0.19).rem_euclid(1.0);
                let angle=i as f32*2.4+phase*0.7;
                let center=p+Vec3::new(angle.cos()*0.18,0.18+phase*0.65,angle.sin()*0.18);
                let size=0.028*(std::f32::consts::PI*phase).sin();
                push_cuboid(&mut mesh.vertices,&mut mesh.indices,center-Vec3::splat(size),center+Vec3::splat(size),[1.0,0.68,0.12],uv);
            }
            for v in &mut mesh.vertices[start..] {v.emission=1.0;}
        }
        for (origin,age) in &self.puffs {
            if !visible(*origin) {continue;}
            let t=(age*12.0).floor()/12.0;
            for i in 0..12 {
                let a=i as f32*2.4;let p=*origin+Vec3::Y*0.9+Vec3::new(a.cos(),(i%3) as f32*0.5,a.sin())*(0.2+t*1.8);
                let size=0.12*(1.0-t/0.6).max(0.0);
                let start=mesh.vertices.len();
                push_cuboid(&mut mesh.vertices,&mut mesh.indices,p-Vec3::splat(size),p+Vec3::splat(size),[0.85;3],atlas::uv_rect(crate::voxel::atlas_tiles::TILE_WHITE));
                for v in &mut mesh.vertices[start..] {v.emission=0.7;}
            }
        }
        mesh
    }
}
