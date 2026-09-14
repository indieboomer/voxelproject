//! Bounded world shaping and material/environment queries. All reads include
//! staged edits; all writes remain in the existing callback transaction.
use super::*;
use crate::voxel::chunk::{world_to_chunk, CHUNK_Y};

pub(super) enum BlockFilter { Exact(BlockType), Any, Wood, Ore, Leaves, Plant, Solid, Liquid }
impl BlockFilter {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "any" => Self::Any, "wood" => Self::Wood, "ore" => Self::Ore,
            "leaves" => Self::Leaves, "plant" => Self::Plant,
            "solid" => Self::Solid, "liquid" => Self::Liquid,
            _ => Self::Exact(BlockType::from_name(name)?),
        })
    }
    pub fn matches(&self, block: BlockType) -> bool {
        match self {
            Self::Exact(b) => *b == block, Self::Any => true,
            Self::Wood => block.is_wood(), Self::Ore => block.id().ends_with("_ore") || block == BlockType::Coal,
            Self::Leaves => block.id().ends_with("_leaves"),
            Self::Plant => (block.def().cross || block.def().only_on_top) && block != BlockType::Campfire,
            Self::Solid => block.is_solid(), Self::Liquid => block == BlockType::Water,
        }
    }
}

fn loaded(tx: &CallbackTransaction, x: i32, y: i32, z: i32) -> bool {
    (0..CHUNK_Y).contains(&y) && tx.world.chunks.contains_key(&world_to_chunk(x,z))
}
fn scan_charge(lua: &Lua, tx: &CallbackTransaction, cells: u32) {
    lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap()
        .charge_native_work(cells.saturating_mul(1 + tx.blocks.borrow().len() as u32));
}

fn fill(lua: &Lua, tx: &CallbackTransaction, budget: &Cell<u32>, a: [i32;3], b: [i32;3],
    sphere: Option<([i32;3],f32)>, kind: String, filter: Option<String>) -> mlua::Result<Option<u32>> {
    let Some(block) = BlockType::from_name(&kind) else { return Ok(None); };
    let Some(filter) = BlockFilter::parse(filter.as_deref().unwrap_or("any")) else { return Ok(None); };
    let lo = std::array::from_fn::<_,3,_>(|i| a[i].min(b[i]));
    let hi = std::array::from_fn::<_,3,_>(|i| a[i].max(b[i]));
    let volume = (0..3).fold(1u64, |n,i| n.saturating_mul((hi[i] as i64-lo[i] as i64+1) as u64));
    if volume > 4096 || lo[1] < 0 || hi[1] >= CHUNK_Y || block == BlockType::Bedrock
        || lo.iter().chain(hi.iter()).any(|&v| v.unsigned_abs() > MAX_API_COORDINATE as u32) { return Ok(None); }
    scan_charge(lua,tx,volume as u32);
    let mut edits = Vec::new();
    for x in lo[0]..=hi[0] { for y in lo[1]..=hi[1] {
        lua.app_data_ref::<Rc<ExecutionBudget>>().unwrap().check();
        for z in lo[2]..=hi[2] {
            if let Some((c,r)) = sphere {
                if ((x-c[0]).pow(2)+(y-c[1]).pow(2)+(z-c[2]).pow(2)) as f32 > r*r { continue; }
            }
            if !loaded(tx,x,y,z) { return Ok(None); }
            let old = tx.get_block(x,y,z);
            if old == block || !filter.matches(old) { continue; }
            if old == BlockType::Bedrock || old == BlockType::AutomationDevice { return Ok(None); }
            edits.push((x,y,z,block));
            if edits.len() > budget.get() as usize { return Ok(None); }
        }
    }}
    let changed = edits.len() as u32;
    budget.set(budget.get()-changed);
    tx.blocks.borrow_mut().extend(edits);
    Ok(Some(changed))
}

pub(super) fn populate<'lua,'scope>(_lua: &'lua Lua, scope: &mlua::Scope<'lua,'scope>,
    api: &Table<'lua>, tx: &'scope CallbackTransaction, blocks: &'scope Cell<u32>) -> mlua::Result<()> {
    api.set("get_player",scope.create_function(move |lua,id:u32| {
        let Some(p) = tx.players().into_iter().find(|p|p.id==id) else { return Ok(None); };
        let t=lua.create_table()?; set_player_fields(&t,&p)?; Ok(Some(t))
    })?)?;
    api.set("get_creature",scope.create_function(move |lua,id:u32| {
        let creatures=tx.creatures.borrow();
        let Some(&(_,kind,pos,health,max_health))=creatures.snapshot.iter().find(|c|c.0==id) else {return Ok(None);};
        let t=lua.create_table()?;
        t.set("id",id)?;t.set("kind",creature_kind_name(kind))?;
        t.set("x",pos[0])?;t.set("y",pos[1])?;t.set("z",pos[2])?;
        t.set("health",health)?;t.set("max_health",max_health)?;
        super::environment::creature_fields(&t,tx,kind,pos)?;
        Ok(Some(t))
    })?)?;
    api.set("heal_creature",scope.create_function(move |_,(id,amount):(u32,f32)| {
        Ok(tx.creatures.borrow_mut().heal(id,amount))
    })?)?;
    api.set("get_block_kinds",scope.create_function(move |lua,()| {
        scan_charge(lua,tx,crate::world_api_gen::BLOCK_KINDS.len() as u32);
        lua.create_sequence_from(crate::world_api_gen::BLOCK_KINDS.iter().copied())
    })?)?;
    api.set("get_block_info",scope.create_function(move |lua,kind:String| {
        scan_charge(lua,tx,COLLECTIBLE_BLOCKS.len() as u32);
        let Some(b)=BlockType::from_name(&kind) else {return Ok(None);};
        let d=b.def();let t=lua.create_table()?;
        t.set("kind",b.id())?;t.set("name",d.display_name)?;t.set("resource_type",d.resource_type)?;
        t.set("solid",b.is_solid())?;t.set("opaque",b.is_opaque())?;
        t.set("collectible",COLLECTIBLE_BLOCKS.contains(&b))?;
        t.set("hardness",d.hardness)?;t.set("emission",d.emission)?;
        t.set("ore",BlockFilter::Ore.matches(b))?;Ok(Some(t))
    })?)?;
    api.set("block_matches",scope.create_function(move |_,(kind,filter):(String,String)| {
        Ok(BlockType::from_name(&kind).zip(BlockFilter::parse(&filter)).is_some_and(|(b,f)|f.matches(b)))
    })?)?;
    api.set("is_block_loaded",scope.create_function(move |_,(x,y,z):(i32,i32,i32)| Ok(loaded(tx,x,y,z)))?)?;
    api.set("surface_height",scope.create_function(move |lua,(x,z):(i32,i32)| {
        if !loaded(tx,x,0,z) {return Ok(None);}
        scan_charge(lua,tx,CHUNK_Y as u32);
        Ok((0..CHUNK_Y).rev().find(|&y| tx.get_block(x,y,z).is_solid()))
    })?)?;
    api.set("is_exposed_to_sky",scope.create_function(move |lua,(x,y,z):(i32,i32,i32)| {
        if !loaded(tx,x,y,z) {return Ok(None);}
        scan_charge(lua,tx,(CHUNK_Y-y) as u32);
        Ok(Some((y..CHUNK_Y).all(|y| !tx.get_block(x,y,z).is_solid())))
    })?)?;
    api.set("fill_box",scope.create_function(move |lua,(x1,y1,z1,x2,y2,z2,kind,filter):
        (i32,i32,i32,i32,i32,i32,String,Option<String>)| {
        fill(lua,tx,blocks,[x1,y1,z1],[x2,y2,z2],None,kind,filter)
    })?)?;
    api.set("fill_sphere",scope.create_function(move |lua,(cx,cy,cz,radius,kind,filter):
        (i32,i32,i32,f32,String,Option<String>)| {
        if !(0.0..=7.0).contains(&radius) {return Ok(None);}
        let r=radius.ceil() as i32;
        fill(lua,tx,blocks,[cx-r,cy-r,cz-r],[cx+r,cy+r,cz+r],Some(([cx,cy,cz],radius)),kind,filter)
    })?)?;
    Ok(())
}
