//! Quarter-resolution screen-space sunlight; reuses sky's cloud mask and depth.
pub struct GodRays {
    pub output_layout: wgpu::BindGroupLayout,
    pub output: wgpu::BindGroup,
    view: wgpu::TextureView,
    inputs_layout: wgpu::BindGroupLayout,
    inputs: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl GodRays {
    pub fn new(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        camera: &wgpu::BindGroupLayout,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> Self {
        let inputs_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("god ray inputs"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: if binding == 0 {
                        wgpu::TextureSampleType::Float { filterable: false }
                    } else {
                        wgpu::TextureSampleType::Depth
                    },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let output_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("god ray composite"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let (view, inputs, output) = Self::targets(
            device,
            width,
            height,
            scene,
            depth,
            &inputs_layout,
            &output_layout,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("god rays"),
            source: wgpu::ShaderSource::Wgsl(include_str!("god_rays.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[camera, &inputs_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quarter resolution god rays"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        });
        Self {
            output_layout,
            output,
            view,
            inputs_layout,
            inputs,
            pipeline,
        }
    }

    fn targets(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        inputs_layout: &wgpu::BindGroupLayout,
        output_layout: &wgpu::BindGroupLayout,
    ) -> (wgpu::TextureView, wgpu::BindGroup, wgpu::BindGroup) {
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("quarter resolution sunlight"),
                size: wgpu::Extent3d {
                    width: width.div_ceil(4).max(1),
                    height: height.div_ceil(4).max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: inputs_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        });
        let output = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: output_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        (view, inputs, output)
    }

    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        (self.view, self.inputs, self.output) = Self::targets(
            device,
            width,
            height,
            scene,
            depth,
            &self.inputs_layout,
            &self.output_layout,
        );
    }

    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, camera: &wgpu::BindGroup) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("god rays"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, &self.inputs, &[]);
        pass.draw(0..3, 0..1);
    }
}
