//! Opt-in GPU regression scene. Baseline shaders can be placed in target/render-before.wgsl
//! and target/sky-before.wgsl before editing; no baseline is needed for normal validation.
use super::*;

#[test]
#[ignore = "requires GPU; writes target/render-*.png and reports fixed-scene render timings"]
fn render_weather_previews() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    println!("Adapter: {:?}", adapter.get_info());
    let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            required_features: if timestamps {
                wgpu::Features::TIMESTAMP_QUERY
            } else {
                wgpu::Features::empty()
            },
            ..Default::default()
        },
        None,
    ))
    .unwrap();
    let queries = timestamps.then(|| {
        device.create_query_set(&wgpu::QuerySetDescriptor {
            label: None,
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        })
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let timing = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
    let camera = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: std::mem::size_of::<CameraUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &camera_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera.as_entire_binding(),
        }],
    });
    let models = Models::load();
    let (atlas_bgl, atlas_bg) = create_atlas_bind_group(&device, &queue, &models);
    let shadow = create_shadow_resources(&device);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&camera_bgl, &atlas_bgl, &shadow.sample_bgl],
        push_constant_ranges: &[],
    });
    let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&camera_bgl],
        push_constant_ranges: &[],
    });
    let make_pipeline = |source: &str, sky: bool| {
        let vertex_layout = [Vertex::layout()];
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(if sky { &sky_layout } else { &layout }),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: if sky { &[] } else { &vertex_layout },
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                front_face: wgpu::FrontFace::Cw,
                cull_mode: if sky { None } else { Some(wgpu::Face::Back) },
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: !sky,
                depth_compare: if sky {
                    wgpu::CompareFunction::Always
                } else {
                    wgpu::CompareFunction::Less
                },
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
        })
    };
    let main = make_pipeline(include_str!("shader.wgsl"), false);
    let sky = make_pipeline(include_str!("sky.wgsl"), true);
    let baseline = std::fs::read_to_string("target/render-before.wgsl")
        .ok()
        .zip(std::fs::read_to_string("target/sky-before.wgsl").ok())
        .map(|(m, s)| (make_pipeline(&m, false), make_pipeline(&s, true)));
    let mut world = World::new(1);
    for cx in -2..3 {
        for cz in -2..3 {
            let mut chunk = crate::voxel::chunk::Chunk::new(cx, cz);
            for x in 0..16 {
                for z in 0..16 {
                    let wx = cx * 16 + x;
                    let wz = cz * 16 + z;
                    let pool = (0..9).contains(&wx) && (0..9).contains(&wz);
                    let floor = if wx < -5 {
                        BlockType::Grass
                    } else if wz > 10 {
                        BlockType::OakWood
                    } else {
                        BlockType::Stone
                    };
                    chunk.set_local(x, 5, z, floor);
                    chunk.set_local(x, 6, z, if pool { BlockType::Water } else { floor });
                    if (12..21).contains(&wx) && (0..10).contains(&wz) {
                        chunk.set_local(x, 12, z, BlockType::OakWood);
                    }
                    if (wx == 12 || wx == 20) && (wz == 0 || wz == 9) {
                        for y in 7..12 {
                            chunk.set_local(x, y, z, BlockType::OakWood);
                        }
                    }
                    if (-12..-8).contains(&wx) && (-6..-2).contains(&wz) {
                        for y in 7..11 {
                            chunk.set_local(x, y, z, BlockType::Stone);
                        }
                    }
                }
            }
            world.chunks.insert((cx, cz), chunk);
        }
    }
    let mut meshes = Vec::new();
    let mut old_meshes = Vec::new();
    for chunk in world.chunks.values() {
        let mut mesh = crate::voxel::mesher::build_chunk_mesh(&world, chunk);
        meshes.push(upload_mesh(&device, &mesh).unwrap());
        for v in &mut mesh.vertices {
            if v.reflectivity >= 2.0 {
                v.reflectivity -= 2.0;
            }
            let shade = if v.normal[1] > 0.0 {
                1.0
            } else if v.normal[1] < 0.0 {
                0.45
            } else if v.normal[0] != 0.0 {
                0.8
            } else {
                0.7
            };
            v.color = [shade; 3];
        }
        old_meshes.push(upload_mesh(&device, &mesh).unwrap());
    }
    let (width, height) = (1280, 720);
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = color.create_view(&Default::default());
    let depth = device
        .create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (width * height * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let eye = Vec3::new(-18.0, 15.0, 27.0);
    let vp = glam::Mat4::perspective_rh(
        65.0f32.to_radians(),
        width as f32 / height as f32,
        0.1,
        200.0,
    ) * glam::Mat4::look_at_rh(eye, Vec3::new(3.0, 7.0, 0.0), Vec3::Y);
    let lighting = crate::daynight::sky_lighting(0.14);
    let light_vp = light_view_proj(lighting.sun_dir, eye);
    queue.write_buffer(
        &shadow.light_buffer,
        0,
        bytemuck::bytes_of(&LightUniform {
            view_proj: light_vp.to_cols_array_2d(),
        }),
    );
    for (name, wet, clouds, old) in [
        ("before", 0.0, 0.0, true),
        ("dry", 0.0, 0.0, false),
        ("rain", 1.0, 0.65, false),
        ("drying", 0.5, 0.0, false),
        ("before-storm", 0.0, 0.85, true),
        ("storm", 1.0, 0.85, false),
    ] {
        let (main, sky) = if old {
            if let Some((m, s)) = &baseline {
                (m, s)
            } else {
                continue;
            }
        } else {
            (&main, &sky)
        };
        let meshes = if old { &old_meshes } else { &meshes };
        queue.write_buffer(
            &camera,
            0,
            bytemuck::bytes_of(&CameraUniform {
                view_proj: vp.to_cols_array_2d(),
                inv_view_proj: vp.inverse().to_cols_array_2d(),
                light_view_proj: light_vp.to_cols_array_2d(),
                camera_pos: eye.extend(1.0).to_array(),
                fog_color: [
                    lighting.sky_color[0],
                    lighting.sky_color[1],
                    lighting.sky_color[2],
                    1.0,
                ],
                zenith_color: [
                    lighting.zenith_color[0],
                    lighting.zenith_color[1],
                    lighting.zenith_color[2],
                    1.0,
                ],
                sun_dir: lighting.sun_dir.extend(lighting.sun_height).to_array(),
                light_params: [
                    lighting.ambient * if old { 1.0 } else { 1.0 - clouds * 0.12 },
                    lighting.sun_intensity * if old { 1.0 } else { 1.0 - clouds * 0.72 },
                    10.0,
                    1.0,
                ],
                weather_fx: [0.0, clouds, 0.0, wet],
            }),
        );
        let mut times = Vec::new();
        for frame in 0..50 {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &shadow.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: queries.as_ref().map(|q| wgpu::RenderPassTimestampWrites {
                        query_set: q,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: None,
                    }),
                    occlusion_query_set: None,
                });
                pass.set_pipeline(&shadow.pipeline);
                pass.set_bind_group(0, &shadow.light_bind_group, &[]);
                for mesh in meshes {
                    pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: queries.as_ref().map(|q| wgpu::RenderPassTimestampWrites {
                        query_set: q,
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(1),
                    }),
                    occlusion_query_set: None,
                });
                pass.set_pipeline(sky);
                pass.set_bind_group(0, &camera_bg, &[]);
                pass.draw(0..3, 0..1);
                pass.set_pipeline(main);
                pass.set_bind_group(1, &atlas_bg, &[]);
                pass.set_bind_group(2, &shadow.sample_bind_group, &[]);
                for mesh in meshes {
                    pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            if let Some(q) = &queries {
                encoder.resolve_query_set(q, 0..2, &resolve, 0);
                encoder.copy_buffer_to_buffer(&resolve, 0, &timing, 0, 16);
            }
            if frame == 49 {
                encoder.copy_texture_to_buffer(
                    color.as_image_copy(),
                    wgpu::ImageCopyBuffer {
                        buffer: &readback,
                        layout: wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: Some(height),
                        },
                    },
                    size,
                );
            }
            queue.submit(Some(encoder.finish()));
            if timestamps {
                let data = read_buffer(&device, &timing);
                let start = u64::from_le_bytes(data[0..8].try_into().unwrap());
                let end = u64::from_le_bytes(data[8..16].try_into().unwrap());
                if frame >= 10 {
                    times.push((end - start) as f64 * queue.get_timestamp_period() as f64 / 1e6);
                }
            } else {
                device.poll(wgpu::Maintain::Wait);
            }
        }
        if !times.is_empty() {
            times.sort_by(f64::total_cmp);
            println!(
                "{name}: median GPU {:.3} ms (shadow + sky + terrain, 1280x720)",
                times[times.len() / 2]
            );
        }
        image::save_buffer(
            format!("target/render-{name}.png"),
            &read_buffer(&device, &readback),
            width,
            height,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}

fn read_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();
    let bytes = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    bytes
}
