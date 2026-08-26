struct CameraUniform {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    sun_dir: vec4<f32>,
    // x = ambient, y = sun_intensity
    light_params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

@group(2) @binding(0)
var shadow_map: texture_depth_2d;
@group(2) @binding(1)
var shadow_sampler: sampler_comparison;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) ao: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) ao: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    out.world_pos = in.position;
    out.normal = in.normal;
    out.uv = in.uv;
    out.ao = in.ao;
    return out;
}

const SHADOW_MAP_SIZE: f32 = 2048.0;

// Orthographic light projection is affine, so re-transforming the already
// (perspective-correctly) interpolated fragment world position by it gives
// the same result as if this were computed per-vertex and interpolated --
// no extra vertex-to-fragment varying needed.
fn shadow_factor(world_pos: vec3<f32>, ndotl: f32) -> f32 {
    let light_space = camera.light_view_proj * vec4<f32>(world_pos, 1.0);
    let ndc = light_space.xyz / light_space.w;
    if ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0 {
        return 1.0; // outside the shadow frustum -- don't shadow it
    }
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    // Slope-scaled bias: grazing-angle surfaces (low ndotl) need a larger
    // bias to avoid acne since depth changes faster across their texels.
    let bias = clamp(0.0035 * (1.0 - ndotl), 0.0007, 0.006);
    let texel = 1.0 / SHADOW_MAP_SIZE;

    var lit = 0.0;
    for (var dx = -1; dx <= 1; dx += 1) {
        for (var dy = -1; dy <= 1; dy += 1) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * texel;
            lit += textureSampleCompare(shadow_map, shadow_sampler, uv + offset, ndc.z - bias);
        }
    }
    return lit / 9.0;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(atlas_texture, atlas_sampler, in.uv);
    if tex.a < 0.5 {
        discard;
    }

    let ndotl = max(dot(in.normal, camera.sun_dir.xyz), 0.0);
    let ambient = camera.light_params.x;
    let sun_intensity = camera.light_params.y;
    let shadow = select(1.0, shadow_factor(in.world_pos, ndotl), sun_intensity > 0.0);
    let base = tex.rgb * in.color * in.ao;
    let lit = base * clamp(ambient + sun_intensity * ndotl * shadow, 0.0, 1.0);

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = 70.0;
    let fog_end = 160.0;
    let fog_amount = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    let final_color = mix(lit, camera.fog_color.rgb, fog_amount);
    return vec4<f32>(final_color, 1.0);
}
