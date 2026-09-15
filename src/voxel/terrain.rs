//! Continuous, seed-only terrain features. No chunk-local hydrology decisions.
use super::{
    chunk::TERRAIN_HEIGHT as CHUNK_Y,
    noise::{column_rand, fbm},
    world::SEA_LEVEL,
};

const RIVER_SPACING: f32 = 384.0;
const LAKE_SPACING: f32 = 336.0;
const MOUNTAIN_SPACING: f32 = 256.0;
/// Local fault faces, broken scree and narrow erosion cuts. Shore attenuation
/// preserves the connected lake/river beds; no chunk-local randomness is used.
pub fn geological_height(x:i32,z:i32,seed:u32,h:i32,relief:f32,version:u8)->i32 {
    // Version 2 gives each formation 2.5 times the horizontal space, while
    // retaining its vertical relief and softening the small rubble layer.
    let scale=if version>=2 {0.4}else{1.};
    let (x,z)=(x as f32*scale,z as f32*scale);
    let inland=((h-SEA_LEVEL-3) as f32/6.).clamp(0.,1.);
    let warp=fbm(x*0.023,z*0.023,seed^0x6E01,2,2.,0.5)*16.;
    let fault=fbm((x+warp)*0.032,(z-warp)*0.032,seed^0x6E02,2,2.,0.5);
    let ledge=((fault-0.5)*38.).clamp(-1.,1.)*3.5;
    let fissure=(1.-(fault-0.36).abs()/0.025).max(0.)*7.;
    let basin=(0.42-fbm(x*0.055,z*0.055,seed^0x6E03,2,2.,0.5)).max(0.)*20.;
    let rubble=(fbm(x*0.29,z*0.29,seed^0x6E04,2,2.,0.5)-0.5)*if version>=2 {1.0}else{3.0};
    (h as f32+inland*(ledge-fissure-basin+rubble)*relief.min(1.5)).round().clamp(2.,(CHUNK_Y-4) as f32) as i32
}

/// Undercut only exposed upland scarps, leaving a two-block roof and solid base.
pub fn undercut(x:i32,z:i32,seed:u32,h:i32,slope:i32,version:u8)->bool {
    let frequency=if version>=2 {0.034}else{0.085};
    h>SEA_LEVEL+9 && slope>=3 && fbm(x as f32*frequency,z as f32*frequency,seed^0x6E05,2,2.,0.5)>0.48
}

/// Jittered elliptical meadow colonies, with ragged edges. Each colony chooses
/// either a colorful mixture or an 88% dominant flower species.
pub fn meadow_plant(x:i32,z:i32,seed:u32)->Option<super::BlockType> {
    use super::BlockType;
    let palette=[BlockType::Lavender,BlockType::RedPoppy,BlockType::Clover,BlockType::Flax];
    let (cx,cz)=(x.div_euclid(48),z.div_euclid(48));
    for a in cx-1..=cx+1 {for b in cz-1..=cz+1 {
        if column_rand(a,b,seed,0xF100)>0.72 {continue;}
        let px=a as f32*48.+column_rand(a,b,seed,0xF101)*48.;
        let pz=b as f32*48.+column_rand(a,b,seed,0xF102)*48.;
        let rx=9.+column_rand(a,b,seed,0xF103)*18.;
        let rz=8.+column_rand(a,b,seed,0xF104)*15.;
        let d=((x as f32-px)/rx).powi(2)+((z as f32-pz)/rz).powi(2);
        if d>1.3 {continue;}
        let edge=fbm(x as f32*0.18,z as f32*0.18,seed^0xF105,2,2.,0.5)*0.5;
        if column_rand(x,z,seed,0xF106)>((1.15-d+edge)*0.55).clamp(0.,0.65) {continue;}
        let mixed=column_rand(a,b,seed,0xF107)<0.45 || column_rand(x,z,seed,0xF108)>0.88;
        let roll=if mixed {column_rand(x,z,seed,0xF109)}else{column_rand(a,b,seed,0xF110)};
        return Some(palette[(roll*4.) as usize%4]);
    }}
    None
}
pub const ROCK_LINE: i32 = 34;
pub const ALPINE_LINE: i32 = 32;
/// Coherent snow patches, bare steep faces, and occasional grassy ledges.
/// All decisions use world coordinates so neighboring chunks share the pattern.
#[cfg(test)]
pub fn mountain_surface(wx:i32,wz:i32,h:i32,seed:u32)->super::BlockType {
    use super::BlockType;
    if h<ALPINE_LINE {return BlockType::Grass;}
    let slope=[height(wx-1,wz,seed),height(wx+1,wz,seed),height(wx,wz-1,seed),height(wx,wz+1,seed)]
        .into_iter().map(|n|(n-h).abs()).max().unwrap_or(0);
    mountain_surface_with_slope(wx,wz,h,seed,slope)
}
pub(super) fn mountain_surface_with_slope(wx:i32,wz:i32,h:i32,seed:u32,slope:i32)->super::BlockType {
    use super::BlockType;
    if h<ALPINE_LINE {return BlockType::Grass;}
    let patch=fbm(wx as f32*0.075,wz as f32*0.075,seed^0x5A0F,3,2.0,0.5);
    let threshold=0.63-(h-ALPINE_LINE) as f32*0.022+(slope as f32*0.055).min(0.20);
    if patch>threshold {return BlockType::Snow;}
    if h<ROCK_LINE {return BlockType::Grass;}
    let grass=fbm(wx as f32*0.11,wz as f32*0.11,seed^0x5A10,2,2.0,0.5);
    if slope<=1 && grass>0.80 {BlockType::Grass}else{BlockType::Stone}
}

fn river_z(x: f32, row: i32, seed: u32) -> f32 {
    let phase = column_rand(row, 0, seed, 0xA710) * std::f32::consts::TAU;
    row as f32 * RIVER_SPACING
        + 140.0
        + (x * 0.013 + phase).sin() * 38.0
        + (x * 0.029 + phase * 1.7).sin() * 12.0
        + (x * 0.055 + phase * 2.3).sin() * 6.0
        + (fbm(x * 0.006, row as f32 * 13.0, seed ^ 0xA711, 2, 2.0, 0.5) - 0.5) * 55.0
}

fn lake_x(column: i32, row: i32, seed: u32) -> f32 {
    column as f32 * LAKE_SPACING
        + 140.0
        + (column_rand(column, row, seed, 0xA712) - 0.5) * 110.0
}
fn lake(column: i32, row: i32, seed: u32) -> (f32, f32, f32, f32) {
    let x = lake_x(column,row,seed);
    (
        x,
        river_z(x, row, seed),
        32.0 + column_rand(column, row, seed, 0xA713) * 20.0,
        24.0 + column_rand(column, row, seed, 0xA714) * 18.0,
    )
}

/// Sparse raised tributaries between lakes. World coordinates keep banks and
/// water levels identical across chunk boundaries and generation order.
/// Returns bed/bank height and local water level; the vertical spill is visual.
pub fn tributary(wx: i32, wz: i32, seed: u32) -> Option<(i32, i32, bool)> {
    let c = (wx as f32 / LAKE_SPACING).floor() as i32;
    let r = (wz as f32 / RIVER_SPACING).floor() as i32;
    for row in r-1..=r+1 { for column in c-1..=c+1 {
        if column_rand(column,row,seed,0xFA110) > 0.55 { continue; }
        let a = lake_x(column,row,seed);
        let b = lake_x(column+1,row,seed);
        let center = ((a+b)*0.5).round();
        let x = (wx as f32-center).abs();
        if x > 12.0 { continue; }
        let z = wz as f32-river_z(center,row,seed).round();
        if !(0.0..=43.0).contains(&z) { continue; }
        let high = SEA_LEVEL + 5 + (column_rand(column,row,seed,0xFA111)*4.0) as i32;
        // Lower receiving pool reaches back into the existing river.
        if z < 14.0 {
            let radius = if z < 4.0 { 3.0 } else { 5.0 };
            if x <= radius { return Some((SEA_LEVEL-3,SEA_LEVEL,false)); }
            continue;
        }
        let pool_distance = (x*x+(z-32.0).powi(2)).sqrt();
        if (x <= 2.0 && z <= 32.0) || pool_distance <= 7.0 {
            return Some((high-2,high,z<26.0));
        }
        // Solid raised rim contains the pool; a narrow opening faces the drop.
        let rim_distance = if z <= 32.0 { (x-2.0).max(0.0).min(pool_distance-7.0) } else { pool_distance-7.0 };
        if rim_distance < 5.0 && pool_distance < 12.0 || (z <= 32.0 && x < 7.0) {
            return Some((high+1-(rim_distance-2.0).max(0.0) as i32,SEA_LEVEL,false));
        }
    } }
    None
}

/// Analytic current tangent. Lake interiors and non-channel water have no drift.
#[cfg(test)]
pub fn current(wx: i32, wz: i32, seed: u32, elevated: bool) -> [f32;2] {
    current_with_lakes(wx,wz,seed,elevated,false)
}
pub fn current_with_lakes(wx: i32, wz: i32, seed: u32, elevated: bool,irregular:bool) -> [f32;2] {
    if elevated {
        if let Some((bed,level,flow)) = tributary(wx,wz,seed) {
            if flow && level > SEA_LEVEL && bed < level { return [0.0,-1.0]; }
        }
        return [0.0;2];
    }
    let (x,z)=(wx as f32,wz as f32);
    let row=(z/RIVER_SPACING).floor() as i32;
    let column=(x/LAKE_SPACING).floor() as i32;
    for r in row-1..=row+1 {
        if (z-river_z(x,r,seed)).abs()>4.0 {continue;}
        for c in column-1..=column+1 {
            let (lx,lz,rx,rz)=lake(c,r,seed);
            let distance=if irregular {lake_distance(x,z,c,r,seed)}else{((x-lx)/rx).powi(2)+((z-lz)/rz).powi(2)};
            if distance<1.25 {return [0.0;2];}
        }
        let slope=(river_z(x+1.0,r,seed)-river_z(x-1.0,r,seed))*0.5;
        let length=(1.0+slope*slope).sqrt();
        return [1.0/length,slope/length];
    }
    [0.0;2]
}

#[cfg(test)]
pub fn height(wx: i32, wz: i32, seed: u32) -> i32 {
    height_with_lakes(wx,wz,seed,false)
}

/// Rotated, elongated basins with broad asymmetric bays and headlands.
/// A positive radial shoreline keeps each carved basin connected to its center.
fn lake_distance(x:f32,z:f32,column:i32,row:i32,seed:u32)->f32 {
    let (lx,lz,rx,rz)=lake(column,row,seed);
    if (x-lx).abs()>rx*3.5 || (z-lz).abs()>rx*3.5 {return 4.;}
    let angle=column_rand(column,row,seed,0x1A01)*std::f32::consts::TAU;
    let (s,c)=angle.sin_cos();let (dx,dz)=(x-lx,z-lz);
    let stretch=1.0+column_rand(column,row,seed,0x1A02)*0.3;
    let u=(dx*c+dz*s)/(rx*stretch);let v=(-dx*s+dz*c)/(rz/stretch);
    let theta=v.atan2(u);
    let phase=column_rand(column,row,seed,0x1A03)*std::f32::consts::TAU;
    let radius=1.+0.26*(theta*2.+phase).sin()+0.16*(theta*3.-phase*1.7).cos()+0.07*(theta*5.+phase*0.8).sin();
    (u*u+v*v)/(radius*radius)
}

pub fn height_with_lakes(wx: i32, wz: i32, seed: u32, irregular:bool) -> i32 {
    let (x, z) = (wx as f32, wz as f32);
    let base = fbm(x * 0.01, z * 0.01, seed, 4, 2.0, 0.5) * 2.0 - 1.0;
    let hills = fbm(x * 0.04, z * 0.04, seed ^ 0x51ed, 3, 2.0, 0.5) * 2.0 - 1.0;
    let mut h = 24.0 + base * 14.0 + hills * 5.0;

    let mx = (x / MOUNTAIN_SPACING).floor() as i32;
    let mz = (z / MOUNTAIN_SPACING).floor() as i32;
    for cx in mx - 1..=mx + 1 {
        for cz in mz - 1..=mz + 1 {
            if column_rand(cx, cz, seed, 0xA720) > 0.45 {
                continue;
            }
            let px =
                (cx as f32 + 0.25 + column_rand(cx, cz, seed, 0xA721) * 0.5) * MOUNTAIN_SPACING;
            let pz =
                (cz as f32 + 0.25 + column_rand(cx, cz, seed, 0xA722) * 0.5) * MOUNTAIN_SPACING;
            let radius = 52.0 + column_rand(cx, cz, seed, 0xA723) * 23.0;
            let stretch = 0.7 + column_rand(cx, cz, seed, 0xA726) * 0.6;
            let d = (((x - px) / radius).powi(2) + ((z - pz) / (radius * stretch)).powi(2)).sqrt();
            if d < 1.0 {
                let peak = (CHUNK_Y - 4) as f32 - column_rand(cx, cz, seed, 0xA724) * 3.0;
                let influence = (1.0 - d).powf(1.15);
                h += (peak - h).max(0.0) * influence;
                h += (fbm(x * 0.09, z * 0.09, seed ^ 0xA725, 2, 2.0, 0.5) - 0.5) * 4.0 * influence;
            }
        }
    }

    // Lake centers sit exactly on the winding channel. Both carve to the same
    // water level, guaranteeing an open connection rather than isolated puddles.
    let row = (z / RIVER_SPACING).floor() as i32;
    let column = (x / LAKE_SPACING).floor() as i32;
    for r in row - 1..=row + 1 {
        let d = (z - river_z(x, r, seed)).abs();
        let half_width = 1.7 + column_rand(r, 0, seed, 0xA715) * 1.1;
        let bed = (SEA_LEVEL as f32 - 1.0 + (d - half_width) * 1.8).max(SEA_LEVEL as f32 - 4.0);
        h = h.min(bed);
        for c in column - 1..=column + 1 {
            let (lx, lz, rx, rz) = lake(c, r, seed);
            // Bound the expensive shoreline evaluation to the lake neighborhood.
            if irregular && ((x-lx).abs()>rx*3.5 || (z-lz).abs()>rx*3.5) {continue;}
            let distance = if irregular {lake_distance(x,z,c,r,seed)}else{((x - lx) / rx).powi(2) + ((z - lz) / rz).powi(2)};
            if distance < 2.0 {
                let shore = 0.8 + fbm(x * 0.045, z * 0.045, seed ^ 0xA716, 2, 2.0, 0.5) * 0.4;
                let lake_bed = SEA_LEVEL as f32 - 6.0 + (distance * shore).powi(2) * 6.0;
                h = h.min(lake_bed);
            }
        }
    }
    h.clamp(2.0, (CHUNK_Y - 4) as f32) as i32
}

#[cfg(test)]
pub fn tributary_preview(seed: u32) -> (i32,i32) {
    let c=(-4..4).find(|&c|column_rand(c,0,seed,0xFA110)<=0.55).unwrap();
    let x=((lake_x(c,0,seed)+lake_x(c+1,0,seed))*0.5).round() as i32;
    (x,river_z(x as f32,0,seed).round() as i32+14)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn irregular_lakes_keep_connected_interiors_and_original_uplands() {
        let mut changed=0;let mut unchanged=0;
        for seed in [7,42,2026] {
            for c in [-1,0,1] {
                let (lx,lz,_,_)=lake(c,0,seed);
                assert_eq!(current_with_lakes(lx.round() as i32,lz.round() as i32,seed,false,true),[0.;2]);
                let mut radii=Vec::new();
                for ray in 0..48 {
                    let theta=ray as f32*std::f32::consts::TAU/48.;
                    let (s,co)=theta.sin_cos();let mut radius=0;
                    for r in 0..160 {
                        let (x,z)=(lx+co*r as f32,lz+s*r as f32);
                        if lake_distance(x,z,c,0,seed)>=0.75 {break;}
                        assert!(height_with_lakes(x.round() as i32,z.round() as i32,seed,true)<SEA_LEVEL,"dry lake interior seed={seed} at {x},{z}");
                        radius=r;
                    }
                    radii.push(radius);
                }
                assert!(radii.iter().max().unwrap()*10>radii.iter().min().unwrap()*17,"lake too round: {radii:?}");
            }
            let config=crate::worldgen::WorldGeneration::default();
            let old=crate::worldgen::WorldGeneration{landscape_version:0,..Default::default()};
            for x in (-256..256).step_by(8) {for z in (-256..256).step_by(8) {
                if config.height(x,z,seed)!=old.height(x,z,seed) {
                    changed+=1;
                    let column=(x as f32/LAKE_SPACING).floor() as i32;let row=(z as f32/RIVER_SPACING).floor() as i32;
                    assert!((column-1..=column+1).any(|c|(row-1..=row+1).any(|r| {
                        let (lx,lz,rx,rz)=lake(c,r,seed);
                        lake_distance(x as f32,z as f32,c,r,seed)<2. || ((x as f32-lx)/rx).powi(2)+((z as f32-lz)/rz).powi(2)<2.
                    })),"Changed upland away from lakes: {x},{z}");
                }else {unchanged+=1;}
            }}
        }
        assert!(changed>100 && unchanged>5000);
    }
    #[test]
    #[ignore="writes a top-down comparison of original and irregular lake shores"]
    fn lake_shape_preview() {
        let mut image=image::RgbImage::new(768,512);
        for row in 0..2 {for column in 0..3 {
            let seed=[7,42,2026][column];let (lx,lz,_,_)=lake(0,0,seed);
            for x in 0..256 {for z in 0..256 {
                let h=height_with_lakes(lx as i32+x-128,lz as i32+z-128,seed,row==1);
                let color=if h<SEA_LEVEL {[30,110,180]}else if h==SEA_LEVEL {[198,186,121]}else{[65,125+(h-18).max(0) as u8*2,65]};
                image.put_pixel(column as u32*256+x as u32,row*256+z as u32,image::Rgb(color));
            }}
        }}
        image.save("target/lake-shapes.png").unwrap();
    }
    #[test]
    fn wider_formations_reduce_short_distance_height_changes() {
        for seed in [7,42,2026] {
            let variation=|version| {
                let mut total=0;
                for z in [-128,0,128] {for x in -512..512 {
                    total+=(geological_height(x+1,z,seed,32,1.,version)-geological_height(x,z,seed,32,1.,version)).abs();
                }}
                total
            };
            let compact=variation(1);let broad=variation(2);
            assert!(broad*100<compact*65,"seed={seed}, compact={compact}, broad={broad}");
        }
    }
    #[test]
    fn geological_detail_has_cliffs_cuts_and_supported_overhangs() {
        use crate::{voxel::{World,BlockType},worldgen::WorldGeneration};
        let config=WorldGeneration{landscape_version:2,..Default::default()};
        let mut changed=0;let mut cliffs=0;let mut depressions=0;let mut candidates=Vec::new();
        for x in (-192..192).step_by(3) {for z in (-192..192).step_by(3) {
            let h=config.height(x,z,42);let old=height(x,z,42);
            changed+=usize::from(h!=old);depressions+=usize::from(h<=old-3);
            let slope=[config.height(x-1,z,42),config.height(x+1,z,42),config.height(x,z-1,42),config.height(x,z+1,42)]
                .into_iter().map(|n|(n-h).abs()).max().unwrap();
            cliffs+=usize::from(slope>=3);
            if undercut(x,z,42,h,slope,config.landscape_version) {candidates.push((x,z,h));}
        }}
        assert!(changed>1000 && cliffs>20 && depressions>50,"changed={changed} cliffs={cliffs} depressions={depressions}");
        let mut world=World::new(42);world.generation=config;world.generation.underground=false;
        let mut roofs=0;
        for &(x,z,h) in candidates.iter().take(40) {
            let (cx,cz)=crate::voxel::chunk::world_to_chunk(x,z);world.ensure_chunk_loaded(cx,cz);
            if world.get_block(x,h-3,z)==BlockType::Air && world.get_block(x,h-1,z).is_solid() {roofs+=1;}
        }
        assert!(roofs>0,"No generated overhang roofs among {} candidates",candidates.len());
    }
    #[test]
    fn meadows_include_mixed_and_dominant_colonies() {
        let mut mixed=0;let mut dominant=0;
        for a in -5..5 {for b in -5..5 {
            let px=(a as f32*48.+column_rand(a,b,42,0xF101)*48.) as i32;
            let pz=(b as f32*48.+column_rand(a,b,42,0xF102)*48.) as i32;
            let mut counts=std::collections::HashMap::new();
            for x in px-6..=px+6 {for z in pz-6..=pz+6 {
                if let Some(plant)=meadow_plant(x,z,42) {*counts.entry(plant.id()).or_insert(0)+=1;}
            }}
            let total:i32=counts.values().sum();let largest=counts.values().copied().max().unwrap_or(0);
            if total>=30 {if largest*100/total>=75 {dominant+=1;}else if counts.len()>=3 {mixed+=1;}}
        }}
        assert!(mixed>=5 && dominant>=5,"mixed={mixed}, dominant={dominant}");
    }
    #[test]
    fn currents_follow_channels_but_stop_inside_lakes_and_upper_pools() {
        for seed in [7,42,2026] { for row in [-1,0,1] {
            for c in -1..=1 {
                let (x,z,_,_)=lake(c,row,seed);
                assert_eq!(current(x.round() as i32,z.round() as i32,seed,false),[0.0;2]);
            }
            let mut moving=0;
            for x in -350..350 {
                let z=river_z(x as f32,row,seed).round() as i32;
                let flow=current(x,z,seed,false);
                if flow!=[0.0;2] {
                    moving+=1;
                    assert!(flow[0]>0.0);
                    assert!((flow[0]*flow[0]+flow[1]*flow[1]-1.0).abs()<0.001);
                }
            }
            assert!(moving>100);
        } }
    }

    #[test]
    fn generated_tributaries_have_contained_pools_and_real_spills_across_chunks() {
        use super::super::{World,BlockType,chunk::world_to_chunk};
        for seed in [7,42,2026] {
            let column=(-4..4).find(|&c|column_rand(c,0,seed,0xFA110)<=0.55).unwrap();
            let x=((lake_x(column,0,seed)+lake_x(column+1,0,seed))*0.5).round() as i32;
            let z=river_z(x as f32,0,seed).round() as i32;
            let (_,level,_)=tributary(x,z+14,seed).unwrap();
            assert_eq!(current(x,z+32,seed,true),[0.0;2]);
            assert_eq!(current(x,z+18,seed,true),[0.0,-1.0]);
            let mut world=World::new(seed);
            let (cx,cz)=world_to_chunk(x,z+14);
            // Deliberately load neighbors in reverse order.
            for dx in (-1..=1).rev() {for dz in (-1..=2).rev() {world.ensure_chunk_loaded(cx+dx,cz+dz);}}
            assert_eq!(world.get_block(x,level,z+14),BlockType::Water);
            assert_eq!(world.get_block(x,SEA_LEVEL,z+13),BlockType::Water);
            assert_eq!(world.get_block(x,level,z+13),BlockType::Air);
            let falls:Vec<_>=world.chunks.values().flat_map(|chunk|crate::water::scan_chunk(&world,chunk)).collect();
            assert!(falls.iter().any(|f|(f.lip.x-x as f32).abs()<3.0 && (f.lip.z-(z+14) as f32).abs()<1.0));
            // The rim is higher than pool water, preventing side spills.
            assert!(world.get_block(x+8,level,z+32).is_solid());
            world.unload_chunk(cx,cz);
            world.ensure_chunk_loaded(cx,cz);
            assert_eq!(world.get_block(x,level,z+14),BlockType::Water);
        }
    }
    #[test]
    fn snowy_summits_have_solid_caps_and_preserve_player_edits() {
        use super::super::{World,BlockType,chunk::world_to_chunk};
        let config=crate::worldgen::WorldGeneration::default();
        let mut summit=None;
        for x in (-512..512).step_by(8) {for z in (-512..512).step_by(8) {
            let h=config.height(x,z,42);
            let slope=[config.height(x-1,z,42),config.height(x+1,z,42),config.height(x,z-1,42),config.height(x,z+1,42)]
                .into_iter().map(|n|(n-h).abs()).max().unwrap();
            if h>=40 && mountain_surface_with_slope(x,z,h,42,slope)==BlockType::Snow {summit=Some((x,z,h));break;}
        }if summit.is_some(){break;}}
        let (x,z,h)=summit.expect("snowy summit");
        let mut world=World::new(42);let (cx,cz)=world_to_chunk(x,z);world.ensure_chunk_loaded(cx,cz);
        assert_eq!(world.get_block(x,h,z),BlockType::Snow);
        assert_eq!(world.get_block(x,h-1,z),BlockType::Stone);
        assert!(BlockType::Snow.is_solid());assert!(BlockType::Snow.hand_pickable());
        assert_eq!(BlockType::from_name("snow"),Some(BlockType::Snow));
        world.set_block(x,h,z,BlockType::Bricks);world.unload_chunk(cx,cz);world.ensure_chunk_loaded(cx,cz);
        assert_eq!(world.get_block(x,h,z),BlockType::Bricks);
    }
    #[test]
    fn winding_channels_connect_lake_centers_across_negative_and_positive_chunks() {
        for seed in [7, 42, 2026] {
            for row in [-1, 0, 1] {
                let (start, _, _, _) = lake(-1, row, seed);
                let (end, _, _, _) = lake(1, row, seed);
                let mut previous = None;
                for step in 0..=((end - start) * 2.0) as i32 {
                    let x = start + step as f32 * 0.5;
                    let z = river_z(x, row, seed);
                    let p = (x.round() as i32, z.round() as i32);
                    assert!(height(p.0, p.1, seed) < SEA_LEVEL, "dry channel at {p:?}");
                    if let Some((px, pz)) = previous {
                        // At a diagonal step, at least one orthogonal neighbor stays wet.
                        assert!(
                            height(px, p.1, seed) < SEA_LEVEL || height(p.0, pz, seed) < SEA_LEVEL
                        );
                    }
                    previous = Some(p);
                }
                for column in -1..=1 {
                    let (x, z, rx, rz) = lake(column, row, seed);
                    for (dx, dz) in [(0., 0.), (rx * 0.6, 0.), (0., rz * 0.6)] {
                        assert!(height((x + dx) as i32, (z + dz) as i32, seed) < SEA_LEVEL);
                    }
                }
            }
        }
    }
    #[test]
    fn generated_channel_water_survives_chunk_boundaries_and_load_order() {
        let mut world = super::super::world::World::new(42);
        for x in [336, 335, 16, 15, 0, -1, -16, -17] {
            let z = river_z(x as f32, 0, 42).round() as i32;
            let (cx, cz) = super::super::chunk::world_to_chunk(x, z);
            world.ensure_chunk_loaded(cx, cz);
            assert_eq!(
                world.get_block(x, SEA_LEVEL, z),
                super::super::block::BlockType::Water
            );
        }
    }
    #[test]
    fn terrain_survey_has_sparse_high_peaks_and_water() {
        let mut rocky = 0;
        let (mut snow,mut bare_rock,mut alpine_grass,mut lower_snow)=(0,0,0,0);
        let mut water = 0;
        let mut peak = 0;
        let mut image = image::RgbImage::new(512, 512);
        for x in 0..512 {
            for z in 0..512 {
                let h = height(x as i32 * 2 - 512, z as i32 * 2 - 512, 42);
                peak = peak.max(h);
                rocky += usize::from(h >= ROCK_LINE);
                water += usize::from(h < SEA_LEVEL);
                let surface=mountain_surface(x as i32*2-512,z as i32*2-512,h,42);
                if h>=ROCK_LINE {
                    match surface {super::super::BlockType::Snow=>snow+=1,super::super::BlockType::Stone=>bare_rock+=1,_=>alpine_grass+=1}
                } else if surface==super::super::BlockType::Snow {lower_snow+=1;}

                let color = if h < SEA_LEVEL {
                    [25, 85 + ((h - 2) * 3) as u8, 155]
                } else if surface==super::super::BlockType::Snow {
                    [223,235,245]
                } else if surface==super::super::BlockType::Stone {
                    [115,119,125]
                } else {
                    [65 + (h - 18) as u8 * 3, 110 + (h - 18) as u8 * 3, 55]
                };
                image.put_pixel(x, z, image::Rgb(color));
            }
        }
        assert!(
            (500..15_000).contains(&rocky),
            "rocky coverage {rocky}/262144"
        );
        assert!(
            (8_000..80_000).contains(&water),
            "water coverage {water}/262144"
        );
        assert!(peak >= 40 && peak < CHUNK_Y);
        assert!(snow>100 && bare_rock>100,"snow={snow}, rock={bare_rock}");
        assert!(alpine_grass>0 && alpine_grass<rocky/10,"grass={alpine_grass}/{rocky}");
        assert!(lower_snow>0,"missing lower snow patches");
        println!("Alpine mix: snow={snow}, rock={bare_rock}, grass={alpine_grass}; lower snow={lower_snow}");

        std::fs::create_dir_all("target").unwrap();
        image.save("target/terrain-map.png").unwrap();
        std::fs::write("target/terrain-survey.txt",format!("seed=42, area=1024x1024, samples=262144\nrocky={rocky}\nwater={water}\npeak={peak}\n")).unwrap();
    }
}
