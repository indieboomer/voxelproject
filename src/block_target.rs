//! Local, depth-tested wire boxes using the same raycast as block edits.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

/// Break outline (cube or shaped, at most 16 edges) plus the build box.
const MAX_VERTICES: usize = 64;

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
            size: (MAX_VERTICES * std::mem::size_of::<Vertex>()) as u64,
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
        shape: Option<crate::voxel::shaping::CellShape>,
    ) {
        let mut vertices = Vec::with_capacity(MAX_VERTICES);
        if let Some((hit, breakable, placeable)) = hit {
            let color = if breakable {
                [1.0, 0.65, 0.12]
            } else {
                [1.0, 0.12, 0.08]
            };
            match shape {
                Some(shape) => append_shape(&mut vertices, hit.target, &shape, color, 0.006),
                None => append_box(&mut vertices, hit.target, color, 0.004),
            }
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

/// Outlines the cell's actual shaped solid -- floor, corner posts, top rim and
/// fan creases -- so the highlight shows exactly the portion a dig removes.
fn append_shape(
    vertices: &mut Vec<Vertex>,
    pos: (i32, i32, i32),
    shape: &crate::voxel::shaping::CellShape,
    color: [f32; 3],
    expand: f32,
) {
    use crate::voxel::shaping::CORNERS;
    let at = |u: f32, y: f32, v: f32| {
        // Push points slightly away from the cell center to avoid z-fighting.
        let out = |c: f32| (c - 0.5).signum() * expand * ((c - 0.5).abs() > 1e-3) as i32 as f32;
        [pos.0 as f32 + u + out(u), pos.1 as f32 + y + expand, pos.2 as f32 + v + out(v)]
    };
    let mut line = |a: [f32; 3], b: [f32; 3]| {
        vertices.push(Vertex { position: a, color });
        vertices.push(Vertex { position: b, color });
    };
    for i in 0..4 {
        let (x0, z0) = CORNERS[i];
        let (x1, z1) = CORNERS[(i + 1) % 4];
        let (u0, v0, u1, v1) = (x0 as f32, z0 as f32, x1 as f32, z1 as f32);
        let (h0, h1) = (shape.corners[i], shape.corners[(i + 1) % 4]);
        line(at(u0, -2.0 * expand, v0), at(u1, -2.0 * expand, v1));
        line(at(u0, -2.0 * expand, v0), at(u0, h0, v0));
        line(at(u0, h0, v0), at(u1, h1, v1));
        line(at(0.5, shape.center, 0.5), at(u0, h0, v0));
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
