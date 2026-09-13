#[cfg(feature = "dev-playtest")]
#[path = "playtest_app.rs"]
mod playtest_app;
use std::borrow::Cow;
use std::collections::HashMap;
use crate::transport::{Peer, Transport, JoinTarget};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window};

use crate::audio::AudioEngine;
use crate::camera::Camera;
use crate::creature::{mesh_for_snapshot, Creatures};
use crate::daynight::{sky_lighting, DAY_LENGTH_SECS};
use crate::input::Input;
use crate::llm::{classify_prompt, derive_rule_name, LlmClient, PendingGeneration, PromptKind};
use crate::model::Models;
use crate::net::{
    self, decode, encode, LaunchConfig, NotifyKind, Packet, PlayerId, ReliableChannel,
    ReliableMsg, SnapshotPlayer, UnreliableMsg, CONNECTION_TIMEOUT, HOST_PLAYER_ID,
    MAX_NICKNAME_LEN, SNAPSHOT_INTERVAL,
};
use crate::player::{
    Player, DROWNING_DAMAGE_PER_SEC, MAX_ATTRIBUTE_MULTIPLIER, MAX_HEALTH, MAX_OXYGEN,
    MIN_ATTRIBUTE_MULTIPLIER, OXYGEN_DRAIN_PER_SEC, OXYGEN_REGEN_PER_SEC,
};
use crate::raycast::raycast;
use crate::remote_player::{self, RemotePlayer};
use crate::save::{load_world, save_world};
use crate::scripting::{
    BlockBreakEvent, InteractEvent, Module, PlayerEffect, PlayerSnapshot, ScriptHost, TickOutcome,
    TICK_INTERVAL as LUA_TICK_INTERVAL,
};
use crate::ui::{
    ChatEntry, Toast, Ui, CHAT_LOG_CAPACITY, CHAT_MESSAGE_COLOR, IMPORTANT_TOAST_COLOR,
};
use crate::voxel::atlas::ATLAS_BYTES;
use crate::voxel::chunk::world_to_chunk;
use crate::voxel::mesher::{build_chunk_mesh, MeshData, Vertex};
use crate::voxel::{BlockType, World, CHUNK_X, CHUNK_Z};
use crate::weather::{Weather, WeatherState};
use crate::world_api_validate;

const REDSTONE_HEAL_RADIUS: f32 = 4.0;
const REDSTONE_HEAL_AMOUNT: f32 = 0.5;
/// How often a poisoned player loses health, and by how much -- see
/// world_api/schema.yaml's `poison_tick_secs`/`api.set_poisoned`. Pure
/// engine state, not driven by Lua at all; a module only ever flips
/// `poisoned` on or off via `api.set_poisoned`.
const POISON_TICK_INTERVAL: f32 = 10.0;
const POISON_DAMAGE_PER_TICK: f32 = 1.0;
/// Soft grayish-white haze the sky/horizon fog color blends toward during
/// `Weather::Mist`, by `MIST_FOG_BLEND` -- see `render`. Not a full
/// fog-distance system (that would need a new uniform field just for one
/// weather), just a cheap, visible tint so mist reads as genuinely hazier
/// rather than being identical to sunny except in name.
const MIST_FOG_TINT: [f32; 3] = [0.75, 0.76, 0.78];
const MIST_FOG_BLEND: f32 = 0.55;
/// Lightning strike timing/intensity -- see `App::update_lightning`. Purely
/// a local visual flourish (no sound yet, no gameplay effect), so each
/// client picks its own strike times independently rather than this being
/// host-authoritative/networked state, the same way `water_time`'s wave/
/// sway animation already is.
const LIGHTNING_MIN_INTERVAL_SECS: f32 = 4.0;
const LIGHTNING_MAX_INTERVAL_SECS: f32 = 14.0;
const LIGHTNING_MIN_PEAK: f32 = 0.7;
const LIGHTNING_MAX_PEAK: f32 = 1.0;
/// How fast the flash fades back out once triggered -- higher decays
/// faster. Applied as `flash *= (1.0 - rate * dt)` each frame, so it takes
/// roughly 0.3-0.4s to become visually negligible regardless of peak.
const LIGHTNING_DECAY_RATE: f32 = 8.0;
/// Horizontal speed above which a remote player (no real sprint flag over
/// the network) is approximated as "running" for `api.players()`. Above the
/// 4.5 walk speed but comfortably below the 7.5 sprint speed in `player.rs`.
const REMOTE_SPRINT_THRESHOLD: f32 = 6.0;

fn horizontal_speed(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
}

/// A seed for a brand-new world, different across (almost) every launch --
/// terrain, creature placement, and weather all derive from `World::seed`
/// (see `App::new`'s `world.seed`-seeded `Creatures::spawn_around`/
/// `WeatherState::new`/`build_rain_particles`), so this one value is what
/// actually varies "New World" from run to run.
///
/// Previously this was `Instant::now().elapsed().as_nanos()` -- but calling
/// `.elapsed()` immediately on an `Instant` just created measures the tiny,
/// near-constant time the two calls take back-to-back, not wall-clock time,
/// so real launches produced almost the same seed every time (hence every
/// world "looking the same"). Wall-clock time since the Unix epoch actually
/// varies between launches; XORed with the process id so two launches
/// starting in the same nanosecond-mod-2^32 window (unlikely, but the low
/// 32 bits of a nanosecond counter wrap every ~4.3s) still diverge.
///
/// Nothing about spawn position, starter modules, starting time-of-day, or
/// creature count depends on the seed (see the call site) -- only terrain/
/// creature-scatter/weather *appearance* varies, so the first-minutes
/// experience stays the same rule set and pacing every time, just on
/// differently laid-out terrain.
fn random_world_seed() -> u32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0);
    nanos ^ std::process::id() ^ 0x9E3779B9
}

/// The user-facing word for a `PromptKind`, used in generation status
/// messages/logs so a spell request doesn't get called a "rule" while it's
/// still in flight (its actual kind, `Module::is_instant`, isn't known
/// until the model's output is loaded).
fn generation_noun(kind: PromptKind) -> &'static str {
    match kind {
        PromptKind::Rule => "rule",
        PromptKind::Instant => "spell",
    }
}

fn is_in_water(world: &World, pos: Vec3) -> bool {
    world.get_block(pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32)
        == BlockType::Water
}

/// Mirrors `Player::update`'s own `below` check, since remote players don't
/// run local physics on the host.
fn approximate_on_ground(world: &World, pos: Vec3) -> bool {
    world.is_solid(
        pos.x.floor() as i32,
        (pos.y - 0.05).floor() as i32,
        pos.z.floor() as i32,
    )
}

/// Trims, caps the length, and falls back to a generic name for an empty
/// or all-whitespace nickname. Applied to our own local nickname at launch
/// and, since it arrives over the network, defensively again wherever the
/// host stores one it received from a client.
fn sanitize_nickname(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "Player".to_string()
    } else {
        trimmed.chars().filter(|c| !c.is_control()).take(MAX_NICKNAME_LEN).collect()
    }
}

const SHADOW_MAP_SIZE: u32 = 2048;
/// Half-width of the sun's orthographic frustum, in blocks. Big enough to
/// cover the visible area around the player without wasting shadow-map
/// resolution on terrain far outside the render distance.
const SHADOW_ORTHO_HALF_SIZE: f32 = 48.0;
/// How far back along the sun direction the "virtual light" sits.
const SHADOW_LIGHT_DISTANCE: f32 = 80.0;

/// Number of falling streaks drawn around the camera while it's raining.
const RAIN_PARTICLE_COUNT: usize = 1400;
/// Half-width of the square column of streaks centered on the camera --
/// kept fairly tight so the streaks read as dense rain up close rather than
/// a handful of sparse lines scattered across the whole view.
const RAIN_RADIUS: f32 = 16.0;
/// Vertical extent of the streak volume -- a streak that falls past the
/// bottom wraps back to the top, so this is also the wrap period.
const RAIN_HEIGHT: f32 = 20.0;
const RAIN_FALL_SPEED: f32 = 20.0;
const RAIN_STREAK_LENGTH: f32 = 0.9;
const RAIN_ALPHA: f32 = 0.55;

fn next_rand(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1u64 << 24) as f32
}

/// Fixed per-particle (x, z, phase) offsets, generated once and reused every
/// frame -- only the fall phase advances (via `App::water_time`), so the
/// particle set doesn't need regenerating or storing per-particle state.
fn build_rain_particles(seed: u32) -> Vec<(f32, f32, f32)> {
    let mut state = (seed as u64) | 1;
    (0..RAIN_PARTICLE_COUNT)
        .map(|_| {
            let ox = (next_rand(&mut state) * 2.0 - 1.0) * RAIN_RADIUS;
            let oz = (next_rand(&mut state) * 2.0 - 1.0) * RAIN_RADIUS;
            let phase = next_rand(&mut state) * RAIN_HEIGHT;
            (ox, oz, phase)
        })
        .collect()
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RainVertex {
    position: [f32; 3],
    alpha: f32,
}

impl RainVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem::size_of;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<RainVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32,
                },
            ],
        }
    }
}

/// A flock of birds crossing the sky -- purely decorative (see
/// `App::update_birds`'s doc comment): no collision, no gameplay effect,
/// not host-authoritative/networked, same "each client renders its own
/// copy" story as `water_time`'s wave/sway animation and the lightning/
/// cloud weather effects. At most one flock exists at a time.
struct BirdFlock {
    /// World-space position of the flock's center; `y` is fixed for the
    /// whole flock's lifetime (picked once at spawn, relative to the
    /// player's height then -- birds don't chase the player's elevation
    /// afterward).
    center: Vec3,
    /// Horizontal-only (y = 0), constant for the flock's lifetime.
    velocity: Vec3,
    age: f32,
    lifetime: f32,
    /// One entry per bird: offset from `center`, wingspan, flap phase
    /// (radians), flap rate (radians/sec) -- randomized per-bird so a
    /// flock doesn't fly in a perfectly rigid, unison-flapping block.
    birds: Vec<(Vec3, f32, f32, f32)>,
}

/// How often a flock appears, how big/fast/high it is, and how it's
/// shaped -- see `App::spawn_bird_flock`/`update_birds`.
const BIRD_MIN_INTERVAL_SECS: f32 = 30.0;
const BIRD_MAX_INTERVAL_SECS: f32 = 90.0;
const BIRD_MIN_COUNT: usize = 5;
const BIRD_MAX_COUNT: usize = 11;
const BIRD_MIN_LIFETIME_SECS: f32 = 30.0;
const BIRD_MAX_LIFETIME_SECS: f32 = 55.0;
/// Blocks/sec -- a gliding cruise speed, not a panicked dash.
const BIRD_SPEED: f32 = 7.0;
/// How far from the player a flock spawns/how far out it can fly --
/// comfortably inside the world's fog_end (160.0, see shader.wgsl) so a
/// flock is never rendered only to immediately vanish into fog.
const BIRD_SPAWN_DISTANCE: f32 = 110.0;
/// Height above the player's *current* Y at spawn time -- well above
/// normal terrain/builds so a flock reads as "high in the sky" rather than
/// weaving between trees, but still well inside view distance.
const BIRD_HEIGHT_MIN: f32 = 40.0;
const BIRD_HEIGHT_MAX: f32 = 70.0;
/// How loosely birds are scattered around the flock's own center.
const BIRD_SPREAD_RADIUS: f32 = 6.0;
/// Back to the original two-line "V" silhouette (see write_bird_vertices),
/// at exactly 2x the original wingspan/flap amplitude -- 1.2-2.2 and 0.35
/// respectively -- rather than the square-billboard version tried in
/// between.
const BIRD_WINGSPAN_MIN: f32 = 2.4;
const BIRD_WINGSPAN_MAX: f32 = 4.4;
const BIRD_FLAP_RATE_MIN: f32 = 2.5;
const BIRD_FLAP_RATE_MAX: f32 = 4.0;
/// Vertical wingtip bob amplitude at the peak of a flap -- also doubled,
/// so the flap motion still looks proportionate on the wider wingspan.
const BIRD_FLAP_AMPLITUDE: f32 = 0.7;
/// 2 line segments (4 vertices) per bird -- see write_bird_vertices.
const BIRD_VERTICES_PER_BIRD: usize = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct BirdVertex {
    position: [f32; 3],
}

impl BirdVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        use std::mem::size_of;
        wgpu::VertexBufferLayout {
            array_stride: size_of::<BirdVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            }],
        }
    }
}

const RENDER_RADIUS: i32 = 5;
const UNLOAD_RADIUS: i32 = RENDER_RADIUS + 2;
const MESH_BUDGET_PER_FRAME: usize = 2;
const CHUNK_GENERATION_BUDGET: Duration = Duration::from_millis(2);
const CHUNK_MESH_BUDGET: Duration = Duration::from_millis(3);
const REACH: f32 = 6.0;
const CREATURE_COUNT: usize = 9;

struct HostNet {
    appearance: remote_player::Appearance,
    socket: Transport,
    clients: HashMap<Peer, PlayerId>,
    remote_players: HashMap<PlayerId, RemotePlayer>,
    next_player_id: PlayerId,
    reliable: ReliableChannel,
    broadcast_timer: f32,
    port: u16,
}

struct ClientNet {
    allow_guest_prompting: bool,
    socket: Transport,
    server_addr: Peer,
    player_id: PlayerId,
    reliable: ReliableChannel,
    remote_players: HashMap<PlayerId, RemotePlayer>,
    creature_snapshot: Vec<([f32; 3], u8, f32, u8, f32)>,
    send_timer: f32,
    last_server_packet: Instant,
    lost_connection_logged: bool,
}

enum NetRole {
    Host(HostNet),
    Joined(ClientNet),
}

/// State machine for one in-flight LLM rule generation: request, and (on a
/// validation failure) exactly one automatic retry with the error fed back
/// to the model, matching the runtime rule pipeline's "validate" step.
enum GenerationState {
    Idle,
    Waiting {
        user_request: String,
        kind: PromptKind,
        pending: PendingGeneration,
        is_retry: bool,
    },
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    /// Inverse of `view_proj`, used only by sky.wgsl to unproject a screen
    /// pixel back into a world-space view ray (the sky has no real geometry
    /// to derive one from otherwise). Computed CPU-side once per frame --
    /// cheap, and far simpler than reconstructing it in the shader.
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    fog_color: [f32; 4],
    zenith_color: [f32; 4],
    /// xyz = normalized sun direction, w = raw `sky_lighting::sun_height`
    /// (see its doc comment for why the raw value travels separately from
    /// the normalized xyz) -- consumed by sky.wgsl to fade the sun disc/
    /// moon/stars in sync with the sky color's own day/night phase.
    sun_dir: [f32; 4],
    /// x = ambient, y = sun_intensity, z = free-running clock (seconds) for
    /// the water wave animation, w = wind_strength (see
    /// `Weather::wind_strength` -- multiplies the grass/leaf sway amplitude
    /// in shader.wgsl's `vs_main`, so storm/windy visibly whip vegetation
    /// harder without needing a whole extra uniform just for that).
    light_params: [f32; 4],
    /// x = lightning_flash (0..1, see `App::update_lightning` -- blended
    /// toward white in both shader.wgsl's and sky.wgsl's final color so a
    /// storm's lightning strike whites out the whole scene, not just the
    /// sky or just the terrain). y = cloud_coverage (see
    /// `Weather::cloud_coverage` -- how much of sky.wgsl's procedural cloud
    /// layer covers the sky dome). z = camera eye underwater; w = surface wetness.
    weather_fx: [f32; 4],
    camp_lights: [[f32;4];4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LightUniform {
    view_proj: [[f32; 4]; 4],
}

/// Places a "virtual light" back along the sun direction from the player
/// and looks at them with an orthographic frustum -- simple directional
/// shadows anchored to wherever the player currently is, rather than
/// covering the whole (effectively infinite) world.
fn light_view_proj(sun_dir: Vec3, player_pos: Vec3) -> glam::Mat4 {
    let light_pos = player_pos + sun_dir * SHADOW_LIGHT_DISTANCE;
    let view = glam::Mat4::look_at_rh(light_pos, player_pos, Vec3::Y);
    let h = SHADOW_ORTHO_HALF_SIZE;
    let proj = glam::Mat4::orthographic_rh(-h, h, -h, h, 1.0, SHADOW_LIGHT_DISTANCE * 2.5);
    proj * view
}

#[cfg(test)]
#[path = "render_preview_tests.rs"]
mod render_preview_tests;

struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

fn upload_mesh(device: &wgpu::Device, mesh: &MeshData) -> Option<GpuMesh> {
    if mesh.indices.is_empty() {
        return None;
    }
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh vbuf"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mesh ibuf"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    Some(GpuMesh {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
    })
}

/// A GPU mesh buffer pair that persists across frames and is updated in
/// place with `queue.write_buffer` instead of being destroyed and recreated
/// every frame the way `entity_mesh` used to be (via a fresh `upload_mesh`/
/// `create_buffer_init` call every single frame, unlike `chunk_meshes`,
/// which only ever get a new `upload_mesh` when that specific chunk
/// actually changes). The creature/player mesh's *CPU-side* data genuinely
/// does need rebuilding every frame -- positions and poses change
/// continuously -- but that never implied the GPU buffer itself needed to
/// be torn down and reallocated that often too. Grows (with headroom, to
/// avoid reallocating on every small fluctuation) when new data no longer
/// fits; never shrinks, since the entity mesh's size only varies within a
/// fairly narrow band frame to frame.
struct DynamicMesh {
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    index_buffer: wgpu::Buffer,
    index_capacity: usize,
    index_count: u32,
}

impl DynamicMesh {
    /// Starting capacities are just a reasonable guess to make the very
    /// first real update unlikely to need an immediate regrow; any size is
    /// safe since `update` grows on demand regardless.
    fn new(device: &wgpu::Device) -> Self {
        let vertex_capacity = 4096;
        let index_capacity = 8192;
        Self {
            vertex_buffer: Self::make_buffer(
                device,
                "entity vbuf",
                vertex_capacity * std::mem::size_of::<Vertex>(),
                wgpu::BufferUsages::VERTEX,
            ),
            vertex_capacity,
            index_buffer: Self::make_buffer(
                device,
                "entity ibuf",
                index_capacity * std::mem::size_of::<u32>(),
                wgpu::BufferUsages::INDEX,
            ),
            index_capacity,
            index_count: 0,
        }
    }

    fn make_buffer(device: &wgpu::Device, label: &str, size_bytes: usize, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            // wgpu requires a nonzero buffer size even when there's
            // nothing to draw yet (index_count stays 0, so nothing reads
            // from it until the first real mesh arrives).
            size: (size_bytes.max(1)) as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads `mesh`'s current contents into the persistent buffers,
    /// growing them first if they're too small to hold it.
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mesh: &MeshData) {
        self.index_count = mesh.indices.len() as u32;

        if mesh.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = mesh.vertices.len() * 3 / 2;
            self.vertex_buffer = Self::make_buffer(
                device,
                "entity vbuf",
                self.vertex_capacity * std::mem::size_of::<Vertex>(),
                wgpu::BufferUsages::VERTEX,
            );
        }
        if mesh.indices.len() > self.index_capacity {
            self.index_capacity = mesh.indices.len() * 3 / 2;
            self.index_buffer = Self::make_buffer(
                device,
                "entity ibuf",
                self.index_capacity * std::mem::size_of::<u32>(),
                wgpu::BufferUsages::INDEX,
            );
        }

        if !mesh.vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&mesh.vertices));
        }
        if !mesh.indices.is_empty() {
            queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&mesh.indices));
        }
    }
}

pub struct App {
    #[cfg(feature = "steam")]
    notified_invite: Option<u64>,
    crafting_registry: std::sync::Arc<crate::crafting::Registry>,
    crafting_ui: crate::crafting_ui::CraftingUi,
    guest_accounts: HashMap<String, crate::crafting::Account>,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,

    block_target: crate::block_target::BlockTarget,
    wind: crate::wind::Wind,
    render_pipeline: wgpu::RenderPipeline,
    depth_view: wgpu::TextureView,

    /// Draws the gradient sky/sun/moon/stars background -- see sky.wgsl.
    /// Issued first in the main pass, no depth test, so every other draw
    /// call simply paints over it.
    sky_pipeline: wgpu::RenderPipeline,

    rain_pipeline: wgpu::RenderPipeline,
    rain_vertex_buffer: wgpu::Buffer,
    /// Fixed (x, z, phase) offsets for each rain streak, relative to the
    /// camera -- see `build_rain_particles`.
    rain_particles: Vec<(f32, f32, f32)>,
    waterfalls: std::collections::HashMap<(i32,i32),Vec<crate::water::Waterfall>>,
    campfires: std::collections::HashMap<(i32,i32),Vec<Vec3>>,
    campfire_mesh: DynamicMesh,

    bird_pipeline: wgpu::RenderPipeline,
    bird_vertex_buffer: wgpu::Buffer,
    /// The currently-flying flock, if any -- see `update_birds`.
    bird_flock: Option<BirdFlock>,
    bird_rng: u64,
    /// Seconds until the next flock spawns; only counts down while no
    /// flock is currently active.
    bird_spawn_timer: f32,

    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,

    shadow_pipeline: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    shadow_light_buffer: wgpu::Buffer,
    shadow_light_bind_group: wgpu::BindGroup,
    shadow_sample_bind_group: wgpu::BindGroup,

    camera: Camera,
    player: Player,
    #[cfg(feature = "dev-playtest")]
    playtest: Option<crate::playtest::Session>,
    world: World,
    input: Input,
    creatures: Creatures,
    /// Loaded once at startup and shared by every creature -- see
    /// `model::Models`.
    models: Models,
    time_of_day: f32,
    weather: WeatherState,
    surface_weather: crate::weather::SurfaceWeather,
    /// Sound output -- see `audio::AudioEngine`. Local-only, like
    /// `lightning_flash`/`bird_flock`: every client (host or joined) drives
    /// its own playback independently from the same shared, replicated
    /// state (weather, creature positions/snapshot) rather than this being
    /// networked itself.
    audio: AudioEngine,
    /// State for `update_lightning`'s local-only strike timer -- see
    /// `LIGHTNING_MIN_INTERVAL_SECS`.
    lightning_rng: u64,
    /// Seconds until the next strike; only counts down while the current
    /// weather has lightning (`Weather::has_lightning`), and is reset to a
    /// fresh random interval otherwise so a storm doesn't strike the
    /// instant it begins.
    lightning_timer: f32,
    /// Current flash brightness, 0..1 -- jumps to a random peak on a
    /// strike, decays exponentially each frame afterward. Written into
    /// `CameraUniform.weather_fx.x` every frame in `render`.
    lightning_flash: f32,
    /// Block breaks (local or network-relayed from joined clients) collected
    /// since the last Lua tick, fed into `on_block_break` and cleared after.
    pending_block_breaks: Vec<BlockBreakEvent>,
    /// Interact-key presses (local or network-relayed from joined clients)
    /// collected since the last Lua tick, fed into `on_interact` and
    /// cleared after -- same lifecycle as `pending_block_breaks`.
    pending_interacts: Vec<InteractEvent>,
    net: NetRole,
    local_player_id: PlayerId,
    /// Shown instead of "P{id}" in chat and join notifications. Set once
    /// at launch from the menu's nickname prompt (or `--nickname`/default
    /// for a direct `--connect` launch); never changes mid-session.
    local_nickname: String,
    scripting: ScriptHost,
    lua_tick_timer: f32,
    /// Host-only, like `lua_tick_timer` -- see `POISON_TICK_INTERVAL`.
    poison_tick_timer: f32,
    llm: LlmClient,
    generation: GenerationState,
    proposal_inbox: crate::rule_sharing::Inbox,
    proposal_waiting: Option<Instant>,
    next_rule_id: u32,
    last_generated_index: Option<usize>,

    ui: Ui,
    console_open: bool,
    prompt_input: String,
    toasts: Vec<Toast>,
    /// Persistent chat/notification scrollback -- every toast (rule
    /// generated/crashed/etc.) plus real player chat, capped at
    /// `CHAT_LOG_CAPACITY`. See `log_message`.
    chat_log: Vec<ChatEntry>,
    chat_bubbles: HashMap<PlayerId, remote_player::ChatBubble>,
    chat_open: bool,
    chat_input: String,
    pending_egui_output: Option<egui::FullOutput>,
    quit_dialog_open: bool,
    /// Set once the player confirms the quit dialog; `main.rs` checks this
    /// after every `update()` and swaps the game out for a fresh main menu
    /// on the same window.
    return_to_menu: bool,

    chunk_meshes: HashMap<(i32, i32), GpuMesh>,
    entity_mesh: DynamicMesh,
    held_mesh: DynamicMesh,
    /// Host-owned mining progress and cooldowns, shared by local and remote interactions.
    interaction_states: HashMap<PlayerId, crate::equipment::Mining>,
    use_animation: f32,
    player_animation: crate::player_animation::Animation,
    automation_clock: crate::automation::Clock,
    automation_timer: f32,
    automation_revision: u64,
    automation_transfer: crate::automation_net::Transfer,
    machine_feedback: crate::machine_feedback::Feedback,
    inventory_ready: bool,
    loot: crate::loot::Effects,
    mana_timer: f32,
    /// Position of the block the player is currently chipping away at with
    /// left-click, and hits landed on it so far -- reset whenever a click
    /// targets a different position. A block breaks once hits reach its
    /// `BlockType::hardness()`; hardness-0 blocks (bedrock) never do.
    mining_target: Option<(i32, i32, i32)>,
    mining_hits: u32,
    cursor_grabbed: bool,

    last_frame: Instant,
    title_timer: f32,
    frame_count: u32,
    last_fps: f32,
    /// Free-running clock, independent of `time_of_day`, purely to drive the
    /// water surface animation in the shader.
    water_time: f32,
}

impl App {
    fn sync_guest_mana(&mut self) {
        if let NetRole::Host(host) = &mut self.net {
            for (peer,id) in &host.clients {
                if let Some(player) = host.remote_players.get(id) {
                    if let Some(account) = self.guest_accounts.get(&peer.account_key(&player.nickname)) {
                        host.reliable.send(&host.socket,*peer,ReliableMsg::CraftState {account:account.clone(),feedback:None});
                    }
                }
            }
        }
    }
    fn regenerate_mana(&mut self, dt: f32) {
        if !matches!(self.net,NetRole::Host(_)) { return; }
        self.mana_timer += dt.max(0.0);
        if self.mana_timer < 5.0 {return;}
        self.mana_timer %= 5.0;
        self.player.crafting.regenerate_mana();
        let mut changed = false;
        if let NetRole::Host(host) = &self.net {
            for (peer,id) in &host.clients {
                if let Some(player) = host.remote_players.get(id) {
                    changed |= self.guest_accounts.entry(peer.account_key(&player.nickname)).or_default().regenerate_mana();
                }
            }
        }
        if changed {self.sync_guest_mana();}
    }
    fn crafting_save(&self) -> crate::save::CraftingSave {
        crate::save::CraftingSave {
            player: Some(crate::save::PlayerSave::capture(&self.player)),
            weather: Some(self.weather.clone()),
            loot: Some(self.loot.drops.clone()),
            underground_discovered: self.world.underground_discovered.clone(),
            automation: self.world.automation.clone(),
            machine_loot: self.loot.drops.iter().filter(|d|d.machine).cloned().collect(),
            host: self.player.crafting.clone(),
            guests: self.guest_accounts.clone(),
            creatures: Some(self.creatures.snapshot_with_ids()),
            behaviors: self.creatures.behaviors.clone(),
            dragons: self.creatures.save_dragons(),
            fish: self.creatures.save_fish(),
            wildlife: self.creatures.wildlife.clone(),
        }
    }

    fn submit_crafting(&mut self, action: crate::crafting::Action) {
        self.player_animation.start(crate::player_animation::Clip::Work);
        let revision = self.player.crafting.revision;
        if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(
                &client.socket,
                client.server_addr,
                ReliableMsg::CraftRequest { revision, action },
            );
            return;
        }
        let players: Vec<_> = self.host_player_positions().iter().map(|p| p.pos).collect();
        let result = self.crafting_registry.execute(
            &mut self.player.crafting,
            revision,
            &action,
            &self.world,
            &mut self.creatures,
            self.player.position,
            &players,
        );
        self.crafting_ui.feedback = result.unwrap_or_else(|e| e);
        self.crafting_ui.pending = false;
    }

    fn handle_crafting_request(
        &mut self,
        from: Peer,
        revision: u64,
        action: crate::crafting::Action,
    ) {
        let players: Vec<_> = self.host_player_positions().iter().map(|p| p.pos).collect();
        let NetRole::Host(host) = &mut self.net else {
            return;
        };
        let Some(rp) = host
            .clients
            .get(&from)
            .and_then(|id| host.remote_players.get(id))
        else {
            return;
        };
        crate::crafting::load_interaction_area(&mut self.world, rp.pos);
        let account = self.guest_accounts.entry(from.account_key(&rp.nickname)).or_default();
        let feedback = self
            .crafting_registry
            .execute(
                account,
                revision,
                &action,
                &self.world,
                &mut self.creatures,
                rp.pos,
                &players,
            )
            .unwrap_or_else(|e| e);
        host.reliable.send(
            &host.socket,
            from,
            ReliableMsg::CraftState {
                account: account.clone(),
                feedback: Some(feedback),
            },
        );
    }

    fn publish_hotbar(&mut self) {
        if let NetRole::Joined(client)=&mut self.net {
            client.reliable.send(&client.socket,client.server_addr,ReliableMsg::Hotbar(self.player.crafting.hotbar.clone()));
        }
    }

    fn handle_remote_hotbar(&mut self, from:Peer, hotbar:crate::equipment::Hotbar) {
        let NetRole::Host(host)=&mut self.net else{return;};
        let Some(rp)=host.clients.get(&from).and_then(|id|host.remote_players.get_mut(id)) else{return;};
        let account=self.guest_accounts.entry(from.account_key(&rp.nickname)).or_default();
        let feedback=crate::equipment::accept_hotbar(account,&hotbar).err();
        rp.held=account.hotbar.entry().filter(|e|e.count(account)>0);
        host.reliable.send(&host.socket,from,ReliableMsg::CraftState{account:account.clone(),feedback});
    }

    fn all_player_positions(&self)->Vec<Vec3> {
        let remote=match &self.net {NetRole::Host(h)=>&h.remote_players,NetRole::Joined(c)=>&c.remote_players};
        std::iter::once(self.player.position).chain(remote.values().map(|p|p.pos)).collect()
    }

    fn submit_automation(&mut self,action:crate::automation::Action) {
        self.player_animation.start(crate::player_animation::Clip::Work);
        if let NetRole::Joined(client)=&mut self.net {
            if !self.inventory_ready {self.ui.automation.feedback="Waiting for host inventory".into();return;}
            client.reliable.send(&client.socket,client.server_addr,ReliableMsg::AutomationAction(action));
        }else{self.perform_automation(None,action);}
    }

    fn perform_automation(&mut self,from:Option<Peer>,action:crate::automation::Action) {
        let (feet,key)=if let Some(peer)=from {
            let NetRole::Host(host)=&self.net else{return;};
            let Some(rp)=host.clients.get(&peer).and_then(|id|host.remote_players.get(id))else{return;};
            (rp.pos,Some(peer.account_key(&rp.nickname)))
        }else{(self.player.position,None)};
        crate::crafting::load_interaction_area(&mut self.world,feet);
        let players=self.all_player_positions();let mut state=self.world.automation.clone();
        let account=if let Some(key)=key {self.guest_accounts.entry(key).or_default()}else{&mut self.player.crafting};
        let result=if let crate::automation::Action::Release{cell,item}=&action {
            crate::automation::release(&self.world,&mut self.creatures,account,feet,&players,*cell,item)
        }else{crate::automation::apply(&self.world,&mut state,account,feet,&players,&action,crate::automation::balance(),&self.crafting_registry)};
        let ok=result.is_ok();let feedback=result.err().unwrap_or_else(||"Device updated".into());
        if ok {self.world.automation=state;self.automation_timer=1.0;}
        if let Some(peer)=from {
            if let NetRole::Host(host)=&mut self.net {
                host.reliable.send(&host.socket,peer,ReliableMsg::CraftState{account:account.clone(),feedback:None});
                host.reliable.send(&host.socket,peer,ReliableMsg::AutomationResult{ok,feedback});
            }
        }else{
            self.ui.automation.feedback=feedback;
            if ok && matches!(action,crate::automation::Action::Place{packed:Some(_),..}) {self.ui.automation.build=None;}
        }
    }

    fn update_automation(&mut self,dt:f32) {
        if !matches!(self.net,NetRole::Host(_)) {return;}
        crate::underground::discover(&mut self.world,&mut self.creatures);
        self.automation_clock.advance_with(dt,&mut self.world.automation,crate::automation::balance(),&self.crafting_registry,
            |auras|crate::automation::apply_auras(&mut self.creatures,auras));
        let players=self.all_player_positions();
        crate::automation::eject_chests(&mut self.world,&mut self.creatures,&mut self.loot,&players);
        let NetRole::Host(host)=&mut self.net else{return;};
        self.automation_timer+=dt;
        if self.automation_timer>=0.5 {
            self.automation_timer=0.0;self.automation_revision+=1;
            let chunks=crate::automation_net::chunks(&self.world.automation,self.automation_revision);
            for &peer in host.clients.keys(){for chunk in &chunks {host.reliable.send(&host.socket,peer,ReliableMsg::AutomationState(chunk.clone()));}}
        }
    }

    fn perform_item_action(&mut self, from:Option<Peer>, intent:crate::equipment::Intent) {
        let (id,feet,key)=if let Some(peer)=from {
            let NetRole::Host(host)=&self.net else{return;};
            let Some(&id)=host.clients.get(&peer) else{return;};
            let Some(rp)=host.remote_players.get(&id) else{return;};
            (id,rp.pos,Some(peer.account_key(&rp.nickname)))
        } else {(self.local_player_id,self.player.position,None)};
        crate::crafting::load_interaction_area(&mut self.world,feet);
        let players:Vec<_>=self.host_player_positions().iter().map(|p|p.pos).collect();
        let account=if let Some(key)=key {self.guest_accounts.entry(key).or_default()}else{&mut self.player.crafting};
        let state=self.interaction_states.entry(id).or_default();
        let result=if matches!(intent.action,crate::equipment::Action::Attack) {
            crate::equipment::attack(&self.world,&mut self.creatures,account,state,feet,&intent).map(|()|None)
        } else {crate::equipment::block_action(&self.world,account,state,feet,&players,&intent)};
        let feedback=result.as_ref().err().filter(|s|!s.is_empty()).cloned();
        if let Some(peer)=from {
            if let NetRole::Host(host)=&mut self.net {
                if let Some(rp)=host.remote_players.get_mut(&id){rp.held=account.hotbar.entry().filter(|e|e.count(account)>0);}
                host.reliable.send(&host.socket,peer,ReliableMsg::CraftState{account:account.clone(),feedback:None});
                if let Some(text)=feedback {host.reliable.send(&host.socket,peer,ReliableMsg::Notify{kind:NotifyKind::Info,text});}
            }
        } else {
            self.mining_hits=state.hits;self.mining_target=state.target.map(|v|v.0);
            if let Some(text)=feedback {self.toasts.push(Toast::new(text));}
        }
        if let Ok(Some((p,block,old)))=result {
            self.apply_block_edit(p.0,p.1,p.2,block);
            if block==BlockType::Air {
                self.pending_block_breaks.push(BlockBreakEvent{x:p.0,y:p.1,z:p.2,block:old,player_id:id});
                if old==BlockType::Crystal && from.is_none(){self.player.carrying_crystal=true;}
                for p in self.world.flood_from(p) {self.apply_block_edit(p.0,p.1,p.2,BlockType::Water);}
            }
        }
    }

    pub async fn new(window: Arc<Window>, launch: LaunchConfig) -> Result<Self, String> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("failed to create surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("failed to find a suitable GPU adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .expect("failed to create device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_view = create_depth_view(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera buffer"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Loaded here (rather than down by `creatures`, where it's used)
        // because building the texture bind group below needs each
        // skinned model's decoded creature texture up front.
        let models = Models::load();
        let (texture_bgl, texture_bind_group) = create_atlas_bind_group(&device, &queue, &models);
        let shadow = create_shadow_resources(&device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[&camera_bgl, &texture_bgl, &shadow.sample_bgl],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("main pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky.wgsl").into()),
        });
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky pipeline layout"),
            bind_group_layouts: &[&camera_bgl],
            push_constant_ranges: &[],
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky pipeline"),
            layout: Some(&sky_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &sky_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &sky_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            // Depth-compatible with the main pass's attachment (required to
            // share it) but always-pass/no-write: this draw runs first and
            // fills every pixel unconditionally, and every subsequent
            // depth-tested draw call naturally paints over it wherever real
            // geometry exists.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let rain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rain shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("rain.wgsl").into()),
        });
        let rain_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rain pipeline layout"),
            bind_group_layouts: &[&camera_bgl],
            push_constant_ranges: &[],
        });
        let rain_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rain pipeline"),
            layout: Some(&rain_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rain_shader,
                entry_point: "vs_main",
                buffers: &[RainVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &rain_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Read-only: streaks should be hidden behind terrain, but
                // shouldn't occlude each other or write depth themselves.
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let rain_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rain vertex buffer"),
            size: ((RAIN_PARTICLE_COUNT * 2 + crate::water::MAX_VERTICES) * std::mem::size_of::<RainVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bird_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bird shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bird.wgsl").into()),
        });
        let bird_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bird pipeline layout"),
            bind_group_layouts: &[&camera_bgl],
            push_constant_ranges: &[],
        });
        let bird_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bird pipeline"),
            layout: Some(&bird_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bird_shader,
                entry_point: "vs_main",
                buffers: &[BirdVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &bird_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Cw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // Read-only, same as rain: birds should be hidden behind
                // terrain/mountains in the distance, but shouldn't occlude
                // each other or write depth themselves.
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        let bird_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bird vertex buffer"),
            // 2 lines (4 vertices) per bird, sized for the largest possible
            // flock -- see BIRD_MAX_COUNT.
            size: (BIRD_MAX_COUNT * BIRD_VERTICES_PER_BIRD * std::mem::size_of::<BirdVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut crafting_save = crate::save::CraftingSave::default();
        let (
            world,
            spawn_pos,
            yaw,
            pitch,
            time_of_day,
            net,
            local_player_id,
            spawn_creatures,
            scripting,
        ) = match launch.connect {
            None => {
                let loaded = if launch.fresh { None } else { Some(load_world(&launch.world_name).ok_or_else(||format!("Could not load world {}. The save is missing, corrupt or incompatible.",launch.world_name))?) };
                let (world, spawn_pos, yaw, pitch, time_of_day, scripting) = match loaded {
                    Some(loaded) => {
                        crafting_save = loaded.crafting;
                        (
                            loaded.world,
                            loaded.player_pos,
                            loaded.yaw,
                            loaded.pitch,
                            loaded.time_of_day,
                            ScriptHost::load_from_save(&loaded.modules),
                        )
                    },
                    None => {
                        let seed = random_world_seed();
                        let mut world = World::new(seed);
                        world.generation = launch.generation.clone();
                        world.generation.underground = true;
                        world.generation.cave_version = crate::worldgen::WorldGeneration::default().cave_version;
                        world.name = launch.world_name.clone();
                        let h = world.terrain_height(0, 0) as f32 + 2.0;
                        (
                            world,
                            Vec3::new(0.5, h, 0.5),
                            -90f32.to_radians(),
                            0.0,
                            0.28,
                            ScriptHost::scan_dir(&crate::runtime_paths::resource("modules").to_string_lossy()),
                        )
                    }
                };
                let network_settings = crate::settings::Settings::load(std::path::Path::new("settings.json")).unwrap_or_default().multiplayer;
                let socket = Transport::host(network_settings.mode, launch.port, network_settings.steam_app_id)
                    .map_err(|e| {
                        format!("Cannot host multiplayer session: {e}")
                    })?;
                log::info!("Hosting on port {}", launch.port);
                // Guests never need local inference. Start it only after hosting succeeds.
                let llm_url = launch.llm_url.clone();
                std::thread::spawn(move || { if let Err(e) = crate::llm_server::ensure_ready(&llm_url) { log::warn!("{e}"); } });
                let host_net = HostNet {
                    appearance: remote_player::Appearance::choose(random_world_seed(), []),
                    socket,
                    clients: HashMap::new(),
                    remote_players: HashMap::new(),
                    next_player_id: 1,
                    reliable: ReliableChannel::new(),
                    broadcast_timer: 0.0,
                    port: launch.port,
                };
                (
                    world,
                    spawn_pos,
                    yaw,
                    pitch,
                    time_of_day,
                    NetRole::Host(host_net),
                    HOST_PLAYER_ID,
                    true,
                    scripting,
                )
            }
            Some(server_addr) => {
                let (socket, server_addr, player_id, world, spawn_pos, time_of_day, reliable) =
                    join_handshake(server_addr, &launch.nickname)?;
                let client_net = ClientNet {
                    allow_guest_prompting: false,
                    socket,
                    server_addr,
                    player_id,
                    reliable,
                    remote_players: HashMap::new(),
                    creature_snapshot: Vec::new(),
                    send_timer: 0.0,
                    last_server_packet: Instant::now(),
                    lost_connection_logged: false,
                };
                (
                    world,
                    spawn_pos,
                    -90f32.to_radians(),
                    0.0,
                    time_of_day,
                    NetRole::Joined(client_net),
                    player_id,
                    false,
                    ScriptHost::new(),
                )
            }
        };

        let mut camera = Camera::new(spawn_pos, config.width as f32 / config.height as f32);
        camera.yaw = yaw;
        camera.pitch = pitch;
        let mut player = Player::new(spawn_pos);
        crate::equipment::remove_bow(&mut crafting_save.host);
        for account in crafting_save.guests.values_mut(){crate::equipment::remove_bow(account);}
        player.crafting = crafting_save.host;
        if let Some(saved)=crafting_save.player {saved.restore(&mut player);}

        let mut creatures = Creatures::new();
        if let Some(saved) = crafting_save.creatures {
            creatures.restore_saved(&saved, world.seed as u64);
            for (id, state) in crafting_save.behaviors {
                if creatures.behaviors.contains_key(&id) { creatures.behaviors.insert(id, state); }
            }
        } else if spawn_creatures {
            creatures.spawn_around(&world, spawn_pos, CREATURE_COUNT, world.seed);
        }
        creatures.restore_dragons(crafting_save.dragons);
        creatures.restore_fish(crafting_save.fish);
        creatures.wildlife.extend(crafting_save.wildlife);
        let weather = crafting_save.weather.unwrap_or_else(||WeatherState::new(world.seed));
        let rain_particles = build_rain_particles(world.seed);
        let lightning_seed = world.seed;

        let llm = LlmClient::new(launch.llm_url.clone());
        let wind=crate::wind::Wind::new(&device,&camera_bgl,config.format);
        let block_target = crate::block_target::BlockTarget::new(&device, &camera_bgl, config.format);
        let ui = Ui::new(&device, config.format, &window);
        let entity_mesh = DynamicMesh::new(&device);
        let campfire_mesh = DynamicMesh::new(&device);
        let held_mesh = DynamicMesh::new(&device);

        let mut app = Self {
            #[cfg(feature = "steam")]
            notified_invite: None,
            crafting_registry: std::sync::Arc::new(crate::crafting::Registry::load()?),
            crafting_ui: crate::crafting_ui::CraftingUi::default(),
            guest_accounts: crafting_save.guests,
            window,
            surface,
            device,
            queue,
            config,
            size,
            block_target,
            wind,
            render_pipeline,
            depth_view,
            sky_pipeline,
            rain_pipeline,
            rain_vertex_buffer,
            rain_particles,
            waterfalls: Default::default(),
            campfires: Default::default(),
            campfire_mesh,
            bird_pipeline,
            bird_vertex_buffer,
            bird_flock: None,
            bird_rng: (lightning_seed as u64) ^ 0x8117_D0AF_610B,
            bird_spawn_timer: BIRD_MIN_INTERVAL_SECS,
            camera_buffer,
            camera_bind_group,
            texture_bind_group,
            shadow_pipeline: shadow.pipeline,
            shadow_view: shadow.view,
            shadow_light_buffer: shadow.light_buffer,
            shadow_light_bind_group: shadow.light_bind_group,
            shadow_sample_bind_group: shadow.sample_bind_group,
            camera,
            player,
            #[cfg(feature = "dev-playtest")]
            playtest: None,
            world,
            input: Input::new(),
            creatures,
            models,
            time_of_day,
            weather,
            surface_weather: Default::default(),
            audio: AudioEngine::new(),
            lightning_rng: (lightning_seed as u64) ^ 0xB0C7_11C4_71E5,
            lightning_timer: LIGHTNING_MIN_INTERVAL_SECS,
            lightning_flash: 0.0,
            pending_block_breaks: Vec::new(),
            pending_interacts: Vec::new(),
            net,
            local_player_id,
            local_nickname: sanitize_nickname(&launch.nickname),
            scripting,
            lua_tick_timer: 0.0,
            poison_tick_timer: 0.0,
            llm,
            generation: GenerationState::Idle,
            proposal_inbox: crate::rule_sharing::Inbox::default(),
            proposal_waiting: None,
            next_rule_id: 0,
            last_generated_index: None,
            ui,
            console_open: false,
            prompt_input: String::new(),
            toasts: Vec::new(),
            chat_log: Vec::new(),
            chat_bubbles: HashMap::new(),
            chat_open: false,
            chat_input: String::new(),
            pending_egui_output: None,
            quit_dialog_open: false,
            return_to_menu: false,
            chunk_meshes: HashMap::new(),
            entity_mesh,
            held_mesh,
            interaction_states: HashMap::new(),
            use_animation: 0.0,
            player_animation: Default::default(),
            automation_clock: Default::default(),
            automation_timer: 0.0,
            automation_revision: 0,
            automation_transfer: Default::default(),
            machine_feedback: Default::default(),
            inventory_ready: false,
            loot: crate::loot::Effects::restore_machine(crafting_save.loot.unwrap_or(crafting_save.machine_loot)),
            mana_timer: 0.0,
            mining_target: None,
            mining_hits: 0,
            cursor_grabbed: false,
            last_frame: Instant::now(),
            title_timer: 0.0,
            frame_count: 0,
            last_fps: 0.0,
            water_time: 0.0,
        };

        app.grab_cursor(true);
        crate::crafting::load_interaction_area(&mut app.world,app.player.position);
        app.update_chunks();
        if launch.fresh && matches!(app.net,NetRole::Host(_)) {
            if let Some((x,_,z))=crate::underground::nearby_entrance(&app.world,app.player.position) {
                app.notify_important(format!("Explore underground: a stone-framed cave entrance is nearby at X {x}, Z {z}."));
            }
        }
        Ok(app)
    }

    fn grab_cursor(&mut self, grab: bool) {
        self.cursor_grabbed = grab;
        if grab {
            if self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok()
            {
                self.window.set_cursor_visible(false);
            }
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.window.set_cursor_visible(true);
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
        self.camera.aspect = new_size.width as f32 / new_size.height as f32;
    }

    pub fn window_event(&mut self, event: &WindowEvent) {
        // Always let egui see the event -- harmless when nothing is
        // focused, and it needs everything (including mouse position) to
        // hit-test the rules panel's buttons.
        self.ui.handle_event(&self.window, event);

        match event {
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let ElementState::Pressed = key_event.state else {
                    if let PhysicalKey::Code(code) = key_event.physical_key {
                        self.input.key_event(code, key_event.state);
                    }
                    return;
                };
                let PhysicalKey::Code(code) = key_event.physical_key else {
                    return;
                };
                if code == KeyCode::F5 && !key_event.repeat {
                    self.input.save_requested = true;
                    return;
                }
                if code == KeyCode::F10 && !key_event.repeat {
                    self.ui.settings.open = !self.ui.settings.open;
                    self.sync_settings_input();
                    return;
                }
                if code==KeyCode::KeyM && !key_event.repeat && !self.console_open && !self.chat_open && !self.quit_dialog_open && !self.crafting_ui.open && !self.ui.map.open && !self.ui.inventory_open && !self.ui.settings.open && !self.ui.automation.open {
                    self.ui.map.toggle(self.player.position);self.sync_settings_input();return;
                }
                if self.ui.map.open {
                    if !key_event.repeat && matches!(code,KeyCode::Escape|KeyCode::KeyM) {self.ui.map.open=false;self.sync_settings_input();}
                    return;
                }
                if code==KeyCode::KeyB && !key_event.repeat && !self.console_open && !self.chat_open && !self.quit_dialog_open && !self.crafting_ui.open && !self.ui.map.open && !self.ui.inventory_open && !self.ui.settings.open {
                    if self.ui.automation.build.take().is_some() {self.ui.automation.open=false;}
                    else {self.ui.automation.open=!self.ui.automation.open;}
                    self.ui.automation.selected=None;self.sync_settings_input();return;
                }
                if self.ui.automation.open {
                    if code==KeyCode::Escape {self.ui.automation.open=false;self.ui.automation.build=None;self.sync_settings_input();}
                    return;
                }
                if self.cursor_grabbed && code==KeyCode::KeyR && !key_event.repeat {
                    if let Some((_,rotation,_))=&mut self.ui.automation.build {*rotation=(*rotation+1)%4;return;}
                }
                if self.cursor_grabbed && code==KeyCode::Escape && self.ui.automation.build.is_some() {self.ui.automation.build=None;return;}
                if self.ui.settings.open {
                    if code == KeyCode::Escape {
                        self.ui.settings.open = false;
                        self.sync_settings_input();
                    }
                    return;
                }
                if code==KeyCode::KeyI && !key_event.repeat && !self.console_open && !self.chat_open && !self.quit_dialog_open && !self.crafting_ui.open {
                    self.ui.inventory_open=!self.ui.inventory_open;self.sync_settings_input();return;
                }
                if self.ui.inventory_open {
                    if code==KeyCode::Escape {self.ui.inventory_open=false;self.sync_settings_input();}
                    return;
                }
                if code == KeyCode::KeyC && !key_event.repeat && !self.console_open && !self.chat_open && !self.quit_dialog_open {
                    self.crafting_ui.open = !self.crafting_ui.open;
                    self.input.release_all();
                    self.grab_cursor(!self.crafting_ui.open);
                    return;
                }
                if self.crafting_ui.open {
                    if code == KeyCode::Escape {
                        self.crafting_ui.open = false;
                        self.grab_cursor(true);
                    }
                    return;
                }
                if code == KeyCode::Backquote && !self.quit_dialog_open && !self.chat_open {
                    self.toggle_console();
                    return;
                }
                if code == KeyCode::KeyT
                    && !self.console_open
                    && !self.quit_dialog_open
                    && !self.chat_open
                {
                    self.open_chat();
                    return;
                }
                if code == KeyCode::Escape {
                    if self.console_open {
                        self.close_console();
                    } else if self.chat_open {
                        self.close_chat();
                    } else if self.quit_dialog_open {
                        self.cancel_quit_dialog();
                    } else {
                        self.open_quit_dialog();
                    }
                    return;
                }
                if !self.ui.map.open && !self.ui.inventory_open && !self.ui.settings.open && !self.crafting_ui.open && !self.console_open && !self.quit_dialog_open && !self.chat_open {
                    self.input.key_event(code, key_event.state);
                }
            }
            WindowEvent::MouseInput { state, button, .. }
                if !self.ui.map.open && !self.ui.inventory_open && !self.ui.settings.open && !self.crafting_ui.open && !self.console_open && !self.quit_dialog_open && !self.chat_open => {
                    self.input.mouse_button_event(*button, *state);
                    if *state == ElementState::Pressed
                        && *button == MouseButton::Left
                        && !self.cursor_grabbed
                    {
                        self.grab_cursor(true);
                    }
            }
            WindowEvent::MouseWheel {delta,..} if self.cursor_grabbed && (matches!(self.net,NetRole::Host(_)) || self.inventory_ready) => {
                let y=match delta {winit::event::MouseScrollDelta::LineDelta(_,y)=>*y,winit::event::MouseScrollDelta::PixelDelta(p)=>p.y as f32};
                if y.abs()>0.01 && !self.ui.automation.tools_suspended() {self.player.crafting.hotbar.cycle(if y>0.0 {-1}else{1});self.publish_hotbar();}
            }
            _ => {}
        }
    }

    fn sync_settings_input(&mut self) {
        self.input.release_all();self.input.end_frame();
        self.grab_cursor(!self.ui.map.open && !self.ui.inventory_open && !self.ui.settings.open && !self.console_open && !self.chat_open
            && !self.crafting_ui.open && !self.quit_dialog_open && !self.ui.automation.open);
    }

    fn toggle_console(&mut self) {
        if self.console_open {
            self.close_console();
        } else {
            self.open_console();
        }
    }

    fn open_quit_dialog(&mut self) {
        self.quit_dialog_open = true;
        self.input.release_all();
        self.grab_cursor(false);
    }

    fn cancel_quit_dialog(&mut self) {
        self.quit_dialog_open = false;
        self.grab_cursor(true);
    }

    fn open_console(&mut self) {
        self.console_open = true;
        self.input.release_all();
        self.grab_cursor(false);
    }

    fn close_console(&mut self) {
        self.console_open = false;
        self.grab_cursor(true);
    }

    fn open_chat(&mut self) {
        self.chat_open = true;
        self.input.release_all();
        self.grab_cursor(false);
    }

    fn close_chat(&mut self) {
        self.chat_open = false;
        self.grab_cursor(true);
    }

    /// Appends one line to the persistent chat/notification scrollback,
    /// dropping the oldest entry once `CHAT_LOG_CAPACITY` is exceeded.
    fn log_message(&mut self, text: String, color: egui::Color32) {
        if self.chat_log.len() >= CHAT_LOG_CAPACITY {
            self.chat_log.remove(0);
        }
        self.chat_log.push(ChatEntry { text, color });
    }

    fn next_lightning_u64(&mut self) -> u64 {
        let mut x = self.lightning_rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.lightning_rng = x;
        x
    }

    fn next_lightning_f32(&mut self) -> f32 {
        (self.next_lightning_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn next_lightning_interval(&mut self) -> f32 {
        LIGHTNING_MIN_INTERVAL_SECS
            + self.next_lightning_f32() * (LIGHTNING_MAX_INTERVAL_SECS - LIGHTNING_MIN_INTERVAL_SECS)
    }

    /// Runs every frame, for host and joined client alike (see
    /// `lightning_timer`'s doc comment on why this is local-only rather
    /// than host-authoritative/networked). While the current weather has
    /// lightning, counts down to a strike; a strike jumps `lightning_flash`
    /// to a random peak, which then decays back toward 0 every frame
    /// regardless of weather so an in-progress flash always finishes
    /// naturally instead of being cut off if the storm ends mid-flash.
    fn update_lightning(&mut self, dt: f32) {
        if self.weather.current.has_lightning() {
            self.lightning_timer -= dt;
            if self.lightning_timer <= 0.0 {
                self.lightning_flash =
                    LIGHTNING_MIN_PEAK + self.next_lightning_f32() * (LIGHTNING_MAX_PEAK - LIGHTNING_MIN_PEAK);
                self.lightning_timer = self.next_lightning_interval();
                self.audio.play_lightning();
            }
        } else {
            // Don't let a stale near-zero timer cause an instant strike
            // the moment a storm begins -- always wait a fresh interval.
            self.lightning_timer = self.next_lightning_interval();
        }
        self.lightning_flash = (self.lightning_flash * (1.0 - LIGHTNING_DECAY_RATE * dt)).max(0.0);
    }

    fn next_bird_u64(&mut self) -> u64 {
        let mut x = self.bird_rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.bird_rng = x;
        x
    }

    fn next_bird_f32(&mut self) -> f32 {
        (self.next_bird_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn next_bird_range(&mut self, min: f32, max: f32) -> f32 {
        min + self.next_bird_f32() * (max - min)
    }

    /// Purely decorative: no collision, no gameplay effect of any kind, and
    /// (like `update_lightning`) local-only rather than host-authoritative/
    /// networked, so runs every frame regardless of `NetRole`. While no
    /// flock is active, counts down to spawning one; while one is active,
    /// advances its flight and clears it once its lifetime runs out.
    fn update_birds(&mut self, dt: f32) {
        if let Some(flock) = &mut self.bird_flock {
            flock.age += dt;
            flock.center += flock.velocity * dt;
            if flock.age >= flock.lifetime {
                self.bird_flock = None;
                self.bird_spawn_timer = self.next_bird_range(BIRD_MIN_INTERVAL_SECS, BIRD_MAX_INTERVAL_SECS);
            }
            return;
        }
        self.bird_spawn_timer -= dt;
        if self.bird_spawn_timer <= 0.0 {
            self.bird_flock = Some(self.spawn_bird_flock());
            self.audio.play_bird_flock();
        }
    }

    /// Builds a new flock starting `BIRD_SPAWN_DISTANCE` away from the
    /// player in a random direction, flying roughly (with some random
    /// deviation, so it's not perfectly predictable) toward the opposite
    /// side -- guaranteeing it visibly crosses a good stretch of sky during
    /// its lifetime instead of a short random walk that might immediately
    /// drift out of view.
    fn spawn_bird_flock(&mut self) -> BirdFlock {
        let spawn_angle = self.next_bird_f32() * std::f32::consts::TAU;
        let deviation = self.next_bird_range(-40f32.to_radians(), 40f32.to_radians());
        let flight_angle = spawn_angle + std::f32::consts::PI + deviation;

        let spawn_dir = Vec3::new(spawn_angle.cos(), 0.0, spawn_angle.sin());
        let flight_dir = Vec3::new(flight_angle.cos(), 0.0, flight_angle.sin());

        let height = self.player.position.y + self.next_bird_range(BIRD_HEIGHT_MIN, BIRD_HEIGHT_MAX);
        let mut center = self.player.position + spawn_dir * BIRD_SPAWN_DISTANCE;
        center.y = height;

        let count = BIRD_MIN_COUNT
            + (self.next_bird_f32() * (BIRD_MAX_COUNT - BIRD_MIN_COUNT + 1) as f32) as usize;
        let count = count.min(BIRD_MAX_COUNT);
        let birds = (0..count)
            .map(|_| {
                let offset_angle = self.next_bird_f32() * std::f32::consts::TAU;
                let offset_radius = self.next_bird_f32() * BIRD_SPREAD_RADIUS;
                let offset = Vec3::new(
                    offset_angle.cos() * offset_radius,
                    self.next_bird_range(-1.0, 1.0),
                    offset_angle.sin() * offset_radius,
                );
                let wingspan = self.next_bird_range(BIRD_WINGSPAN_MIN, BIRD_WINGSPAN_MAX);
                let phase = self.next_bird_f32() * std::f32::consts::TAU;
                let flap_rate = self.next_bird_range(BIRD_FLAP_RATE_MIN, BIRD_FLAP_RATE_MAX);
                (offset, wingspan, phase, flap_rate)
            })
            .collect();

        BirdFlock {
            center,
            velocity: flight_dir * BIRD_SPEED,
            age: 0.0,
            lifetime: self.next_bird_range(BIRD_MIN_LIFETIME_SECS, BIRD_MAX_LIFETIME_SECS),
            birds,
        }
    }

    /// Rebuilds the bird vertex buffer for the current flock (a no-op if
    /// none is active). Each bird is two line segments -- left wingtip to
    /// center, center to right wingtip -- meeting at a shared center point
    /// for a continuous shallow "V" silhouette, with the wing axis
    /// perpendicular to the flock's flight direction so birds visibly face
    /// the way they're flying rather than always being wing-aligned to
    /// world X. Wingtips bob vertically on a per-bird phase/rate so the
    /// flock doesn't flap in unison. (A square-billboard version of this
    /// was tried and reverted -- back to two lines, just at 2x the
    /// original wingspan/flap amplitude; see BIRD_WINGSPAN_MIN/MAX.)
    fn write_bird_vertices(&self) {
        let Some(flock) = &self.bird_flock else {
            return;
        };
        let forward = if flock.velocity.length_squared() > 0.0001 {
            flock.velocity.normalize()
        } else {
            Vec3::X
        };
        let wing_axis = Vec3::new(-forward.z, 0.0, forward.x);

        let mut verts: Vec<BirdVertex> = Vec::with_capacity(flock.birds.len() * BIRD_VERTICES_PER_BIRD);
        for &(offset, wingspan, phase, flap_rate) in &flock.birds {
            let pos = flock.center + offset;
            let flap = (flock.age * flap_rate + phase).sin() * BIRD_FLAP_AMPLITUDE;
            let half = wing_axis * (wingspan * 0.5);
            let left = pos - half + Vec3::new(0.0, flap, 0.0);
            let right = pos + half + Vec3::new(0.0, flap, 0.0);
            verts.push(BirdVertex { position: left.to_array() });
            verts.push(BirdVertex { position: pos.to_array() });
            verts.push(BirdVertex { position: pos.to_array() });
            verts.push(BirdVertex { position: right.to_array() });
        }
        self.queue
            .write_buffer(&self.bird_vertex_buffer, 0, bytemuck::cast_slice(&verts));
    }

    /// Sends a chat message from the local player. The host logs and
    /// broadcasts it directly; a joined client hands it to the host, which
    /// attributes it to the sender and relays it back to everyone
    /// (including the sender) as `PlayerChat` -- so there's no separate local
    /// echo path to keep in sync with the relayed one.
    fn send_chat(&mut self, text: String) {
        let Some(text) = net::chat_text(&text) else { return; };
        if matches!(self.net, NetRole::Host(_)) {
            self.broadcast_chat(self.local_player_id, self.local_nickname.clone(), text);
        } else if let NetRole::Joined(client) = &mut self.net {
            client.reliable.send(
                &client.socket,
                client.server_addr,
                ReliableMsg::ChatMessage(text),
            );
        }
    }

    pub fn device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.cursor_grabbed {
                self.input.mouse_delta.0 += delta.0 as f32;
                self.input.mouse_delta.1 += delta.1 as f32;
            }
        }
    }

    pub fn update(&mut self) {
        // Only the host chooses the testing policy; clients receive it with
        // the crafting registry so previews and authoritative charges agree.
        if let NetRole::Host(host)=&mut self.net {
            let mana_free=self.ui.settings.values.gameplay.mana_free;
            if self.crafting_registry.mana_free!=mana_free {
                std::sync::Arc::make_mut(&mut self.crafting_registry).mana_free=mana_free;
                self.scripting.inventory_registry=self.crafting_registry.clone();
                for &peer in host.clients.keys() {
                    host.reliable.send(&host.socket,peer,ReliableMsg::CraftRegistry((*self.crafting_registry).clone()));
                }
            }
        }
        #[cfg(feature = "steam")]
        {
            crate::steam_transport::poll_runtime();
            if let Some(id) = crate::steam_transport::pending_invite() {
                if self.notified_invite != Some(id) {
                    self.notified_invite = Some(id);
                    self.toasts.push(Toast::important("Steam invitation received. Return to the main menu to join it."));
                }
            }
        }
        let now = Instant::now();
        let elapsed = (now - self.last_frame).as_secs_f32();
        let dt = elapsed.min(0.1);
        self.last_frame = now;
        #[cfg(feature = "dev-playtest")]
        self.update_playtest(dt);
        // Wrap well before f32 precision would start eating into a sine's
        // period -- the animation is periodic anyway so this is seamless.
        self.water_time = (self.water_time + dt) % 10_000.0;
        self.surface_weather.update(self.weather.current, dt);
        self.update_lightning(dt);
        self.update_birds(dt);
        self.audio.update_ambience(self.weather.current, self.time_of_day, dt);

        const SENSITIVITY: f32 = 0.0022;
        self.camera.yaw += self.input.mouse_delta.0 * SENSITIVITY;
        self.camera.pitch -= self.input.mouse_delta.1 * SENSITIVITY;
        self.camera.pitch = self
            .camera
            .pitch
            .clamp(-89f32.to_radians(), 89f32.to_radians());

        let forward = self.camera.forward();
        let right = self.camera.right();
        // Collision-critical terrain must exist before physics, including after teleport.
        crate::crafting::load_interaction_area(&mut self.world,self.player.position);
        self.player
            .update(&self.world, &self.input, forward, right, dt);
        self.camera.position = self.player.position;
        self.wind.update(dt,self.camera.eye_position(),self.weather.current);
        self.audio.update_listener(self.camera.eye_position(), right);
        for _ in 0..self.player.take_steps() {
            self.audio.play_player_step();
        }

        self.use_animation=(self.use_animation-dt).max(0.0);
        let machine_input=self.ui.automation.tools_suspended();
        if self.cursor_grabbed {
            if let Some(hit)=raycast(&self.world,self.camera.eye_position(),self.camera.forward(),REACH) {
                if self.input.left_clicked || self.input.right_clicked {
                    if let Some((kind,rotation,packed))=self.ui.automation.build {
                        self.submit_automation(crate::automation::Action::Place{kind,cell:hit.place,rotation,packed});
                        self.input.left_clicked=false;
                        self.input.right_clicked=false;
                    }
                }
                if let Some(base)=self.world.automation.device_at(hit.target).map(|d|d.cell) {
                    if !machine_input && self.input.left_clicked {self.submit_automation(crate::automation::Action::Pack{cell:base});self.input.left_clicked=false;}
                    if self.input.interact_clicked {
                        if let Some(d)=self.world.automation.devices.get(&base) {self.ui.automation.inspect(d);}
                        self.sync_settings_input();
                    }
                }
            }
            // Capture the mode before placing: placing the last packed machine
            // can exit it, but that same click must never reach a restored tool.
            if machine_input {self.input.left_clicked=false;self.input.right_clicked=false;}
        }
        self.player_animation.advance(dt,
            glam::Vec2::new(self.player.velocity.x, self.player.velocity.z).length(),
            self.player.on_ground);
        if self.cursor_grabbed {
            if let Some(gesture) = self.input.gesture { self.player_animation.start(gesture); }
        }
        if let Some(i)=self.input.hotbar_select.filter(|_| !machine_input && (matches!(self.net,NetRole::Host(_)) || self.inventory_ready)) {
            self.player.crafting.hotbar.select(i);self.publish_hotbar();
        }
        if self.cursor_grabbed && !machine_input && (matches!(self.net,NetRole::Host(_)) || self.inventory_ready) {
            use crate::equipment::{Entry,Gear,Action,Intent};
            let entry=self.player.crafting.hotbar.entry();
            if self.input.left_clicked || self.input.right_clicked {
                let action=if self.input.right_clicked {Action::Place} else {match entry {Some(Entry::Resource(_))=>Action::Place,Some(Entry::Gear(Gear::Sword))=>Action::Attack,_=>Action::Mine}};
                let intent=Intent{hotbar:self.player.crafting.hotbar.clone(),item:entry,target:raycast(&self.world,self.camera.eye_position(),self.camera.forward(),REACH).map(|h|h.target),direction:self.camera.forward().to_array(),action};
                if entry.is_none() || entry.is_some_and(|e|e.count(&self.player.crafting)>0) {
                    self.use_animation=0.28;self.audio.play_player_attack();
                    self.player_animation.start(if matches!(action, Action::Attack) {
                        crate::player_animation::Clip::Attack
                    } else { crate::player_animation::Clip::Work });
                }
                if let NetRole::Joined(client)=&mut self.net {client.reliable.send(&client.socket,client.server_addr,ReliableMsg::ItemAction(intent));}
                else {self.perform_item_action(None,intent);}
            }
        }

        // Interact (F): a deliberate, non-destructive "use" aimed at
        // whatever's under the crosshair, distinct from mining (left click)
        // and placing (right click) -- see world_api/schema.yaml's
        // `on_interact`. Only reports the event; a rule decides what, if
        // anything, happens.
        if !self.console_open && !self.chat_open && self.input.interact_clicked {
            let origin = self.camera.eye_position();
            let dir = self.camera.forward();
            if let Some(hit) = raycast(&self.world, origin, dir, REACH) {
                self.player_animation.start(crate::player_animation::Clip::Work);
                match &mut self.net {
                    NetRole::Host(_) => {
                        let block = self
                            .world
                            .get_block(hit.target.0, hit.target.1, hit.target.2);
                        self.pending_interacts.push(InteractEvent {
                            x: hit.target.0,
                            y: hit.target.1,
                            z: hit.target.2,
                            block,
                            player_id: self.local_player_id,
                        });
                    }
                    NetRole::Joined(client) => {
                        client.reliable.send(
                            &client.socket,
                            client.server_addr,
                            ReliableMsg::Interact {
                                x: hit.target.0,
                                y: hit.target.1,
                                z: hit.target.2,
                            },
                        );
                    }
                }
            }
        }

        if self.input.save_requested {
            if matches!(self.net, NetRole::Host(_)) {
                let result = save_world(
                    &self.world,
                    &self.player,
                    &self.camera,
                    self.time_of_day,
                    self.scripting.save_entries(),
                    &self.crafting_save(),
                );
                match result {Ok(())=>self.notify_important(format!("Saved world: {}",self.world.name)),Err(e)=>self.notify_important(e)}
            } else {
                log::warn!("Only the host can save the world.");
            }
        }

        self.input.end_frame();
        self.ui.settings.poll_name(self.llm.base_url());
        let display=crate::fantasy_name::clean(&self.ui.settings.values.player_name);
        if !display.is_empty() && display!=self.local_nickname {
            self.local_nickname=display.clone();
            if let NetRole::Joined(client)=&mut self.net {client.reliable.send(&client.socket,client.server_addr,ReliableMsg::DisplayName(display));}
        }
        self.update_chunks();
        self.poll_network(dt);
        self.update_automation(elapsed);
        self.scripting.inventory_registry = self.crafting_registry.clone();
        self.regenerate_mana(dt);
        self.poll_generation();
        self.poll_proposals();

        if matches!(self.net, NetRole::Host(_)) {
            self.time_of_day = (self.time_of_day + dt / DAY_LENGTH_SECS).rem_euclid(1.0);
            self.weather.update(dt);
            let player_targets: Vec<(PlayerId, Vec3)> = self
                .host_player_positions()
                .iter()
                .map(|p| (p.id, p.pos))
                .collect();
            self.scripting.sync_attack_policies(&mut self.creatures);
            self.creatures.update_start_protection(dt,&player_targets);
            self.creatures.populate_wildlife(&self.world, &player_targets, dt);
            self.creatures.discover_dragons(&self.world, &player_targets);
            self.creatures.discover_fish(&self.world, &player_targets, dt);
            let golem_attacks = self.creatures.update(&self.world, dt, &player_targets);
            for (player_id, damage) in golem_attacks {
                #[cfg(feature = "dev-playtest")]
                if player_id==crate::playtest::AGENT_ID {
                    if let Some(session)=&mut self.playtest {session.hostile_hit(damage);}
                }
                self.apply_player_effect(PlayerEffect::Health {
                    player_id,
                    delta: -damage,
                });
            }

            self.lua_tick_timer += dt;
            if self.lua_tick_timer >= LUA_TICK_INTERVAL {
                self.lua_tick_timer = 0.0;
                let players = self.host_player_positions();
                let outcome = self.scripting.run_tick(
                    &self.world,
                    &mut self.creatures,
                    &players,
                    &mut self.time_of_day,
                    &mut self.weather,
                    &self.pending_block_breaks,
                    &self.pending_interacts,
                    self.player.resources_snapshot(),
                );
                self.pending_block_breaks.clear();
                self.pending_interacts.clear();
                self.apply_tick_outcome(outcome);
                for &(x, y, z) in self.world.redstone_positions.iter() {
                    let center = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                    self.creatures
                        .heal_near(center, REDSTONE_HEAL_RADIUS, REDSTONE_HEAL_AMOUNT);
                }
            }

            self.poison_tick_timer += dt;
            if self.poison_tick_timer >= POISON_TICK_INTERVAL {
                self.poison_tick_timer = 0.0;
                self.apply_poison_ticks();
            }

            self.update_oxygen(dt);

            // Creature sound events (steps/attacks queued every tick inside
            // `creatures.update` above; deaths queued during the Lua tick
            // just above, if one ran this frame) -- host-only, same as
            // creature AI itself. A joined client doesn't run creature AI
            // locally at all (see `Creatures::update`'s doc comment), so it
            // doesn't get these; only the fully-local ambience/lightning/
            // birds/footstep sounds play for it too.
            let events = self.creatures.take_audio_events();
            for (kind, pos) in events.attacks {
                self.audio.play_creature_attack(kind, pos);
            }
            for (_, pos) in events.steps {
                self.audio.play_creature_step(pos);
            }
            for (kind, pos) in events.ambient_calls {
                self.audio.play_creature_ambient(kind, pos);
            }
            let puffs:Vec<_>=events.deaths.iter().take(16).map(|(_,p)|p.to_array()).collect();
            if !puffs.is_empty() {if let NetRole::Host(host)=&mut self.net {for &peer in host.clients.keys() {host.reliable.send(&host.socket,peer,ReliableMsg::DeathPuffs(puffs.clone()));}}}
            for (_, pos) in events.deaths {
                self.loot.puff(pos);
                self.audio.play_creature_death(pos);
            }
            for death in self.creatures.player_kills.drain(..) {self.loot.spawn(&self.world,death.kind,death.pos);}
            let inventory_revision=self.player.crafting.revision;
            let acquired=self.loot.collect(&self.world,self.player.position,&mut self.player.crafting);
            if self.player.crafting.revision!=inventory_revision {self.audio.play_loot();}
            self.ui.show_pickup(acquired);
            let mut changed=false;
            if let NetRole::Host(host)=&mut self.net {for (peer,id) in &host.clients {
                if let Some(p)=host.remote_players.get(id) {let account=self.guest_accounts.entry(peer.account_key(&p.nickname)).or_default();let rev=account.revision;
                    let acquired=self.loot.collect(&self.world,p.pos,account);changed|=rev!=account.revision;
                    if rev!=account.revision {host.reliable.send(&host.socket,*peer,ReliableMsg::LootCollected(acquired));}
                }
            }}
            if changed {self.sync_guest_mana();}
        }
        self.loot.update(dt,matches!(self.net,NetRole::Host(_)));
        for (cell,cue) in self.machine_feedback.update(&self.world.automation,dt,self.camera.eye_position()) {
            self.audio.play_machine(crate::automation::center(cell),cue);
        }

        match &self.net {
            NetRole::Host(_) => self.audio.update_creature_flight(&self.creatures.snapshot()),
            NetRole::Joined(client) => self.audio.update_creature_flight(&client.creature_snapshot),
        }
        let mut visible_creatures = match &self.net {
            NetRole::Host(_) => self.creatures.snapshot(),
            NetRole::Joined(client) => client.creature_snapshot.clone(),
        };
        let center = chunk_of(self.player.position);
        visible_creatures.retain(|entry| {
            let cell = chunk_of(Vec3::from_array(entry.0));
            crate::visibility::within_terrain_range(cell, center, RENDER_RADIUS)
                && self.chunk_meshes.contains_key(&cell)
        });
        let mut mesh = mesh_for_snapshot(&visible_creatures, &self.models);
        for d in self.world.automation.devices.values() {
            let cell=chunk_of(crate::automation::center(d.cell));
            if crate::visibility::within_terrain_range(cell,center,RENDER_RADIUS) && self.chunk_meshes.contains_key(&cell) {
                let mut prop=crate::automation_mesh::device(d,self.water_time,None,&self.world.automation);
                self.machine_feedback.decorate(d.cell,&mut prop);
                mesh.extend(prop);
            }
        }
        mesh.extend(self.machine_feedback.particles());
        if let Some((kind,rotation,packed))=self.ui.automation.build {
            if let Some(hit)=raycast(&self.world,self.camera.eye_position(),self.camera.forward(),REACH) {
                let mut preview=packed.and_then(|i|self.player.crafting.packed_devices.get(i)).cloned()
                    .unwrap_or_else(||crate::automation::Device::new(kind,hit.place,rotation));
                preview.cell=hit.place;preview.rotation=rotation;
                let mut state=self.world.automation.clone();let mut account=self.player.crafting.clone();
                let positions=self.all_player_positions();
                let valid=crate::automation::apply(&self.world,&mut state,&mut account,self.player.position,&positions,
                    &crate::automation::Action::Place{kind,cell:hit.place,rotation,packed},crate::automation::balance(),&self.crafting_registry).is_ok();
                mesh.extend(crate::automation_mesh::device(&preview,self.water_time,Some(valid),&self.world.automation));
            }
        }
        mesh.extend(self.loot.mesh(|pos|{let cell=chunk_of(pos);crate::visibility::within_terrain_range(cell,center,RENDER_RADIUS)&&self.chunk_meshes.contains_key(&cell)}));
        let remote_players = match &self.net {
            NetRole::Host(host) => &host.remote_players,
            NetRole::Joined(client) => &client.remote_players,
        };
        mesh.extend(remote_player::build_mesh(
            remote_players,
            self.local_player_id,
            &self.models,
        ));
        #[cfg(feature = "dev-playtest")]
        if let Some(session) = &self.playtest { mesh.extend(remote_player::build_mesh(&session.visual,self.local_player_id,&self.models)); }
        self.entity_mesh.update(&self.device, &self.queue, &mesh);
        let entry=self.player.crafting.hotbar.entry().filter(|e|!self.ui.automation.tools_suspended() && e.count(&self.player.crafting)>0);
        let forward=self.camera.forward();let right=self.camera.right();let up=right.cross(forward);
        let swing=(self.use_animation/0.28*std::f32::consts::PI).sin();
        let camera_basis=glam::Mat3::from_cols(right,up,-forward);
        // Turn the original held pose AND its use motion around the vertical
        // axis at the grip: +Y rotation is counterclockwise viewed from above.
        let turn=if entry.is_some() {glam::Mat3::from_rotation_y(std::f32::consts::FRAC_PI_2)}else{glam::Mat3::IDENTITY};
        let basis=camera_basis*turn*glam::Mat3::from_rotation_z(-0.30-swing*0.6);
        let motion=Vec3::new(-swing*0.12,0.0,-swing*0.08);
        let origin=self.camera.eye_position()+camera_basis*(Vec3::new(0.32,-0.42,-0.65)+turn*motion);
        self.held_mesh.update(&self.device,&self.queue,&crate::held_item::mesh(entry,origin,basis,0.45));


        self.toasts.retain(|t| !t.is_expired());
        let is_host = matches!(self.net, NetRole::Host(_));
        let can_prompt = self.can_prompt();
        let generation_status: Option<String> = match &self.generation {
            GenerationState::Waiting {
                kind, is_retry: true, ..
            } => Some(format!(
                "Retrying the {} with error feedback...",
                generation_noun(*kind)
            )),
            GenerationState::Waiting {
                kind,
                is_retry: false,
                ..
            } => Some(format!("Interpreting, generating and checking a {}...", generation_noun(*kind))),
            GenerationState::Idle => self.proposal_waiting.map(|sent| if sent.elapsed() > Duration::from_secs(30) {
                "Host response is delayed; you can keep playing while waiting.".into()
            } else { "Waiting for host validation...".into() }),
        };
        let mut crafting_players: Vec<_> = self.host_player_positions().iter().map(|p| p.pos).collect();
        if let NetRole::Joined(client) = &self.net {
            crafting_players.push(self.player.position);
            crafting_players.extend(client.remote_players.values().map(|p| p.pos));
            crafting_players.extend(client.creature_snapshot.iter().map(|c| Vec3::from_array(c.0)));
        }
        let lobby_code = match &self.net {
            NetRole::Host(host) => host.socket.lobby_code(),
            NetRole::Joined(client) => client.socket.lobby_code(),
        };
        let settings_was_open = self.ui.settings.open;
        let map_was_open=self.ui.map.open;
        let automation_was_open=self.ui.automation.open;
        let (full_output, requests) = self.ui.draw(
            &self.window,
            self.console_open,
            &mut self.prompt_input,
            is_host,
            can_prompt,
            &self.scripting,
            self.last_generated_index,
            generation_status.as_deref(),
            &self.toasts,
            self.last_fps,
            self.quit_dialog_open,
            &self.player,
            self.chat_open,
            &mut self.chat_input,
            &self.chat_log,
            &mut self.crafting_ui,
            &self.crafting_registry,
            &self.world,
            &self.creatures,
            &crafting_players,
            lobby_code.as_deref(),
        );
        self.pending_egui_output = Some(full_output);
        if map_was_open!=self.ui.map.open {self.sync_settings_input();}
        if automation_was_open!=self.ui.automation.open {self.sync_settings_input();}
        if let Some(action)=requests.automation {self.submit_automation(action);}
        if settings_was_open != self.ui.settings.open { self.sync_settings_input(); }
        if requests.close_inventory {self.ui.inventory_open=false;self.sync_settings_input();}
        if let Some(action) = requests.crafting {
            self.submit_crafting(action);
        }

        if requests.invite_friends {
            match &self.net { NetRole::Host(host) => host.socket.invite_friends(), NetRole::Joined(client) => client.socket.invite_friends() }
        }
        if let Some(slot)=requests.select_slot.filter(|_|!self.ui.automation.tools_suspended() && (matches!(self.net,NetRole::Host(_)) || self.inventory_ready)) {self.player.crafting.hotbar.select(slot);self.publish_hotbar();}
        if let Some(entry)=requests.assign_entry.filter(|_|!self.ui.automation.tools_suspended() && (matches!(self.net,NetRole::Host(_)) || self.inventory_ready)) {
            if entry.is_none() || entry.is_some_and(|e|e.count(&self.player.crafting)>0) {
                self.player.crafting.hotbar.assign(entry);self.publish_hotbar();
            }
        }

        if let Some(text) = requests.send_chat {
            self.chat_input.clear();
            self.send_chat(text);
        }

        if requests.confirm_quit {
            self.return_to_menu = true;
        }
        if requests.cancel_quit {
            self.cancel_quit_dialog();
        }

        if let Some(idx) = requests.toggle_index {
            if let Some((name, enabled)) = self.scripting.toggle_at(idx) {
                self.notify_all(format!(
                    "Rule '{name}' {}",
                    if enabled { "activated" } else { "deactivated" }
                ));
            }
        }
        if let Some(idx) = requests.run_index {
            self.run_instant(idx);
        }
        if let Some(idx) = requests.delete_index {
            if let Some(name) = self.scripting.remove(idx) {
                if self.last_generated_index == Some(idx) {
                    self.last_generated_index = None;
                }
                self.notify_all(format!("Rule '{name}' deleted"));
            }
        }
        if let Some(prompt) = requests.submit_prompt {
            if self.can_prompt() && self.proposal_waiting.is_none() && matches!(self.generation, GenerationState::Idle) {
                self.prompt_input.clear();
                self.start_generation(prompt);
            }
        }

        self.title_timer += dt;
        self.frame_count += 1;
        if self.title_timer >= 0.5 {
            let fps = self.frame_count as f32 / self.title_timer;
            self.last_fps = fps;
            let hour = (self.time_of_day * 24.0) as u32;
            let minute = ((self.time_of_day * 24.0 - hour as f32) * 60.0) as u32;
            let role_info = match &self.net {
                NetRole::Host(host) => {
                    format!("Hosting {} ({}/4 players)", host.socket.lobby_code().unwrap_or_else(||format!(":{}",host.port)), host.clients.len()+1)
                }
                NetRole::Joined(client) => format!(
                    "Connected to {} as P{}",
                    client.server_addr, client.player_id
                ),
            };
            let block_info = self.player.crafting.hotbar.entry().map_or("Empty hand",|e|e.name());
            let mining_info = match self.mining_target {
                Some(pos) => {
                    let target = self.world.get_block(pos.0, pos.1, pos.2);
                    format!(" | Mining {}: {}/{}", target.name(), self.mining_hits, target.hardness())
                }
                None => String::new(),
            };
            self.window.set_title(&format!(
                "Voxel Project | {:02}:{:02} | {} | Block: {} | FPS: {:.0}{} | ~ rules/generate, T chat, F5 save, F11 fullscreen, Esc quit",
                hour,
                minute,
                role_info,
                block_info,
                fps,
                mining_info
            ));
            self.title_timer = 0.0;
            self.frame_count = 0;
        }
    }

    /// Sends a raw `Notify` to every connected client; no-op if not
    /// hosting. Callers add their own local toast/log entry with whatever
    /// styling fits the message.
    fn broadcast_notify(&mut self, kind: NotifyKind, text: &str) {
        if let NetRole::Host(host) = &mut self.net {
            let addrs: Vec<Peer> = host.clients.keys().copied().collect();
            for addr in addrs {
                host.reliable.send(
                    &host.socket,
                    addr,
                    ReliableMsg::Notify {
                        kind,
                        text: text.to_string(),
                    },
                );
            }
        }
    }

    fn notify_all(&mut self, text: String) {
        self.toasts.push(Toast::new(text.clone()));
        self.log_message(text.clone(), egui::Color32::WHITE);
        self.broadcast_notify(NotifyKind::Info, &text);
    }

    /// Like `notify_all`, but for things the player actually needs to
    /// notice and act on (e.g. "a new rule is waiting to be enabled"), not
    /// just an FYI that's fine to scroll past.
    fn notify_all_important(&mut self, text: String) {
        self.toasts.push(Toast::important(text.clone()));
        self.log_message(text.clone(), IMPORTANT_TOAST_COLOR);
        self.broadcast_notify(NotifyKind::Important, &text);
    }

    /// Local-only version of `notify_all_important` -- toast + chat log,
    /// no network broadcast. For host-side status only the host can act
    /// on, e.g. rule generation failing (only the host generates rules).
    fn notify_important(&mut self, text: String) {
        self.toasts.push(Toast::important(text.clone()));
        self.log_message(text, IMPORTANT_TOAST_COLOR);
    }

    fn show_player_chat(&mut self, player_id: PlayerId, name: &str, text: String) {
        let formatted = format!("{name}: {text}");
        self.toasts.push(Toast::new(formatted.clone()));
        self.log_message(formatted, CHAT_MESSAGE_COLOR);
        self.chat_bubbles.insert(player_id, remote_player::ChatBubble { text, started: Instant::now() });
    }

    /// The host attributes chat by connection, never by parsing display names.
    fn broadcast_chat(&mut self, player_id: PlayerId, name: String, text: String) {
        self.show_player_chat(player_id, &name, text.clone());
        if let NetRole::Host(host) = &mut self.net {
            for &peer in host.clients.keys() {
                host.reliable.send(&host.socket,peer,ReliableMsg::PlayerChat { player_id, name:name.clone(), text:text.clone() });
            }
        }
    }

    /// All known player state, host included. Used for the Lua World API.
    /// Remote players' `velocity`/`on_ground`/`sprinting` are approximated
    /// on the host from position deltas and shared-world queries, since the
    /// network protocol only carries a client's position/yaw per tick.
    fn host_player_positions(&self) -> Vec<PlayerSnapshot> {
        let NetRole::Host(host) = &self.net else {
            return Vec::new();
        };
        let mut players = vec![PlayerSnapshot {
                finances: crate::scripting::InventoryBalances::from_account(&self.player.crafting),
                resources: self.player.resources_snapshot(),
            id: HOST_PLAYER_ID,
            pos: self.player.position,
            carrying_crystal: self.player.carrying_crystal,
            velocity: self.player.velocity,
            on_ground: self.player.on_ground,
            sprinting: self.player.sprinting,
            in_water: is_in_water(&self.world, self.player.position),
            health: self.player.health,
            poisoned: self.player.poisoned,
            speed_multiplier: self.player.speed_multiplier,
            jump_multiplier: self.player.jump_multiplier,
            oxygen: self.player.oxygen,
        }];
        for (&id, rp) in host.remote_players.iter() {
            players.push(PlayerSnapshot {
                finances: host.clients.iter().find(|(_,pid)|**pid==id).and_then(|(peer,_)|self.guest_accounts.get(&peer.account_key(&rp.nickname))).map(crate::scripting::InventoryBalances::from_account).unwrap_or_default(),
                resources: host.clients.iter().find(|(_, pid)| **pid == id)
                    .and_then(|(peer, _)| self.guest_accounts.get(&peer.account_key(&rp.nickname)))
                    .map(|a| a.resources).unwrap_or([0; crate::voxel::COLLECTIBLE_BLOCKS.len()]),
                id,
                pos: rp.pos,
                carrying_crystal: rp.carrying_crystal,
                velocity: rp.velocity,
                on_ground: approximate_on_ground(&self.world, rp.pos),
                sprinting: horizontal_speed(rp.velocity) > REMOTE_SPRINT_THRESHOLD,
                in_water: is_in_water(&self.world, rp.pos),
                health: rp.health,
                poisoned: rp.poisoned,
                speed_multiplier: rp.speed_multiplier,
                jump_multiplier: rp.jump_multiplier,
                oxygen: rp.oxygen,
            });
        }
        #[cfg(feature = "dev-playtest")]
        if let Some(agent)=self.playtest_player_snapshot() { players.push(agent); }
        players
    }

    /// Runtime rule pipeline step 1-2: the host submits a prompt (typed into
    /// the in-game console), and it goes to the LLM along with the World
    /// API doc and an example module.
    fn start_generation(&mut self, user_request: String) {
        if !self.can_prompt() { return; }
        if user_request.trim().is_empty() || user_request.len() > crate::rule_sharing::MAX_PROMPT_BYTES {
            self.notify_important("Use a nonempty prompt of at most 2048 bytes.".into());
            return;
        }
        // A cheap deterministic heuristic (see llm::classify_prompt), not a
        // hard requirement -- it just tells the model which contract to
        // write; `poll_generation` validates the generated contract
        // after at most one corrective retry.
        let kind = classify_prompt(&user_request);
        if kind == PromptKind::Rule && self.player.crafting.mana < self.crafting_registry.mana_charge(crate::crafting::RULE_MANA) {
            self.notify_important(format!("Creating a rule requires {} mana. Mana recovers over time; convert elements in Crafting [C] to refill faster.",self.crafting_registry.mana_charge(crate::crafting::RULE_MANA)));
            return;
        }
        let noun = generation_noun(kind);
        log::info!("Generating a {noun} from: {user_request}");
        if matches!(self.net, NetRole::Host(_)) {
            self.notify_all(format!("Host is generating a {noun}: \"{user_request}\""));
        } else {
            self.notify_important(format!("Generating a {noun} locally for host review..."));
        }
        let pending = self.llm.generate(&user_request, kind);
        self.generation = GenerationState::Waiting {
            user_request,
            kind,
            pending,
            is_retry: false,
        };
    }

    fn can_prompt(&self) -> bool {
        match &self.net {
            NetRole::Host(_) => true,
            NetRole::Joined(client) => client.allow_guest_prompting && !client.lost_connection_logged,
        }
    }

    fn poll_proposals(&mut self) {
        let NetRole::Host(host) = &self.net else { return; };
        self.proposal_inbox.retain_peers(|peer| host.clients.contains_key(peer));
        let Some((peer, result)) = self.proposal_inbox.poll() else { return; };
        if !host.clients.contains_key(&peer) { return; }
        let result = if self.ui.settings.values.multiplayer.allow_guest_prompting {
            result
        } else { Err("Guest prompting was disabled before validation finished.".into()) };
        let result = result.and_then(|proposal| {
            let NetRole::Host(host) = &self.net else { return Err("Session ended.".into()); };
            let connected_account = host.clients.get(&peer).and_then(|id| host.remote_players.get(id))
                .map(|player| peer.account_key(&player.nickname));
            if connected_account != crate::rule_sharing::caster_account(&proposal.source) {
                return Err("The submitting player disconnected.".into());
            }
            let name = self.make_rule_name(&proposal.prompt);
            let module = Module::load(name.clone(), proposal.prompt,
                world_api_validate::tag_with_api_version(&proposal.source))?;
            let charge = !module.is_instant;
            let key = connected_account.ok_or("Submitting account unavailable")?;
            let mut charged = self.guest_accounts.get(&key).cloned().ok_or("Submitting account unavailable")?;
            if charge {charged.spend_mana(self.crafting_registry.mana_charge(crate::crafting::RULE_MANA))?;}
            let index = self.scripting.add_generated(module)?;
            if charge {self.guest_accounts.insert(key,charged);self.sync_guest_mana();}
            self.next_rule_id += 1;
            self.last_generated_index = Some(index);
            Ok(format!("'{}' from {} is ready for host review. Nothing has been activated.", name, proposal.nickname))
        });
        let accepted = result.is_ok();
        // Lua error strings are untrusted too; keep feedback within a network packet/UI row.
        let message: String = result.unwrap_or_else(|error| error).chars().take(1024).collect();
        if let NetRole::Host(host) = &mut self.net {
            host.reliable.send(&host.socket, peer, ReliableMsg::RuleProposalResult { accepted, message: message.clone() });
        }
        if accepted { self.notify_all_important(message); } else { self.notify_important(format!("Guest proposal rejected: {message}")); }
    }

    /// Picks a short, meaningful name for a newly generated rule (e.g.
    /// "chickens_flee" instead of "rule_3") by pulling keywords out of the
    /// user's own prompt -- see `llm::derive_rule_name`. Falls back to the
    /// old numbered scheme if the prompt didn't yield anything usable, and
    /// disambiguates against currently loaded rules either way so two
    /// similar prompts don't collide.
    fn make_rule_name(&self, prompt: &str) -> String {
        let base =
            derive_rule_name(prompt).unwrap_or_else(|| format!("rule_{}", self.next_rule_id));
        if !self.scripting.modules.iter().any(|m| m.name == base) {
            return base;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}_{n}");
            if !self.scripting.modules.iter().any(|m| m.name == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Runtime rule pipeline steps 3-4: receives the generated Lua source
    /// and validates it (syntax + `on_tick` contract, via `Module::load`).
    /// On failure, gives the model one chance to fix it before giving up --
    /// still step 4, just with the error fed back instead of a silent drop.
    fn poll_generation(&mut self) {
        let GenerationState::Waiting { pending, .. } = &self.generation else {
            return;
        };
        let Some(result) = pending.poll() else {
            return;
        };
        let GenerationState::Waiting {
            user_request,
            kind,
            is_retry,
            ..
        } = std::mem::replace(&mut self.generation, GenerationState::Idle)
        else {
            unreachable!()
        };
        let noun = generation_noun(kind);

        let code = match result {
            Ok(code) => code,
            Err(e) => {
                log::error!("{noun} generation failed: {e}");
                self.notify_important(format!("{noun} generation failed: {e}"));
                return;
            }
        };

        let name = self.make_rule_name(&user_request);
        // Pre-flight lint (see world_api_validate) before ever handing the
        // source to a real Lua VM -- catches hallucinated/misspelled World
        // API calls and stale block-kind literals with a specific,
        // actionable message, rather than letting them surface as an
        // opaque Module::load or first-tick runtime error.
        let validation_issues = world_api_validate::validate_source(&code);
        let load_result = if validation_issues.is_empty() {
            let tagged_code = world_api_validate::tag_with_api_version(&code);
            Module::load(name.clone(), user_request.clone(), tagged_code)
        } else {
            let joined = validation_issues
                .iter()
                .map(|i| i.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            Err(joined)
        };
        match load_result {
            Ok(module) => {
                // Never silently turn a one-time command into a repeating rule
                // (or vice versa), even if the corrective retry ignores intent.
                let kind_matches = module.is_instant == matches!(kind, PromptKind::Instant);
                if !kind_matches && is_retry {
                    self.notify_important(format!(
                        "{noun} generation failed: the model returned the wrong execution type after retry. Nothing was added."
                    ));
                    return;
                }
                if !kind_matches && !is_retry {
                    log::warn!(
                        "Generated module's contract didn't match the expected kind, asking \
                         the model to redo it"
                    );
                    let hint = match kind {
                        PromptKind::Instant => {
                            "This should have been an instant spell -- define \
                             `function on_cast(api, event)` that does its whole effect once, \
                             not `on_tick`."
                        }
                        PromptKind::Rule => {
                            "This should have been a continuous rule -- define \
                             `function on_tick(api)`, not `on_cast`."
                        }
                    };
                    let pending = self.llm.retry(&user_request, kind, &code, hint);
                    self.generation = GenerationState::Waiting {
                        user_request,
                        kind,
                        pending,
                        is_retry: true,
                    };
                    return;
                }

                if let NetRole::Joined(client) = &mut self.net {
                    if !client.allow_guest_prompting {
                        self.notify_important("The host disabled guest prompting. Nothing was submitted.".into());
                        return;
                    }
                    if let Err(error) = crate::rule_sharing::validate_sizes(&user_request, &code) {
                        self.notify_important(error);
                        return;
                    }
                    client.reliable.send(&client.socket, client.server_addr, ReliableMsg::RuleProposal {
                        prompt: user_request, source: code,
                    });
                    self.proposal_waiting = Some(Instant::now());
                    // Keep a local read-only copy so the author can inspect what was submitted.
                    if let Ok(index) = self.scripting.add_generated(module) { self.last_generated_index = Some(index); }
                    self.notify_important("Proposal sent. The host must review and activate it.".into());
                    return;
                }
                let is_instant = module.is_instant;
                let mut charged = self.player.crafting.clone();
                if !is_instant {
                    if let Err(error) = charged.spend_mana(self.crafting_registry.mana_charge(crate::crafting::RULE_MANA)) {
                        self.notify_important(error);return;
                    }
                }
                self.next_rule_id += 1;
                let idx = match self.scripting.add_generated(module) {
                    Ok(index) => index,
                    Err(error) => {
                        self.notify_important(error);
                        return;
                    }
                };
                self.last_generated_index = Some(idx);
                if !is_instant {self.player.crafting = charged;}
                let (label, action) = if is_instant {
                    ("spell", "click Run to cast it")
                } else {
                    ("rule", "click Enable to activate it")
                };
                log::info!("Generated {label} module '{name}' from prompt (open the Rules panel and {action})");
                self.notify_all_important(format!(
                    "New {label} generated: '{name}' -- open the Rules panel and {action}"
                ));
            }
            Err(err) if !is_retry => {
                log::warn!("Generated {noun} failed validation, asking the model to fix it: {err}");
                let pending = self.llm.retry(&user_request, kind, &code, &err);
                self.generation = GenerationState::Waiting {
                    user_request,
                    kind,
                    pending,
                    is_retry: true,
                };
            }
            Err(err) => {
                log::error!("Generated {noun} failed validation twice, giving up: {err}");
                self.notify_important(format!("{noun} generation failed: {err}"));
            }
        }
    }

    fn apply_block_edit(&mut self, x: i32, y: i32, z: i32, block: BlockType) {
        match &mut self.net {
            NetRole::Host(host) => {
                self.world.set_block(x, y, z, block);
                let addrs: Vec<Peer> = host.clients.keys().copied().collect();
                for addr in addrs {
                    host.reliable.send(
                        &host.socket,
                        addr,
                        ReliableMsg::BlockEdit { x, y, z, block },
                    );
                }
            }
            NetRole::Joined(_) => {} // Only the host publishes block edits.

        }
    }

    /// Applies everything one `ScriptHost::run_tick`/`run_cast` call
    /// produced -- shared between the regular tick loop and `run_instant`
    /// (a spell's Run click) so both go through identical block-edit
    /// replication, crash/broadcast notification, and item-grant handling.
    fn apply_tick_outcome(&mut self, outcome: TickOutcome) {
        for warning in outcome.warnings {
            self.notify_all_important(warning);
        }
        for (x, y, z, block) in outcome.block_edits {
            self.apply_block_edit(x, y, z, block);
        }
        // A rule/spell can pass load-time validation and still hit a
        // runtime error the first time it actually executes (e.g. calling
        // an undefined helper) -- surface that loudly rather than letting
        // it go silently dark.
        for message in outcome.crashes {
            self.notify_all_important(message);
        }
        // api.broadcast: relay through the same host-to-all path every
        // other rule-triggered notification already uses.
        for message in outcome.broadcasts {
            self.notify_all(message);
        }
        for effect in outcome.player_effects {
            self.apply_player_effect(effect);
        }
    }

    /// Applies one `PlayerEffect` to its target's authoritative state --
    /// directly on the host's own `Player` for `HOST_PLAYER_ID`, or on that
    /// connection's `RemotePlayer` record for anyone else (silently
    /// dropped if `player_id` isn't connected, matching the World API
    /// action that produced it already having checked this in Lua and
    /// returned `false`). Health/poisoned/speed/jump reach the *remote*
    /// player's own client purely by riding the next `Snapshot` broadcast
    /// (see world_api/schema.yaml's `replication.player_attributes`) --
    /// only item grants need an explicit targeted message, since inventory
    /// doesn't ride the snapshot.
    fn apply_player_effect(&mut self, effect: PlayerEffect) {
        #[cfg(feature = "dev-playtest")]
        if self.playtest_player_effect(&effect) { return; }
        match effect {
            PlayerEffect::AutomationState{state}=> {self.world.automation=*state;self.automation_timer=1.0;}
            PlayerEffect::Inventory {player_id,balances,resources} => {
                let account = if player_id==HOST_PLAYER_ID {&mut self.player.crafting} else {
                    let NetRole::Host(host)=&self.net else {return;};
                    let Some((peer,_))=host.clients.iter().find(|(_,id)|**id==player_id) else {return;};
                    let Some(player)=host.remote_players.get(&player_id) else {return;};
                    self.guest_accounts.entry(peer.account_key(&player.nickname)).or_default()
                };
                account.resources=resources;account.elements=balances.elements;account.mana=balances.mana;account.gear=balances.items;
                account.revision=account.revision.saturating_add(1);
                self.sync_guest_mana();
            }
            PlayerEffect::GiveItem { player_id, block, amount } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.add_resources(block, amount);
                    return;
                }
                let NetRole::Host(host) = &mut self.net else {
                    return;
                };
                let Some(addr) = host
                    .clients
                    .iter()
                    .find(|(_, &id)| id == player_id)
                    .map(|(&addr, _)| addr)
                else {
                    return;
                };
                if let Some(rp) = host.remote_players.get(&player_id) {
                    let account = self.guest_accounts.entry(addr.account_key(&rp.nickname)).or_default();
                    if let Some(i) = crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b| *b == block) {
                        account.resources[i] = account.resources[i].saturating_add(amount);
                        account.revision += 1;
                    }
                    host.reliable.send(&host.socket, addr, ReliableMsg::CraftState { account: account.clone(), feedback: None });
                }
            }
            PlayerEffect::TakeItem { player_id, block, amount } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.take_resources(block, amount);
                    return;
                }
                let NetRole::Host(host) = &mut self.net else { return; };
                let Some((&peer, _)) = host.clients.iter().find(|(_, id)| **id == player_id) else { return; };
                let Some(rp) = host.remote_players.get(&player_id) else { return; };
                let account = self.guest_accounts.entry(peer.account_key(&rp.nickname)).or_default();
                if let Some(i) = crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|b| *b == block) {
                    if account.resources[i] >= amount {
                        account.resources[i] -= amount;
                        account.revision += 1;
                        host.reliable.send(&host.socket, peer, ReliableMsg::CraftState { account: account.clone(), feedback: None });
                    }
                }
            }
            PlayerEffect::Health { player_id, delta } => {
                if player_id == HOST_PLAYER_ID {
                    if delta >= 0.0 {
                        self.player.heal(delta);
                    } else {
                        self.player.damage(-delta);
                    }
                } else if let NetRole::Host(host) = &mut self.net {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.health = (rp.health + delta).clamp(0.0, MAX_HEALTH);
                    }
                }
            }
            PlayerEffect::Poisoned { player_id, poisoned } => {
                let changed = if player_id == HOST_PLAYER_ID {
                    let was = self.player.poisoned;
                    self.player.poisoned = poisoned;
                    was != poisoned
                } else if let NetRole::Host(host) = &mut self.net {
                    match host.remote_players.get_mut(&player_id) {
                        Some(rp) => {
                            let was = rp.poisoned;
                            rp.poisoned = poisoned;
                            was != poisoned
                        }
                        None => false,
                    }
                } else {
                    false
                };
                // Edge-triggered, not every call -- a rule re-asserting
                // "still poisoned" every tick shouldn't spam a toast.
                if changed {
                    let verb = if poisoned { "poisoned" } else { "no longer poisoned" };
                    self.notify_all(format!("P{player_id} is {verb}"));
                }
            }
            PlayerEffect::SpeedMultiplier { player_id, multiplier } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.set_speed_multiplier(multiplier);
                } else if let NetRole::Host(host) = &mut self.net {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.speed_multiplier =
                            multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
                    }
                }
            }
            PlayerEffect::JumpMultiplier { player_id, multiplier } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.set_jump_multiplier(multiplier);
                } else if let NetRole::Host(host) = &mut self.net {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        rp.jump_multiplier =
                            multiplier.clamp(MIN_ATTRIBUTE_MULTIPLIER, MAX_ATTRIBUTE_MULTIPLIER);
                    }
                }
            }
            PlayerEffect::Teleport { player_id, pos } => {
                if player_id == HOST_PLAYER_ID {
                    self.player.position = pos;
                    // Zeroed so a spell/rule teleport reads as an instant
                    // relocation, not a jump-cut that then keeps carrying
                    // whatever velocity the player had right before it.
                    self.player.velocity = Vec3::ZERO;
                    self.camera.position = pos;
                    return;
                }
                let NetRole::Host(host) = &mut self.net else {
                    return;
                };
                if let Some(rp) = host.remote_players.get_mut(&player_id) {
                    // Snaps the host's own view of them immediately, ahead
                    // of the next PlayerState update the client itself will
                    // send once it applies the same teleport locally.
                    rp.pos = pos;
                }
                let Some(addr) = host
                    .clients
                    .iter()
                    .find(|(_, &id)| id == player_id)
                    .map(|(&addr, _)| addr)
                else {
                    return;
                };
                host.reliable.send(
                    &host.socket,
                    addr,
                    ReliableMsg::Teleport { pos: pos.to_array() },
                );
            }
        }
    }

    /// Damages every currently-poisoned player by `POISON_DAMAGE_PER_TICK`
    /// -- called every `POISON_TICK_INTERVAL` seconds, host-only. Pure
    /// engine state, not a `PlayerEffect`/World API action: no Lua callback
    /// fires for this, matching world_api/schema.yaml's `poison_tick_secs`
    /// note that it's "only observable by polling ... .health from on_tick".
    fn apply_poison_ticks(&mut self) {
        #[cfg(feature = "dev-playtest")]
        if let Some(session)=&mut self.playtest {
            if session.actor.player.poisoned && session.script.finished.is_none() {session.actor.player.damage(POISON_DAMAGE_PER_TICK);session.external_event("Poison damage tick");}
        }
        if self.player.poisoned {
            self.player.damage(POISON_DAMAGE_PER_TICK);
        }
        if let NetRole::Host(host) = &mut self.net {
            for rp in host.remote_players.values_mut() {
                if rp.poisoned {
                    rp.health = (rp.health - POISON_DAMAGE_PER_TICK).max(0.0);
                }
            }
        }
    }

    /// Every frame, host-only: drains oxygen for a submerged player,
    /// regenerates it otherwise, and applies drowning damage once a
    /// player's oxygen has hit 0 -- see `player::OXYGEN_DRAIN_PER_SEC`/
    /// `_REGEN_PER_SEC`/`DROWNING_DAMAGE_PER_SEC`. Continuous rather than a
    /// discrete multi-second timer (unlike `apply_poison_ticks`) so the HUD
    /// bar moves smoothly instead of in visible steps. Pure engine state,
    /// like poison -- no Lua callback fires for any of this.
    fn update_oxygen(&mut self, dt: f32) {
        let host_submerged = is_in_water(&self.world, self.player.position);
        if host_submerged {
            self.player.drain_oxygen(OXYGEN_DRAIN_PER_SEC * dt);
            if self.player.oxygen <= 0.0 {
                self.player.damage(DROWNING_DAMAGE_PER_SEC * dt);
            }
        } else {
            self.player.regenerate_oxygen(OXYGEN_REGEN_PER_SEC * dt);
        }

        let NetRole::Host(host) = &mut self.net else {
            return;
        };
        for rp in host.remote_players.values_mut() {
            let submerged = is_in_water(&self.world, rp.pos);
            if submerged {
                rp.oxygen = (rp.oxygen - OXYGEN_DRAIN_PER_SEC * dt).max(0.0);
                if rp.oxygen <= 0.0 {
                    rp.health = (rp.health - DROWNING_DAMAGE_PER_SEC * dt).max(0.0);
                }
            } else {
                rp.oxygen = (rp.oxygen + OXYGEN_REGEN_PER_SEC * dt).min(MAX_OXYGEN);
            }
        }
    }

    /// Runs one instant spell exactly once -- the Rules panel Run button's
    /// entry point. Host-only, like generating/activating a rule; a no-op
    /// otherwise (mirrors the same restriction `is_host` already gates in
    /// the UI, checked again here since `requests.run_index` is plain user
    /// input the UI layer can't fully trust on its own).
    fn run_instant(&mut self, index: usize) {
        if !matches!(self.net, NetRole::Host(_)) {
            return;
        }
        let Some(module) = self.scripting.modules.get(index) else {
            return;
        };
        if !module.is_instant {
            return;
        }
        if !self.scripting.can_cast_immediately() {
            self.notify_important("Rules are busy. Try casting again shortly; no mana spent.".into());return;
        }
        let name = module.name.clone();
        let caster_account = crate::rule_sharing::caster_account(&module.source);
        let players = self.host_player_positions();
        let caster_id = if let Some(ref account) = caster_account {
            let NetRole::Host(host) = &self.net else { return; };
            let Some(id) = host.clients.iter().find_map(|(peer, id)| host.remote_players.get(id)
                .filter(|p| peer.account_key(&p.nickname) == *account).map(|_| *id)) else {
                self.notify_important("The player who requested this spell is not connected.".into());
                return;
            };
            id
        } else { HOST_PLAYER_ID };
        let Some(caster) = players.iter().find(|player| player.id == caster_id) else { return; };
        let caster_resources = caster.resources;
        let balance = caster_account.as_ref().and_then(|key|self.guest_accounts.get(key)).unwrap_or(&self.player.crafting);
        let mut check = balance.clone();
        if let Err(error) = check.spend_mana(self.crafting_registry.mana_charge(crate::crafting::INSTANT_MANA)) {
            self.notify_important(error);return;
        }
        // Reserve the cast fee before Lua observes or spends the caster's mana.
        let account = if let Some(ref key)=caster_account {self.guest_accounts.get_mut(key).unwrap()} else {&mut self.player.crafting};
        account.mana-=self.crafting_registry.mana_charge(crate::crafting::INSTANT_MANA);
        account.revision=account.revision.saturating_add(1);
        let players=self.host_player_positions();
        let outcome = self.scripting.run_cast(
            index,
            &self.world,
            &mut self.creatures,
            &players,
            &mut self.time_of_day,
            &mut self.weather,
            caster_id,
            caster_resources,
        );
        self.apply_tick_outcome(outcome);
        if self.scripting.cast_succeeded(index, caster_id) {
            self.notify_all(format!("Host requested cast '{name}'"));
        } else {
            let account = if let Some(key)=caster_account {self.guest_accounts.get_mut(&key).unwrap()} else {&mut self.player.crafting};
            account.mana=account.mana.saturating_add(self.crafting_registry.mana_charge(crate::crafting::INSTANT_MANA));
            account.revision=account.revision.saturating_add(1);
        }
        self.sync_guest_mana();
    }

    fn poll_network(&mut self, dt: f32) {
        match &mut self.net {
            NetRole::Host(_) => self.poll_host_network(dt),
            NetRole::Joined(_) => self.poll_client_network(dt),
        }
    }

    fn poll_host_network(&mut self, dt: f32) {
        let mut buf = [0u8; crate::transport::MAX_PACKET_BYTES];
        for _ in 0..256 {
            let (n, from) = {
                let NetRole::Host(host) = &self.net else {
                    unreachable!()
                };
                match host.socket.recv_from(&mut buf) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            };
            let Some(packet) = decode(&buf[..n]) else {
                continue;
            };
            self.handle_host_packet(packet, from);
        }

        let NetRole::Host(host) = &mut self.net else {
            unreachable!()
        };
        host.reliable.resend_due(&host.socket);

        // Drop clients we haven't heard from in a while.
        let stale: Vec<PlayerId> = host
            .remote_players
            .iter()
            .filter(|(_, rp)| rp.last_seen.elapsed() > CONNECTION_TIMEOUT)
            .map(|(&id, _)| id)
            .collect();
        for id in stale {
            host.remote_players.remove(&id);
            let peers: Vec<_> = host.clients.iter().filter(|(_,pid)| **pid==id).map(|(&peer,_)|peer).collect();
            for peer in peers { host.clients.remove(&peer); host.reliable.forget_peer(peer); }
            for &peer in host.clients.keys() { host.reliable.send(&host.socket,peer,ReliableMsg::PlayerLeft { player_id:id }); }
            log::info!("Player {id} timed out.");
        }

        host.broadcast_timer += dt;
        if host.broadcast_timer >= SNAPSHOT_INTERVAL {
            host.broadcast_timer = 0.0;
            let mut players: Vec<SnapshotPlayer> = vec![SnapshotPlayer {
                animation: self.player_animation,
                name:self.local_nickname.clone(),
                appearance: host.appearance,
                id: HOST_PLAYER_ID,
                pos: self.player.position.to_array(),
                yaw: self.camera.yaw,
                carrying_crystal: self.player.carrying_crystal,
                health: self.player.health,
                poisoned: self.player.poisoned,
                speed_multiplier: self.player.speed_multiplier,
                jump_multiplier: self.player.jump_multiplier,
                oxygen: self.player.oxygen,
                held: self.player.crafting.hotbar.entry().filter(|e|e.count(&self.player.crafting)>0),
            }];
            for (&id, rp) in host.remote_players.iter_mut() {
                if let Some(peer)=host.clients.iter().find_map(|(peer,pid)|(*pid==id).then_some(peer)) {
                    if let Some(account)=self.guest_accounts.get(&peer.account_key(&rp.nickname)) {rp.held=account.hotbar.entry().filter(|e|e.count(account)>0);}
                }
                players.push(SnapshotPlayer {
                    animation: rp.animation,
                    name:rp.display_name.clone(),
                    appearance: rp.appearance,
                    id,
                    pos: rp.pos.to_array(),
                    yaw: rp.yaw,
                    carrying_crystal: rp.carrying_crystal,
                    health: rp.health,
                    poisoned: rp.poisoned,
                    speed_multiplier: rp.speed_multiplier,
                    jump_multiplier: rp.jump_multiplier,
                    oxygen: rp.oxygen,
                    held: rp.held,
                });
            }
            let snapshot = UnreliableMsg::Snapshot {
                loot:self.loot.drops.clone(),
                allow_guest_prompting: self.ui.settings.values.multiplayer.allow_guest_prompting,
                time_of_day: self.time_of_day,
                weather: self.weather.current.to_u8(),
                players,
                creatures: self.creatures.snapshot(),
            };
            let bytes = encode(&Packet::Unreliable(snapshot));
            for addr in host.clients.keys() {
                let _ = host.socket.send_to(&bytes, *addr);
            }
        }
    }

    fn handle_host_packet(&mut self, packet: Packet, from: Peer) {
        let NetRole::Host(host) = &mut self.net else {
            unreachable!()
        };
        match packet {
            Packet::Reliable { id, msg } => {
                ReliableChannel::ack_reply(&host.socket, from, id);
                if !host.reliable.mark_seen(from, id) {
                    return;
                }
                match msg {
                    ReliableMsg::RuleProposal { prompt, source } => {
                        let Some(player) = host.clients.get(&from).and_then(|id| host.remote_players.get(id)) else { return; };
                        let result = self.proposal_inbox.submit(from,
                            self.ui.settings.values.multiplayer.allow_guest_prompting,
                            player.nickname.clone(), from.account_key(&player.nickname), prompt, source);
                        if let Err(message) = result {
                            host.reliable.send(&host.socket, from, ReliableMsg::RuleProposalResult { accepted: false, message });
                        }
                    }
                    ReliableMsg::Hello { nickname, protocol } => {
                        if host.clients.contains_key(&from) { return; }
                        if let Some(reason) = net::join_rejection(protocol, host.clients.len()) {
                            host.reliable.send(&host.socket, from, ReliableMsg::JoinRejected(reason.into()));
                            return;
                        }
                        let nickname = sanitize_nickname(&host.socket.peer_name(from).unwrap_or(nickname));
                        if matches!(from, Peer::Direct(_)) && host.remote_players.values().any(|p| p.nickname == nickname) {
                            host.reliable.send(&host.socket, from, ReliableMsg::JoinRejected("Nickname already connected".into()));
                            return;
                        }
                        let edits: Vec<_> = self.world.edits.iter().map(|(k,v)|(*k,*v)).collect();
                        let edit_chunks = edits.len().div_ceil(net::EDITS_PER_CHUNK) as u32;
                        if edit_chunks > net::MAX_WORLD_CHUNKS {
                            host.reliable.send(&host.socket, from, ReliableMsg::JoinRejected("World exceeds the supported multiplayer save size".into()));
                            return;
                        }
                        let player_id = host.next_player_id;
                        host.next_player_id += 1;
                        // Terrain-height-snapped, not just offset from the
                        // host's own Y -- otherwise a new player spawns
                        // floating or buried whenever the ground isn't flat
                        // between the two spawn columns.
                        let spawn_x = self.player.position.x + (player_id as f32) * 2.0;
                        let spawn_z = self.player.position.z + 2.0;
                        let spawn_y = self
                            .world
                            .terrain_height(spawn_x.floor() as i32, spawn_z.floor() as i32)
                            as f32
                            + 2.0;
                        let spawn = Vec3::new(spawn_x, spawn_y, spawn_z);
                        let appearance = remote_player::Appearance::choose(random_world_seed(),
                            std::iter::once(host.appearance).chain(host.remote_players.values().map(|p| p.appearance)));
                        let mut remote = RemotePlayer::new(spawn, 0.0, false, nickname.clone());
                        remote.appearance = appearance;
                        host.clients.insert(from, player_id);
                        host.remote_players.insert(
                            player_id,
                            remote,
                        );
                        for msg in net::welcome_messages(player_id,self.world.seed,self.world.generation.clone(),self.time_of_day,spawn.to_array(),edits)
                            .expect("world size checked before admission") {
                            host.reliable.send(&host.socket,from,msg);
                        }
                        let key = from.account_key(&nickname);
                        // Migrate legacy nickname accounts without letting Direct users claim Steam balances.
                        if matches!(from, Peer::Direct(_)) && !nickname.starts_with("steam:") && !nickname.starts_with("direct:") && !self.guest_accounts.contains_key(&key) {
                            if let Some(old) = self.guest_accounts.remove(&nickname) { self.guest_accounts.insert(key.clone(), old); }
                        }
                        let account = self.guest_accounts.entry(key).or_default().clone();
                        host.reliable.send(&host.socket, from, ReliableMsg::CraftRegistry((*self.crafting_registry).clone()));
                        host.reliable.send(&host.socket, from, ReliableMsg::CraftState { account, feedback: None });
                        log::info!("Player {player_id} ('{nickname}') joined from {from}");
                        self.notify_all(format!("{nickname} joined"));
                    }
                    ReliableMsg::Goodbye => {
                        if let Some(id) = host.clients.remove(&from) {
                            host.remote_players.remove(&id);
                            host.reliable.forget_peer(from);
                            for &addr in host.clients.keys() {
                                host.reliable.send(&host.socket, addr, ReliableMsg::PlayerLeft { player_id: id });
                            }
                        }
                    }
                    ReliableMsg::CraftRequest { revision, action } => {
                        self.handle_crafting_request(from, revision, action);
                    }
                    ReliableMsg::Hotbar(hotbar)=>self.handle_remote_hotbar(from,hotbar),
                    ReliableMsg::ItemAction(intent)=>self.perform_item_action(Some(from),intent),
                    ReliableMsg::AutomationAction(action)=>self.perform_automation(Some(from),action),
                    // BlockEdit is host-to-client only. Never accept claimed destruction.
                    ReliableMsg::BlockEdit { .. } => {}
                    ReliableMsg::ChatMessage(text) => {
                        let sender = host
                            .clients
                            .get(&from)
                            .and_then(|player_id| host.remote_players.get(player_id).map(|rp|(*player_id,rp.display_name.clone())));
                        if let (Some((player_id,nickname)), Some(text)) = (sender, net::chat_text(&text)) {
                            self.broadcast_chat(player_id,nickname,text);
                        }
                    }
                    ReliableMsg::DisplayName(name) => {
                        if let Some(id)=host.clients.get(&from) {if let Some(p)=host.remote_players.get_mut(id) {p.display_name=sanitize_nickname(&name);}}
                    }
                    ReliableMsg::Interact { x, y, z } => {
                        // The host reads the block itself rather than
                        // trusting anything the client claims about it --
                        // the host's world state is authoritative, and a
                        // client's view of it could be stale by the time
                        // this reliable message arrives.
                        if let Some(&player_id) = host.clients.get(&from) {
                            let block = self.world.get_block(x, y, z);
                            self.pending_interacts.push(InteractEvent {
                                x,
                                y,
                                z,
                                block,
                                player_id,
                            });
                        }
                    }
                    _ => {}
                }
            }
            Packet::Ack { id } => {
                if let Some(rp) = host.clients.get(&from).and_then(|id|host.remote_players.get_mut(id)) { rp.last_seen = Instant::now(); }
                host.reliable.ack(id, from);
            },
            Packet::Unreliable(UnreliableMsg::PlayerState {
                animation,
                pos,
                yaw,
                carrying_crystal,
            }) => {
                if let Some(&player_id) = host.clients.get(&from) {
                    if let Some(rp) = host.remote_players.get_mut(&player_id) {
                        let new_pos = Vec3::from_array(pos);
                        let dt = rp.last_seen.elapsed().as_secs_f32().max(0.001);
                        rp.velocity = (new_pos - rp.pos) / dt;
                        rp.pos = new_pos;
                        rp.yaw = yaw;
                        rp.carrying_crystal = carrying_crystal;
                        rp.receive_animation(animation);
                        rp.last_seen = Instant::now();
                    }
                }
            }
            Packet::Unreliable(_) => {}
        }
    }

    fn poll_client_network(&mut self, dt: f32) {
        let mut buf = [0u8; crate::transport::MAX_PACKET_BYTES];
        for _ in 0..256 {
            let (n, from) = {
                let NetRole::Joined(client) = &self.net else {
                    unreachable!()
                };
                match client.socket.recv_from(&mut buf) {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionAborted => {
                        self.return_to_menu = true;
                        break;
                    }
                    Err(_) => break,
                }
            };
            let NetRole::Joined(client) = &self.net else {
                unreachable!()
            };
            if from != client.server_addr {
                continue;
            }
            let Some(packet) = decode(&buf[..n]) else {
                continue;
            };
            self.handle_client_packet(packet);
        }

        let NetRole::Joined(client) = &mut self.net else {
            unreachable!()
        };
        client.reliable.resend_due(&client.socket);
        if !client.lost_connection_logged
            && client.last_server_packet.elapsed() > CONNECTION_TIMEOUT
        {
            log::warn!("Lost connection to host.");
            client.lost_connection_logged = true;
            self.crafting_ui.feedback = "Connection to host lost. Rejoin to recover authoritative inventory.".into();
            self.return_to_menu = true;
        }

        client.send_timer += dt;
        if client.send_timer >= SNAPSHOT_INTERVAL {
            client.send_timer = 0.0;
            let msg = UnreliableMsg::PlayerState {
                animation: self.player_animation,
                pos: self.player.position.to_array(),
                yaw: self.camera.yaw,
                carrying_crystal: self.player.carrying_crystal,
            };
            let bytes = encode(&Packet::Unreliable(msg));
            let _ = client.socket.send_to(&bytes, client.server_addr);
        }
    }

    fn handle_client_packet(&mut self, packet: Packet) {
        let NetRole::Joined(client) = &mut self.net else {
            unreachable!()
        };
        client.last_server_packet = Instant::now();
        client.lost_connection_logged = false;
        match packet {
            Packet::Reliable { id, msg } => {
                ReliableChannel::ack_reply(&client.socket, client.server_addr, id);
                if !client.reliable.mark_seen(client.server_addr, id) {
                    return;
                }
                match msg {
                    ReliableMsg::RuleProposalResult { accepted, message } => {
                        self.proposal_waiting = None;
                        self.notify_important(format!("{}: {message}", if accepted { "Host accepted proposal for review" } else { "Proposal rejected" }));
                    }
                    ReliableMsg::Goodbye => {
                        self.crafting_ui.feedback = "Host ended the session. Return to the menu to join another game.".into();
                        self.toasts.push(Toast::important("Host ended the session"));
                        self.return_to_menu = true;
                        client.lost_connection_logged = true;
                    }

                    ReliableMsg::CraftRegistry(registry) => {
                        if registry.validate().is_ok() { self.crafting_registry = std::sync::Arc::new(registry); }
                    }
                    ReliableMsg::DeathPuffs(positions) => {
                        for pos in positions.into_iter().take(16).map(Vec3::from_array).filter(|p|p.is_finite()) {self.loot.puff(pos);}
                    }
                    ReliableMsg::LootCollected(contents) => {
                        self.audio.play_loot();
                        self.ui.show_pickup(contents);
                    }
                    ReliableMsg::CraftState { account, feedback } => {
                        if account.revision >= self.player.crafting.revision || !self.inventory_ready {
                            let local_hotbar=self.player.crafting.hotbar.clone();
                            let keep_local=self.inventory_ready && local_hotbar.revision>account.hotbar.revision;
                            self.player.crafting = account;
                            if keep_local {self.player.crafting.hotbar=local_hotbar;}
                            self.inventory_ready=true;
                            self.player.carrying_crystal |= self.player.resource_count(BlockType::Crystal)>0;
                        }
                        if let Some(text) = feedback {
                            self.toasts.push(Toast::new(text.clone()));
                            self.crafting_ui.feedback = text;
                            self.crafting_ui.pending = false;
                        }
                    }

                    ReliableMsg::BlockEdit { x, y, z, block } => {
                        self.world.set_block(x, y, z, block);
                    }
                    ReliableMsg::PlayerLeft { player_id } => {
                        let NetRole::Joined(client) = &mut self.net else {
                            unreachable!()
                        };
                        client.remote_players.remove(&player_id);
                    }
                    ReliableMsg::Notify { kind, text } => {
                        let color = match kind {
                            NotifyKind::Info => egui::Color32::WHITE,
                            NotifyKind::Important => IMPORTANT_TOAST_COLOR,
                            NotifyKind::Chat => CHAT_MESSAGE_COLOR,
                        };
                        self.toasts.push(if matches!(kind, NotifyKind::Important) {
                            Toast::important(text.clone())
                        } else {
                            Toast::new(text.clone())
                        });
                        self.log_message(text, color);
                    }
                    ReliableMsg::PlayerChat { player_id, name, text } => {
                        if let Some(text) = net::chat_text(&text) {
                            self.show_player_chat(player_id, &sanitize_nickname(&name), text);
                        }
                    }
                    ReliableMsg::AutomationResult{ok,feedback}=> {
                        self.ui.automation.feedback=feedback;
                        if ok && self.ui.automation.build.is_some_and(|b|b.2.is_some()){self.ui.automation.build=None;}
                    }
                    ReliableMsg::AutomationState(chunk) => {
                        let initial=!self.automation_transfer.has_snapshot();
                        match self.automation_transfer.accept(chunk,&self.crafting_registry) {
                            Ok(Some(state))=>{
                                if initial {self.machine_feedback.synchronize(&state);}
                                self.world.automation=state;
                            },
                            Ok(None)=>(),Err(e)=>log::warn!("Automation sync: {e}"),
                        }
                    }
                    ReliableMsg::GrantItem { block, amount } => {
                        self.player.add_resources(block, amount);
                    }
                    ReliableMsg::Teleport { pos } => {
                        let pos = Vec3::from_array(pos);
                        self.player.position = pos;
                        self.player.velocity = Vec3::ZERO;
                        self.camera.position = pos;
                    }
                    _ => {}
                }
            }
            Packet::Ack { id } => client.reliable.ack(id, client.server_addr),
            Packet::Unreliable(UnreliableMsg::Snapshot {
                loot,
                allow_guest_prompting,
                time_of_day,
                weather,
                players,
                creatures,
            }) => {
                self.time_of_day = time_of_day;
                self.weather.current = Weather::from_u8(weather);
                let NetRole::Joined(client) = &mut self.net else {
                    unreachable!()
                };
                client.creature_snapshot = creatures;
                self.loot.drops=loot.into_iter().take(48).collect();
                client.allow_guest_prompting = allow_guest_prompting;
                let local_player_id = self.local_player_id;
                let now = Instant::now();
                for sp in players {
                    // The host is authoritative for health/poison/movement
                    // attributes -- for *my own* entry, apply them straight
                    // to the local Player that actually simulates my
                    // physics/HUD, not to a RemotePlayer record of myself.
                    if sp.id == local_player_id {
                        self.player.health = sp.health;
                        self.player.poisoned = sp.poisoned;
                        self.player.speed_multiplier = sp.speed_multiplier;
                        self.player.jump_multiplier = sp.jump_multiplier;
                        self.player.oxygen = sp.oxygen;
                        continue;
                    }
                    let NetRole::Joined(client) = &mut self.net else {
                        unreachable!()
                    };
                    client
                        .remote_players
                        .entry(sp.id)
                        .and_modify(|rp| {
                            rp.display_name=sanitize_nickname(&sp.name);
                            rp.velocity = (Vec3::from_array(sp.pos) - rp.pos) / now.duration_since(rp.last_seen).as_secs_f32().max(0.001);
                            rp.appearance = sp.appearance;
                            rp.pos = Vec3::from_array(sp.pos);
                            rp.yaw = sp.yaw;
                            rp.carrying_crystal = sp.carrying_crystal;
                            rp.health = sp.health;
                            rp.poisoned = sp.poisoned;
                            rp.speed_multiplier = sp.speed_multiplier;
                            rp.jump_multiplier = sp.jump_multiplier;
                            rp.oxygen = sp.oxygen;
                            rp.held = sp.held;
                            rp.receive_animation(sp.animation);
                            rp.last_seen = now;
                        })
                        .or_insert_with(|| {
                            // Unused on the client -- only the host formats
                            // chat/join text, so it never asks a client for
                            // another player's nickname.
                            let mut rp = RemotePlayer::new(
                                Vec3::from_array(sp.pos),
                                sp.yaw,
                                sp.carrying_crystal,
                                sanitize_nickname(&sp.name),
                            );
                            rp.appearance = sp.appearance;
                            rp.health = sp.health;
                            rp.poisoned = sp.poisoned;
                            rp.speed_multiplier = sp.speed_multiplier;
                            rp.jump_multiplier = sp.jump_multiplier;
                            rp.oxygen = sp.oxygen;
                            rp.held = sp.held;
                            rp.receive_animation(sp.animation);
                            rp
                        });
                }
            }
            Packet::Unreliable(_) => {}
        }
    }

    fn update_chunks(&mut self) {
        let pcx = (self.player.position.x.floor() as i32).div_euclid(CHUNK_X);
        let pcz = (self.player.position.z.floor() as i32).div_euclid(CHUNK_Z);

        // The authoritative world needs spawn terrain around distant guests too.
        let remote_chunks: Vec<_> = match &self.net {
            NetRole::Host(host) => host.remote_players.values()
                .filter(|p| p.pos.is_finite() && p.pos.abs().max_element() < 1_000_000.0)
                .map(|p| chunk_of(p.pos)).collect(),
            NetRole::Joined(_) => Vec::new(),
        };
        #[cfg(feature = "dev-playtest")]
        { self.ui.agent_nameplate = None; self.playtest_nameplate(); }
        self.ui.nameplates.clear();
        self.ui.chat_bubbles.clear();
        let now = Instant::now();
        self.chat_bubbles.retain(|_,bubble|bubble.opacity(now)>0.0);
        let remote_players=match &self.net {NetRole::Host(host)=>&host.remote_players,NetRole::Joined(client)=>&client.remote_players};
        let eye=self.camera.eye_position();let matrix=self.camera.view_proj();
        let screen=self.window.inner_size();let scale=self.window.scale_factor() as f32;
        for (&id,p) in remote_players {
            let target=p.pos+Vec3::Y*2.15;let distance=eye.distance(target);
            if id==self.local_player_id || distance>48.0 || crate::raycast::raycast(&self.world,eye,target-eye,(distance-0.3).max(0.0)).is_some() {continue;}
            let clip=matrix*target.extend(1.0);if clip.w<=0.0 {continue;}
            let ndc=clip.truncate()/clip.w;if ndc.x.abs()>1.0 || ndc.y.abs()>1.0 || !(0.0..=1.0).contains(&ndc.z) {continue;}
            let screen_pos = egui::pos2((ndc.x+1.0)*0.5*screen.width as f32/scale,(1.0-ndc.y)*0.5*screen.height as f32/scale);
            self.ui.nameplates.push((screen_pos,p.display_name.clone()));
            if let Some(bubble) = self.chat_bubbles.get(&id) {
                self.ui.chat_bubbles.push((screen_pos-egui::vec2(0.0,26.0),bubble.text.clone(),bubble.opacity(now)));
            }
        }
        let mut centers = vec![(pcx,pcz)];
        centers.extend(remote_chunks.iter().copied());
        let mut missing=Vec::new();
        for &(center_x,center_z) in &centers {
        for cx in center_x-RENDER_RADIUS..=center_x+RENDER_RADIUS {for cz in center_z-RENDER_RADIUS..=center_z+RENDER_RADIUS {
            if !self.world.chunks.contains_key(&(cx,cz)){missing.push((cx,cz));}
        }}}
        missing.sort_unstable();
        missing.dedup();
        missing.sort_unstable_by_key(|&(x,z)|(centers.iter().map(|&(cx,cz)|(i64::from(x)-i64::from(cx)).pow(2)+(i64::from(z)-i64::from(cz)).pow(2)).min().unwrap(),x,z));
        let generation_start=Instant::now();
        for (i,(cx,cz)) in missing.into_iter().enumerate() {
            if i>0 && (i>=2 || generation_start.elapsed()>=CHUNK_GENERATION_BUDGET){break;}
            self.world.ensure_chunk_loaded(cx,cz);
        }

        let mut rebuilt = 0usize;
        let mut dirty_keys: Vec<(i32, i32)> = self.world.chunks.iter()
            .filter(|(&(x,z),c)|c.dirty && (x-pcx).abs()<=RENDER_RADIUS && (z-pcz).abs()<=RENDER_RADIUS)
            .map(|(k,_)|*k).collect();
        dirty_keys.sort_unstable_by_key(|&(x,z)|((x-pcx).pow(2)+(z-pcz).pow(2),x,z));
        let mesh_start=Instant::now();
        for key in dirty_keys {
            if rebuilt>0 && (rebuilt>=MESH_BUDGET_PER_FRAME || mesh_start.elapsed()>=CHUNK_MESH_BUDGET){break;}
            let mesh_data = {
                let chunk = self.world.chunks.get(&key).unwrap();
                self.waterfalls.insert(key, crate::water::scan_chunk(&self.world,chunk));
                self.campfires.insert(key, crate::campfire::positions(chunk));
                build_chunk_mesh(&self.world, chunk)
            };
            match upload_mesh(&self.device, &mesh_data) {
                Some(gpu_mesh) => {
                    self.chunk_meshes.insert(key, gpu_mesh);
                }
                None => {
                    self.chunk_meshes.remove(&key);
                }
            }
            if let Some(chunk) = self.world.chunks.get_mut(&key) {
                chunk.dirty = false;
            }
            rebuilt += 1;
        }
        let _ = rebuilt;

        let unload: Vec<(i32, i32)> = self
            .world
            .chunks
            .keys()
            .filter(|(cx, cz)| ((cx - pcx).abs() > UNLOAD_RADIUS || (cz - pcz).abs() > UNLOAD_RADIUS)
                && !remote_chunks.iter().any(|(rx, rz)| (cx - rx).abs() <= UNLOAD_RADIUS && (cz - rz).abs() <= UNLOAD_RADIUS))
            .copied()
            .collect();
        for key in unload {
            self.world.unload_chunk(key.0, key.1);
            self.chunk_meshes.remove(&key);
            self.waterfalls.remove(&key);
            self.campfires.remove(&key);
        }
    }

    /// Rebuilds the rain streak vertex buffer for this frame. Each streak is
    /// a short vertical line whose y wraps within `RAIN_HEIGHT`, driven by
    /// the already free-running `water_time` clock -- so it never needs its
    /// own per-particle state, just a fixed (x, z, phase) offset from the
    /// camera picked once at load in `build_rain_particles`.
    fn write_rain_vertices(&self, cam_pos: Vec3, raining: bool, falls: &[crate::water::Waterfall]) -> u32 {
        let half_h = RAIN_HEIGHT * 0.5;
        let mut verts: Vec<RainVertex> = Vec::with_capacity(self.rain_particles.len() * 2);
        for &(ox, oz, phase) in self.rain_particles.iter().take(if raining {self.rain_particles.len()} else {0}) {
            let y = (phase - self.water_time * RAIN_FALL_SPEED).rem_euclid(RAIN_HEIGHT) - half_h;
            // Fade out near the top/bottom of the volume so a streak
            // doesn't visibly pop in/out of existence as it wraps.
            let alpha = RAIN_ALPHA * (half_h - y.abs()).clamp(0.0, 1.0);
            let x = cam_pos.x + ox;
            let z = cam_pos.z + oz;
            verts.push(RainVertex {
                position: [x, cam_pos.y + y + RAIN_STREAK_LENGTH * 0.5, z],
                alpha,
            });
            verts.push(RainVertex {
                position: [x, cam_pos.y + y - RAIN_STREAK_LENGTH * 0.5, z],
                alpha,
            });
        }
        for fall in falls.iter().take(crate::water::MAX_VISIBLE_FALLS) {
            let fade = ((64.0-fall.sound_position().distance(cam_pos))/16.0).clamp(0.0,1.0);
            for (p,q,alpha) in fall.lines(self.water_time) {
                verts.push(RainVertex {position:p.to_array(),alpha:alpha*fade});
                verts.push(RainVertex {position:q.to_array(),alpha:alpha*fade});
            }
        }
        self.queue
            .write_buffer(&self.rain_vertex_buffer, 0, bytemuck::cast_slice(&verts));
        verts.len() as u32
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let lighting = sky_lighting(self.time_of_day);
        let sky = lighting.sky_color;
        let zenith = lighting.zenith_color;
        let clouds = self.surface_weather.clouds;
        let cam_pos = self.camera.eye_position();
        let underwater = is_in_water(&self.world, cam_pos);
        let light_view_proj = light_view_proj(lighting.sun_dir, self.player.position);
        let view_proj = self.camera.view_proj();
        let camera_frustum=crate::visibility::Frustum::new(view_proj);
        let sun_frustum=crate::visibility::Frustum::new(light_view_proj);
        let (render_cx,render_cz)=chunk_of(self.player.position);
        let fog_color = if self.weather.current == Weather::Mist {
            [
                sky[0] * (1.0 - MIST_FOG_BLEND) + MIST_FOG_TINT[0] * MIST_FOG_BLEND,
                sky[1] * (1.0 - MIST_FOG_BLEND) + MIST_FOG_TINT[1] * MIST_FOG_BLEND,
                sky[2] * (1.0 - MIST_FOG_BLEND) + MIST_FOG_TINT[2] * MIST_FOG_BLEND,
            ]
        } else {
            [sky[0], sky[1], sky[2]]
        };
        let mut campfires:Vec<_>=self.campfires.values().flatten().copied()
            .filter(|p|p.distance_squared(cam_pos)<48.0*48.0).collect();
        campfires.sort_by(|a,b|a.distance_squared(cam_pos).total_cmp(&b.distance_squared(cam_pos)));
        campfires.truncate(8);
        self.audio.update_campfires(&campfires);
        self.audio.update_auras(&self.world.automation);
        let fire_effects=crate::campfire::effects(&campfires,cam_pos,self.water_time);
        self.campfire_mesh.update(&self.device,&self.queue,&fire_effects);
        let uniform = CameraUniform {
            camp_lights: self.machine_feedback.lights(&campfires,cam_pos),
            view_proj: view_proj.to_cols_array_2d(),
            light_view_proj: light_view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            fog_color: [fog_color[0], fog_color[1], fog_color[2], 1.0],
            zenith_color: [zenith[0], zenith[1], zenith[2], 1.0],
            sun_dir: [
                lighting.sun_dir.x,
                lighting.sun_dir.y,
                lighting.sun_dir.z,
                lighting.sun_height,
            ],
            light_params: [
                lighting.ambient * (1.0 - clouds * 0.12),
                lighting.sun_intensity * (1.0 - clouds * 0.72),
                self.water_time,
                self.weather.current.wind_strength(),
            ],
            weather_fx: [
                self.lightning_flash,
                clouds,
                if underwater {1.0} else {0.0},
                self.surface_weather.wetness,
            ],
        };
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
        self.queue.write_buffer(
            &self.shadow_light_buffer,
            0,
            bytemuck::bytes_of(&LightUniform { view_proj: light_view_proj.to_cols_array_2d() }),
        );

        let target = if self.cursor_grabbed && !self.ui.automation.tools_suspended() && self.ui.settings.values.gameplay.show_block_target {
            raycast(&self.world, cam_pos, self.camera.forward(), REACH).map(|hit| {
                let breakable = crate::equipment::can_mine(self.player.crafting.hotbar.entry(),self.world.get_block(hit.target.0,hit.target.1,hit.target.2),&self.player.crafting);
                let mut account=self.player.crafting.clone();
                let players=match &self.net {
                    NetRole::Host(host)=>host.remote_players.values().map(|p|p.pos).collect::<Vec<_>>(),
                    NetRole::Joined(client)=>client.remote_players.values().map(|p|p.pos).collect::<Vec<_>>(),
                };
                let intent=crate::equipment::Intent{hotbar:account.hotbar.clone(),item:account.hotbar.entry(),target:Some(hit.target),direction:self.camera.forward().to_array(),action:crate::equipment::Action::Place};
                let placeable=matches!(crate::equipment::block_action(&self.world,&mut account,&mut crate::equipment::Mining::default(),self.player.position,&players,&intent),Ok(Some(_)));
                (hit, breakable, placeable)
            })
        } else { None };
        self.block_target.update(&self.queue, target);

        self.wind.prepare(&self.queue,cam_pos,self.camera.right(),self.camera.right().cross(self.camera.forward()),&self.world,underwater,self.weather.current);
        let raining = self.weather.current.has_rain_particles() && !underwater;
        let mut falls: Vec<_> = self.waterfalls.values().flatten().copied()
            .filter(|f|f.sound_position().distance_squared(cam_pos)<64.0*64.0).collect();
        falls.sort_by(|a,b|a.sound_position().distance_squared(cam_pos).total_cmp(&b.sound_position().distance_squared(cam_pos)));
        falls.truncate(crate::water::MAX_VISIBLE_FALLS);
        self.audio.update_waterfalls(&falls);
        let rain_vertex_count = self.write_rain_vertices(cam_pos,raining,&falls);
        let bird_vertex_count = if underwater { 0 } else if let Some(flock) = &self.bird_flock {
            let count = (flock.birds.len() * BIRD_VERTICES_PER_BIRD) as u32;
            self.write_bird_vertices();
            count
        } else {
            0
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("encoder"),
            });

        // Skip the shadow pass entirely when the sun is below the horizon
        // -- the main shader already zeroes sun_intensity then, so shadows
        // wouldn't be visible anyway.
        if lighting.sun_intensity > 0.0 {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            shadow_pass.set_pipeline(&self.shadow_pipeline);
            shadow_pass.set_bind_group(0, &self.shadow_light_bind_group, &[]);
            for (&(cx,cz),mesh) in &self.chunk_meshes {
                if !sun_frustum.chunk(cx,cz){continue;}
                shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            if self.entity_mesh.index_count > 0 {
                let mesh = &self.entity_mesh;
                shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                shadow_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: sky[0] as f64,
                            g: sky[1] as f64,
                            b: sky[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            // Sky background first -- a single fullscreen triangle with no
            // depth test, so every draw after this one simply paints over
            // it wherever real geometry exists. See sky.wgsl.
            rpass.set_pipeline(&self.sky_pipeline);
            rpass.set_bind_group(0, &self.camera_bind_group, &[]);
            rpass.draw(0..3, 0..1);

            rpass.set_pipeline(&self.render_pipeline);
            rpass.set_bind_group(0, &self.camera_bind_group, &[]);
            rpass.set_bind_group(1, &self.texture_bind_group, &[]);
            rpass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
            for (&(cx,cz),mesh) in &self.chunk_meshes {
                if !crate::visibility::within_terrain_range((cx,cz),(render_cx,render_cz),RENDER_RADIUS) || !camera_frustum.chunk(cx,cz){continue;}
                rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            if self.entity_mesh.index_count > 0 {
                let mesh = &self.entity_mesh;
                rpass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                rpass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            // Fire/smoke use the main material pipeline but never cast sun shadows.
            if self.campfire_mesh.index_count > 0 {
                let mesh=&self.campfire_mesh;
                rpass.set_vertex_buffer(0,mesh.vertex_buffer.slice(..));
                rpass.set_index_buffer(mesh.index_buffer.slice(..),wgpu::IndexFormat::Uint32);
                rpass.draw_indexed(0..mesh.index_count,0,0..1);
            }
            if rain_vertex_count > 0 {
                rpass.set_pipeline(&self.rain_pipeline);
                rpass.set_bind_group(0, &self.camera_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.rain_vertex_buffer.slice(..));
                rpass.draw(0..rain_vertex_count, 0..1);
            }
            if bird_vertex_count > 0 {
                rpass.set_pipeline(&self.bird_pipeline);
                rpass.set_bind_group(0, &self.camera_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.bird_vertex_buffer.slice(..));
                rpass.draw(0..bird_vertex_count, 0..1);
            }
            self.wind.draw(&mut rpass,&self.camera_bind_group);
            self.block_target.draw(&mut rpass, &self.camera_bind_group);
        }

        if self.cursor_grabbed {
            // Separate depth so the local view model cannot clip through nearby terrain.
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("held item pass"),
                color_attachments:&[Some(wgpu::RenderPassColorAttachment {view:&view,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment{view:&self.depth_view,depth_ops:Some(wgpu::Operations{load:wgpu::LoadOp::Clear(1.0),store:wgpu::StoreOp::Store}),stencil_ops:None}),
                occlusion_query_set:None,timestamp_writes:None,
            });
            pass.set_pipeline(&self.render_pipeline);pass.set_bind_group(0,&self.camera_bind_group,&[]);pass.set_bind_group(1,&self.texture_bind_group,&[]);pass.set_bind_group(2,&self.shadow_sample_bind_group,&[]);
            pass.set_vertex_buffer(0,self.held_mesh.vertex_buffer.slice(..));pass.set_index_buffer(self.held_mesh.index_buffer.slice(..),wgpu::IndexFormat::Uint32);pass.draw_indexed(0..self.held_mesh.index_count,0,0..1);
        }

        if let Some(full_output) = self.pending_egui_output.take() {
            self.ui.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &view,
                &self.window,
                full_output,
                [self.config.width, self.config.height],
            );
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }

    pub fn save(&self) {
        let goodbye = encode(&Packet::Reliable { id: u64::MAX, msg: ReliableMsg::Goodbye });
        match &self.net {
            NetRole::Host(host) => for &peer in host.clients.keys() { let _ = host.socket.send_to(&goodbye,peer); },
            NetRole::Joined(client) => { let _ = client.socket.send_to(&goodbye,client.server_addr); }
        }
        if matches!(self.net, NetRole::Host(_)) {
            if let Err(error) = save_world(
                &self.world,
                &self.player,
                &self.camera,
                self.time_of_day,
                self.scripting.save_entries(),
                &self.crafting_save(),
            ) { log::error!("{error}"); }
        }
    }

    /// Whether the player confirmed the Esc quit dialog -- `main.rs` checks
    /// this after every frame and, if set, drops this game and returns to
    /// the main menu on the same window.
    pub fn wants_return_to_menu(&self) -> bool {
        self.return_to_menu
    }
}

/// Blocks (with a resend-until-acked handshake) until the host answers with
/// a `Welcome`, or gives up with an error after a timeout. Never exits the
/// process itself -- the caller decides how to handle a failed join (the
/// main menu shows it and lets the user retry; a CLI `--connect` launch
/// prints it and exits).
type JoinedSession = (Transport, Peer, PlayerId, World, Vec3, f32, ReliableChannel);

fn join_handshake(
    target: JoinTarget,
    nickname: &str,
) -> Result<JoinedSession, String> {
    let settings = crate::settings::Settings::load(std::path::Path::new("settings.json")).unwrap_or_default();
    let (socket, server_addr) = Transport::join(target, settings.multiplayer.steam_app_id)?;
    let mut reliable = ReliableChannel::new();
    let hello_id = reliable.send(
        &socket,
        server_addr,
        ReliableMsg::Hello {
            nickname: nickname.to_string(),
            protocol: net::PROTOCOL_VERSION,
        },
    );
    log::info!("Connecting to {server_addr}...");

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut transfer = net::WorldTransfer::default();
    let mut buf = [0u8; crate::transport::MAX_PACKET_BYTES];
    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "Failed to connect to {server_addr}: no response from host. \
                 Is it running, and is the address/port correct?"
            ));
        }
        reliable.resend_due(&socket);
        match socket.recv_from(&mut buf) {
            Ok((n, from)) if from == server_addr => {
                if let Some(Packet::Reliable { id, msg: ReliableMsg::JoinRejected(reason) }) = decode(&buf[..n]) {
                    ReliableChannel::ack_reply(&socket, server_addr, id);
                    return Err(reason);
                }
                if let Some(Packet::Reliable { id, msg }) = decode(&buf[..n]) {
                    if !matches!(msg, ReliableMsg::Welcome {..} | ReliableMsg::WorldEditsChunk {..}) { continue; }
                    ReliableChannel::ack_reply(&socket, server_addr, id);
                    if !reliable.mark_seen(server_addr,id) { continue; }
                    if let Some(initial) = transfer.accept(msg)? {
                        reliable.forget(hello_id);
                        let mut world = World::new(initial.seed);
                        world.generation = initial.generation;
                        world.edits.extend(initial.edits);
                        world.rebuild_redstone_positions();
                        return Ok((socket,server_addr,initial.player_id,world,Vec3::from_array(initial.spawn),initial.time_of_day,reliable));
                    }
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                return Err(format!("Network error while connecting: {e}"));
            }
        }
    }
}

/// Side length (in pixels) every layer of `creature_texture` is resized to
/// -- matches every bundled skinned model's actual embedded texture (see
/// `model.rs`), but resizing defensively means a future model with a
/// differently-sized texture still loads instead of panicking.
const CREATURE_TEXTURE_SIZE: u32 = 256;

/// Decodes the embedded texture atlas (see `voxel::atlas`) plus every
/// skinned creature model's own real texture (see `model.rs`'s
/// `Models::creature_texture_layers`), uploading both, and returns the bind
/// group layout (needed once, for the pipeline) and the bind group itself
/// (bound every frame). Nearest filtering keeps the pixel-art look sharp
/// instead of blurring it like a photo texture would.
fn create_atlas_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    models: &Models,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let image = image::load_from_memory(ATLAS_BYTES)
        .expect("embedded atlas.png should decode")
        .to_rgba8();
    let (width, height) = image.dimensions();
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("atlas sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    // One layer per `CreatureKind::to_u8()` slot -- Sheep/Chicken have no
    // real texture (see `model.rs`'s rigid-vs-skinned doc comment) and get
    // a blank white layer that `push_model` never actually indexes into,
    // so every kind can still be laid out at its own fixed array index.
    let layers = models.creature_texture_layers();
    let layer_count = layers.len() as u32;
    let creature_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("creature texture array"),
        size: wgpu::Extent3d {
            width: CREATURE_TEXTURE_SIZE,
            height: CREATURE_TEXTURE_SIZE,
            depth_or_array_layers: layer_count,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let blank_layer = vec![255u8; (CREATURE_TEXTURE_SIZE * CREATURE_TEXTURE_SIZE * 4) as usize];
    for (layer_idx, layer_image) in layers.iter().enumerate() {
        let pixels: std::borrow::Cow<[u8]> = match layer_image {
            Some(img) if img.dimensions() == (CREATURE_TEXTURE_SIZE, CREATURE_TEXTURE_SIZE) => {
                Cow::Borrowed(img.as_raw())
            }
            Some(img) => Cow::Owned(
                image::imageops::resize(
                    *img,
                    CREATURE_TEXTURE_SIZE,
                    CREATURE_TEXTURE_SIZE,
                    image::imageops::FilterType::Nearest,
                )
                .into_raw(),
            ),
            None => Cow::Borrowed(&blank_layer),
        };
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &creature_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer_idx as u32 },
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * CREATURE_TEXTURE_SIZE),
                rows_per_image: Some(CREATURE_TEXTURE_SIZE),
            },
            wgpu::Extent3d { width: CREATURE_TEXTURE_SIZE, height: CREATURE_TEXTURE_SIZE, depth_or_array_layers: 1 },
        );
    }
    let creature_view = creature_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("atlas bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atlas bind group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&creature_view),
            },
        ],
    });

    (layout, bind_group)
}

struct ShadowResources {
    pipeline: wgpu::RenderPipeline,
    view: wgpu::TextureView,
    light_buffer: wgpu::Buffer,
    light_bind_group: wgpu::BindGroup,
    /// Bind group layout for sampling the shadow map from the main
    /// pipeline (its group 2) -- exposed so `App::new` can include it when
    /// building that pipeline's layout.
    sample_bgl: wgpu::BindGroupLayout,
    sample_bind_group: wgpu::BindGroup,
}

/// Sets up everything needed for a simple single-cascade directional
/// shadow map: a depth texture rendered from the sun's point of view each
/// frame, and the resources the main pass needs to sample it back.
fn create_shadow_resources(device: &wgpu::Device) -> ShadowResources {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow map"),
        size: wgpu::Extent3d { width: SHADOW_MAP_SIZE, height: SHADOW_MAP_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let light_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shadow light buffer"),
        size: std::mem::size_of::<LightUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow light bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow light bind group"),
        layout: &light_bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: light_buffer.as_entire_binding() }],
    });

    let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
    });
    let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow pipeline layout"),
        bind_group_layouts: &[&light_bgl],
        push_constant_ranges: &[],
    });
    // Only position is read; reusing the main Vertex buffer's stride means
    // no separate shadow-only mesh data is needed.
    let position_only_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 }],
    };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow pipeline"),
        layout: Some(&shadow_pipeline_layout),
        vertex: wgpu::VertexState { module: &shadow_shader, entry_point: "vs_main", buffers: &[position_only_layout] },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Cw,
            // No culling: the mesher only emits *exposed* faces (no
            // interior/underside geometry), so unlike a closed mesh there's
            // often no "back face" at a similar depth to fall back on --
            // culling either side can leave gaps with nothing to write
            // shadow depth. Acne is handled by the shader-side bias instead.
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let sample_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow sample bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    });
    let sample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow sample bind group"),
        layout: &sample_bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
        ],
    });

    ShadowResources { pipeline, view, light_buffer, light_bind_group, sample_bgl, sample_bind_group }
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth texture"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

#[allow(dead_code)]
fn chunk_of(pos: Vec3) -> (i32, i32) {
    world_to_chunk(pos.x.floor() as i32, pos.z.floor() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_nickname_trims_surrounding_whitespace() {
        assert_eq!(sanitize_nickname("  Steve  "), "Steve");
    }

    #[test]
    fn sanitize_nickname_falls_back_to_a_default_when_empty_or_blank() {
        assert_eq!(sanitize_nickname(""), "Player");
        assert_eq!(sanitize_nickname("   "), "Player");
    }

    #[test]
    fn sanitize_nickname_caps_overly_long_input() {
        let long_name = "a".repeat(50);
        let result = sanitize_nickname(&long_name);
        assert_eq!(result.chars().count(), MAX_NICKNAME_LEN);
        assert_eq!(result, "a".repeat(MAX_NICKNAME_LEN));
    }

    #[test]
    fn sanitize_nickname_passes_through_a_normal_name_unchanged() {
        assert_eq!(sanitize_nickname("Alex"), "Alex");
    }

    /// Regression test for the bug this replaced: seeding from
    /// `Instant::now().elapsed()` measured almost nothing (the tiny gap
    /// between two back-to-back calls) instead of real wall-clock time, so
    /// consecutive "New World" launches got the same seed -- and since
    /// terrain/creatures/weather all derive from it, every world looked the
    /// same. A handful of calls spread over a few milliseconds should not
    /// all collide.
    #[test]
    fn random_world_seed_varies_across_calls() {
        let mut seeds = Vec::new();
        for _ in 0..20 {
            seeds.push(random_world_seed());
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        let first = seeds[0];
        assert!(
            seeds.iter().any(|&s| s != first),
            "expected at least one different seed across 20 calls, got {seeds:?}"
        );
    }
}

#[cfg(test)]
mod join_tests {
    use super::*;
    #[test]
    fn actual_join_handshake_loads_chunked_saved_world() {
        let host = Transport::direct("127.0.0.1:0").unwrap();
        let Peer::Direct(address) = host.local_peer() else { unreachable!() };
        let server = std::thread::spawn(move || {
            let mut reliable = ReliableChannel::new();
            let mut admitted = false;
            let mut bytes = [0; crate::transport::MAX_PACKET_BYTES];
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok((n, peer)) = host.recv_from(&mut bytes) {
                    match decode(&bytes[..n]) {
                        Some(Packet::Reliable { id, msg: ReliableMsg::Hello { protocol, .. } }) => {
                            assert_eq!(protocol,net::PROTOCOL_VERSION);
                            ReliableChannel::ack_reply(&host,peer,id);
                            if !admitted {
                                admitted = true;
                                let edits = (0..5000).map(|x| ((x,40,1), BlockType::Stone)).collect();
                                for msg in net::welcome_messages(1,123,Default::default(),0.7,[2.,70.,2.],edits).unwrap() {
                                    reliable.send(&host,peer,msg);
                                }
                            }
                        }
                        Some(Packet::Ack { id }) => reliable.ack(id,peer),
                        Some(Packet::Reliable { msg: ReliableMsg::Goodbye, .. }) => return,
                        _ => {}
                    }
                }
                reliable.resend_due(&host);
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("join did not finish");
        });
        let (socket,peer,id,mut world,spawn,time,_) = join_handshake(JoinTarget::Direct(address),"Test").unwrap();
        assert_eq!(id,1);
        assert_eq!(world.seed,123);
        assert_eq!(world.edits.len(),5000);
        let (cx,cz)=world_to_chunk(4999,1);
        world.ensure_chunk_loaded(cx,cz);
        assert_eq!(world.get_block(4999,40,1),BlockType::Stone);
        assert_eq!(spawn,Vec3::new(2.,70.,2.));
        assert_eq!(time,0.7);
        socket.send_to(&encode(&Packet::Reliable {id:999,msg:ReliableMsg::Goodbye}),peer).unwrap();
        server.join().unwrap();
    }
}

#[cfg(test)]
mod underwater_tests {
    use super::*;
    #[test]
    fn underwater_view_uses_eyes_and_clears_above_surface() {
        let mut world=World::new(42);
        let mut chunk=crate::voxel::chunk::Chunk::new(0,0);
        chunk.set_local(2,18,2,BlockType::Water);
        chunk.set_local(2,17,2,BlockType::Water);
        world.chunks.insert((0,0),chunk);
        let mut camera=Camera::new(Vec3::new(2.5,17.0,2.5),1.0);
        assert!(is_in_water(&world,camera.eye_position()));
        camera.position.y=17.5;
        assert!(is_in_water(&world,camera.position));
        assert!(!is_in_water(&world,camera.eye_position()),"wading must not tint the view");
        camera.position.x=3.5;
        assert!(!is_in_water(&world,camera.eye_position()));
    }
    #[test]
    #[ignore = "requires a GPU adapter; validates terrain and underwater sky shaders"]
    fn validate_underwater_shaders() {
        let instance=wgpu::Instance::default();
        let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        let (device,_)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default(),None)).unwrap();
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        for source in [include_str!("shader.wgsl"),include_str!("sky.wgsl")] {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("underwater validation"),source:wgpu::ShaderSource::Wgsl(source.into())});
        }
        assert!(pollster::block_on(device.pop_error_scope()).is_none());
    }
}
