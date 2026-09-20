//! Bounded postconditions for simple spatial intentions, independent of generated Lua.
//! These fixtures prove the declared checks, not arbitrary natural-language fidelity.
use super::*;
use crate::{
    creature::Creatures,
    scripting::{Module, PlayerSnapshot, ScriptHost},
    spell_target::{Target, TargetContext},
    voxel::{BlockType, World, COLLECTIBLE_BLOCKS},
    weather::WeatherState,
};
use glam::Vec3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub action: String,
    pub material: String,
    pub scope: String,
    pub count: u32,
    pub radius: f32,
}
impl Check {
    pub fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,
        "required":["action","material","scope","count","radius"],"properties":{
            "action":{"type":"string","enum":["place_block","spawn_creature","remove_blocks"]},
            "material":{"type":"string"},
            "scope":{"type":"string","enum":["nearby","front","attached"]},
            "count":{"type":"integer","minimum":0,"maximum":8},
            "radius":{"type":"number","minimum":2,"maximum":10}
        }})
    }
    pub fn validate(&self, kind: PromptKind) -> Result<(), String> {
        let valid_material = if self.action == "spawn_creature" {
            super::SPECIES.contains(&self.material.as_str())
        } else {
            BlockType::from_name(&self.material).is_some_and(|b| {
                !matches!(b, BlockType::Air | BlockType::Bedrock | BlockType::Water)
            }) || self.action == "remove_blocks" && self.material == "poisonous"
        };
        if !["place_block", "spawn_creature", "remove_blocks"].contains(&self.action.as_str())
            || !["nearby", "front", "attached"].contains(&self.scope.as_str())
            || !valid_material
            || !self.radius.is_finite()
            || !(2.0..=10.0).contains(&self.radius)
            || self.count > 8
            || (self.action != "remove_blocks" && self.count == 0)
            || (self.scope == "attached"
                && (kind != PromptKind::Rule || self.action != "remove_blocks"))
            || (self.scope != "attached" && kind != PromptKind::Instant)
        {
            return Err(format!("Invalid spatial behavior check {self:?} for {kind:?}: use a real material/species, supported scope, count 1..8 (0 means all removals), radius 2..10; attached removal is a rule, other checks are instant"));
        }
        Ok(())
    }
    fn matches(&self, b: BlockType) -> bool {
        if self.material == "poisonous" {
            crate::resource_defs::definition(b).poisonous
        } else {
            b.id() == self.material
        }
    }
}

pub fn verify(code: &str, kind: PromptKind, checks: &[Check]) -> Result<(), String> {
    for check in checks {
        check.validate(kind)?;
        for facing in [
            Vec3::X,
            -Vec3::X,
            Vec3::Z,
            -Vec3::Z,
            Vec3::new(1., 0., 1.).normalize(),
        ] {
            run(code, kind, check, facing, false, false)?;
        }
        if check.action == "remove_blocks" {
            run(code, kind, check, Vec3::X, true, false)?;
        }
        if check.scope == "front" || check.scope == "nearby" {
            run(code, kind, check, Vec3::X, false, true)?;
        }
    }
    Ok(())
}

/// Repeat only the contracts needed by the declared checks near the generation request.
pub fn guidance(checks: &[Check]) -> String {
    let mut lines = Vec::new();
    for check in checks {
        match check.action.as_str() {
            "spawn_creature" => lines.push(format!("Spawn {} with api.spawn_creature({:?},x+0.5,y,z+0.5). At y=surface_height(x,z)+1 the destination is air and y-1 is SOLID support; requiring air below makes ground spawning impossible. No sky-exposure condition is needed.", check.material, check.material)),
            "place_block" if check.material != "campfire" => lines.push(format!("To place {:?}, call api.replace_block(x,y,z,{:?}), NOT api.place_block (which does not exist). Use y=surface_height(x,z)+1, check air and suitable support, and stop after {} successful placements.", check.material, check.material, check.count)),
            _ => {}
        }
        if check.scope == "nearby" {
            lines.push(format!("Nearby is a radius of {}, not a square. In any dx/dz search require dx*dx+dz*dz <= radius*radius. Use the caster's coordinates; optional aim is not required. Search/query before edits. A valid empty removal search can do nothing.", check.radius));
        }
        if check.scope == "front" {
            lines.push(format!("The declared radius is {}: use a forward offset of {} blocks (not a larger example distance), normalized in the horizontal plane. Require event.facing. Ground is one below the destination, not at the same Y.", check.radius, check.radius.min(3.0)));
        }
        if check.scope == "attached" {
            lines.push("Attached material checks: local t=api.get_rule_target(); if not t or t.kind~='block' then return end; local info=api.get_block_info(t.material); if not info then return end. get_block_info accepts block materials ONLY, not creature.kind. For toxicity check info.poisonous, never creature.poisoned. Guard unsupported creature/device targets without error.".into());
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(action: &str, material: &str, scope: &str) -> Check {
        Check {
            action: action.into(),
            material: material.into(),
            scope: scope.into(),
            count: if action == "remove_blocks" { 0 } else { 1 },
            radius: 6.,
        }
    }
    const FRONT: &str = r#"
        local p=api.get_player(event.player_id)
        if not event.facing then error('Aim at nearby ground first') end
        local f=event.facing
        local length=math.sqrt(f.x*f.x+f.z*f.z)
        if length<0.001 then error('Aim forward') end
        local x,z=math.floor(p.x+f.x/length*3),math.floor(p.z+f.z/length*3)
        local y=api.surface_height(x,z)
        if not y then error('Ground unavailable') end
    "#;
    #[test]
    fn placement_checks_reject_surface_off_by_one_noop_and_fixed_direction() {
        let checks = [check("place_block", "campfire", "front")];
        let good =
            format!("function on_cast(api,event) {FRONT} assert(api.place_campfire(x,y+1,z)) end");
        verify(&good, PromptKind::Instant, &checks).unwrap();
        for broken in [
            good.replace("y+1", "y"),
            "function on_cast(api,event) api.broadcast('No suitable ground') end".into(),
            good.replace("p.x+f.x/length*3", "p.x+3")
                .replace("p.z+f.z/length*3", "p.z"),
            good.replace(
                "if not event.facing then error('Aim at nearby ground first') end",
                "if not event.facing then return end",
            ),
        ] {
            assert!(
                verify(&broken, PromptKind::Instant, &checks).is_err(),
                "{broken}"
            );
        }
        let nearby = [check("place_block", "campfire", "nearby")];
        verify(
            "function on_cast(api,e) assert(api.place_campfire_near_player(e.player_id,6)) end",
            PromptKind::Instant,
            &nearby,
        )
        .unwrap();
    }
    #[test]
    fn herbs_and_creature_checks_measure_counts_direction_and_caster() {
        let herbs = [check("place_block", "wild_herbs", "front")];
        let good = format!("function on_cast(api,event) {FRONT} assert(api.replace_block(x,y+1,z,'wild_herbs')) end");
        verify(&good, PromptKind::Instant, &herbs).unwrap();
        assert!(verify(&good.replace("y+1", "y"), PromptKind::Instant, &herbs).is_err());
        let chickens = [check("spawn_creature", "chicken", "front")];
        let good = format!("function on_cast(api,event) {FRONT} assert(api.spawn_creature('chicken',x+0.5,y+1,z+0.5)) end");
        verify(&good, PromptKind::Instant, &chickens).unwrap();
        assert!(verify(
            &good.replace("event.player_id", "0"),
            PromptKind::Instant,
            &chickens
        )
        .is_err());
        let logged = "function on_cast(api,e) local p=api.get_player(e.player_id);local dx=0;if p.facing then dx=p.facing.x end;api.spawn_creature('chicken',p.x+dx,p.y,p.z) end";
        assert!(verify(logged, PromptKind::Instant, &chickens).is_err());
    }
    #[test]
    fn removal_checks_preserve_harmless_and_out_of_range_plants() {
        let checks = [check("remove_blocks", "poisonous", "nearby")];
        let good = r#"function on_cast(api,e)
            local p=api.get_player(e.player_id)
            local candidates=api.find_blocks('plant',math.floor(p.x),math.floor(p.y),math.floor(p.z),6)
            local matches={}
            for _,b in ipairs(candidates) do
                if api.get_block_info(api.get_block(b.x,b.y,b.z)).poisonous then table.insert(matches,b) end
            end
            for _,b in ipairs(matches) do assert(api.replace_block(b.x,b.y,b.z,'air')) end
        end"#;
        verify(good, PromptKind::Instant, &checks).unwrap();
        for broken in [
            good.replace(".poisonous", ".edible"),
            good.replace("'plant'", "'poison_ivy'"),
            good.replace(".poisonous", ".poisoned"),
            good.replace(",6)", ",10)"),
        ] {
            assert!(
                verify(&broken, PromptKind::Instant, &checks).is_err(),
                "{broken}"
            );
        }
    }
    #[test]
    fn attachment_checks_require_matching_block_branch_and_preserve_nonmatches() {
        let checks = [check("remove_blocks", "poisonous", "attached")];
        let good = "function on_tick(api) local t=api.get_rule_target();if t and t.kind=='block' and api.get_block_info(t.material).poisonous then assert(api.replace_block(t.x,t.y,t.z,'air')) end end";
        verify(good, PromptKind::Rule, &checks).unwrap();
        for broken in [good.replace(".poisonous", ".poisoned"),
            good.replace(" and api.get_block_info(t.material).poisonous", ""),
            "function on_tick(api) local t=api.get_rule_target();if t and t.kind=='creature' then api.destroy(t.id) end end".into()] {
            assert!(verify(&broken, PromptKind::Rule, &checks).is_err());
        }
    }
}

fn run(
    code: &str,
    kind: PromptKind,
    check: &Check,
    facing: Vec3,
    negative: bool,
    missing_context: bool,
) -> Result<(), String> {
    let fail = |reason: &str| {
        format!("Behavior check {}/{}/{} (facing {facing:?}, negative={negative}, missing_context={missing_context}): {reason}", check.action, check.material, check.scope)
    };
    let mut world = World::new(1);
    for cx in -1..=1 {
        for cz in -1..=1 {
            let mut chunk = crate::voxel::chunk::Chunk::new(cx, cz);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set_local(x, 24, z, BlockType::Grass);
                }
            }
            world.chunks.insert((cx, cz), chunk);
        }
    }
    let pos = Vec3::new(8.5, 25., 8.5);
    // Guest ID and a distant host catch hard-coded player 0 and array-index assumptions.
    let player = |id, pos| PlayerSnapshot {
        id,
        pos,
        finances: crate::scripting::InventoryBalances {
            mana: 100,
            ..Default::default()
        },
        resources: [1; COLLECTIBLE_BLOCKS.len()],
        carrying_crystal: false,
        velocity: Vec3::ZERO,
        on_ground: true,
        sprinting: false,
        in_water: false,
        health: 100.,
        poisoned: false,
        speed_multiplier: 1.,
        jump_multiplier: 1.,
        oxygen: 100.,
    };
    let players = [player(0, Vec3::new(-8.5, 25., -8.5)), player(7, pos)];
    let mut expected = Vec::new();
    let mut decoys = Vec::new();
    if check.action == "remove_blocks" {
        let material = if check.material == "poisonous" {
            COLLECTIBLE_BLOCKS
                .iter()
                .copied()
                .find(|b| check.matches(*b))
                .ok_or_else(|| fail("No matching catalog material"))?
        } else {
            BlockType::from_name(&check.material).unwrap()
        };
        // Several candidates, including one behind the caster, exercise radius/scope.
        for offset in [facing, -facing, Vec3::new(-facing.z, 0., facing.x)] {
            let p = (pos + offset * 1.5).floor().as_ivec3();
            let cell = (p.x, 25, p.z);
            let included = !negative && (check.scope == "nearby" || offset == facing);
            let b = if included {
                material
            } else {
                if check.material == "poisonous" {
                    BlockType::BrownMushroom
                } else if material == BlockType::Bluebell {
                    BlockType::Flax
                } else {
                    BlockType::Bluebell
                }
            };
            world.set_block(cell.0, cell.1, cell.2, b);
            if included {
                expected.push(cell);
            } else {
                decoys.push((cell, b));
            }
        }
        let p = (pos + facing * (check.radius + 2.)).floor().as_ivec3();
        world.set_block(p.x, 25, p.z, material);
        decoys.push(((p.x, 25, p.z), material));
        // An edible but harmless plant distinguishes toxicity from edibility.
        let harmless = if material == BlockType::BrownMushroom {
            BlockType::WildHerbs
        } else {
            BlockType::BrownMushroom
        };
        world.set_block(8, 25, 8, harmless);
        decoys.push(((8, 25, 8), harmless));
    }
    let mut creatures = Creatures::new();
    let mut host = ScriptHost::new();
    host.modules.push(Module::load(
        "behavior_check".into(),
        String::new(),
        code.into(),
    )?);
    host.modules[0].enabled = true;
    if check.scope == "attached" {
        let p = (pos + facing * 1.5).floor().as_ivec3();
        let cell = (p.x, 25, p.z);
        let target = Target::Block {
            position: cell,
            material: world.get_block(cell.0, cell.1, cell.2),
        };
        host.attach_at(
            0,
            crate::enchantment::Binding {
                source_spell: None,
                owner: String::new(),
                id: 1,
                creator: 7,
                target: crate::enchantment::Reference::capture(&mut world, &creatures, target)?,
                lost: None,
            },
        )?;
    }
    let mut time = 0.25;
    let mut weather = WeatherState::new(1);
    let outcome = if kind == PromptKind::Rule {
        host.run_tick(
            &world,
            &mut creatures,
            &players,
            &mut time,
            &mut weather,
            &[],
            &[],
            [1; COLLECTIBLE_BLOCKS.len()],
        )
    } else if missing_context {
        host.run_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time,
            &mut weather,
            7,
            [1; COLLECTIBLE_BLOCKS.len()],
        )
    } else {
        let context = TargetContext::resolve(
            &world,
            &creatures,
            pos + Vec3::Y * 1.62,
            facing - Vec3::Y * 0.6,
        )
        .ok_or_else(|| fail("Fixture could not resolve aim"))?;
        host.run_targeted_cast(
            0,
            &world,
            &mut creatures,
            &players,
            &mut time,
            &mut weather,
            7,
            [1; COLLECTIBLE_BLOCKS.len()],
            context,
        )
    };
    let spawned = creatures.snapshot_with_ids();
    if missing_context && check.scope == "front" {
        if !outcome.block_edits.is_empty()
            || !spawned.is_empty()
            || !outcome.player_effects.is_empty()
            || time != 0.25
        {
            return Err(fail(
                "Missing aim must not silently cast in a fixed direction",
            ));
        }
        if outcome.crashes.is_empty() && outcome.broadcasts.is_empty() {
            return Err(fail("Missing aim must report a clear failure"));
        }
        return Ok(());
    }
    if !outcome.crashes.is_empty() {
        return Err(fail(&outcome.crashes.join("; ")));
    }
    if !outcome.warnings.is_empty() {
        return Err(fail(&outcome.warnings.join("; ")));
    }
    let within_scope = |p: Vec3| {
        let delta = p - pos;
        delta.length() <= check.radius + 0.75
            && (check.scope != "front" || {
                let horizontal = Vec3::new(delta.x, 0., delta.z);
                horizontal.dot(facing) > 0.5 && horizontal.normalize_or_zero().dot(facing) >= 0.7
            })
    };
    match check.action.as_str() {
        "place_block" => {
            let placed: Vec<_> = outcome
                .block_edits
                .iter()
                .filter(|(_, _, _, b)| check.matches(*b))
                .collect();
            if placed.len() != check.count as usize {
                return Err(fail(&format!(
                    "Expected {} placed {}, observed {}",
                    check.count,
                    check.material,
                    placed.len()
                )));
            }
            for &&(x, y, z, _) in &placed {
                if y != 25
                    || world.get_block(x, y, z) != BlockType::Air
                    || !within_scope(Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5))
                {
                    return Err(fail(&format!("Placement at ({x},{y},{z}) must be above support Y=24 and within radius {} of caster (8.5,25,8.5), with the requested direction. A square search corner is outside its radius; use dx*dx+dz*dz<=radius*radius.", check.radius)));
                }
            }
        }
        "spawn_creature" => {
            if spawned.len() != check.count as usize
                || spawned.iter().any(|(_, species, p, _, _)| {
                    crate::scripting::creature_kind_name(*species) != check.material
                        || !within_scope(Vec3::from_array(*p))
                })
            {
                return Err(fail("Expected the requested creature count/species in the requested direction and radius"));
            }
        }
        "remove_blocks" => {
            let edits = &outcome.block_edits;
            let removed = expected
                .iter()
                .filter(|&&(x, y, z)| edits.iter().any(|e| *e == (x, y, z, BlockType::Air)))
                .count();
            let wanted = if negative {
                0
            } else if check.count == 0 {
                expected.len()
            } else {
                (check.count as usize).min(expected.len())
            };
            if removed != wanted
                || edits
                    .iter()
                    .any(|&(x, y, z, b)| b != BlockType::Air || !expected.contains(&(x, y, z)))
            {
                return Err(fail(&format!("Expected {wanted} matching removals with unrelated blocks preserved; observed {removed}")));
            }
            for ((x, y, z), b) in decoys {
                if edits
                    .iter()
                    .any(|e| e.0 == x && e.1 == y && e.2 == z && e.3 != b)
                {
                    return Err(fail("Unrelated or out-of-range block changed"));
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
