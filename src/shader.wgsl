struct CameraUniform {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    sun_dir: vec4<f32>,
    // x = ambient, y = sun_intensity, z = free-running clock (seconds) for
    // the water wave animation
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
    @location(5) reflectivity: f32,
    @location(6) emission: f32,
    @location(7) wind: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) ao: f32,
    @location(5) reflectivity: f32,
    @location(6) emission: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    var pos = in.position;
    if (in.wind > 0.0) {
        // Gentle per-tuft sway: only the top of a cross-billboard card
        // (short grass) moves -- the base stays planted -- and each tuft's
        // world x/z feed the phase so a whole field doesn't sway in lockstep.
        let t = camera.light_params.z;
        let sway = sin(t * 1.6 + pos.x * 0.9 + pos.z * 0.7) * 0.09
            + sin(t * 2.3 + pos.x * 0.3 - pos.z * 0.5) * 0.05;
        pos.x += sway * in.wind;
        pos.z += sway * 0.6 * in.wind;
    }
    out.clip_position = camera.view_proj * vec4<f32>(pos, 1.0);
    out.color = in.color;
    out.world_pos = pos;
    out.normal = in.normal;
    out.uv = in.uv;
    out.ao = in.ao;
    out.reflectivity = in.reflectivity;
    out.emission = in.emission;
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

// Water is the only block whose reflectivity is this high (0.85, vs. 0.5
// for crystal and 0.15 for stone/redstone) -- reused here as a cheap "is
// this a water fragment" test instead of threading a whole extra per-vertex
// flag through the mesher just for this.
const WATER_REFLECTIVITY_THRESHOLD: f32 = 0.7;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let is_water = in.reflectivity > WATER_REFLECTIVITY_THRESHOLD;

    // Slightly wavy *still* water: only the shading normal is animated (the
    // sun glint and sky reflection dance across the surface), never the
    // sampled texture coordinate -- perturbing UVs pushed sampling past the
    // water tile's own edge in the atlas and bled in whatever tile happens
    // to sit next to it there, showing up as flickering white margins. This
    // keeps water reading as one smooth, undistorted surface.
    var shading_normal = in.normal;
    if is_water {
        let t = camera.light_params.z;
        let wx = in.world_pos.x * 0.6 + t * 1.3;
        let wz = in.world_pos.z * 0.5 + t * 1.7;
        shading_normal = normalize(
            in.normal + vec3<f32>(cos(wx) * 0.6, 0.0, -sin(wz) * 0.6) * 0.15
        );
    }

    let tex = textureSample(atlas_texture, atlas_sampler, in.uv);
    if tex.a < 0.5 {
        discard;
    }

    let ndotl = max(dot(shading_normal, camera.sun_dir.xyz), 0.0);
    let ambient = camera.light_params.x;
    let sun_intensity = camera.light_params.y;
    let shadow = select(1.0, shadow_factor(in.world_pos, ndotl), sun_intensity > 0.0);
    let base = tex.rgb * in.color * in.ao;
    let lit = base * clamp(ambient + sun_intensity * ndotl * shadow, 0.0, 1.0);

    // Cheap reflection for shiny materials (water, crystal, stone): a
    // fresnel-weighted tint of the sky color plus a Blinn-Phong sun glint,
    // both scaled by the per-vertex `reflectivity` so matte blocks (grass,
    // dirt, wood...) are completely unaffected. Uses the (possibly
    // wave-perturbed) shading normal so water's glint shimmers too.
    let view_dir = normalize(camera.camera_pos.xyz - in.world_pos);
    let fresnel = pow(clamp(1.0 - max(dot(shading_normal, view_dir), 0.0), 0.0, 1.0), 5.0);
    let sky_reflection = camera.fog_color.rgb * in.reflectivity * mix(0.15, 1.0, fresnel);
    let half_dir = normalize(view_dir + camera.sun_dir.xyz);
    let spec_angle = max(dot(shading_normal, half_dir), 0.0);
    let specular = pow(spec_angle, 64.0) * in.reflectivity * sun_intensity * shadow;
    // Emission is a purely visual glow on the block's own surface (ores),
    // added on top of the lit/reflected result rather than folded into the
    // lighting math -- it never affects neighboring geometry.
    let glow = tex.rgb * in.emission;
    let reflected = lit + sky_reflection + vec3<f32>(specular) + glow;

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let fog_start = 70.0;
    let fog_end = 160.0;
    let fog_amount = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    let final_color = mix(reflected, camera.fog_color.rgb, fog_amount);
    return vec4<f32>(final_color, 1.0);
}
