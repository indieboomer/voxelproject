/// Small self-contained value-noise implementation so world generation
/// doesn't depend on an external noise crate's exact API surface.

#[inline]
fn hash(x: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x1f1f_1f1f)
        ^ (z as u32).wrapping_mul(0x9e37_79b9)
        ^ seed.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x2976_57ee);
    h ^= h >> 16;
    h
}

#[inline]
fn rand01(x: i32, z: i32, seed: u32) -> f32 {
    (hash(x, z, seed) as f32) / (u32::MAX as f32)
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn value_noise2(x: f32, z: f32, seed: u32) -> f32 {
    let x0 = x.floor() as i32;
    let z0 = z.floor() as i32;
    let x1 = x0 + 1;
    let z1 = z0 + 1;

    let tx = smoothstep(x - x0 as f32);
    let tz = smoothstep(z - z0 as f32);

    let v00 = rand01(x0, z0, seed);
    let v10 = rand01(x1, z0, seed);
    let v01 = rand01(x0, z1, seed);
    let v11 = rand01(x1, z1, seed);

    let a = v00 + (v10 - v00) * tx;
    let b = v01 + (v11 - v01) * tx;
    a + (b - a) * tz
}

/// Fractal Brownian motion, returns value roughly in [0, 1].
pub fn fbm(x: f32, z: f32, seed: u32, octaves: u32, lacunarity: f32, gain: f32) -> f32 {
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    let mut sum = 0.0;
    let mut max = 0.0;
    for i in 0..octaves {
        sum += value_noise2(x * frequency, z * frequency, seed.wrapping_add(i * 101)) * amplitude;
        max += amplitude;
        amplitude *= gain;
        frequency *= lacunarity;
    }
    sum / max
}

/// Deterministic pseudo-random float in [0,1) for a given world column, for
/// scattering features like trees.
pub fn column_rand(x: i32, z: i32, seed: u32, salt: u32) -> f32 {
    rand01(x, z, seed ^ salt)
}

#[inline]
fn hash3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x1f1f_1f1f)
        ^ (y as u32).wrapping_mul(0x2545_f491)
        ^ (z as u32).wrapping_mul(0x9e37_79b9)
        ^ seed.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x2976_57ee);
    h ^= h >> 16;
    h
}

/// Deterministic pseudo-random float in [0,1) for a given world block
/// position, for 3D scatter (ore veins, underground pockets) that plain
/// column-based `column_rand` can't express since it ignores height.
pub fn block_rand(x: i32, y: i32, z: i32, seed: u32, salt: u32) -> f32 {
    (hash3(x, y, z, seed ^ salt) as f32) / (u32::MAX as f32)
}

/// Deterministic pseudo-random `i32` in `[lo, hi]` (inclusive) for a given
/// world block position and salt -- used to pick random walk directions when
/// growing an ore vein without needing a stateful RNG (chunk generation must
/// stay a pure function of (seed, cx, cz)).
pub fn block_rand_range(x: i32, y: i32, z: i32, seed: u32, salt: u32, lo: i32, hi: i32) -> i32 {
    let span = (hi - lo + 1).max(1) as u32;
    lo + (hash3(x, y, z, seed ^ salt) % span) as i32
}
