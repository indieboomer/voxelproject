use glam::Vec3;

/// How long a full day/night cycle takes in real seconds.
pub const DAY_LENGTH_SECS: f32 = 300.0;

const NIGHT_COLOR: [f32; 3] = [0.02, 0.03, 0.09];
const SUNSET_COLOR: [f32; 3] = [0.85, 0.55, 0.35];
const DAY_COLOR: [f32; 3] = [0.55, 0.78, 0.92];

/// Sky color straight overhead, blended through the same three phases as
/// `*_COLOR` (the horizon color) but deeper/richer -- a real zenith-to-
/// horizon gradient (rendered by the new sky pass, see sky.wgsl) reads far
/// more like a real sky than one flat fog color ever could. Night's zenith
/// is near-black so stars read clearly; sunset's zenith leans into a
/// dusky violet rather than repeating the horizon's orange; day's zenith is
/// a deeper, more saturated blue than the hazy color near the ground.
const ZENITH_NIGHT_COLOR: [f32; 3] = [0.01, 0.01, 0.04];
const ZENITH_SUNSET_COLOR: [f32; 3] = [0.25, 0.20, 0.45];
const ZENITH_DAY_COLOR: [f32; 3] = [0.15, 0.42, 0.85];

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Blends a night/sunset/day color triple by the sun's height above the
/// horizon -- the shared phase curve behind both the horizon (`sky_color`)
/// and zenith (`zenith_color`) gradients, so they always transition in sync
/// with each other even though their actual colors differ.
fn blend_phase(sun_height: f32, night: [f32; 3], sunset: [f32; 3], day: [f32; 3]) -> [f32; 3] {
    if sun_height <= -0.2 {
        night
    } else if sun_height <= 0.0 {
        lerp3(night, sunset, (sun_height + 0.2) / 0.2)
    } else if sun_height <= 0.35 {
        lerp3(sunset, day, sun_height / 0.35)
    } else {
        day
    }
}

/// True once the sun is below the horizon (matches the point the sky
/// finishes fading to full night color). Used by the Lua World API's
/// `is_night` field.
pub fn is_night(time_of_day: f32) -> bool {
    let angle = time_of_day * std::f32::consts::TAU;
    angle.sin() <= 0.0
}

pub struct SkyLighting {
    pub sun_dir: Vec3,
    pub sky_color: [f32; 3],
    /// Sky color straight overhead -- see `ZENITH_*_COLOR`. Paired with
    /// `sky_color` (the horizon) by the sky pass to draw a real gradient
    /// instead of one flat background color.
    pub zenith_color: [f32; 3],
    pub ambient: f32,
    pub sun_intensity: f32,
}

/// Computes sun direction, sky/fog color and lighting levels for a given
/// point in the day/night cycle. `time_of_day` is in [0, 1): 0 = sunrise,
/// 0.25 = noon, 0.5 = sunset, 0.75 = midnight.
pub fn sky_lighting(time_of_day: f32) -> SkyLighting {
    let angle = time_of_day * std::f32::consts::TAU;
    let sun_height = angle.sin();
    let sun_dir = Vec3::new(angle.cos() * 0.8, sun_height, 0.35).normalize();

    let sky_color = blend_phase(sun_height, NIGHT_COLOR, SUNSET_COLOR, DAY_COLOR);
    let zenith_color = blend_phase(
        sun_height,
        ZENITH_NIGHT_COLOR,
        ZENITH_SUNSET_COLOR,
        ZENITH_DAY_COLOR,
    );

    let ambient = 0.08 + 0.32 * (sun_height * 0.5 + 0.5).clamp(0.0, 1.0);
    // Nudged up slightly from the old hard-clamped peak of 0.7: direct
    // sunlight is no longer clamped against ambient before display (see
    // shader.wgsl's move to filmic tonemapping), so highlights get a
    // little extra punch from the tonemapper's soft rolloff. Kept modest
    // (not pushed all the way to/past 1.0) -- combined with unclamped
    // ambient this is already enough to noticeably brighten highlights
    // without oversaturating flat-lit surfaces like grass.
    let sun_intensity = sun_height.clamp(0.0, 1.0) * 0.85;

    SkyLighting {
        sun_dir,
        sky_color,
        zenith_color,
        ambient,
        sun_intensity,
    }
}
