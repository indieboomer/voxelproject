// Same layout as the main shader's CameraUniform -- this pipeline reuses
// that buffer/bind group verbatim, it just doesn't need most of the fields.
struct CameraUniform {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    fog_color: vec4<f32>,
    sun_dir: vec4<f32>,
    light_params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) alpha: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) alpha: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.alpha = in.alpha;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Faint blue-grey streak, tinted toward the current sky color so it
    // still reads correctly at dawn/dusk/night, not just under a blue sky.
    let tint = mix(vec3<f32>(0.75, 0.8, 0.9), camera.fog_color.rgb, 0.3);
    return vec4<f32>(tint, in.alpha);
}
