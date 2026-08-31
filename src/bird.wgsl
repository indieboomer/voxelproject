// Renders a flock of birds as tiny dark line-silhouettes (a shallow "V" per
// bird, two line segments meeting at a center point) high in the sky --
// purely decorative, see App::update_birds/spawn_bird_flock.
//
// Only view_proj is needed, so only it is declared here -- WGSL requires
// every uniform field to be declared in order up to the last one actually
// read (a skipped field silently shifts every later one to the wrong byte
// offset, see rain.wgsl's comment for a real example of that bug), and
// since nothing after view_proj is used, nothing after it needs declaring.
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // A small, near-black silhouette -- at the distance/height birds fly
    // here, they read as a dark mark against the sky regardless of time of
    // day, so a flat unlit tone (rather than anything fog/light-blended)
    // is both correct-looking and the simplest possible fragment shader.
    return vec4<f32>(0.05, 0.05, 0.06, 0.85);
}
