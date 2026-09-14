//! Four bounded voxel visibility volumes; no per-light shadow pass or screen-space rays.
use crate::voxel::{chunk::world_to_chunk, World};
use glam::Vec3;
use std::hash::{Hash, Hasher};
const N: u32 = 18;
pub struct Visibility {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    texture: wgpu::Texture,
    uniform: wgpu::Buffer,
    keys: [Option<([i32; 3], u32, u64)>; 4],
    origins: [[f32; 4]; 4],
}
pub fn clear_ray(world: &World, from: Vec3, to: Vec3) -> bool {
    trace(from, to, |cell| {
        !world.chunks.contains_key(&world_to_chunk(cell.x, cell.z))
            || world.get_block(cell.x, cell.y, cell.z).is_opaque()
    })
}
fn trace(from: Vec3, to: Vec3, blocked: impl Fn(glam::IVec3) -> bool) -> bool {
    let delta = to - from;
    let end = to.floor().as_ivec3();
    let mut cell = from.floor().as_ivec3();
    let step = delta.signum().as_ivec3();
    let inv = Vec3::new(
        if delta.x == 0. {
            f32::INFINITY
        } else {
            1. / delta.x.abs()
        },
        if delta.y == 0. {
            f32::INFINITY
        } else {
            1. / delta.y.abs()
        },
        if delta.z == 0. {
            f32::INFINITY
        } else {
            1. / delta.z.abs()
        },
    );
    let mut next = Vec3::splat(f32::INFINITY);
    for axis in 0..3 {
        if delta[axis] != 0. {
            next[axis] =
                ((cell[axis] + i32::from(step[axis] > 0)) as f32 - from[axis]) / delta[axis];
        }
    }
    for _ in 0..64 {
        if cell == end {
            return true;
        }
        let axis = if next.x <= next.y && next.x <= next.z {
            0
        } else if next.y <= next.z {
            1
        } else {
            2
        };
        cell[axis] += step[axis];
        next[axis] += inv[axis];
        if blocked(cell) {
            return false;
        }
    }
    false
}
// A voxel can be partly lit even when its centre is hidden by the edge of a
// freshly mined opening. Test inset face centres only when the central ray fails.
// Never light a solid cell: this also avoids tracing thousands of rays into rock.
fn visible_cell(from: Vec3, center: Vec3, blocked: impl Fn(glam::IVec3) -> bool) -> bool {
    if blocked(center.floor().as_ivec3()) { return false; }
    if trace(from, center, &blocked) { return true; }
    [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
        .into_iter().any(|axis| trace(from, center + axis * 0.45, &blocked))
}
fn cache_key(world: &World, pos: Vec3, radius: f32) -> ([i32; 3], u32, u64) {
    let quantized = (pos * 4.).floor().as_ivec3().to_array();
    let (cx, cz) = world_to_chunk(pos.x.floor() as i32, pos.z.floor() as i32);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    world.seed.hash(&mut hash);
    (cx, cz).hash(&mut hash);
    for z in cz - 1..=cz + 1 {
        for x in cx - 1..=cx + 1 {
            world.chunks.get(&(x, z)).map(|c| c.revision).hash(&mut hash);
        }
    }
    for d in world.automation.devices.values() {
        d.cell.hash(&mut hash);
        (d.kind as u8).hash(&mut hash);
    }
    (quantized, radius.to_bits(), hash.finish())
}
impl Visibility {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("local light visibility"),
            size: wgpu::Extent3d {
                width: N,
                height: N,
                depth_or_array_layers: N * 4,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("visibility origins"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("visibility layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("visibility bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&[[0f32; 4]; 4]));
        Self {
            layout,
            bind_group,
            texture,
            uniform,
            keys: [None; 4],
            origins: [[0.; 4]; 4],
        }
    }
    pub fn update(&mut self, world: &World, lights: &[[f32; 4]; 4], queue: &wgpu::Queue) {
        for (i, light) in lights.iter().enumerate() {
            if light[3] == 0. {
                self.origins[i][3] = 0.;
                self.keys[i] = None;
                continue;
            }
            let pos = Vec3::new(light[0], light[1], light[2]);
            let key = cache_key(world, pos, light[3]);
            if self.keys[i] == Some(key) {
                continue;
            }
            let origin = pos.floor() - Vec3::splat(9.);
            let radius = if light[3] < -100. {
                -light[3] - 100.
            } else {
                light[3].abs()
            };
            let mut pixels = vec![0u8; (N * N * N) as usize];
            // Resolve each column once instead of querying chunk/device maps at
            // every step of thousands of rays.
            let mut solid = vec![true; (N * N * N) as usize];
            let base = origin.as_ivec3();
            for z in 0..N {
                for x in 0..N {
                    let wx = base.x + x as i32;
                    let wz = base.z + z as i32;
                    let Some(chunk) = world.chunks.get(&world_to_chunk(wx, wz)) else {
                        continue;
                    };
                    for y in 0..N {
                        let block = if world.automation.devices.is_empty() {
                            chunk.get_local(wx.rem_euclid(16), base.y + y as i32, wz.rem_euclid(16))
                        } else {
                            world.get_block(wx, base.y + y as i32, wz)
                        };
                        solid[((z * N + y) * N + x) as usize] = block.is_opaque();
                    }
                }
            }
            for z in 0..N {
                for y in 0..N {
                    for x in 0..N {
                        let target =
                            origin + Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                        if target.distance_squared(pos) < (radius + 1.).powi(2)
                            && visible_cell(pos, target, |cell| {
                                let p = cell - base;
                                if p.min_element() < 0 || p.max_element() >= N as i32 {
                                    return true;
                                }
                                solid[((p.z * N as i32 + p.y) * N as i32 + p.x) as usize]
                            })
                        {
                            pixels[((z * N + y) * N + x) as usize] = 255;
                        }
                    }
                }
            }
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: N * i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(N),
                    rows_per_image: Some(N),
                },
                wgpu::Extent3d {
                    width: N,
                    height: N,
                    depth_or_array_layers: N,
                },
            );
            self.origins[i] = origin.extend(1.).to_array();
            self.keys[i] = Some(key);
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&self.origins));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aligned_rays_on_grid_planes_do_not_step_along_zero_axes() {
        let a = Vec3::new(2.5, 3., 4.5);
        let b = Vec3::new(8.5, 3., 4.5);
        assert!(trace(a, b, |_| false));
        assert!(trace(b, a, |_| false));
        assert!(!trace(a, b, |p| p.x == 5));
    }
    #[test]
    fn mining_recess_updates_stationary_torch_across_chunk_boundary() {
        use crate::voxel::{chunk::Chunk, BlockType};
        let mut w = World::new(42);
        w.chunks.insert((0, 0), Chunk::new(0, 0));
        w.chunks.insert((1, 0), Chunk::new(1, 0));
        let light = Vec3::new(13.5, 2.5, 2.5);
        let target = Vec3::new(16.5, 2.5, 3.5);
        w.set_block(15, 2, 3, BlockType::Stone);
        w.set_block(16, 2, 3, BlockType::Stone);
        let key = cache_key(&w, light, -106.);
        let blocked = |p: glam::IVec3| w.get_block(p.x, p.y, p.z).is_opaque();
        assert!(!visible_cell(light, target, blocked));
        w.set_block(16, 2, 3, BlockType::Air);
        assert_ne!(cache_key(&w, light, -106.), key);
        let blocked = |p: glam::IVec3| w.get_block(p.x, p.y, p.z).is_opaque();
        assert!(!trace(light, target, blocked), "centre is hidden by the opening's edge");
        assert!(visible_cell(light, target, blocked), "exposed recess must light without movement");
        for y in 0..6 { for z in 0..6 { w.set_block(15, y, z, BlockType::Stone); } }
        assert!(!visible_cell(light, target, |p| w.get_block(p.x, p.y, p.z).is_opaque()),
            "a complete wall must still block the torch");
    }
    #[test]
    fn light_rays_stop_at_walls_and_follow_open_doorways() {
        let mut w = World::new(1);
        w.chunks
            .insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        let a = Vec3::new(2.5, 2.5, 4.5);
        let b = Vec3::new(8.5, 2.5, 4.5);
        assert!(clear_ray(&w, a, b));
        w.set_block(5, 2, 4, crate::voxel::BlockType::Stone);
        assert!(!clear_ray(&w, a, b));
        assert!(!clear_ray(&w, b, a));
        w.set_block(5, 2, 4, crate::voxel::BlockType::Air);
        assert!(clear_ray(&w, a, b));
        assert!(!clear_ray(&w, a, Vec3::new(-2.5, 2.5, 4.5)));
    }
}
