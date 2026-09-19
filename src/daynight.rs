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
    /// sin of the sun's angle above the horizon: 0 exactly at the horizon,
    /// 1 straight up, negative once it's set. The same value `blend_phase`
    /// uses to decide the sky/zenith color, exposed here too so the sky
    /// pass's sun-disc/moon/star fade (see sky.wgsl) can key off the exact
    /// same phase boundaries instead of drifting out of sync with the
    /// color transition -- `sun_dir` alone isn't enough for that since
    /// normalizing it changes its y component away from this raw value.
    pub sun_height: f32,
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
    // Retain enough low-angle light to illuminate golden-hour surfaces,
    // while still fading continuously to zero at the horizon.
    let sun_intensity = sun_height.clamp(0.0, 1.0).powf(0.65) * 0.85;

    SkyLighting {
        sun_dir,
        sun_height,
        sky_color,
        zenith_color,
        ambient,
        sun_intensity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sun_height` (and therefore the sky color/sun disc/moon fade timing
    /// that's keyed off it) should match `is_night`'s own sun_height <= 0
    /// threshold exactly at every quarter of the day -- these are the same
    /// four moments the module doc comment names (sunrise/noon/sunset/
    /// midnight), so a regression here would desync the visible sun/moon
    /// from what `api.is_night` reports to Lua rules too.
    #[test]
    fn sun_height_matches_the_documented_quarter_day_points() {
        let sunrise = sky_lighting(0.0);
        assert!(
            sunrise.sun_height.abs() < 1e-4,
            "sunrise (0.0) should have the sun exactly at the horizon, got {}",
            sunrise.sun_height
        );

        let noon = sky_lighting(0.25);
        assert!(
            (noon.sun_height - 1.0).abs() < 1e-4,
            "noon (0.25) should have the sun at its highest, got {}",
            noon.sun_height
        );

        let sunset = sky_lighting(0.5);
        assert!(
            sunset.sun_height.abs() < 1e-4,
            "sunset (0.5) should have the sun exactly at the horizon, got {}",
            sunset.sun_height
        );

        let midnight = sky_lighting(0.75);
        assert!(
            (midnight.sun_height - (-1.0)).abs() < 1e-4,
            "midnight (0.75) should have the sun at its lowest, got {}",
            midnight.sun_height
        );
    }

    #[test]
    fn is_night_agrees_with_sun_height_sign_across_a_full_day() {
        // Sampled finely enough to catch a sign-convention mismatch
        // between is_night and sky_lighting's sun_height without the two
        // computing the angle in subtly different ways.
        for i in 0..1000 {
            let t = i as f32 / 1000.0;
            let lighting = sky_lighting(t);
            assert_eq!(
                is_night(t),
                lighting.sun_height <= 0.0,
                "is_night({t}) disagreed with sun_height's sign ({})",
                lighting.sun_height
            );
        }
    }

    #[test]
    fn sun_height_rises_through_the_first_quarter_and_falls_through_the_second() {
        let early_morning = sky_lighting(0.1).sun_height;
        let noon = sky_lighting(0.25).sun_height;
        let afternoon = sky_lighting(0.4).sun_height;
        assert!(
            early_morning < noon,
            "sun should still be rising between sunrise and noon"
        );
        assert!(
            afternoon < noon,
            "sun should already be descending between noon and sunset"
        );
    }

    #[test]
    fn sky_color_reaches_full_night_exactly_where_blend_phase_says_it_should() {
        // blend_phase locks into full NIGHT_COLOR once sun_height <= -0.2;
        // find a time_of_day on the descending (post-sunset) side where
        // that holds and confirm the color is pinned to it, not still
        // mid-transition.
        let deep_night = sky_lighting(0.75); // midnight, sun_height == -1.0
        assert_eq!(deep_night.sky_color, NIGHT_COLOR);
        assert_eq!(deep_night.zenith_color, ZENITH_NIGHT_COLOR);
    }
}
