struct LightUniform {view_proj:mat4x4<f32>,motion:vec4<f32>};
@group(0) @binding(0) var<uniform> light:LightUniform;
@group(0) @binding(1) var alpha_sampler:sampler;
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
    // Evaluate derivatives before divergent discard/texture branches.
    let atlas_size=vec2<f32>(textureDimensions(atlas));
    let footprint=max(length(dpdx(in.uv)*atlas_size),length(dpdy(in.uv)*atlas_size));
    let lod=clamp(log2(max(footprint,1.0)),0.0,6.0);
    if in.skip!=0u {discard;}
    if in.cutout!=0u {
        var alpha=1.0;
        if in.layer>0.0 {alpha=textureSampleLevel(models,alpha_sampler,in.uv,i32(in.layer)-1,0.0).a;}
        else {
            // The atlas contains 64-pixel tiles. Keep bilinear taps inside
            // their tile at BOTH mip levels; adjacent tiles must not leak.
            let tiles=atlas_size/64.0;
            let tile=floor(in.uv*tiles);
            let padding=vec2<f32>(0.5*exp2(ceil(lod)))/atlas_size;
            let uv=clamp(in.uv,tile/tiles+padding,(tile+1.0)/tiles-padding);
            alpha=textureSampleLevel(atlas,alpha_sampler,uv,lod).a;
        }
        if alpha<0.5 {discard;}
    }
}
