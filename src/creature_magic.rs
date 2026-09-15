//! Bounded creature displacement and timed status, separate from permanent behavior overrides.
use super::*;
use crate::voxel::BlockType;
#[derive(Clone,Copy,Debug,Default,serde::Serialize,serde::Deserialize)]
pub struct Status {pub slow:f32,pub stun:f32}
impl Status {
    pub fn valid(&self)->bool {[self.slow,self.stun].iter().all(|t|t.is_finite()&&(0.0..=30.0).contains(t))}
    pub fn active(&self)->bool {self.slow>0. || self.stun>0.}
    pub fn rate(&self)->f32 {if self.stun>0. {0.}else if self.slow>0. {0.5}else{1.}}
    pub fn tick(&mut self,dt:f32) {self.slow=(self.slow-dt).max(0.);self.stun=(self.stun-dt).max(0.);}
}
impl CreatureDraft {
    pub fn apply_status(&mut self,id:u32,name:&str,seconds:f32)->bool {
        if !seconds.is_finite() || !(0.0..=30.0).contains(&seconds) || !self.snapshot.iter().any(|c|c.0==id) {return false;}
        let mut status=self.magic_statuses.get(&id).copied().unwrap_or_default();
        match name {"slow"=>status.slow=seconds,"stun"=>status.stun=seconds,_=>return false}
        self.magic_statuses.insert(id,status);self.commands.push(CreatureCommand::Status(id,status));true
    }
    /// Follow the whole requested segment in <=0.2-block steps; stop at the first obstruction.
    /// Uses staged block reads supplied by the transaction, including loaded-chunk checks.
    pub fn push(&mut self,id:u32,delta:Vec3,clear_block:impl Fn(i32,i32,i32)->Option<BlockType>)->bool {
        if !delta.is_finite() || delta.length()>8. || delta.length()<0.001 {return false;}
        let Some(entry)=self.snapshot.iter_mut().find(|c|c.0==id) else{return false;};
        let kind=CreatureKind::from_u8(entry.1);
        // Large flying creatures need their own flight/landing rules.
        if kind.is_dragon() {return false;}
        let start=Vec3::from_array(entry.2);let mut end=start;
        let (radius,height)=collision::body(kind);
        let steps=(delta.length()/0.2).ceil() as u32;
        for step in 1..=steps {
            let next=start+delta*(step as f32/steps as f32);
            if !next.is_finite() || next.abs().max_element()>1_000_000. {break;}
            let bottom=if kind==CreatureKind::Fish {next.y-height*0.5}else{next.y};
            let mut clear=true;
            for x in (next.x-radius).floor() as i32..=(next.x+radius-0.001).floor() as i32 {
                for z in (next.z-radius).floor() as i32..=(next.z+radius-0.001).floor() as i32 {
                    for y in (bottom+0.001).floor() as i32..=(bottom+height-0.001).floor() as i32 {
                        if !clear_block(x,y,z).is_some_and(|b|!b.is_solid()) {clear=false;}
                    }
                }
            }
            if kind==CreatureKind::Fish && clear_block(next.x.floor() as i32,next.y.floor() as i32,next.z.floor() as i32)!=Some(BlockType::Water) {clear=false;}
            if !clear {break;}
            end=next;
        }
        if end.distance_squared(start)<0.000001 {return false;}
        entry.2=end.to_array();self.commands.push(CreatureCommand::Move(id,end));true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene()->(World,Creatures,u32) {
        let mut world=World::new(1);let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
        for x in 0..16 {for z in 0..16 {chunk.set_local(x,24,z,BlockType::Stone);}}
        world.chunks.insert((0,0),chunk);
        let mut c=Creatures::new();let id=c.spawn_one(CreatureKind::Sheep,Vec3::new(5.,25.,5.),1);
        c.set_chase_target(id,Vec3::new(12.,25.,5.));(world,c,id)
    }
    #[test]
    fn slow_changes_motion_stun_expires_and_saved_status_retains_duration() {
        let (world,mut normal,id)=scene();let (_,mut slowed,_)=scene();
        slowed.magic_statuses.insert(id,Status{slow:10.,stun:0.});
        normal.update(&world,0.1,&[]);slowed.update(&world,0.1,&[]);
        let normal_dx=normal.snapshot_with_ids()[0].2[0]-5.;let slow_dx=slowed.snapshot_with_ids()[0].2[0]-5.;
        assert!(normal_dx>0. && (normal_dx*0.5-slow_dx).abs()<0.001);
        let original=slowed.snapshot_with_ids()[0].2;
        slowed.magic_statuses.get_mut(&id).unwrap().stun=0.2;
        slowed.update(&world,0.1,&[]);assert_eq!(slowed.snapshot_with_ids()[0].2,original);
        let save=crate::save::CraftingSave {creature_statuses:slowed.magic_statuses.clone(),..Default::default()};
        let saved:crate::save::CraftingSave=serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
        assert!((saved.creature_statuses[&id].stun-0.1).abs()<0.001);
        slowed.update(&world,0.1,&[]);assert_eq!(slowed.magic_statuses[&id].stun,0.);
        slowed.update(&world,0.1,&[]);assert!(slowed.snapshot_with_ids()[0].2[0]>original[0]);
        slowed.update(&world,30.,&[]);assert!(slowed.magic_statuses.is_empty());
    }
    #[test]
    fn pushes_reject_unloaded_space_and_nonfinite_or_excessive_displacements() {
        let (_,c,id)=scene();let mut draft=CreatureDraft::new(&c);
        let before=draft.snapshot.clone();
        assert!(!draft.push(id,Vec3::X,|_,_,_|None));
        assert!(!draft.push(id,Vec3::NAN,|_,_,_|Some(BlockType::Air)));
        assert!(!draft.push(id,Vec3::X*9.,|_,_,_|Some(BlockType::Air)));
        assert_eq!(draft.snapshot,before);
    }
}
