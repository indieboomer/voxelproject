//! Local atmospheric motes and stepped wind ribbons; no simulation/network effects.
use crate::{
    voxel::{chunk::CHUNK_Y, BlockType, World},
    weather::Weather,
};
use glam::Vec3;
const MOTES: usize = 144;
const RIBBONS: usize = 48;
const SEGMENTS: usize = 8;
const MAX_VERTICES: usize = MOTES * 6 + RIBBONS * SEGMENTS * 6;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    alpha: f32,
    uv: [f32; 2],
}
struct Fleck {
    pos: Vec3,
    age: f32,
    life: f32,
    phase: f32,
}
struct Air {
    flecks: Vec<Fleck>,
    strength: f32,
    clock: f32,
    rng: u64,
}
impl Air {
    fn new() -> Self {
        Self {
            flecks: Vec::new(),
            strength: 1.0,
            clock: 0.0,
            rng: 0x712B4ED9,
        }
    }
    fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.rng >> 40) as u32) as f32 / 16777216.0
    }
    fn spawn(&mut self, eye: Vec3) -> Fleck {
        Fleck {
            pos: eye
                + Vec3::new(
                    (self.random() - 0.5) * 42.0,
                    (self.random() - 0.5) * 14.0,
                    (self.random() - 0.5) * 42.0,
                ),
            age: 0.0,
            life: 4.0 + self.random() * 5.0,
            phase: self.random() * std::f32::consts::TAU,
        }
    }
    fn update(&mut self, dt: f32, eye: Vec3, weather: Weather) {
        let dt = dt.clamp(0.0, 0.1);
        self.strength += (weather.wind_strength() - self.strength) * (1.0 - (-dt * 1.4).exp());
        self.clock = (self.clock + dt) % 10000.0;
        while self.flecks.len() < MOTES + RIBBONS {
            let f = self.spawn(eye);
            self.flecks.push(f);
        }
        let gust = 0.85 + 0.22 * (self.clock * 0.7).sin() + 0.12 * (self.clock * 1.9).sin();
        let velocity = direction() * (0.5 + self.strength * 2.5) * gust;
        for i in 0..self.flecks.len() {
            let f = &mut self.flecks[i];
            f.age += dt;
            f.pos += velocity * dt;
            f.pos.y += (self.clock * 1.1 + f.phase).sin() * dt * (0.12 + self.strength * 0.12);
            if f.age > f.life || f.pos.distance(eye) > 29.0 {
                self.flecks[i] = self.spawn(eye);
            }
        }
    }
    fn vertices(
        &self,
        eye: Vec3,
        right: Vec3,
        up: Vec3,
        world: &World,
        underwater: bool,
        weather: Weather,
    ) -> Vec<Vertex> {
        let mut out = Vec::with_capacity(MAX_VERTICES);
        if underwater || matches!(weather, Weather::Rain | Weather::Storm) {
            return out;
        }
        let mut order: Vec<_> = (0..self.flecks.len()).collect();
        order.sort_unstable_by(|a, b| {
            self.flecks[*b]
                .pos
                .distance_squared(eye)
                .total_cmp(&self.flecks[*a].pos.distance_squared(eye))
        });
        for i in order {
            let f = &self.flecks[i];
            let ribbon = i >= MOTES;
            if ribbon && weather == Weather::Mist {
                continue;
            }
            let limit = if ribbon {
                2.0 + (self.strength - 0.5).max(0.0) * 21.0
            } else {
                24.0 + self.strength * 44.0
            };
            let index = if ribbon { i - MOTES } else { i };
            let density = (limit - index as f32).clamp(0.0, 1.0);
            let life = (f.age / 0.8).min(1.0) * ((f.life - f.age) / 1.0).clamp(0.0, 1.0);
            let distance = f.pos.distance(eye);
            let fade = ((distance - 1.5) / 2.0).clamp(0.0, 1.0)
                * ((27.0 - distance) / 5.0).clamp(0.0, 1.0);
            let alpha = density
                * life
                * fade
                * if ribbon {
                    0.08 + self.strength * 0.075
                } else {
                    0.16 + self.strength * 0.065
                };
            if alpha < 0.002 || !open_air(world, f.pos) {
                continue;
            }
            if !ribbon {
                let size = 0.018 + 0.012 * (f.phase.sin() + 1.0);
                quad(
                    &mut out,
                    [
                        f.pos - right * size - up * size,
                        f.pos + right * size - up * size,
                        f.pos + right * size + up * size,
                        f.pos - right * size + up * size,
                    ],
                    [alpha; 4],
                );
            } else {
                let length = 0.8 + self.strength * 1.7;
                let phase = f.phase + (self.clock * 8.0).floor() / 8.0 * 0.65;
                let path = |t: f32| {
                    f.pos - direction() * length * t
                        + Vec3::Y * (((t * 4.0 + phase).sin() - phase.sin()) * 3.0).round() * 0.06
                };
                for segment in 0..SEGMENTS {
                    let t0 = segment as f32 / SEGMENTS as f32;
                    let t1 = (segment + 1) as f32 / SEGMENTS as f32;
                    let a = path(t0);
                    // Flat short strokes form stair steps rather than a silky curve.
                    let b = Vec3::new(path(t1).x, a.y, path(t1).z);
                    if !open_air(world, a) || !open_air(world, b) {
                        continue;
                    }
                    let side = (b - a).cross(eye - (a + b) * 0.5).normalize_or_zero() * 0.024;
                    let band = (((t0 + t1) * 0.5 * std::f32::consts::PI).sin() * 4.0).ceil() / 4.0;
                    ribbon_quad(
                        &mut out,
                        [a - side, b - side, b + side, a + side],
                        [alpha * band; 4],
                    );
                }
            }
        }
        out
    }
}
fn direction() -> Vec3 {
    Vec3::new(0.92, 0.0, 0.38).normalize()
}
fn open_air(world: &World, p: Vec3) -> bool {
    let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
    if y < 0 || world.get_block(x, y, z) != BlockType::Air {
        return false;
    }
    !(y + 1..CHUNK_Y).any(|h| world.get_block(x, h, z).is_solid())
}
fn quad(out: &mut Vec<Vertex>, points: [Vec3; 4], alpha: [f32; 4]) {
    emit(
        out,
        points,
        alpha,
        [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
    );
}
fn ribbon_quad(out: &mut Vec<Vertex>, points: [Vec3; 4], alpha: [f32; 4]) {
    emit(
        out,
        points,
        alpha,
        [[0.0, -1.0], [0.0, -1.0], [0.0, 1.0], [0.0, 1.0]],
    );
}
fn emit(out: &mut Vec<Vertex>, points: [Vec3; 4], alpha: [f32; 4], uv: [[f32; 2]; 4]) {
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            position: points[i].to_array(),
            alpha: alpha[i],
            uv: uv[i],
        });
    }
}
pub struct Wind {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    count: u32,
    air: Air,
}
impl Wind {
    pub fn new(
        device: &wgpu::Device,
        camera: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wind shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("wind.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wind layout"),
            bind_group_layouts: &[camera],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wind"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x2],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
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
            label: Some("wind vertices"),
            size: (MAX_VERTICES * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            buffer,
            count: 0,
            air: Air::new(),
        }
    }
    pub fn update(&mut self, dt: f32, eye: Vec3, weather: Weather) {
        self.air.update(dt, eye, weather);
    }
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        eye: Vec3,
        right: Vec3,
        up: Vec3,
        world: &World,
        underwater: bool,
        weather: Weather,
    ) {
        let vertices = self
            .air
            .vertices(eye, right, up, world, underwater, weather);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "CPU wind benchmark"]
    fn profile_wind_geometry() {
        let mut air = Air::new();
        let eye = Vec3::new(2048.0, 30.0, 0.0);
        let mut world = World::new(42);
        for x in 126..=130 {
            for z in -2..=2 {
                world.ensure_chunk_loaded(x, z);
            }
        }
        for _ in 0..100 {
            air.update(0.1, eye, Weather::Windy);
        }
        let start = std::time::Instant::now();
        for _ in 0..300 {
            std::hint::black_box(air.vertices(
                eye,
                Vec3::X,
                Vec3::Y,
                &world,
                false,
                Weather::Windy,
            ));
        }
        println!("wind geometry average {:?}", start.elapsed() / 300);
    }
    #[test]
    fn weather_visibility_stops_and_resumes_without_resetting_particles() {
        let eye = Vec3::new(0.0, 70.0, 0.0);
        let world = World::new(5);
        let mut air = Air::new();
        for _ in 0..60 {
            air.update(0.1, eye, Weather::Windy);
        }
        let draw = |weather| air.vertices(eye, Vec3::X, Vec3::Y, &world, false, weather);
        let windy = draw(Weather::Windy);
        assert!(!windy.is_empty());
        assert!(draw(Weather::Rain).is_empty());
        assert!(draw(Weather::Storm).is_empty());
        let mist = draw(Weather::Mist);
        assert!(!mist.is_empty() && mist.len() < windy.len());
        // Motes have +/-1 horizontal UVs; ribbons use zero.
        assert!(mist.iter().all(|v| v.uv[0].abs() == 1.0));
        assert_eq!(draw(Weather::Sunny).len(), windy.len());
        assert_eq!(draw(Weather::Windy).len(), windy.len());
    }
    #[test]
    fn gusts_follow_weather_smoothly_and_geometry_stays_bounded() {
        let eye = Vec3::new(0.0, 70.0, 0.0);
        let world = World::new(5);
        let mut air = Air::new();
        air.update(0.1, eye, Weather::Storm);
        assert!(air.strength > 1.0 && air.strength < 2.5);
        for _ in 0..100 {
            air.update(0.1, eye, Weather::Storm);
        }
        assert!((air.strength - 2.5).abs() < 0.001);
        let storm = air.vertices(eye, Vec3::X, Vec3::Y, &world, false, Weather::Windy);
        assert!(!storm.is_empty() && storm.len() <= MAX_VERTICES);
        assert!(storm
            .iter()
            .all(|v| v.alpha >= 0.0 && v.alpha < 0.5 && v.position.iter().all(|p| p.is_finite())));
        assert!(air
            .vertices(eye, Vec3::X, Vec3::Y, &world, true, Weather::Windy)
            .is_empty());
        // Compare density using identical positions/lifetimes rather than random respawns.
        air.strength = 0.5;
        let calm = air.vertices(eye, Vec3::X, Vec3::Y, &world, false, Weather::Windy);
        assert!(storm.len() > calm.len());
        for _ in 0..100 {
            air.update(0.1, eye, Weather::Windy);
        }
        assert!((air.strength - 2.0).abs() < 0.001);
        for _ in 0..100 {
            air.update(0.1, eye, Weather::Mist);
        }
        assert!((air.strength - 0.5).abs() < 0.001);
    }
    #[test]
    fn wind_particles_do_not_appear_in_blocks_water_or_under_roofs() {
        let mut world = World::new(42);
        world
            .chunks
            .insert((0, 0), crate::voxel::chunk::Chunk::new(0, 0));
        let point = Vec3::new(2.5, 20.5, 2.5);
        assert!(open_air(&world, point));
        world.set_block(2, 25, 2, BlockType::Stone);
        assert!(!open_air(&world, point));
        world.set_block(2, 25, 2, BlockType::Air);
        world.set_block(2, 20, 2, BlockType::Water);
        assert!(!open_air(&world, point));
        world.set_block(2, 20, 2, BlockType::Stone);
        assert!(!open_air(&world, point));
    }
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn validates_wind_pipeline() {
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
        let _target = Wind::new(&device, &camera, wgpu::TextureFormat::Bgra8UnormSrgb);
        assert!(pollster::block_on(device.pop_error_scope()).is_none());
    }
}
