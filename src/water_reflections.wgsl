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
    graphics: vec4<f32>,
    camp_lights: array<vec4<f32>,4>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var scene: texture_2d<f32>;
@group(1) @binding(1) var depth: texture_depth_2d;
@group(1) @binding(2) var blur_sampler: sampler;
@group(2) @binding(0) var sunlight_rays: texture_2d<f32>;

@vertex fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
fn world_at(uv: vec2<f32>, z: f32) -> vec3<f32> {
    let h = camera.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, z, 1.0);
    return h.xyz / h.w;
}
// UV plus signed distance behind the nearest visible surface.
fn probe(p: vec3<f32>) -> vec3<f32> {
    let clip = camera.view_proj * vec4<f32>(p, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if clip.w <= 0.0 || ndc.z < 0.0 || ndc.z > 1.0 || any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        return vec3<f32>(uv, -10000.0);
    }
    let z = textureLoad(depth, vec2<i32>(uv * vec2<f32>(textureDimensions(depth))), 0);
    if z >= 1.0 { return vec3<f32>(uv, -10000.0); }
    return vec3<f32>(uv, distance(p, camera.camera_pos.xyz) - distance(world_at(uv, z), camera.camera_pos.xyz));
}
fn eye_depth(z: f32) -> f32 {
    let near = camera.graphics.z;
    let far = camera.graphics.w;
    return near * far / max(far - z * (far - near), 0.0001);
}

// Tiny depth-aware kernel in the existing resolve: no new targets or passes.
// Reject other depth layers so a near silhouette cannot smear the background.
fn near_blur(pixel: vec2<i32>, original: vec3<f32>) -> vec3<f32> {
    if camera.graphics.y < 0.5 { return original; }
    let size = vec2<i32>(textureDimensions(scene));
    // Keep the middle 40% of screen width sharp at every height. Fade the
    // effect in over the next 10% on either side, without a hard seam.
    let from_center = abs((f32(pixel.x) + 0.5) / f32(size.x) - 0.5);
    if from_center <= 0.2 { return original; }
    let side_amount = smoothstep(0.2, 0.3, from_center);
    let z = textureLoad(depth, pixel, 0);
    if z >= 1.0 { return original; }
    let distance = eye_depth(z);
    if distance >= 4.0 { return original; }
    let amount = 1.0 - smoothstep(0.6, 4.0, distance);
    // Keep the immediate silhouette sharp: bilinear taps must not blend the
    // neighboring background into a foreground edge before depth rejection.
    var edge_offsets = array<vec2<i32>, 4>(vec2<i32>(1,0), vec2<i32>(-1,0), vec2<i32>(0,1), vec2<i32>(0,-1));
    for (var e = 0u; e < 4u; e += 1u) {
        let q = clamp(pixel + edge_offsets[e], vec2<i32>(0), size - 1);
        if abs(eye_depth(textureLoad(depth, q, 0)) - distance) > 0.2 + distance * 0.2 { return original; }
    }
    // Fractional disk taps avoid the grid-aligned resampling that left large
    // nearest-filtered voxel texels sharp. Radius follows circle of confusion.
    let radius = clamp(f32(size.y) / 108.0, 2.0, 18.0) * amount;
    var offsets = array<vec2<f32>, 12>(
        vec2<f32>(0.28,0.0), vec2<f32>(-0.21,0.25), vec2<f32>(0.04,-0.42),
        vec2<f32>(0.31,0.40), vec2<f32>(-0.56,-0.10), vec2<f32>(0.53,-0.34),
        vec2<f32>(-0.18,0.66), vec2<f32>(-0.35,-0.65), vec2<f32>(0.75,0.27),
        vec2<f32>(-0.78,0.33), vec2<f32>(0.39,-0.81), vec2<f32>(0.28,0.91));
    var color = original;
    var total = 1.0;
    for (var i = 0u; i < 12u; i += 1u) {
        let sample_pixel = clamp(vec2<f32>(pixel) + vec2<f32>(0.5) + offsets[i] * radius,
            vec2<f32>(0.5), vec2<f32>(size) - vec2<f32>(0.5));
        let p = vec2<i32>(sample_pixel);
        let neighbor_z = textureLoad(depth, p, 0);
        let gap = abs(eye_depth(neighbor_z) - distance);
        let tolerance = 0.1 + distance * 0.1;
        let weight = (1.0 - smoothstep(tolerance, tolerance * 2.0, gap))
            * select(1.0, 0.0, neighbor_z >= 1.0);
        color += textureSampleLevel(scene, blur_sampler, sample_pixel / vec2<f32>(size), 0.0).rgb * weight;
        total += weight;
    }
    return mix(original, color / total, side_amount * 0.95 * smoothstep(0.0, 0.2, amount));
}

fn resolve_scene(frag: vec4<f32>) -> vec4<f32> {
    let pixel = vec2<i32>(frag.xy);
    let sample = textureLoad(scene, pixel, 0);
    let original = vec4<f32>(near_blur(pixel, sample.rgb), sample.a);
    if original.a > 0.05 || camera.weather_fx.z > 0.5 { return vec4<f32>(original.rgb, 1.0); }
    let size = vec2<f32>(textureDimensions(scene));
    let pos = world_at(frag.xy / size, textureLoad(depth, pixel, 0));
    let view = normalize(camera.camera_pos.xyz - pos);
    let t = camera.light_params.z;
    // Match shader.wgsl's world-space water waves.
    let slope = water_slope(pos.xz, t);
    let normal = normalize(vec3<f32>(slope.x, 1.0, slope.y));
    let direction = reflect(-view, normal);
    // Match the broad Fresnel response used by the main water material.
    let fresnel_base = select(0.06, 0.06 + 0.94 * pow(1.0 - max(dot(normal, view), 0.0), 3.0), (u32(camera.graphics.x + 0.5) & 1u) != 0u);
    let fresnel_noise = 0.90 + 0.10 * water_noise(pos.xz * 0.13);
    // The natural look keeps water color readable: weaker mirror reflections.
    let fresnel = fresnel_base * fresnel_noise * select(1.0, 0.4, (u32(camera.graphics.x + 0.5) & 2u) != 0u);
    if direction.y <= 0.01 || fresnel < 0.025 { return vec4<f32>(original.rgb, 1.0); }
    let origin = pos + vec3<f32>(0.0, 0.08, 0.0);
    var previous = 0.0;
    var previous_gap = -1.0;
    // Fixed 96-block range, 40 samples, at most five refinement samples per crossing.
    // Quadratic spacing preserves useful shoreline detail near the camera.
    for (var i = 1u; i <= 40u; i += 1u) {
        let fraction = f32(i) / 40.0;
        let travel = 0.15 + fraction * fraction * 96.0;
        let hit = probe(origin + direction * travel);
        if any(hit.xy <= vec2<f32>(0.0)) || any(hit.xy >= vec2<f32>(1.0)) { break; }
        if hit.z >= 0.0 && previous_gap < 0.0 {
            var lo = previous;
            var hi = travel;
            for (var j = 0u; j < 5u; j += 1u) {
                let mid = (lo + hi) * 0.5;
                if probe(origin + direction * mid).z > 0.0 { hi = mid; } else { lo = mid; }
            }
            let refined = probe(origin + direction * hi);
            let coord = clamp(vec2<i32>(refined.xy * size), vec2<i32>(0), vec2<i32>(size) - 1);
            let color = textureLoad(scene, coord, 0);
            let surface = world_at(refined.xy, textureLoad(depth, coord, 0));
            if refined.z >= 0.0 && refined.z < 0.55 && surface.y > pos.y + 0.08 && color.a > 0.95 {
                let edge = min(min(refined.x, refined.y), min(1.0 - refined.x, 1.0 - refined.y));
                let confidence = smoothstep(0.0, 0.08, edge) * (1.0 - smoothstep(64.0, 96.0, hi));
                let fog = 1.0 - smoothstep(70.0, 160.0, distance(pos, camera.camera_pos.xyz));
                return vec4<f32>(mix(original.rgb, color.rgb, fresnel * confidence * fog), 1.0);
            }
        }
        previous = travel;
        previous_gap = hit.z;
    }
    return vec4<f32>(original.rgb, 1.0);
}

// Inverse eye depth (0 for sky): linear in screen space across any plane.
fn inverse_depth(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth));
    let z = textureLoad(depth, clamp(p, vec2<i32>(0), size - 1), 0);
    if z >= 1.0 { return 0.0; }
    return 1.0 / eye_depth(z);
}

// Ink outlines (graphics flag 8): silhouettes where a neighbor is much
// farther away, folds where inverse depth bends (it is linear on planes, so
// coplanar block seams draw nothing). Lines thin out with distance.
fn edge_at(p: vec2<i32>) -> f32 {
    let c = inverse_depth(p);
    if c <= 0.0 { return 0.0; }
    let l = inverse_depth(p + vec2<i32>(-1, 0));
    let r = inverse_depth(p + vec2<i32>(1, 0));
    let u = inverse_depth(p + vec2<i32>(0, -1));
    let d = inverse_depth(p + vec2<i32>(0, 1));
    // Only the nearer side of a depth jump draws.
    let gap = max(max(c - l, c - r), max(c - u, c - d)) / c;
    let silhouette = smoothstep(0.025, 0.07, gap);
    let bend = (abs(l + r - 2.0 * c) + abs(u + d - 2.0 * c)) / c;
    let fold = smoothstep(0.0007, 0.0025, bend) * (1.0 - silhouette);
    let fade = 1.0 - smoothstep(35.0, 100.0, 1.0 / c);
    return clamp(silhouette + fold * 0.8, 0.0, 1.0) * fade;
}

fn ink(frag: vec4<f32>, color: vec3<f32>) -> vec3<f32> {
    if (u32(camera.graphics.x + 0.5) & 8u) == 0u || camera.weather_fx.z > 0.5 { return color; }
    let p = vec2<i32>(frag.xy);
    let alpha = textureLoad(scene, p, 0).a;
    if alpha > 0.9 && alpha < 0.98 { return color; } // foliage: no ink on leaf holes
    // Two pixels wide: this pixel's edge or its right/lower neighbor's.
    let edge = max(edge_at(p), max(edge_at(p + vec2<i32>(1, 0)), edge_at(p + vec2<i32>(0, 1))) * 0.8);
    return mix(color, color * vec3<f32>(0.26, 0.23, 0.22), edge * 0.85);
}

@fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let color = resolve_scene(frag);
    return vec4<f32>(ink(frag, color.rgb), color.a);
}

@fragment fn fs_rays(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let resolved = resolve_scene(frag);
    let color = vec4<f32>(ink(frag, resolved.rgb), resolved.a);
    let z = textureLoad(depth,vec2<i32>(frag.xy),0);
    let uv = frag.xy / vec2<f32>(textureDimensions(scene));
    // Keep nearby walls/held geometry from receiving an artificial glowing veil.
    let visibility = select(smoothstep(1.0,4.0,eye_depth(z)),1.0,z >= 1.0);
    let rays = textureSampleLevel(sunlight_rays,blur_sampler,uv,0.0).rgb * visibility;
    return vec4<f32>(color.rgb + rays * (vec3<f32>(1.0)-color.rgb),1.0);
}
