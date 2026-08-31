// Renders the sky background: a horizon-to-zenith gradient, a glowing sun
// disc, a pale moon at night, and faint stars -- drawn as the very first
// thing in the main pass (a single fullscreen triangle, no depth test) so
// every other draw call simply paints over it wherever real geometry
// exists. Replaces what used to be one flat clear color.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    zenith_color: vec4<f32>,
    // xyz = normalized sun direction, w = raw sun_height (sin of the sun's
    // angle above the horizon -- see daynight.rs's SkyLighting.sun_height)
    sun_dir: vec4<f32>,
    light_params: vec4<f32>,
    // x = lightning_flash, 0..1 -- see shader.wgsl's copy of this struct
    // and App::update_lightning. y = cloud_coverage, 0..1 -- see
    // Weather::cloud_coverage. z/w reserved, currently always 0.
    weather_fx: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// Classic fullscreen-triangle trick: 3 vertices, no vertex buffer, covering
// the whole viewport (and a bit beyond, clipped away) so a single draw call
// with no attributes renders one background pixel per fragment.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // (-1,-1), (3,-1), (-1,3) -- a triangle twice the size of the screen,
    // clipped down to exactly cover it with a single draw call.
    let x = f32((vertex_index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vertex_index & 2u) * 2.0 - 1.0;
    var out: VertexOutput;
    out.clip_position = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// See shader.wgsl's grade() for why this is a partial blend rather than
// the full tonemap curve -- kept identical here so the sky and the world
// geometry it's blended with (fog) always agree tonally.
const TONEMAP_BLEND: f32 = 0.35;

fn grade(color: vec3<f32>) -> vec3<f32> {
    let linear = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let toned = aces_tonemap(color);
    return mix(linear, toned, TONEMAP_BLEND);
}

// Cheap hash for star placement -- doesn't need to be high quality, just
// deterministic and evenly scattered.
fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Bilinear-interpolated value noise for clouds -- unlike hash21's raw
// per-cell randomness (fine for a sparse star field, since each star is a
// single point), this needs to vary smoothly between neighboring cells to
// read as soft cloud shapes instead of static.
fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Three octaves of value_noise at increasing frequency/decreasing weight
// (a small manually-unrolled fbm) -- reads as organic, uneven cloud puffs
// rather than one layer's obviously-gridded blobs.
fn cloud_density(p: vec2<f32>) -> f32 {
    let n1 = value_noise(p);
    let n2 = value_noise(p * 2.03 + vec2<f32>(5.2, 1.3));
    let n3 = value_noise(p * 4.01 + vec2<f32>(1.7, 9.2));
    return n1 * 0.55 + n2 * 0.3 + n3 * 0.15;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Unproject this pixel's far-plane point back into a world-space ray
    // direction through the camera -- the sky has no real geometry, so this
    // is the only way to know what direction a given screen pixel looks.
    let far_world = camera.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far_world.xyz / far_world.w - camera.camera_pos.xyz);
    let sun_dir = normalize(camera.sun_dir.xyz);
    let sun_height = camera.sun_dir.w;

    // Horizon-to-zenith gradient. dir.y in [-1,1] maps to t in [0,1]; a
    // small horizon "haze" boost (brighter band right at dir.y = 0) is a
    // cheap trick that reads much more like a real atmosphere than a plain
    // linear blend.
    let t = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
    var color = mix(camera.fog_color.rgb, camera.zenith_color.rgb, pow(t, 0.55));
    let haze = pow(1.0 - abs(dir.y), 5.0) * 0.2;
    color += camera.fog_color.rgb * haze;

    // Clouds: projected onto the sky dome the same way stars are below
    // (dir.xz/dir.y -- compresses near the zenith, stretches near the
    // horizon, same as looking up at a real overcast sky), drifting slowly
    // via the free-running clock so they visibly scroll rather than
    // sitting static. cloud_coverage is 0 for every weather except storm
    // right now (see Weather::cloud_coverage), so this is a no-op cost
    // elsewhere -- smoothstep's lower/upper bounds collapse to the same
    // point and cloud_alpha stays 0.
    let cloud_coverage = camera.weather_fx.y;
    var cloud_alpha = 0.0;
    if dir.y > 0.02 {
        let drift = vec2<f32>(camera.light_params.z * 0.015, camera.light_params.z * 0.008);
        let cloud_uv = dir.xz / dir.y * 0.12 + drift;
        let density = cloud_density(cloud_uv);
        // Higher coverage lowers the threshold a puff needs to clear, so
        // more of the noise field reads as cloud instead of clear sky.
        let threshold = 1.0 - cloud_coverage;
        cloud_alpha = smoothstep(threshold, threshold + 0.3, density);
    }
    // Dark, faintly blue-gray overcast tone; dims further at night via the
    // same ambient term the terrain's own lighting uses, so clouds don't
    // read as glowing white at midnight.
    let cloud_color = vec3<f32>(0.34, 0.35, 0.39) * (0.55 + camera.light_params.x);
    color = mix(color, cloud_color, cloud_alpha);
    // How much of the sun/moon/stars' own light still gets through -- 1
    // where the sky is clear, fading toward 0 under thick cloud so a
    // storm's clouds actually hide the sky behind them instead of just
    // painting over an otherwise-still-visible sun/moon.
    let sky_visibility = 1.0 - cloud_alpha;

    // Sun/moon/star visibility is keyed off sun_height directly (not the
    // scene's own sun_intensity, which daynight.rs already clamps to 0 for
    // the entire sun_height <= 0 range since that's meant for terrain
    // lighting, not the disc) using the same -0.2/0 breakpoints
    // daynight.rs's blend_phase uses for the sky color itself -- otherwise
    // the sun would vanish and the moon/stars would already be at full
    // brightness the instant the sun touched the horizon, well before the
    // sky color had actually finished transitioning to night. sun_visibility
    // and night_amount are complements of each other, so the sun fades out
    // exactly as the moon/stars fade in, over the same dusk/dawn window.
    let sun_visibility = smoothstep(-0.2, 0.05, sun_height);
    let night_amount = 1.0 - sun_visibility;

    // Sun: a small sharp core plus a much softer, wider halo.
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    let sun_core = pow(sun_dot, 800.0) * 6.0;
    let sun_halo = pow(sun_dot, 32.0) * 0.6;
    color += vec3<f32>(1.0, 0.92, 0.75) * (sun_core + sun_halo) * sun_visibility * sky_visibility;

    // Moon: the same trick mirrored to -sun_dir -- astronomically that's
    // also correct for "always full": a full moon is one directly opposite
    // the sun, fully lit from Earth's point of view, which this mirroring
    // already models for free. No phases (waxing/waning) are rendered --
    // deliberate for this stage of the project, not a missing feature.
    let moon_dot = max(dot(dir, -sun_dir), 0.0);
    let moon = pow(moon_dot, 2000.0) * 1.2 + pow(moon_dot, 64.0) * 0.08;
    color += vec3<f32>(0.85, 0.88, 0.95) * moon * night_amount * sky_visibility;

    // Stars: a sparse hash-thresholded field, only above the horizon and
    // only once night has properly set in, twinkling faintly via a slow
    // per-star phase offset so they aren't perfectly static.
    if dir.y > 0.05 {
        let cell = floor(dir.xz / dir.y * 40.0);
        let n = hash21(cell);
        if n > 0.9975 {
            let twinkle = 0.6 + 0.4 * sin(camera.light_params.z * 3.0 + n * 62.8);
            color += vec3<f32>(1.0) * (n - 0.9975) * 400.0 * night_amount * twinkle * 0.5 * sky_visibility;
        }
    }

    // Lightning flash: same straight blend to white shader.wgsl applies,
    // so the sky whites out along with the terrain instead of just one or
    // the other.
    let flashed = mix(grade(color), vec3<f32>(1.0), camera.weather_fx.x);
    return vec4<f32>(flashed, 1.0);
}
