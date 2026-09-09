struct CameraUniform {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    // Sky color straight overhead, paired with fog_color (the horizon) for
    // a real gradient -- see daynight.rs's zenith_color and sky.wgsl.
    zenith_color: vec4<f32>,
    sun_dir: vec4<f32>,
    // x = ambient, y = sun_intensity, z = free-running clock (seconds) for
    // the water wave animation, w = wind_strength (multiplies grass/leaf
    // sway amplitude below -- see weather.rs's Weather::wind_strength)
    light_params: vec4<f32>,
    // x = lightning_flash, 0..1 (see App::update_lightning) -- blended
    // toward white in fs_main below during a storm's lightning strike.
    // y = cloud coverage, z = camera eye underwater, w = reserved.
    weather_fx: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;
// One 256x256 layer per `CreatureKind` (see `model.rs`'s `push_model`) --
// Sheep/Chicken have no real texture and keep an unused blank layer, so
// every kind can index this array directly by its own `to_u8()`.
@group(1) @binding(2)
var creature_texture: texture_2d_array<f32>;

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
    @location(8) tex_layer: f32,
    @location(9) glimmer: f32,
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
    @location(7) tex_layer: f32,
    @location(8) glimmer: f32,
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
        let wind_strength = camera.light_params.w;
        let sway = (sin(t * 1.6 + pos.x * 0.9 + pos.z * 0.7) * 0.09
            + sin(t * 2.3 + pos.x * 0.3 - pos.z * 0.5) * 0.05) * wind_strength;
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
    out.tex_layer = in.tex_layer;
    out.glimmer = in.glimmer;
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

// Krzysztof Narkowicz's fitted ACES approximation -- cheap (no LUT/matrices)
// filmic tonemap that compresses highlights with a soft shoulder instead of
// hard-clipping them to white, and deepens shadows slightly for more
// contrast. Also used by sky.wgsl, kept as a small self-contained copy in
// each shader file rather than a shared include (this project has no WGSL
// include mechanism, and duplicating five lines is simpler than adding one).
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// A full tonemap curve reshapes the *entire* range, not just the highlights
// past 1.0 -- there's no way to stay strictly identity below 1.0 and only
// affect values above it, since the display can't show anything past 1.0
// either way. So rather than applying the curve at full strength (which
// visibly oversaturated ordinary diffuse surfaces like grass in testing),
// blend a modest amount of it over the original hard-clamped linear
// result: normal-brightness surfaces stay close to how they always looked,
// while pixels pushed well past 1.0 by additive highlights (specular, sky
// reflection, emissive glow) -- which the blend is dominated by, since the
// linear side is already pinned at 1.0 there -- still pick up a
// noticeably softer, richer rolloff than a flat clip to white.
const TONEMAP_BLEND: f32 = 0.35;

fn grade(color: vec3<f32>) -> vec3<f32> {
    let linear = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let toned = aces_tonemap(color);
    return mix(linear, toned, TONEMAP_BLEND);
}

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

    // A skinned creature's real texture lives in its own array layer
    // (`tex_layer` is `CreatureKind::to_u8() + 1.0`, see `model.rs`); every
    // other vertex (terrain, and a creature's flat-colored rigid parts)
    // keeps sampling the shared terrain atlas as before.
    var tex: vec4<f32>;
    if in.tex_layer > 0.5 {
        tex = textureSample(creature_texture, atlas_sampler, in.uv, i32(in.tex_layer - 0.5));
    } else {
        tex = textureSample(atlas_texture, atlas_sampler, in.uv);
    }
    if tex.a < 0.5 {
        discard;
    }

    let ndotl = max(dot(shading_normal, camera.sun_dir.xyz), 0.0);
    let ambient = camera.light_params.x;
    let sun_intensity = camera.light_params.y;
    let shadow = select(1.0, shadow_factor(in.world_pos, ndotl), sun_intensity > 0.0);
    let base = tex.rgb * in.color * in.ao;
    // Deliberately kept exactly as before (a flat scalar, hard-clamped):
    // tinting this per-channel by the sky color was tried and reverted --
    // multiplied straight onto an already-saturated texture like grass, it
    // recolored ordinary matte terrain far more than intended, well before
    // `grade`'s tonemap even entered the picture. The "epic" lighting
    // upgrades (sky gradient/sun/moon/stars, filmic highlight rolloff,
    // emissive glow/pulse) all live elsewhere and don't need this term
    // touched.
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
    // lighting math -- it never affects neighboring geometry. A gentle
    // pulse (phased by world position so a whole ore vein doesn't pulse in
    // lockstep) and a brightness boost give emissive blocks a "hot",
    // faintly alive look even without a real bloom pass -- `grade` below
    // gives the boosted highlight a soft rolloff instead of just clipping.
    let pulse = 0.85 + 0.15 * sin(camera.light_params.z * 2.2 + in.world_pos.x * 0.3 + in.world_pos.z * 0.3);
    let glow = tex.rgb * in.emission * 1.6 * pulse;
    // Sparse world-anchored pixel facets catch light as the camera moves.
    // Derivatives fade subpixel detail before it can shimmer at a distance.
    let face_uv = vec2<f32>(
        dot(in.world_pos, vec3<f32>(abs(in.normal.z), abs(in.normal.x), abs(in.normal.y))),
        dot(in.world_pos, vec3<f32>(abs(in.normal.y), abs(in.normal.z), abs(in.normal.x)))
    ) * 16.0;
    let footprint = max(length(dpdx(face_uv)), length(dpdy(face_uv)));
    var glimmer = vec3<f32>(0.0);
    if in.glimmer > 0.0 {
        let cell = floor(face_uv);
        let seed = fract(sin(dot(cell, vec2<f32>(127.1, 311.7))
            + dot(floor(in.world_pos - in.normal * 0.01), vec3<f32>(17.3, 43.1, 91.7))) * 43758.5453);
        let phase = seed * 6.283185;
        let facet = normalize(shading_normal + vec3<f32>(sin(phase), cos(phase * 1.7), sin(phase * 2.3)) * 0.65);
        let catch_light = pow(max(dot(facet, half_dir), 0.0), 36.0);
        let twinkle = pow(0.5 + 0.5 * sin(camera.light_params.z * 1.4 + phase), 8.0);
        let bright = max(tex.r, max(tex.g, tex.b));
        let chroma = bright - min(tex.r, min(tex.g, tex.b));
        // Ores: favor colored mineral flecks and bright inclusions over dull host rock.
        let inclusion = select(1.0, smoothstep(0.05, 0.22, chroma) + smoothstep(0.45, 0.8, bright) * 0.5, in.glimmer < 0.6);
        let visibility = (1.0 - smoothstep(0.6, 1.8, footprint)) * in.ao;
        let illumination = sun_intensity * shadow * ndotl + ambient * 0.18 + in.emission * 0.3;
        let sparkle = step(0.82, seed) * (catch_light * 2.5 + twinkle * 0.55);
        let sheen = pow(spec_angle, 40.0) * sun_intensity * shadow * ndotl * 0.65;
        glimmer = mix(vec3<f32>(1.0), tex.rgb, 0.25) * in.glimmer
            * clamp(inclusion, 0.0, 1.0) * (sparkle * visibility * illumination + sheen);
    }
    let reflected = lit + sky_reflection + vec3<f32>(specular) + glow + glimmer;

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let underwater = camera.weather_fx.z;
    let fog_start = mix(70.0, 1.5, underwater);
    let fog_end = mix(160.0, 26.0, underwater);
    let fog_amount = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);

    let fog_tint = mix(camera.fog_color.rgb, vec3<f32>(0.025, 0.16, 0.28), underwater);
    let tinted = mix(reflected, reflected * vec3<f32>(0.50, 0.78, 0.95) + vec3<f32>(0.01, 0.04, 0.09), underwater);
    let final_color = mix(tinted, fog_tint, fog_amount);
    // Lightning flash: a straight blend to white over the final graded
    // color, so a strike reads as a clean, even whiteout regardless of
    // what was underneath -- applied identically in sky.wgsl so the whole
    // screen (terrain and sky both) flashes together, not just one or the
    // other.
    let flashed = mix(grade(final_color), vec3<f32>(1.0), camera.weather_fx.x);
    return vec4<f32>(flashed, 1.0);
}
