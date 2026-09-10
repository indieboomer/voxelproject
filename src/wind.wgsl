struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) alpha: f32,
    @location(1) uv: vec2<f32>,
};
@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) alpha: f32, @location(2) uv: vec2<f32>) -> Output {
    var out: Output;
    out.position = camera.view_proj * vec4<f32>(position, 1.0);
    out.alpha = alpha;
    out.uv = uv;
    return out;
}
@fragment fn fs_main(in: Output) -> @location(0) vec4<f32> {
    let edge = (1.0-smoothstep(0.35,1.0,abs(in.uv.y))) * (1.0-smoothstep(0.65,1.0,abs(in.uv.x)));
    return vec4<f32>(0.94,0.97,1.0,in.alpha * edge);
}
