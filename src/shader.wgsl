// Smooth, deterministic world-space variation: no block/chunk boundaries or repeating grid.
fn water_hash(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    q += vec3<f32>(dot(q, q.yzx + vec3<f32>(33.33)));
    return fract((q.x + q.y) * q.z);
}
fn water_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(water_hash(cell), water_hash(cell + vec2<f32>(1.0, 0.0)), u.x),
        mix(water_hash(cell + vec2<f32>(0.0, 1.0)), water_hash(cell + vec2<f32>(1.0, 1.0)), u.x), u.y);
}
fn water_slope(p: vec2<f32>, t: f32) -> vec2<f32> {
    let warp = water_noise(p * 0.037) * 3.0;
    let a = dot(p, vec2<f32>(0.51, 0.23)) + t * 0.42 + warp;
    let b = dot(p, vec2<f32>(-0.19, 0.43)) - t * 0.31 + warp * 1.7;
    let c = dot(p, vec2<f32>(0.83, -0.57)) + t * 0.57;
    return (vec2<f32>(0.051, 0.023) * cos(a)
        + vec2<f32>(-0.019, 0.043) * cos(b)
        + vec2<f32>(0.021, -0.014) * cos(c))
        * (0.7 + 0.6 * water_noise(p * 0.071 + vec2<f32>(19.7, 5.3)));
}
// Natural-coast shore foam (docs/NATURAL_COAST.md): broken bands travel
// shoreward at locally varying phases, plus a lacy swash line at contact.
// `shore` is 1 at land contact and 0 three cells out (narrower at rock).
fn shore_foam(p: vec2<f32>, shore: f32, t: f32) -> f32 {
    if shore <= 0.02 { return 0.0; }
    let drift = water_noise(p * 0.35 + vec2<f32>(t * 0.03, -t * 0.02));
    let wave = fract(shore * 1.8 - t * 0.16 + drift * 0.9);
    let band = smoothstep(0.70, 0.86, wave) * (1.0 - smoothstep(0.88, 0.98, wave));
    let patches = smoothstep(0.30, 0.62, water_noise(p * 0.8 + vec2<f32>(11.0, t * 0.05)));
    let swash = smoothstep(0.78, 0.98, shore) * (0.45 + 0.55 * water_noise(p * 2.1 + vec2<f32>(t * 0.25, 3.0)));
    let lace = smoothstep(0.30, 0.55, water_noise(p * 5.5 + vec2<f32>(-t * 0.3, t * 0.2)));
    return clamp((band * patches * smoothstep(0.2, 0.6, shore) + swash) * lace, 0.0, 1.0);
}
// Face-plane world coordinates for procedural material grain.
fn face_plane(p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let a = abs(n);
    if a.y > 0.7 { return p.xz; }
    if a.x > a.z { return vec2<f32>(p.z, p.y); }
    return p.xy;
}
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
    // x = water Fresnel toggle (1 enabled, 0 disabled), +2 in the natural
    // (shaped terrain) look; y = depth of field, z/w = near/far.
    graphics: vec4<f32>,
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
@group(3) @binding(0) var light_visibility:texture_3d<f32>;
struct LightOrigins {origins:array<vec4<f32>,4>};
@group(3) @binding(1) var<uniform> light_origins:LightOrigins;
fn local_visibility(index:u32,pos:vec3<f32>)->f32 {
    let origin=light_origins.origins[index];if origin.w==0.0 {return 1.0;}
    let cell=vec3<i32>(floor(pos-origin.xyz));
    if any(cell<vec3<i32>(0)) || any(cell>=vec3<i32>(18)) {return 0.0;}
    return textureLoad(light_visibility,cell+vec3<i32>(0,0,i32(index)*18),0).r;
}

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
    @location(10) skylight: f32,
    @location(11) wet: vec2<f32>,
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
    @location(10) skylight: f32,
    @location(11) wet: vec2<f32>,
    // Wind sway weight (> 0 only for growing foliage and plant cards).
    @location(12) sway: f32,
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
    out.ao = abs(in.ao);
    out.skylight = in.skylight;
    out.wet = in.wet;
    out.sway = in.wind;
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

    // A separable [1,2,1] tent, bilinearly shifted with the receiver's texel
    // phase. Pair adjacent weights into four hardware bilinear comparisons.
    // Unlike the sparse rotated pattern, coverage varies smoothly at edges.
    let depth = ndc.z - bias;
    let grid=uv*SHADOW_MAP_SIZE+0.5;
    let cell=floor(grid);
    let phase=grid-cell;
    let w0=3.0-2.0*phase;
    let w1=1.0+2.0*phase;
    let o0=(2.0-phase)/w0-1.0;
    let o1=phase/w1+1.0;
    let base_uv=(cell-0.5)*texel;
    let lit = w0.x*w0.y*textureSampleCompareLevel(shadow_map, shadow_sampler, base_uv+vec2<f32>(o0.x,o0.y)*texel,depth)
        + w1.x*w0.y*textureSampleCompareLevel(shadow_map, shadow_sampler, base_uv+vec2<f32>(o1.x,o0.y)*texel,depth)
        + w0.x*w1.y*textureSampleCompareLevel(shadow_map, shadow_sampler, base_uv+vec2<f32>(o0.x,o1.y)*texel,depth)
        + w1.x*w1.y*textureSampleCompareLevel(shadow_map, shadow_sampler, base_uv+vec2<f32>(o1.x,o1.y)*texel,depth);
    // Avoid a hard moving edge at the limited-resolution shadow coverage.
    let edge = smoothstep(0.85, 1.0, max(abs(ndc.x), abs(ndc.y)));
    return mix(lit * 0.0625, 1.0, edge);
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
        let grid=select(8.0,12.0,in.tex_layer > -1.5);
        let cell=floor(in.uv*grid);
        let uv=(cell+vec2<f32>(0.5))/grid;
        let t=floor(camera.light_params.z*8.0)/8.0;
        if in.tex_layer > -1.5 {
            let wobble=sin(uv.y*9.0-t*8.0)*0.10*uv.y;
            let width=(1.0-uv.y)*0.48;
            if abs(uv.x-0.5+wobble)>width {discard;}
            let core=1.0-abs(uv.x-0.5+wobble)/max(width,0.01);
            let band=floor(core*(1.0-uv.y)*4.0)/3.0;
            let fire=mix(vec3<f32>(1.6,0.12,0.01),vec3<f32>(2.8,1.7,0.25),clamp(band,0.0,1.0));
            return vec4<f32>(grade(fire),1.0);
        }
        let age=-in.tex_layer-2.0;
        let edge=abs(uv-vec2<f32>(0.5));
        if max(edge.x,edge.y)>0.44 || edge.x+edge.y>0.65 {discard;}
        // Particle-local pixel mask: chunky puffs, not screen-space stippling.
        let pixel=vec2<u32>(cell);
        let fade=f32((pixel.x*3u+pixel.y*5u)%16u)/16.0;
        if age>0.35 && fade<(age-0.35)/0.65 {discard;}
        let shade=0.28+f32((pixel.x+pixel.y)%3u)*0.035;
        return vec4<f32>(vec3<f32>(shade)*(0.35+camera.light_params.x),1.0);
    }
    let rain_exposure = step(1.5, in.reflectivity);
    let reflectivity = in.reflectivity - rain_exposure * 2.0;
    let is_water = reflectivity > WATER_REFLECTIVITY_THRESHOLD;
    let absorption=clamp(in.wet.x,0.0,1.0);
    let terrain_wet=camera.weather_fx.w*rain_exposure*(0.25+0.75*max(in.normal.y,0.0));
    let moisture=select(clamp(in.wet.y,0.0,1.0)*(0.65+0.35*max(in.normal.y,0.0)),terrain_wet,in.wet.y<0.0);
    // Broad, stable variation breaks up uniform wet pavement without extra textures.
    // Models use UVs, so the pattern follows animation instead of sliding in space.
    let pattern_pos=select(vec3<f32>(in.uv*4.0,0.0),in.world_pos,in.wet.y<0.0);
    let damp_variation=0.88+0.12*sin(dot(pattern_pos,vec3<f32>(1.31,0.73,1.17)));
    let wet=moisture*mix(1.0,damp_variation,absorption)*(1.0-clamp(in.emission,0.0,1.0));

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
        // Slow, broad waves keep the surface calm and avoid rapid shimmer.
        let slope = water_slope(in.world_pos.xz, t);
        shading_normal = normalize(in.normal + vec3<f32>(slope.x, 0.0, slope.y));
        if moving {
            // A continuous world-space phase avoids seams when neighboring
            // blocks have slightly different river tangents, even far from origin.
            let river = in.flow.x > 0.1;
            let along = select(-in.world_pos.z, in.world_pos.x, river) - t * 0.48;
            let across = select(in.world_pos.x, in.world_pos.z, river);
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
    // Natural foliage tests a coarser mip's coverage: fewer, chunkier gaps.
    let foliage = in.tex_layer > 0.3 && in.tex_layer < 0.45;
    let coverage = select(tex.a, textureSampleLevel(atlas_texture, atlas_sampler, in.uv, 2.0).a, foliage);
    // Natural crowns read as dense leaf masses with occasional gaps.
    if coverage < select(0.5, 0.3, foliage) {
        discard;
    }
    // Natural terrain: the vertex color carries a world-space palette; keep
    // only a soft share of the atlas's pixel detail plus continuous grain.
    var albedo = tex.rgb;
    if in.tex_layer > 0.1 && in.tex_layer < 0.5 {
        let average = textureSampleLevel(atlas_texture, atlas_sampler, in.uv, 6.0).rgb;
        let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
        let detail = clamp(dot(tex.rgb, luma) / max(dot(average, luma), 0.02), 0.55, 1.45);
        let q = face_plane(in.world_pos, in.normal);
        let grain = 0.88 + 0.12 * water_noise(q * 2.9) + 0.08 * water_noise(q * 9.7 + vec2<f32>(7.3, 1.9));
        albedo = vec3<f32>(mix(1.0, detail, 0.3) * grain);
        if foliage {
            // Foliage palette in the vertex color; more leaf texture detail.
            albedo = vec3<f32>(mix(1.0, detail, 0.8) * grain);
        }
    }
    // Feature bits (settings::graphics_flags): 1 Fresnel, 2 natural, 4 toon.
    let flags = u32(camera.graphics.x + 0.5);
    let natural = (flags & 2u) != 0u;
    let toon = (flags & 4u) != 0u && !is_water;

    let ndotl = max(dot(shading_normal, camera.sun_dir.xyz), 0.0);
    let ambient = mix(0.012, camera.light_params.x, in.skylight);
    let sun_intensity = camera.light_params.y * in.skylight;
    var shadow = 1.0;
    // `select` evaluates both operands: the old shader sampled shadows even
    // at night. This branch also skips back-facing surfaces entirely.
    if sun_intensity > 0.001 && ndotl > 0.0 {
        shadow = shadow_factor(in.world_pos, ndotl);
    }
    let material_wet = select(wet, 0.0, is_water);
    let base = albedo * in.color * (1.0 - material_wet * (0.06+0.30*absorption));
    let view_dir = normalize(camera.camera_pos.xyz - in.world_pos);
    let roughness = select(mix(0.85 - reflectivity * 0.55, mix(0.16,0.55,absorption), material_wet), 0.15, is_water);
    let exponent = mix(12.0, 128.0, (1.0 - roughness) * (1.0 - roughness));
    // Hemisphere fill and a subtle warm ground bounce give normals shape
    // without irradiance probes. AO mainly occludes indirect illumination.
    let hemisphere = shading_normal.y * 0.5 + 0.5;
    let golden_hour = (1.0 - smoothstep(0.0, 0.4, camera.sun_dir.w))
        * smoothstep(-0.2, 0.0, camera.sun_dir.w);
    // The natural look: warmer sun, cooler sky fill.
    let fill_tint = mix(vec3<f32>(1.02, 0.96, 0.88), vec3<f32>(0.94, 0.98, 1.04), hemisphere)
        * mix(vec3<f32>(1.0), vec3<f32>(1.12, 0.94, 0.76), golden_hour)
        * select(vec3<f32>(1.0), vec3<f32>(0.90, 0.97, 1.10), natural);
    // Toon: cooler, flatter fill so lit and shadowed sides read as two tones.
    let fill = fill_tint * ambient * select(mix(0.65, 1.15, hemisphere), 0.9, toon) * in.ao
        * select(vec3<f32>(1.0), vec3<f32>(0.86, 0.94, 1.12), toon);
    let sun_tint = mix(vec3<f32>(1.0, 0.98, 0.93), vec3<f32>(1.0, 0.62, 0.24), golden_hour)
        * select(vec3<f32>(1.0), vec3<f32>(1.04, 0.96, 0.84), natural);
    // Toon: sunlight in soft-edged bands (shadow, half-lit, full) instead of a ramp.
    let sunlit = ndotl * shadow;
    let banded = smoothstep(0.03, 0.09, sunlit) * 0.55 + smoothstep(0.38, 0.46, sunlit) * 0.45;
    let direct = sun_tint * sun_intensity * select(sunlit, banded, toon) * mix(0.8, 1.0, in.ao);
    var local_light=vec3<f32>(0.0);
    var local_specular=vec3<f32>(0.0);
    let night=1.0-smoothstep(-0.1,0.25,camera.sun_dir.w);
    if camera.camp_lights[0].w!=0.0 {
        for (var i=0u;i<4u;i=i+1u) {
            let light=camera.camp_lights[i];
            if light.w==0.0 {continue;}
            let torch=light.w < -100.0;
            let radius=select(abs(light.w),abs(light.w)-100.0,torch);
            let sway=select(vec3<f32>(0.0),vec3<f32>(sin(camera.light_params.z*8.3+light.z)*0.06,sin(camera.light_params.z*11.7)*0.035,cos(camera.light_params.z*6.7+light.x)*0.06),torch);
            let delta=light.xyz+sway-in.world_pos;
            let distance2=dot(delta,delta);
            if distance2<radius*radius {
                let fade=1.0-distance2/(radius*radius);
                let facing=max(dot(shading_normal,delta*inverseSqrt(max(distance2,0.01))),0.0);
                let cool=light.w<0.0 && !torch;
                let flicker=select(0.92+0.08*sin(camera.light_params.z*7.0+light.x),1.0,cool);
                let tint=select(select(vec3<f32>(1.0,0.38,0.09),vec3<f32>(1.0,0.72,0.22),torch),vec3<f32>(0.72,0.86,1.0),cool);
                // Portable lights also work in daytime caves.
                let strength=select(max(night,1.0-in.skylight),1.0,cool || torch);
                // Original block AO: occlude the combined local diffuse below.
                let local_diffuse=0.2+facing;
                let visibility=local_visibility(i,in.world_pos+in.normal*0.08);
                let energy=tint*fade*fade*strength*flicker*1.8*visibility;
                local_light+=energy*local_diffuse;
                if material_wet>0.005 && facing>0.0 {
                    let half_vector=normalize(view_dir+delta*inverseSqrt(max(distance2,0.01)));
                    let highlight=pow(max(dot(shading_normal,half_vector),0.0),exponent);
                    local_specular+=energy*highlight*facing*material_wet*mix(0.6,0.18,absorption)*mix(0.8,1.0,in.ao);
                }
            }
        }
    }
    let lit = base * (fill + direct + local_light * in.ao)+local_specular;

    // View-dependent sky reflection and roughness-dependent sun highlights.
    // Dry matte blocks stay diffuse; rain adds a reflective surface coat.
    let grazing = 1.0 - max(dot(shading_normal, view_dir), 0.0);
    let grazing2 = grazing * grazing;
    // Broaden the Fresnel lobe: reflections remain visible away from the
    // horizon while still becoming strongest at shallow viewing angles.
    // Keep a small base reflection when disabled; the setting only removes
    // the view-angle boost and leaves screen-space reflections intact.
    let fresnel_base = select(0.06, 0.06 + 0.94 * grazing2 * grazing, (flags & 1u) != 0u);
    // Break up large, repeated reflection patches with a subtle continuous
    // world-space modulation. It is deliberately low contrast so waves and
    // reflected silhouettes remain the visual focus.
    let fresnel_noise = 0.90 + 0.10 * water_noise(in.world_pos.xz * 0.13);
    let fresnel = fresnel_base * fresnel_noise;
    let reflection_dir = reflect(-view_dir, shading_normal);
    let sky_gradient = mix(camera.fog_color.rgb, camera.zenith_color.rgb, clamp(reflection_dir.y, 0.0, 1.0));
    // Analytic environment reflection: no cubemap, screen-space ray march,
    // or extra samples. Wet surfaces gain a clear coat at grazing angles.
    let overcast = vec3<f32>(0.48, 0.51, 0.55) * ambient * 2.2;
    let environment = mix(sky_gradient, overcast, camera.weather_fx.y * 0.7) * in.skylight;
    let reflection_strength = mix(reflectivity * 0.45, mix(0.32,0.12,absorption) + reflectivity * 0.35, material_wet);
    let sky_reflection = environment * reflection_strength * mix(0.08, 1.0, fresnel) * in.ao;
    let half_dir = normalize(view_dir + camera.sun_dir.xyz);
    let spec_angle = max(dot(shading_normal, half_dir), 0.0);
    let coat = max(reflectivity, material_wet * mix(0.7,0.3,absorption));
    let specular = pow(spec_angle, exponent) * coat * sun_intensity * shadow * ndotl;
    // Emission is a purely visual glow on the block's own surface (ores),
    // added on top of the lit/reflected result rather than folded into the
    // lighting math -- it never affects neighboring geometry. A gentle
    // pulse (phased by world position so a whole ore vein doesn't pulse in
    // lockstep) and a brightness boost give emissive blocks a "hot",
    // faintly alive look even without a real bloom pass -- `grade` below
    // gives the boosted highlight a soft rolloff instead of just clipping.
    let pulse = 0.85 + 0.15 * sin(camera.light_params.z * 2.2 + in.world_pos.x * 0.3 + in.world_pos.z * 0.3);
    // Procedural props/particles use a white atlas texel with vertex tint.
    // Their glow must retain that tint rather than washing every cue white.
    let glow = tex.rgb * in.color * in.emission * 1.6 * pulse;
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
    var reflected = lit + sky_reflection + sun_tint * specular + glow + glimmer
        + vec3<f32>(0.65, 0.85, 0.92) * current_foam * (ambient + sun_intensity * ndotl * shadow);

    var water_mask = select(1.0, 0.0, is_water && in.normal.y > 0.5);
    if is_water && in.normal.y > 0.5 {
        // Schlick water/air interface: 2% reflection at normal incidence.
        var water_fresnel = 0.06 + 0.94 * fresnel;
        var body = mix(lit, vec3<f32>(0.025, 0.15, 0.19) * (fill + direct), 0.55);
        var foam = 0.0;
        if in.wet.x > 9.5 {
            // Natural shallows: turquoise over a visible seabed, deepening blue.
            let depth = in.wet.x - 10.0;
            let clarity = exp(-depth * 0.6);
            let column = mix(vec3<f32>(0.010, 0.085, 0.20), vec3<f32>(0.035, 0.40, 0.42), exp(-depth * 0.3));
            let seabed = in.color * vec3<f32>(0.55, 0.95, 0.90);
            body = mix(column, seabed, clarity * 0.75) * (fill + direct);
            water_fresnel *= 0.3;
            foam = shore_foam(in.world_pos.xz, clamp(in.wet.y, 0.0, 1.0), camera.light_params.z);
        }
        // Natural water: a softer sun glint so the turquoise stays readable.
        let glint = select(1.0, 0.45, in.wet.x > 9.5);
        reflected = mix(body, environment, water_fresnel) + sun_tint * specular * glint
            + vec3<f32>(0.65, 0.85, 0.92) * current_foam * (ambient + sun_intensity * ndotl * shadow);
        reflected = mix(reflected, vec3<f32>(0.93, 0.96, 0.97) * (fill + direct) * 1.1, foam * 0.9);
        // Foam is not a mirror: keep it out of the screen-space reflection pass.
        water_mask = select(water_mask, 1.0, foam > 0.35);
    }
    let dist = distance(in.world_pos, camera.camera_pos.xyz);
    let underwater = camera.weather_fx.z;
    let fog_start = mix(mix(70.0, 40.0, camera.weather_fx.y), 1.5, underwater);
    let fog_end = mix(160.0, 26.0, underwater);
    let fog_linear = clamp((dist - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    let fog_amount = fog_linear * fog_linear * (3.0 - 2.0 * fog_linear);

    let fog_tint = mix(camera.fog_color.rgb * in.skylight, vec3<f32>(0.025, 0.16, 0.28), underwater);
    let tinted = mix(reflected, reflected * vec3<f32>(0.50, 0.78, 0.95) + vec3<f32>(0.01, 0.04, 0.09), underwater);
    let final_color = mix(tinted, fog_tint, fog_amount);
    // Lightning flash: a straight blend to white over the final graded
    // color, so a strike reads as a clean, even whiteout regardless of
    // what was underneath -- applied identically in sky.wgsl so the whole
    // screen (terrain and sky both) flashes together, not just one or the
    // other.
    let flashed = mix(grade(final_color), vec3<f32>(1.0), camera.weather_fx.x * in.skylight);
    // Internal water mask; the reflection resolve restores opaque alpha.
    // 0.96 marks swaying foliage and tufts so ink outlines skip their cutout
    // holes; it stays above the reflection pass's 0.95 opaque threshold.
    let foliage_mask = select(water_mask, 0.96, (in.sway > 0.0 || foliage || (in.tex_layer > 0.01 && in.tex_layer < 0.1)) && water_mask > 0.5);
    return vec4<f32>(flashed, select(foliage_mask, 1.0, underwater >= 0.5));
}
