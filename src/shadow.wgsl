struct LightUniform {view_proj:mat4x4<f32>,motion:vec4<f32>};
@group(0) @binding(0) var<uniform> light:LightUniform;
@group(1) @binding(0) var atlas:texture_2d<f32>;
@group(1) @binding(1) var tex_sampler:sampler;
@group(1) @binding(2) var models:texture_2d_array<f32>;
struct Output {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
    @location(1) @interpolate(flat) layer:f32,
    @location(2) @interpolate(flat) cutout:u32,
    @location(3) @interpolate(flat) skip:u32,
};
@vertex fn vs_main(@location(0) position:vec3<f32>,@location(3) uv:vec2<f32>,@location(6) emission:f32,@location(7) wind:f32,@location(8) layer:f32)->Output {
    var pos=position;
    if wind>0.0 {
        let t=light.motion.x;
        let sway=(sin(t*1.6+pos.x*0.9+pos.z*0.7)*0.09+sin(t*2.3+pos.x*0.3-pos.z*0.5)*0.05)*light.motion.y;
        pos.x+=sway*wind;pos.z+=sway*0.6*wind;
    }
    var out:Output;out.position=light.view_proj*vec4<f32>(pos,1.0);out.uv=uv;out.layer=layer;
    out.cutout=select(0u,1u,wind>0.0 || layer>0.0);
    out.skip=select(0u,1u,layer<0.0 || (layer==36.0 && emission>0.5));
    return out;
}
@fragment fn fs_main(in:Output) {
    if in.skip!=0u {discard;}
    if in.cutout!=0u {
        var alpha=1.0;
        if in.layer>0.0 {alpha=textureSampleLevel(models,tex_sampler,in.uv,i32(in.layer)-1,0.0).a;}
        else {alpha=textureSampleLevel(atlas,tex_sampler,in.uv,0.0).a;}
        if alpha<0.5 {discard;}
    }
}