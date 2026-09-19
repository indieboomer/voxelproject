// Renders the sky background: a horizon-to-zenith gradient, a glowing square
// sun, a pale moon at night, and faint stars -- drawn as the very first
// thing in the main pass (a single fullscreen triangle, no depth test) so
// every other draw call simply paints over it wherever real geometry
// exists. Replaces what used to be one flat clear color.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    // rgb = sky color, w = smoothed base cloud fullness (0 sunny, 1 otherwise).
    zenith_color: vec4<f32>,
    // xyz = normalized sun direction, w = raw sun_height (sin of the sun's
    // angle above the horizon -- see daynight.rs's SkyLighting.sun_height)
    sun_dir: vec4<f32>,
    light_params: vec4<f32>,
    // x = lightning_flash, 0..1 -- see shader.wgsl's copy of this struct
    // and App::update_lightning. y = cloud_coverage, 0..1 -- see
    // Weather::cloud_coverage. z = camera eye underwater, w = surface wetness.
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
const TONEMAP_BLEND: f32 = 0.55;

fn grade(color: vec3<f32>) -> vec3<f32> {
    let linear = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let toned = aces_tonemap(color * 0.9);
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

// Two octaves of value_noise at increasing frequency/decreasing weight
// (a small manually-unrolled fbm) -- reads as organic, uneven cloud puffs
// rather than one layer's obviously-gridded blobs.
fn cloud_density(p: vec2<f32>) -> f32 {
    let n1 = value_noise(p);
    let n2 = value_noise(p * 2.03 + vec2<f32>(5.2, 1.3));
    return n1 * 0.65 + n2 * 0.35;
}

// A bounded voxel traversal through a visual-only layer above the build ceiling.
// Translating the whole grid gives steady wind without reshuffling cloud cells.
fn voxel_clouds(dir: vec3<f32>, coverage: f32) -> vec2<f32> {
    if dir.y < 0.035 || camera.camera_pos.y >= 184.0 { return vec2<f32>(0.0); }
    let start = max(0.0, (160.0-camera.camera_pos.y)/dir.y);
    let wind = vec3<f32>(camera.light_params.z*0.65, 0.0, camera.light_params.z*0.22);
    let origin = (camera.camera_pos.xyz + dir*(start+0.001)-wind)/8.0;
    var cell = floor(origin);
    let step_dir = sign(dir);
    let delta = 1.0/max(abs(dir),vec3<f32>(0.00001));
    var next = (select(origin-cell,cell+1.0-origin,dir>=vec3<f32>(0.0)))*delta;
    var shade = 0.72;
    var travel = 0.0;
    // A 0.71 threshold retains about 30% of the old sunny cloud footprint.
    // Restore the original 0.58 threshold for other weather; overcast is unchanged.
    let threshold = mix(0.71, 0.58, camera.zenith_color.w)-coverage*0.24;
    for (var i=0; i<48; i=i+1) {
        if cell.y >= 23.0 { break; }
        let density = cloud_density(cell.xz*0.19);
        let thickness = 1.0+floor(clamp((density-threshold)*10.0,0.0,2.0));
        if cell.y>=20.0 && cell.y<20.0+thickness && density>threshold {
            let fade = (1.0-smoothstep(1100.0,1800.0,start+travel*8.0))*smoothstep(0.035,0.12,dir.y);
            return vec2<f32>(fade,shade);
        }
        if next.x < next.y && next.x < next.z {
            travel=next.x; next.x+=delta.x; cell.x+=step_dir.x; shade=0.85;
        } else if next.z < next.y {
            travel=next.z; next.z+=delta.z; cell.z+=step_dir.z; shade=0.95;
        } else {
            travel=next.y; next.y+=delta.y; cell.y+=step_dir.y; shade=0.72;
        }
    }
    return vec2<f32>(0.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if camera.weather_fx.z > 0.5 {
        return vec4<f32>(grade(vec3<f32>(0.025, 0.16, 0.28)), 1.0);
    }

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

    // Sunny skies retain scattered voxel clouds; weather adds overcast cover.
    // Distant detail fades into haze before it becomes horizon stripes.
    let cloud_coverage = camera.weather_fx.y;
    var cloud_alpha = 0.0;
    let voxel = voxel_clouds(dir,cloud_coverage);
    cloud_alpha = voxel.x;
    // Dark, faintly blue-gray overcast tone; dims further at night via the
    // same ambient term the terrain's own lighting uses, so clouds don't
    // read as glowing white at midnight.
    let cloud_color = vec3<f32>(0.34, 0.35, 0.39) * camera.light_params.x * 2.2;
    color = mix(color, cloud_color * 1.2, cloud_coverage * 0.65);
    let voxel_color = mix(vec3<f32>(1.0,0.98,0.95),vec3<f32>(0.53,0.56,0.61),cloud_coverage)
        * min(1.0,camera.light_params.x*1.8+camera.light_params.y*0.35)*voxel.y;
    color = mix(color, voxel_color, cloud_alpha);
    // How much of the sun/moon/stars' own light still gets through -- 1
    // where the sky is clear, fading toward 0 under thick cloud so a
    // storm's clouds actually hide the sky behind them instead of just
    // painting over an otherwise-still-visible sun/moon.
    let sky_visibility = (1.0 - cloud_alpha) * (1.0 - cloud_coverage * 0.65);

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

    // Sun: a softly edged square in a world-oriented tangent plane, with
    // a warm glow following its edges and a wider atmospheric halo.
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    let golden_hour = 1.0 - smoothstep(0.0, 0.4, sun_height);
    let sun_size = mix(1.0, 1.4, golden_hour);
    let sun_axis = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(sun_dir.y) > 0.99);
    let sun_right = normalize(cross(sun_axis, sun_dir));
    let sun_up = cross(sun_dir, sun_right);
    let sun_uv = vec2<f32>(dot(dir, sun_right), dot(dir, sun_up)) / max(sun_dot, 0.001);
    let square = abs(sun_uv) - vec2<f32>(0.027 * sun_size);
    let sun_edge = length(max(square, vec2<f32>(0.0))) + min(max(square.x, square.y), 0.0);
    let sun_softness = max(fwidth(sun_edge), 0.0008);
    let sun_core = 1.0 - smoothstep(-sun_softness, sun_softness, sun_edge);
    let sun_glow = exp(-max(sun_edge, 0.0) / (0.022 * sun_size)) * 0.85;
    let sun_halo = pow(sun_dot, 48.0) * 0.35;
    let sun_front = smoothstep(0.0, 0.1, sun_dot);
    let sun_color = mix(vec3<f32>(1.0, 0.96, 0.84), vec3<f32>(1.0, 0.16, 0.015), golden_hour);
    let halo_color = mix(vec3<f32>(1.0, 0.76, 0.38), vec3<f32>(1.0, 0.38, 0.08), golden_hour);
    let sun_visible = sun_front * sun_visibility * sky_visibility;
    // Composite the core so bright horizon haze cannot wash its orange to white.
    color = mix(color, sun_color * mix(3.0, 1.15, golden_hour), sun_core * sun_visible);
    color += halo_color * (sun_glow + sun_halo) * (1.0 - sun_core) * sun_visible;

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
    if night_amount > 0.001 && dir.y > 0.05 {
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
    // Internal sunlight mask: 0.1..0.9 is sky visibility, 1 is geometry,
    // and 0 remains reserved for the water reflection mask.
    return vec4<f32>(flashed, 0.1 + 0.8 * sky_visibility);
}
