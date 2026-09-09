//! Continuous, seed-only terrain features. No chunk-local hydrology decisions.
use super::{
    chunk::CHUNK_Y,
    noise::{column_rand, fbm},
    world::SEA_LEVEL,
};

const RIVER_SPACING: f32 = 384.0;
const LAKE_SPACING: f32 = 336.0;
const MOUNTAIN_SPACING: f32 = 256.0;
pub const ROCK_LINE: i32 = 34;

fn river_z(x: f32, row: i32, seed: u32) -> f32 {
    let phase = column_rand(row, 0, seed, 0xA710) * std::f32::consts::TAU;
    row as f32 * RIVER_SPACING
        + 140.0
        + (x * 0.013 + phase).sin() * 38.0
        + (x * 0.029 + phase * 1.7).sin() * 12.0
        + (x * 0.055 + phase * 2.3).sin() * 6.0
        + (fbm(x * 0.006, row as f32 * 13.0, seed ^ 0xA711, 2, 2.0, 0.5) - 0.5) * 55.0
}

fn lake(column: i32, row: i32, seed: u32) -> (f32, f32, f32, f32) {
    let x = column as f32 * LAKE_SPACING
        + 140.0
        + (column_rand(column, row, seed, 0xA712) - 0.5) * 110.0;
    (
        x,
        river_z(x, row, seed),
        32.0 + column_rand(column, row, seed, 0xA713) * 20.0,
        24.0 + column_rand(column, row, seed, 0xA714) * 18.0,
    )
}

pub fn height(wx: i32, wz: i32, seed: u32) -> i32 {
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
            let distance = ((x - lx) / rx).powi(2) + ((z - lz) / rz).powi(2);
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
mod tests {
    use super::*;
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
        let mut water = 0;
        let mut peak = 0;
        let mut image = image::RgbImage::new(512, 512);
        for x in 0..512 {
            for z in 0..512 {
                let h = height(x as i32 * 2 - 512, z as i32 * 2 - 512, 42);
                peak = peak.max(h);
                rocky += usize::from(h >= ROCK_LINE);
                water += usize::from(h < SEA_LEVEL);
                let color = if h < SEA_LEVEL {
                    [25, 85 + ((h - 2) * 3) as u8, 155]
                } else if h >= ROCK_LINE {
                    let v = 100 + (h - ROCK_LINE) as u8 * 12;
                    [v, v, v]
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
        std::fs::create_dir_all("target").unwrap();
        image.save("target/terrain-map.png").unwrap();
        std::fs::write("target/terrain-survey.txt",format!("seed=42, area=1024x1024, samples=262144\nrocky={rocky}\nwater={water}\npeak={peak}\n")).unwrap();
    }
}
