//! GPU regression for the shared resolve, including actual depth reconstruction.
use super::*;

#[test]
#[ignore = "requires a GPU adapter; checks near blur, sharp distance/sky and depth edges"]
fn near_depth_of_field_resolve() {
    resolve_gpu_checks(false);
}

#[test]
#[ignore = "requires GPU; checks sun shafts, occlusion, night, underwater, sun position and toggle"]
fn god_rays_occlusion_and_toggle() {
    resolve_gpu_checks(true);
}

fn resolve_gpu_checks(test_rays: bool) {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default(), None)).unwrap();
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let size = wgpu::Extent3d {
        width: 64,
        height: 16,
        depth_or_array_layers: 1,
    };
    let make_texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("DOF test"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let depth = make_texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let depth_view = depth.create_view(&Default::default());
    let output = make_texture(
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let view = output.create_view(&Default::default());
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: std::mem::size_of::<CameraUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let mut resolve = crate::water_reflections::Reflections::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        64,
        16,
        &depth_view,
        &layout,
    );
    // Exercise rebinding to a recreated depth texture as a window resize does.
    resolve.resize(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        64,
        16,
        &depth_view,
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("DOF fixture"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
            return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
        }
        struct Out { @location(0) color: vec4<f32>, @builtin(frag_depth) depth: f32 };
        @fragment fn fs(@builtin(position) p: vec4<f32>) -> Out {
            var o: Out;
            var color = f32(i32(p.x) % 2);
            if p.x >= 16.0 && p.x < 24.0 { color = 1.0; }
            if p.x >= 24.0 && p.x < 32.0 { color = 0.0; }
            o.color = vec4<f32>(vec3<f32>(color), 1.0);
            var distance = 0.3;
            if p.x >= 24.0 { distance = 4.0; }
            if p.x >= 32.0 && p.x < 40.0 { distance = 3.5; }
            if p.x >= 48.0 && p.x < 56.0 { distance = 0.3; }
            o.depth = (400.0 - 0.05 * 400.0 / distance) / (400.0 - 0.05);
            if p.x >= 56.0 { o.depth = 1.0; }
            return o;
        }"#
            .into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vs",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fs",
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        multisample: Default::default(),
        multiview: None,
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: Default::default(),
            bias: Default::default(),
        }),
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 64 * 16 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let render = |enabled, ray_case: Option<u8>| {
        let mut uniform = CameraUniform::zeroed();
        uniform.graphics = [
            1.0,
            if enabled && ray_case.is_none() {
                1.0
            } else {
                0.0
            },
            0.05,
            400.0,
        ];
        if let Some(case) = ray_case {
            uniform.view_proj =
                glam::Mat4::perspective_rh(70f32.to_radians(), 4.0, 0.05, 400.0).to_cols_array_2d();
            uniform.sun_dir = [0., 0.15, if case == 5 { 1.0 } else { -1.0 }, 1.0];
            if case == 6 {
                uniform.sun_dir[0] = 20.0;
            }
            uniform.light_params = [0.4, if case == 3 { 0.0 } else { 0.85 }, 0.0, 1.0];
            uniform.weather_fx[2] = if case == 4 { 1.0 } else { 0.0 };
        }
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resolve.scene,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if let Some(case) = ray_case {
                            wgpu::Color {
                                r: 0.2,
                                g: 0.2,
                                b: 0.2,
                                a: if case == 1 { 0.1 } else { 0.9 },
                            }
                        } else {
                            wgpu::Color::BLACK
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if ray_case == Some(2) { 0.5 } else { 1.0 }),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            if ray_case.is_none() {
                pass.set_pipeline(&pipeline);
                pass.draw(0..3, 0..1);
            }
        }
        resolve.draw(&mut encoder, &view, &camera, enabled && ray_case.is_some());
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(16),
                },
            },
            size,
        );
        queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
        device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let bytes = readback.slice(..).get_mapped_range().to_vec();
        readback.unmap();
        bytes
    };
    if test_rays {
        let off = render(false, Some(0));
        let on = render(true, Some(0));
        assert!(on
            .chunks_exact(4)
            .zip(off.chunks_exact(4))
            .any(|(a, b)| a[0] > b[0] + 8));
        for case in 1..=6 {
            assert_eq!(
                render(false, Some(case)),
                render(true, Some(case)),
                "occlusion/gating case {case}"
            );
        }
        assert_eq!(
            off,
            render(false, Some(0)),
            "off bypasses stale previous rays"
        );
        assert!(pollster::block_on(device.pop_error_scope()).is_none());
        return;
    }
    let off = render(false, None);
    let on = render(true, None);
    let red = |bytes: &[u8], x: usize| bytes[(8 * 64 + x) * 4];
    assert_eq!(red(&off, 8), 0);
    assert!(
        red(&on, 8) >= 100 && red(&on, 8) < 140,
        "near blur visibly softens fine detail"
    );
    for y in 0..16 {
        for x in 19..45 {
            let i = (y * 64 + x) * 4;
            assert_eq!(&on[i..i + 4], &off[i..i + 4], "central pixel {x},{y} stays sharp");
        }
    }
    assert!(red(&on, 51) < red(&off, 51), "right side still blurs near detail");
    assert_eq!(red(&on, 23), 255, "near edge rejects distant black");
    for x in 24..32 {
        assert_eq!(red(&on, x), red(&off, x));
    }
    for x in (40..48).chain(56..64) {
        assert_eq!(red(&on, x), red(&off, x), "far/sky pixel {x}");
    }
    assert_eq!(
        off,
        render(false, None),
        "toggle off restores exact original"
    );
    assert!(pollster::block_on(device.pop_error_scope()).is_none());
}
