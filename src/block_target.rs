//! Local, depth-tested wire boxes using the same raycast as block edits.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

pub struct BlockTarget {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    count: u32,
}

impl BlockTarget {
    pub fn new(
        device: &wgpu::Device,
        camera: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("block target shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("block_target.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("block target layout"),
            bind_group_layouts: &[camera],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("block target"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("block target vertices"),
            size: (48 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            buffer,
            count: 0,
        }
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        hit: Option<(crate::raycast::RaycastHit, bool, bool)>,
    ) {
        let mut vertices = Vec::with_capacity(48);
        if let Some((hit, breakable, placeable)) = hit {
            append_box(
                &mut vertices,
                hit.target,
                if breakable {
                    [1.0, 0.65, 0.12]
                } else {
                    [1.0, 0.12, 0.08]
                },
                0.004,
            );
            if placeable {
                // Slightly inset to separate shared edges from the break outline.
                append_box(&mut vertices, hit.place, [0.12, 1.0, 0.45], -0.008);
            }
        }
        self.count = vertices.len() as u32;
        if !vertices.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, camera: &'a wgpu::BindGroup) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}

fn append_box(vertices: &mut Vec<Vertex>, pos: (i32, i32, i32), color: [f32; 3], expand: f32) {
    let origin = [pos.0 as f32, pos.1 as f32, pos.2 as f32];
    for corner in 0..8 {
        for axis in 0..3 {
            if corner & (1 << axis) != 0 {
                continue;
            }
            for endpoint in [corner, corner | (1 << axis)] {
                let position = std::array::from_fn(|i| {
                    origin[i]
                        + if endpoint & (1 << i) == 0 {
                            -expand
                        } else {
                            1.0 + expand
                        }
                });
                vertices.push(Vertex { position, color });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn validates_target_pipeline() {
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        let (device, _) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
                .unwrap();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let camera = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let _target = BlockTarget::new(&device, &camera, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert!(pollster::block_on(device.pop_error_scope()).is_none());
    }
}
