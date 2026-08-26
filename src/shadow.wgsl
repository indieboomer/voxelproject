// Depth-only pass: renders the scene from the sun's point of view into a
// shadow map. Reuses the same vertex buffers as the main pass (color/
// normal/uv/ao are simply unread) so no separate shadow-specific mesh data
// is needed.

struct LightUniform {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> light: LightUniform;

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return light.view_proj * vec4<f32>(position, 1.0);
}
