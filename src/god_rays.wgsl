struct CameraUniform {
    view_proj: mat4x4<f32>, light_view_proj: mat4x4<f32>, inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>, fog_color: vec4<f32>, zenith_color: vec4<f32>, sun_dir: vec4<f32>,
    light_params: vec4<f32>, weather_fx: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var scene: texture_2d<f32>;
@group(1) @binding(1) var depth: texture_depth_2d;
struct Varying { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Varying {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Varying; out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0-p.y); return out;
}
@fragment fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    if camera.light_params.y <= 0.0 || camera.sun_dir.w <= 0.0 || camera.weather_fx.z > 0.5 { return vec4<f32>(0.0); }
    // w=0 projects a direction: no camera translation, no arbitrary sun distance.
    let clip = camera.view_proj * vec4<f32>(camera.sun_dir.xyz, 0.0);
    if clip.w <= 0.0 { return vec4<f32>(0.0); }
    let sun = vec2<f32>(0.5 + clip.x/clip.w*0.5, 0.5-clip.y/clip.w*0.5);
    let edge = min(min(sun.x, sun.y), min(1.0-sun.x,1.0-sun.y));
    let fade = smoothstep(-0.1,0.1,edge);
    if fade <= 0.0 { return vec4<f32>(0.0); }
    let size = vec2<i32>(textureDimensions(scene));
    var light = 0.0;
    // Fixed midpoint integration avoids animated noise and temporal history.
    for (var i=0u; i<32u; i+=1u) {
        let t = (f32(i)+0.5)/32.0;
        let uv = mix(in.uv, sun, t*0.85);
        if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) { continue; }
        let p = clamp(vec2<i32>(uv*vec2<f32>(size)), vec2<i32>(0),size-1);
        if textureLoad(depth,p,0) >= 0.999999 {
            let alpha = textureLoad(scene,p,0).a;
            // Dead zone absorbs RGBA8 rounding of fully opaque cloud masks.
            let mask = clamp((alpha-0.105)/0.79,0.0,1.0) * select(0.0,1.0,alpha < 0.95);
            light += mask * (1.0-t*0.6);
        }
    }
    // Longer, more visible shafts instead of the previous faint sun glow.
    // Boost sparse canopy openings as well as broad cloud gaps, with an
    // explicit ceiling to keep clear skies from washing out completely.
    let radial = 1.0-smoothstep(0.35,1.15,length(in.uv-sun));
    let strength = min(0.6, pow(light/32.0,0.85) * fade * radial * 1.8 * camera.light_params.y
        * (1.0-camera.weather_fx.y*0.75) * (1.0-camera.weather_fx.x));
    let golden_hour = 1.0-smoothstep(0.0,0.4,camera.sun_dir.w);
    let tint = mix(vec3<f32>(1.0,0.87,0.63),vec3<f32>(1.0,0.62,0.24),golden_hour);
    return vec4<f32>(tint*strength,1.0);
}
