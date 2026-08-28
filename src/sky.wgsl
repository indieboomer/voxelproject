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
    sun_dir: vec4<f32>,
    light_params: vec4<f32>,
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

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Unproject this pixel's far-plane point back into a world-space ray
    // direction through the camera -- the sky has no real geometry, so this
    // is the only way to know what direction a given screen pixel looks.
    let far_world = camera.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far_world.xyz / far_world.w - camera.camera_pos.xyz);
    let sun_dir = normalize(camera.sun_dir.xyz);
    let sun_intensity = camera.light_params.y;

    // Horizon-to-zenith gradient. dir.y in [-1,1] maps to t in [0,1]; a
    // small horizon "haze" boost (brighter band right at dir.y = 0) is a
    // cheap trick that reads much more like a real atmosphere than a plain
    // linear blend.
    let t = clamp(dir.y * 0.5 + 0.5, 0.0, 1.0);
    var color = mix(camera.fog_color.rgb, camera.zenith_color.rgb, pow(t, 0.55));
    let haze = pow(1.0 - abs(dir.y), 5.0) * 0.2;
    color += camera.fog_color.rgb * haze;

    // Sun: a small sharp core plus a much softer, wider halo, both scaled
    // by sun_intensity so it fades out (rather than popping off) as it sets.
    let sun_dot = max(dot(dir, sun_dir), 0.0);
    let sun_core = pow(sun_dot, 800.0) * 6.0;
    let sun_halo = pow(sun_dot, 32.0) * 0.6;
    color += vec3<f32>(1.0, 0.92, 0.75) * (sun_core + sun_halo) * clamp(sun_intensity, 0.0, 1.0);

    // Moon: the same trick mirrored to -sun_dir, only visible once the sun
    // has properly set, pale and much dimmer than the sun.
    let night_amount = clamp(1.0 - sun_intensity * 3.0, 0.0, 1.0);
    let moon_dot = max(dot(dir, -sun_dir), 0.0);
    let moon = pow(moon_dot, 2000.0) * 1.2 + pow(moon_dot, 64.0) * 0.08;
    color += vec3<f32>(0.85, 0.88, 0.95) * moon * night_amount;

    // Stars: a sparse hash-thresholded field, only above the horizon and
    // only once night has properly set in, twinkling faintly via a slow
    // per-star phase offset so they aren't perfectly static.
    if dir.y > 0.05 {
        let cell = floor(dir.xz / dir.y * 40.0);
        let n = hash21(cell);
        if n > 0.9975 {
            let twinkle = 0.6 + 0.4 * sin(camera.light_params.z * 3.0 + n * 62.8);
            color += vec3<f32>(1.0) * (n - 0.9975) * 400.0 * night_amount * twinkle * 0.5;
        }
    }

    return vec4<f32>(grade(color), 1.0);
}
