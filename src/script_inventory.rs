//! Inventory capabilities reuse the game's crafting formulas and private callback state.
use super::*;
use super::transaction::CallbackTransaction;
use crate::crafting::{Action,Element,Registry};
use crate::equipment::Gear;

pub(super) fn gear(id: &str) -> Option<Gear> {
    match id {"axe"=>Some(Gear::Axe),"pickaxe"=>Some(Gear::Pickaxe),"sword"=>Some(Gear::Sword),_=>None}
}
fn element(id: &str) -> Option<Element> {
    Element::ALL.into_iter().find(|e|format!("{e:?}").eq_ignore_ascii_case(id))
}
pub(super) fn equipment_change(tx: &CallbackTransaction, id: PlayerId, gear: Gear, amount: u32, take: bool) -> bool {
    if amount==0 || amount>MAX_ITEM_GRANT_AMOUNT {return false;}
    let Some(mut account)=tx.inventory(id) else {return false;};
    let n=&mut account.gear[gear as usize];
    let next=if take {n.checked_sub(amount)} else {n.checked_add(amount)};
    let Some(next)=next else {return false;};*n=next;
    tx.stage_inventory(id,&account);true
}
fn balance_table<'lua>(lua: &'lua Lua, values: &[u32;5]) -> mlua::Result<Table<'lua>> {
    let table=lua.create_table()?;
    for e in Element::ALL {table.set(format!("{e:?}").to_lowercase(),values[e.index()])?;}
    Ok(table)
}
pub(super) fn populate<'lua,'scope>(_lua: &'lua Lua, scope: &mlua::Scope<'lua,'scope>, api: &Table<'lua>, tx: &'scope CallbackTransaction) -> mlua::Result<()> where 'lua:'scope {
    api.set("instant_mana_cost",crate::crafting::INSTANT_MANA)?;
    api.set("rule_mana_cost",crate::crafting::RULE_MANA)?;
    api.set("mana_regen_cap",crate::crafting::MANA_REGEN_CAP)?;
    api.set("get_player_inventory",scope.create_function(move |lua,id:PlayerId| {
        let Some(a)=tx.inventory(id) else {return Ok(None);};
        let result=lua.create_table()?;let resources=lua.create_table()?;let items=lua.create_table()?;
        for (i,b) in COLLECTIBLE_BLOCKS.iter().enumerate() {resources.set(b.id(),a.resources[i])?;}
        for (id,g) in [("axe",Gear::Axe),("pickaxe",Gear::Pickaxe),("sword",Gear::Sword)] {items.set(id,a.gear[g as usize])?;}
        result.set("resources",resources)?;result.set("items",items)?;result.set("elements",balance_table(lua,&a.elements)?)?;result.set("mana",a.mana)?;
        Ok(Some(result))
    })?)?;
    api.set("get_mana",scope.create_function(move |_,id:PlayerId|Ok(tx.inventory(id).map(|a|a.mana)))?)?;
    api.set("get_element_count",scope.create_function(move |_,(id,name):(PlayerId,String)|Ok(element(&name).and_then(|e|tx.inventory(id).map(|a|a.elements[e.index()]))))?)?;
    api.set("get_item_count",scope.create_function(move |_,(id,name):(PlayerId,String)| {
        Ok(tx.inventory(id).and_then(|a| if let Some(g)=gear(&name) {Some(a.gear[g as usize])} else {
            BlockType::from_name(&name).and_then(|b|COLLECTIBLE_BLOCKS.iter().position(|v|*v==b)).map(|i|a.resources[i])
        }))
    })?)?;
    for name in ["give_mana","take_mana"] {
        api.set(name,scope.create_function(move |_,(id,amount):(PlayerId,u32)| {
            if amount==0 || amount>MAX_ITEM_GRANT_AMOUNT {return Ok(false);}
            let Some(mut a)=tx.inventory(id) else {return Ok(false);};
            let next=if name=="take_mana" {a.mana.checked_sub(amount)} else {a.mana.checked_add(amount)};
            let Some(next)=next else {return Ok(false);};a.mana=next;tx.stage_inventory(id,&a);Ok(true)
        })?)?;
    }
    for name in ["give_element","take_element"] {
        api.set(name,scope.create_function(move |_,(id,e,amount):(PlayerId,String,u32)| {
            if amount==0 || amount>MAX_ITEM_GRANT_AMOUNT {return Ok(false);}
            let (Some(mut a),Some(e))=(tx.inventory(id),element(&e)) else {return Ok(false);};
            let n=&mut a.elements[e.index()];
            let next=if name=="take_element" {n.checked_sub(amount)} else {n.checked_add(amount)};
            let Some(next)=next else {return Ok(false);};*n=next;tx.stage_inventory(id,&a);Ok(true)
        })?)?;
    }
    for name in ["craft_item","decompose_item","decompose_resource","convert_elements_to_mana"] {
        api.set(name,scope.create_function(move |lua,(id,kind,amount):(PlayerId,String,Option<u32>)| {
            let amount=amount.unwrap_or(1);
            if amount==0 || amount>MAX_ITEM_GRANT_AMOUNT {return Ok(false);}
            let Some(mut a)=tx.inventory(id) else {return Ok(false);};
            let action=match name {
                "craft_item" | "decompose_item" => {
                    if amount!=1 {return Ok(false);}
                    let Some(g)=gear(&kind) else {return Ok(false);};
                    if name=="craft_item" {Action::CraftGear(g)} else {Action::SalvageGear(g)}
                }
                "decompose_resource" => {let Some(block)=BlockType::from_name(&kind) else {return Ok(false);};Action::Extract{block,amount}}
                _=> {let Some(element)=element(&kind) else {return Ok(false);};Action::Convert{element,amount:amount as i64}}
            };
            let registry=lua.app_data_ref::<std::sync::Arc<Registry>>().unwrap();
            if registry.prepare(&mut a,&action).is_err() {return Ok(false);}
            tx.stage_inventory(id,&a);Ok(true)
        })?)?;
    }
    api.set("get_resource_elements",scope.create_function(move |lua,id:String| {
        let Some(block)=BlockType::from_name(&id).filter(|b|COLLECTIBLE_BLOCKS.contains(b)) else {return Ok(None);};
        let registry=lua.app_data_ref::<std::sync::Arc<Registry>>().unwrap();
        Ok(Some(balance_table(lua,&registry.composition(crate::crafting::ObjectKind::Resource,block.id()))?))
    })?)?;
    api.set("get_item_recipe",scope.create_function(move |lua,id:String| {
        let Some(g)=gear(&id) else {return Ok(None);};
        let result=lua.create_table()?;
        for (label,salvage) in [("create",false),("decompose",true)] {
            let (iron,wood,mana)=crate::crafting::gear_formula(g,salvage).unwrap();
            let part=lua.create_table()?;let resources=lua.create_table()?;
            resources.set("iron",iron)?;resources.set("oak_wood",wood)?;part.set("resources",resources)?;part.set("mana",mana)?;result.set(label,part)?;
        }
        Ok(Some(result))
    })?)?;
    Ok(())
}
