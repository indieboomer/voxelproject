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
@fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(frag.xy);
    let original = textureLoad(scene, pixel, 0);
    if original.a > 0.05 || camera.weather_fx.z > 0.5 { return vec4<f32>(original.rgb, 1.0); }
    let size = vec2<f32>(textureDimensions(scene));
    let pos = world_at(frag.xy / size, textureLoad(depth, pixel, 0));
    let view = normalize(camera.camera_pos.xyz - pos);
    let t = camera.light_params.z;
    // Match shader.wgsl's world-space water waves.
    let normal = normalize(vec3<f32>(cos(pos.x * 0.6 + t * 0.42) * 0.09, 1.0, -sin(pos.z * 0.5 + t * 0.55) * 0.09));
    let direction = reflect(-view, normal);
    // Match the broad Fresnel response used by the main water material.
    let fresnel_base = select(0.06, 0.06 + 0.94 * pow(1.0 - max(dot(normal, view), 0.0), 3.0), camera.graphics.x > 0.5);
    let fresnel_noise = 0.93 + 0.07 * sin(pos.x * 1.71 + sin(pos.z * 1.13));
    let fresnel = fresnel_base * fresnel_noise;
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
