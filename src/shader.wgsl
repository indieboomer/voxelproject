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
    // y = cloud coverage, z = camera eye underwater, w = surface wetness.
    weather_fx: vec4<f32>,
    camp_lights: array<vec4<f32>,4>,
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
    @location(9) flow: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    var pos = in.position;
    out.flow = vec2<f32>(0.0);
    if in.wind < -0.5 {
        let angle = -in.wind - 4.14159265;
        out.flow = vec2<f32>(cos(angle), sin(angle));
    }
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

    // Four bilinear PCF taps cover a soft footprint with fewer fetches than
    // the previous nine-tap square. Level-zero sampling permits early-outs.
    let depth = ndc.z - bias;
    let lit = textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(-0.75, -0.25) * texel, depth)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(0.25, -0.75) * texel, depth)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(0.75, 0.25) * texel, depth)
        + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(-0.25, 0.75) * texel, depth);
    // Avoid a hard moving edge at the limited-resolution shadow coverage.
    let edge = smoothstep(0.85, 1.0, max(abs(ndc.x), abs(ndc.y)));
    return mix(lit * 0.25, 1.0, edge);
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
const TONEMAP_BLEND: f32 = 0.55;

fn grade(color: vec3<f32>) -> vec3<f32> {
    let linear = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let toned = aces_tonemap(color * 0.9);
    return mix(linear, toned, TONEMAP_BLEND);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Procedural fire and stippled smoke cards need no animated textures,
    // offscreen targets, transparency sorting, or per-particle draw calls.
    if in.tex_layer < -0.5 {
        let uv=in.uv;
        let t=camera.light_params.z;
        if in.tex_layer > -1.5 {
            let wobble=sin(uv.y*9.0-t*8.0+in.world_pos.x*2.0)*0.10*uv.y;
            let width=(1.0-uv.y)*0.48;
            if abs(uv.x-0.5+wobble)>width {discard;}
            let core=1.0-abs(uv.x-0.5+wobble)/max(width,0.01);
            let fire=mix(vec3<f32>(1.6,0.12,0.01),vec3<f32>(2.8,1.7,0.25),core*(1.0-uv.y));
            return vec4<f32>(grade(fire),1.0);
        }
        let age=-in.tex_layer-2.0;
        let roundness=length((uv-vec2<f32>(0.5))*2.0);
        let alpha=(1.0-smoothstep(0.35,1.0,roundness))*(1.0-age)*0.3;
        let pixel=vec2<u32>(in.clip_position.xy);
        let dither=f32((pixel.x*3u+pixel.y*5u)%16u)/16.0;
        if alpha<=dither {discard;}
        return vec4<f32>(vec3<f32>(0.14,0.13,0.12)*(0.3+camera.light_params.x),1.0);
    }
    let rain_exposure = step(1.5, in.reflectivity);
    let reflectivity = in.reflectivity - rain_exposure * 2.0;
    let is_water = reflectivity > WATER_REFLECTIVITY_THRESHOLD;
    let wet = camera.weather_fx.w * rain_exposure * (0.25 + 0.75 * max(in.normal.y, 0.0))
        * (1.0 - clamp(in.emission, 0.0, 1.0));

    // Slightly wavy *still* water: only the shading normal is animated (the
    // sun glint and sky reflection dance across the surface), never the
    // sampled texture coordinate -- perturbing UVs pushed sampling past the
    // water tile's own edge in the atlas and bled in whatever tile happens
    // to sit next to it there, showing up as flickering white margins. This
    // keeps water reading as one smooth, undistorted surface.
    var shading_normal = in.normal;
    var current_foam = 0.0;
    if is_water {
        let t = camera.light_params.z;
        let moving = dot(in.flow, in.flow) > 0.25;
        let wx = in.world_pos.x * 0.6 + t * 1.3;
        let wz = in.world_pos.z * 0.5 + t * 1.7;
        shading_normal = normalize(
            in.normal + vec3<f32>(cos(wx) * 0.6, 0.0, -sin(wz) * 0.6) * 0.15
        );
        if moving {
            // A continuous world-space phase avoids seams when neighboring
            // blocks have slightly different river tangents, even far from origin.
            let river = in.flow.x > 0.1;
            let along = select(-in.world_pos.z, in.world_pos.x, river) - t * 1.4;
            let across = select(in.world_pos.x, in.world_pos.z, river);
            let wave = cos(along * 2.0 + sin(across * 0.5)) * 0.10;
            shading_normal = normalize(in.normal + vec3<f32>(-in.flow.x * wave, 0.0, -in.flow.y * wave));
            let ripple = sin(along * 5.0 + sin(across * 2.0));
            current_foam = smoothstep(0.86, 1.0, ripple) * 0.10;
        }
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
    var shadow = 1.0;
    // `select` evaluates both operands: the old shader sampled shadows even
    // at night. This branch also skips back-facing surfaces entirely.
    if sun_intensity > 0.001 && ndotl > 0.0 {
        shadow = shadow_factor(in.world_pos, ndotl);
    }
    let material_wet = select(wet, 0.0, is_water);
    let base = tex.rgb * in.color * (1.0 - material_wet * 0.28);
    // Hemisphere fill and a subtle warm ground bounce give normals shape
    // without irradiance probes. AO mainly occludes indirect illumination.
    let hemisphere = shading_normal.y * 0.5 + 0.5;
    let fill_tint = mix(vec3<f32>(1.02, 0.96, 0.88), vec3<f32>(0.94, 0.98, 1.04), hemisphere);
    let fill = fill_tint * ambient * mix(0.65, 1.15, hemisphere) * in.ao;
    let sun_tint = mix(vec3<f32>(1.0, 0.79, 0.60), vec3<f32>(1.0, 0.98, 0.93), smoothstep(0.0, 0.45, camera.sun_dir.w));
    let direct = sun_tint * sun_intensity * ndotl * shadow * mix(0.8, 1.0, in.ao);
    var local_light=vec3<f32>(0.0);
    let night=1.0-smoothstep(-0.1,0.25,camera.sun_dir.w);
    if night>0.001 && camera.camp_lights[0].w>0.0 {
        for (var i=0u;i<4u;i=i+1u) {
            let light=camera.camp_lights[i];
            if light.w<=0.0 {continue;}
            let delta=light.xyz-in.world_pos;
            let distance2=dot(delta,delta);
            if distance2<light.w*light.w {
                let fade=1.0-distance2/(light.w*light.w);
                let facing=max(dot(shading_normal,delta*inverseSqrt(max(distance2,0.01))),0.0);
                let flicker=0.92+0.08*sin(camera.light_params.z*7.0+light.x);
                local_light+=vec3<f32>(1.0,0.38,0.09)*fade*fade*(0.2+facing)*night*flicker*1.8;
            }
        }
    }
    let lit = base * (fill + direct + local_light * in.ao);

    // View-dependent sky reflection and roughness-dependent sun highlights.
    // Dry matte blocks stay diffuse; rain adds a reflective surface coat.
    let view_dir = normalize(camera.camera_pos.xyz - in.world_pos);
    let grazing = 1.0 - max(dot(shading_normal, view_dir), 0.0);
    let grazing2 = grazing * grazing;
    let fresnel = grazing2 * grazing2 * grazing;
    let roughness = select(mix(0.85 - reflectivity * 0.55, 0.22, material_wet), 0.15, is_water);
    let reflection_dir = reflect(-view_dir, shading_normal);
    let sky_gradient = mix(camera.fog_color.rgb, camera.zenith_color.rgb, clamp(reflection_dir.y, 0.0, 1.0));
    // Analytic environment reflection: no cubemap, screen-space ray march,
    // or extra samples. Wet surfaces gain a clear coat at grazing angles.
    let overcast = vec3<f32>(0.48, 0.51, 0.55) * ambient * 2.2;
    let environment = mix(sky_gradient, overcast, camera.weather_fx.y * 0.7);
    let reflection_strength = mix(reflectivity * 0.45, 0.55, material_wet);
    let sky_reflection = environment * reflection_strength * mix(0.08, 1.0, fresnel) * in.ao;
    let half_dir = normalize(view_dir + camera.sun_dir.xyz);
    let spec_angle = max(dot(shading_normal, half_dir), 0.0);
    let exponent = mix(12.0, 128.0, (1.0 - roughness) * (1.0 - roughness));
    let coat = max(reflectivity, material_wet * 0.7);
    let specular = pow(spec_angle, exponent) * coat * sun_intensity * shadow * ndotl;
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
    let reflected = lit + sky_reflection + sun_tint * specular + glow + glimmer
        + vec3<f32>(0.65, 0.85, 0.92) * current_foam * (ambient + sun_intensity * ndotl * shadow);

    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let underwater = camera.weather_fx.z;
    let fog_start = mix(mix(70.0, 40.0, camera.weather_fx.y), 1.5, underwater);
    let fog_end = mix(160.0, 26.0, underwater);
    let fog_linear = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    let fog_amount = fog_linear * fog_linear * (3.0 - 2.0 * fog_linear);

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
