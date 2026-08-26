use glam::Vec3;

/// How long a full day/night cycle takes in real seconds.
pub const DAY_LENGTH_SECS: f32 = 300.0;

const NIGHT_COLOR: [f32; 3] = [0.02, 0.03, 0.09];
const SUNSET_COLOR: [f32; 3] = [0.85, 0.55, 0.35];
const DAY_COLOR: [f32; 3] = [0.55, 0.78, 0.92];

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
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

    let sky_color = if sun_height <= -0.2 {
        NIGHT_COLOR
    } else if sun_height <= 0.0 {
        lerp3(NIGHT_COLOR, SUNSET_COLOR, (sun_height + 0.2) / 0.2)
    } else if sun_height <= 0.35 {
        lerp3(SUNSET_COLOR, DAY_COLOR, sun_height / 0.35)
    } else {
        DAY_COLOR
    };

    let ambient = 0.08 + 0.32 * (sun_height * 0.5 + 0.5).clamp(0.0, 1.0);
    let sun_intensity = sun_height.clamp(0.0, 1.0) * 0.7;

    SkyLighting {
        sun_dir,
        sky_color,
        ambient,
        sun_intensity,
    }
}
