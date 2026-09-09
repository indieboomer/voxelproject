//! Opt-in offscreen visual check; writes PNGs to target without opening a game window.
use crate::{settings::UiTheme, ui_theme};

#[test]
#[ignore = "requires a GPU adapter; generates target/ui-*.png"]
fn render_ui_previews() {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(), None))
            .unwrap();
    let width = 1280;
    for (theme, name) in [(UiTheme::Generic, "generic"), (UiTheme::Fantasy, "fantasy")] {
        for panel in ["settings", "crafting", "resources"] {
            let height = if panel == "resources" { 1024 } else { 720 };
            let ctx = egui::Context::default();
            ui_theme::apply(&ctx, theme);
            let mut renderer =
                egui_wgpu::Renderer::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb, None, 1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("UI preview"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let registry =
                crate::crafting::Registry::parse(include_str!("../data/crafting.json")).unwrap();
            let account = crate::crafting::Account {
                elements: [12, 8, 4, 8, 3],
                mana: 24,
                ..Default::default()
            };
            let world = crate::voxel::World::new(1);
            let creatures = crate::creature::Creatures::new();
            let mut craft = crate::crafting_ui::CraftingUi::default();
            craft.open = true;
            craft.slots[0] = Some(crate::crafting::Slot {
                element: crate::crafting::Element::Earth,
                amount: 2,
            });
            craft.slots[1] = Some(crate::crafting::Slot {
                element: crate::crafting::Element::Fire,
                amount: 1,
            });
            let mut settings = crate::settings::SettingsPanel::new(&ctx);
            settings.values.appearance.ui_theme = theme;
            ui_theme::apply(&ctx, theme);
            // Multiple frames settle egui window measurements and font atlas updates.
            for frame in 0..4 {
                let output = ctx.run(
                    egui::RawInput {
                        time: Some(frame as f64 * 0.2),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width as f32, height as f32),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if theme == UiTheme::Fantasy {
                                ui_theme::menu_backdrop(ui);
                            }
                        });
                        if panel == "crafting" {
                            craft.draw(
                                ctx,
                                &registry,
                                &account,
                                &world,
                                &creatures,
                                glam::Vec3::ZERO,
                                &[],
                            );
                        } else if panel == "resources" {
                            egui::Window::new("Resources - pixel atlas preview")
                                .fixed_pos([40., 30.])
                                .fixed_size([1180., 650.])
                                .show(ctx, |ui| {
                                    ui.columns(4, |columns| {
                                        for (i, block) in
                                            crate::voxel::COLLECTIBLE_BLOCKS.iter().enumerate()
                                        {
                                            columns[i / crate::voxel::COLLECTIBLE_BLOCKS
                                                .len()
                                                .div_ceil(4)]
                                            .horizontal(|ui| {
                                                crate::resource_ui::icon(ui, *block);
                                                ui.label(block.name()).on_hover_text(
                                                    crate::resource_ui::description(*block),
                                                );
                                            });
                                        }
                                    });
                                });
                        } else {
                            settings.draw(ctx);
                        }
                    },
                );
                for (id, delta) in &output.textures_delta.set {
                    renderer.update_texture(&device, &queue, *id, delta);
                }
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                let screen = egui_wgpu::ScreenDescriptor {
                    size_in_pixels: [width, height],
                    pixels_per_point: output.pixels_per_point,
                };
                let mut encoder = device.create_command_encoder(&Default::default());
                let buffers =
                    renderer.update_buffers(&device, &queue, &mut encoder, &primitives, &screen);
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
                        depth_stencil_attachment: None,
                        occlusion_query_set: None,
                        timestamp_writes: None,
                    });
                    renderer.render(&mut pass, &primitives, &screen);
                }
                queue.submit(buffers.into_iter().chain(std::iter::once(encoder.finish())));
                for id in &output.textures_delta.free {
                    renderer.free_texture(id);
                }
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (width * height * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::ImageCopyBuffer {
                    buffer: &buffer,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (send, recv) = std::sync::mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    send.send(result).unwrap();
                });
            device.poll(wgpu::Maintain::Wait);
            recv.recv().unwrap().unwrap();
            let bytes = buffer.slice(..).get_mapped_range();
            image::save_buffer(
                format!("target/ui-{name}-{panel}.png"),
                &bytes,
                width,
                height,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
    }
}
