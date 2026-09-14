use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::block::BlockType;
use super::chunk::{world_to_chunk, world_to_local, Chunk, CHUNK_X, CHUNK_Y, CHUNK_Z};
use super::noise::{block_rand, block_rand_range, column_rand};

pub const SEA_LEVEL: i32 = 18;

/// 6-connected neighbor offsets, used by `World::flood_from`.
const NEIGHBOR_OFFSETS: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

/// Hard cap on how many blocks a single dig can flood in one go. Not real
/// fluid dynamics -- just enough that tunneling into a lake fills the hole
/// you made instead of leaving it as a dry pocket, without a single unlucky
/// break being able to drain an entire lake or hang the game on a huge cave.
const MAX_FLOOD_BLOCKS: usize = 48;

/// Bottom-most layers of every chunk are unbreakable bedrock (hardness 0),
/// regardless of terrain height, so the world always has a floor.
const BEDROCK_DEPTH: i32 = 2;

/// Wood/leaves pairs for the four tree species in `textures/blocks.csv`;
/// `place_tree` picks one per tree via `column_rand`.
const TREE_SPECIES: [(BlockType, BlockType); 4] = [
    (BlockType::OakWood, BlockType::OakLeaves),
    (BlockType::SpruceWood, BlockType::SpruceLeaves),
    (BlockType::BirchWood, BlockType::BirchLeaves),
    (BlockType::CherryWood, BlockType::CherryLeaves),
];

/// One kind of underground deposit grown by `World::scatter_veins`: a small
/// cluster of `block` replacing `Stone`, seeded by a per-chunk-attempt roll.
struct VeinConfig {
    block: BlockType,
    salt: u32,
    attempts_per_chunk: u32,
    spawn_chance: f32,
    min_y: i32,
    max_y: i32,
    min_size: i32,
    max_size: i32,
}

/// Ore/stone-variant veins, roughly ordered shallow-and-common to
/// deep-and-rare like the CSV's hardness column suggests. Basalt and
/// cobblestone aren't ores but generate the same way -- small underground
/// pockets replacing plain stone.
const VEINS: [VeinConfig; 6] = [
    VeinConfig {
        block: BlockType::CopperOre,
        salt: 0xC099_7E01,
        attempts_per_chunk: 6,
        spawn_chance: 0.55,
        min_y: 4,
        max_y: 34,
        min_size: 3,
        max_size: 6,
    },
    VeinConfig {
        block: BlockType::Cobblestone,
        salt: 0xC0BB_1E02,
        attempts_per_chunk: 5,
        spawn_chance: 0.4,
        min_y: 2,
        max_y: 30,
        min_size: 3,
        max_size: 6,
    },
    VeinConfig {
        block: BlockType::GoldOre,
        salt: 0x6014_D003,
        attempts_per_chunk: 3,
        spawn_chance: 0.4,
        min_y: 3,
        max_y: 22,
        min_size: 2,
        max_size: 4,
    },
    VeinConfig {
        block: BlockType::Basalt,
        salt: 0xBA5A_1704,
        attempts_per_chunk: 4,
        spawn_chance: 0.35,
        min_y: 2,
        max_y: 20,
        min_size: 4,
        max_size: 7,
    },
    VeinConfig {
        block: BlockType::EmeraldOre,
        salt: 0xE3E7_A105,
        attempts_per_chunk: 2,
        spawn_chance: 0.18,
        min_y: 3,
        max_y: 16,
        min_size: 2,
        max_size: 3,
    },
    VeinConfig {
        block: BlockType::DiamondOre,
        salt: 0xD1A5_0906,
        attempts_per_chunk: 2,
        spawn_chance: 0.22,
        min_y: 2,
        max_y: 10,
        min_size: 2,
        max_size: 3,
    },
];

const RESOURCE_VEINS: &[VeinConfig] = include!("resource_veins.rs");

struct SurfaceResource { block: BlockType, habitat: &'static str, chance: f32, salt: u32 }
const SURFACE_RESOURCES: &[SurfaceResource] = include!("resource_surface.rs");

pub struct World {
    pub starter_camp: Option<(i32, i32, i32)>,
    pub(crate) cave_layouts: std::cell::RefCell<HashMap<(i32,i32),std::sync::Arc<crate::underground::Layout>>>,
    pub name: String,
    pub underground_discovered: std::collections::BTreeSet<(i32,i32)>,
    pub automation: crate::automation::State,
    pub generation: crate::worldgen::WorldGeneration,
    pub seed: u32,
    pub chunks: HashMap<(i32, i32), Chunk>,
    /// Blocks that differ from the procedurally generated terrain. Kept
    /// separately so the whole world doesn't need to be serialized.
    pub edits: HashMap<(i32, i32, i32), BlockType>,
    /// Positions of every RedStone block currently placed, maintained
    /// incrementally by `set_block` so the passive "heal nearby creatures"
    /// effect doesn't need to rescan the world every tick.
    pub redstone_positions: Vec<(i32, i32, i32)>,
}

impl World {
    pub fn new(seed: u32) -> Self {
        Self {
            starter_camp: None,
            seed,
            cave_layouts: Default::default(),            automation: Default::default(),
            generation: Default::default(),
            name: "world".into(),
            underground_discovered: Default::default(),
            chunks: HashMap::new(),
            edits: HashMap::new(),
            redstone_positions: Vec::new(),
        }
    }

    /// Rebuilds `redstone_positions` from `edits` -- call once after bulk
    /// loading edits (e.g. from a save file), since those bypass
    /// `set_block`'s incremental tracking.
    pub fn rebuild_redstone_positions(&mut self) {
        self.redstone_positions = self
            .edits
            .iter()
            .filter(|(_, &b)| b == BlockType::RedStone)
            .map(|(&pos, _)| pos)
            .collect();
    }

    pub fn terrain_height(&self, wx: i32, wz: i32) -> i32 {
        self.generation.height(wx,wz,self.seed)
    }

    fn generate_chunk(&self, cx: i32, cz: i32) -> Chunk {
        let mut chunk = Chunk::new(cx, cz);
        let (ox, oz) = chunk.world_origin();

        // Reuse the seed-only height map for all generation passes. The one-cell
        // halo supplies exact mountain slopes at chunk boundaries.
        let halo:[[i32;(CHUNK_Z+2) as usize];(CHUNK_X+2) as usize]=std::array::from_fn(|x|std::array::from_fn(|z|self.terrain_height(ox+x as i32-1,oz+z as i32-1)));
        let heights:[[i32;CHUNK_Z as usize];CHUNK_X as usize]=std::array::from_fn(|x|std::array::from_fn(|z|halo[x+1][z+1]));
        for lx in 0..CHUNK_X {
            for lz in 0..CHUNK_Z {
                let wx = ox + lx;
                let wz = oz + lz;
                let (x,z)=(lx as usize+1,lz as usize+1);
                let height = halo[x][z];
                let water_level = if self.generation.shape == crate::worldgen::Shape::Mainland {
                    super::terrain::tributary(wx,wz,self.seed).map(|p|p.1).unwrap_or(SEA_LEVEL)
                } else { SEA_LEVEL };
                let slope=[halo[x-1][z],halo[x+1][z],halo[x][z-1],halo[x][z+1]].into_iter().map(|h|(h-height).abs()).max().unwrap_or(0);
                let mountain_surface=super::terrain::mountain_surface_with_slope(wx,wz,height,self.seed,slope);

                for ly in 0..super::chunk::TERRAIN_HEIGHT {
                    let block = if ly < BEDROCK_DEPTH {
                        BlockType::Bedrock
                    } else if ly > height {
                        if ly <= water_level {
                            BlockType::Water
                        } else {
                            BlockType::Air
                        }
                    } else if ly == height {
                        if self.generation.surface != crate::worldgen::Surface::Natural {
                            match self.generation.surface {
                                crate::worldgen::Surface::Sand => BlockType::Sand,
                                crate::worldgen::Surface::Snow => BlockType::Snow,
                                _ => BlockType::Stone,
                            }
                        } else if height <= SEA_LEVEL + 1 {
                            BlockType::Sand
                        } else if height>=super::terrain::ALPINE_LINE {
                            mountain_surface
                        } else {
                            BlockType::Grass
                        }
                    } else if self.generation.surface == crate::worldgen::Surface::Sand && ly > height - 5 {
                        BlockType::Sand
                    } else if height>=super::terrain::ALPINE_LINE {
                        BlockType::Stone
                    } else if ly > height - 4 {
                        BlockType::Soil
                    } else {
                        BlockType::Stone
                    };
                    chunk.set_local(lx, ly, lz, block);
                }

                // Simple tree scattering, away from the shoreline. Species
                // is picked per-tree so all four wood types show up.
                let on_dry_land = self.generation.surface == crate::worldgen::Surface::Natural && height > water_level + 2 && height < super::terrain::ALPINE_LINE;
                let mut placed_topper = false;
                let tree_land = if self.generation.surface == crate::worldgen::Surface::Natural {
                    on_dry_land
                } else {
                    height > water_level + 2 && height < super::chunk::TERRAIN_HEIGHT - 12
                        && self.generation.surface != crate::worldgen::Surface::Stone
                };
                if tree_land && column_rand(wx, wz, self.seed, 0xA11CE) < 0.006 * self.generation.trees as f32 / 100. {
                    let species_roll = column_rand(wx, wz, self.seed, 0x5FEC1E5);
                    let idx = ((species_roll * TREE_SPECIES.len() as f32) as usize)
                        .min(TREE_SPECIES.len() - 1);
                    let (wood, leaves) = TREE_SPECIES[idx];
                    self.place_tree(&mut chunk, lx, height, lz, wx, wz, wood, leaves);
                    placed_topper = true;
                }

                // Rare pumpkin patches on open grass, never on a tree's own
                // trunk cell.
                if on_dry_land
                    && !placed_topper
                    && column_rand(wx, wz, self.seed, 0x9A9E27) < 0.0025
                {
                    chunk.set_local(lx, height + 1, lz, BlockType::Pumpkin);
                    placed_topper = true;
                }

                // Short grass tufts -- a common, purely decorative "only on
                // top" cover, so it should never overwrite a tree/pumpkin.
                if on_dry_land
                    && !placed_topper
                    && column_rand(wx, wz, self.seed, 0x5407A55) < 0.06
                {
                    chunk.set_local(lx, height + 1, lz, BlockType::ShortGrass);
                }

                // Rare crystal outcrops on dry land, for the "carrying a
                // crystal" rule condition. Sits on top of the surface block.
                if on_dry_land && column_rand(wx, wz, self.seed, 0xC4157A1) < 0.0015 {
                    chunk.set_local(lx, height + 1, lz, BlockType::Crystal);
                }
            }
        }

        self.scatter_veins(&mut chunk, cx, cz, &heights);
        self.scatter_brick_ruins(&mut chunk, cx, cz);
        self.scatter_surface_resources(&mut chunk, &heights);
        crate::campfire::generate(&mut chunk, self.seed);
        // Keep a stone support directly beneath generated snow even where
        // ore/deposit passes have changed the surrounding mountain interior.
        for lx in 0..CHUNK_X {for lz in 0..CHUNK_Z {
            let h=heights[lx as usize][lz as usize];
            if chunk.get_local(lx,h,lz)==BlockType::Snow {chunk.set_local(lx,h-1,lz,BlockType::Stone);}
        }}

        crate::underground::carve(self, &mut chunk);
        chunk.dirty = true;
        chunk
    }

    /// A second pass never overwrites tree trunks, leaves, existing decorations or water.
    fn scatter_surface_resources(&self, chunk: &mut Chunk, heights:&[[i32;CHUNK_Z as usize];CHUNK_X as usize]) {
        let (ox,oz)=chunk.world_origin();
        for lx in 0..CHUNK_X { for lz in 0..CHUNK_Z {
            let (wx,wz)=(ox+lx,oz+lz);
            let y=heights[lx as usize][lz as usize];
            let surface=chunk.get_local(lx,y,lz);
            let wet=(SEA_LEVEL..=SEA_LEVEL+2).contains(&y);
            if wet && matches!(surface,BlockType::Sand | BlockType::Grass | BlockType::Soil) {
                let roll=column_rand(wx,wz,self.seed,0x7200A001);
                let ground=if roll<0.12 {Some(BlockType::Clay)} else if roll<0.20 {Some(BlockType::Peat)} else if roll<0.28 {Some(BlockType::Mud)} else {None};
                if let Some(block)=ground { chunk.set_local(lx,y,lz,block); }
            }
            if y<SEA_LEVEL || chunk.get_local(lx,y+1,lz)!=BlockType::Air {continue;}
            let roll=column_rand(wx,wz,self.seed,0x7200A002);
            let plant=if wet && roll<0.07 {Some(BlockType::Reeds)}
                else if surface==BlockType::Grass && roll<0.012 {Some(BlockType::Flax)}
                else if surface==BlockType::Grass && roll<0.028 {Some(BlockType::WildHerbs)} else {None};
            if let Some(block)=plant {chunk.set_local(lx,y+1,lz,block);continue;}
            if !chunk.get_local(lx,y,lz).is_solid() {continue;}
            let shaded=(-2..=2).any(|dx|(-2..=2).any(|dz|(2..=6).any(|dy| {
                let block=chunk.get_local(lx+dx,y+dy,lz+dz);
                block.is_wood() || block.def().cutout
            })));
            for resource in SURFACE_RESOURCES {
                let suitable=match resource.habitat {
                    "shade"=>shaded,
                    "wet"=>wet,
                    "meadow"=>surface==BlockType::Grass && !shaded,
                    "dry"=>surface==BlockType::Grass && y>=SEA_LEVEL+6,
                    _=>false,
                };
                // Coarse patches plus independent local rolls give natural clumps, not a grid.
                if suitable && column_rand(wx.div_euclid(5),wz.div_euclid(5),self.seed,resource.salt^0x99)<0.7
                    && column_rand(wx,wz,self.seed,resource.salt)<resource.chance {
                    chunk.set_local(lx,y+1,lz,resource.block);break;
                }
            }
        }}
    }

    #[allow(clippy::too_many_arguments)]
    fn place_tree(
        &self,
        chunk: &mut Chunk,
        lx: i32,
        ground_y: i32,
        lz: i32,
        wx: i32,
        wz: i32,
        wood: BlockType,
        leaves: BlockType,
    ) {
        let trunk_height = 4 + (column_rand(wx, wz, self.seed, 0xBEEF) * 3.0) as i32;
        for i in 1..=trunk_height {
            chunk.set_local(lx, ground_y + i, lz, wood);
        }
        let top = ground_y + trunk_height;

        // Canopy layers relative to the treetop, bottom to top, each with
        // its own radius -- tapered so the crown reads as round-ish rather
        // than a flat box, with a narrow fringe above and below the full
        // middle spread.
        const CANOPY_LAYERS: [(i32, i32); 4] = [(-1, 1), (0, 2), (1, 2), (2, 1)];
        // Per-tree roll for how full the outer ring of each layer is, so
        // canopies vary a little instead of all being an identical cutout.
        let fullness = column_rand(wx, wz, self.seed, 0x7EAF01);

        for &(dy, radius) in CANOPY_LAYERS.iter() {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let dist = dx.abs().max(dz.abs());
                    if dist > radius {
                        continue;
                    }
                    // Corners of every layer's square are cut, including
                    // the radius-1 top/bottom fringes -- a diamond/octagon
                    // cross-section reads far less like a rectangular box
                    // than a full square.
                    if dx.abs() == radius && dz.abs() == radius {
                        continue;
                    }
                    // Per-cell jitter on the outer ring, so the edge isn't
                    // a perfectly even circle either.
                    if dist == radius {
                        let jitter =
                            column_rand(wx + dx, wz + dz, self.seed, 0x1EAF ^ (dy as u32));
                        if jitter > 0.55 + fullness * 0.3 {
                            continue;
                        }
                    }
                    let bx = lx + dx;
                    let by = top + dy;
                    let bz = lz + dz;
                    if Chunk::in_bounds(bx, by, bz) && chunk.get_local(bx, by, bz) == BlockType::Air
                    {
                        chunk.set_local(bx, by, bz, leaves);
                    }
                }
            }
        }
    }

    /// Grows small underground deposits (ores, basalt, cobblestone) by
    /// replacing stone (and buried soil for ores) with deposit entries. Each vein starts from a
    /// deterministic per-chunk-attempt roll and grows via a short random
    /// walk, so results are reproducible from `(seed, cx, cz)` alone like
    /// the rest of chunk generation.
    fn scatter_veins(&self, chunk: &mut Chunk, cx: i32, cz: i32, heights:&[[i32;CHUNK_Z as usize];CHUNK_X as usize]) {
        for vein in VEINS.iter().chain(RESOURCE_VEINS.iter()) {
            let ore=vein.block.id().ends_with("_ore") || vein.block==BlockType::Coal;
            let attempts=vein.attempts_per_chunk * if ore {2} else {1};
            for attempt in 0..attempts as i32 {
                if block_rand(cx, attempt, cz, self.seed, vein.salt) >= vein.spawn_chance {
                    continue;
                }
                let lx = block_rand_range(cx, attempt, cz, self.seed, vein.salt ^ 0xA1, 0, CHUNK_X - 1);
                let ly = block_rand_range(
                    cx,
                    attempt,
                    cz,
                    self.seed,
                    vein.salt ^ 0xB2,
                    vein.min_y,
                    vein.max_y,
                );
                let lz = block_rand_range(cx, attempt, cz, self.seed, vein.salt ^ 0xC3, 0, CHUNK_Z - 1);
                let size =
                    block_rand_range(cx, attempt, cz, self.seed, vein.salt ^ 0xD4, vein.min_size, vein.max_size) + if ore {2} else {0};

                let (mut x, mut y, mut z) = (lx, ly, lz);
                for step in 0..size {
                    if Chunk::in_bounds(x, y, z) && (vein.min_y..=vein.max_y).contains(&y)
                        && y < heights[x as usize][z as usize]
                        && (chunk.get_local(x,y,z)==BlockType::Stone || (ore && chunk.get_local(x,y,z)==BlockType::Soil)) {
                        chunk.set_local(x, y, z, vein.block);
                    }
                    let dir = block_rand_range(x, y, z, self.seed, vein.salt ^ (step as u32), 0, 5) as usize;
                    let (dx, dy, dz) = NEIGHBOR_OFFSETS[dir];
                    x += dx;
                    y += dy;
                    z += dz;
                }
            }
        }
    }

    /// Extremely rare single brick blocks buried in stone, as if leftover
    /// ruins -- "single items" per the CSV, so unlike `scatter_veins` this
    /// never clusters: exactly one `Stone` cell becomes `Bricks` per roll.
    fn scatter_brick_ruins(&self, chunk: &mut Chunk, cx: i32, cz: i32) {
        const ATTEMPTS: i32 = 2;
        const SPAWN_CHANCE: f32 = 0.05;
        const SALT: u32 = 0xB61C_5301;
        const MIN_Y: i32 = 3;
        const MAX_Y: i32 = 25;

        for attempt in 0..ATTEMPTS {
            if block_rand(cx, attempt, cz, self.seed, SALT) >= SPAWN_CHANCE {
                continue;
            }
            let lx = block_rand_range(cx, attempt, cz, self.seed, SALT ^ 0xA1, 0, CHUNK_X - 1);
            let ly = block_rand_range(cx, attempt, cz, self.seed, SALT ^ 0xB2, MIN_Y, MAX_Y);
            let lz = block_rand_range(cx, attempt, cz, self.seed, SALT ^ 0xC3, 0, CHUNK_Z - 1);
            if chunk.get_local(lx, ly, lz) == BlockType::Stone {
                chunk.set_local(lx, ly, lz, BlockType::Bricks);
            }
        }
    }

    pub fn ensure_chunk_loaded(&mut self, cx: i32, cz: i32) {
        if self.chunks.contains_key(&(cx, cz)) {
            return;
        }
        let mut chunk = self.generate_chunk(cx, cz);
        self.apply_edits_to_chunk(&mut chunk);
        self.chunks.insert((cx, cz), chunk);
        // Previously meshed neighbors may have exposed boundary faces/AO
        // calculated while this chunk was absent. Refresh them incrementally.
        self.dirty_neighbors(cx,cz);
    }

    fn dirty_neighbors(&mut self,cx:i32,cz:i32) {
        for dx in -1..=1 {for dz in -1..=1 {
            if let Some(chunk)=self.chunks.get_mut(&(cx+dx,cz+dz)){chunk.dirty=true;*chunk.sky_cache.get_mut()=None;}
        }}
    }

    fn apply_edits_to_chunk(&self, chunk: &mut Chunk) {
        if self.edits.is_empty() {
            return;
        }
        let (ox, oz) = chunk.world_origin();
        for lx in 0..CHUNK_X {
            for lz in 0..CHUNK_Z {
                for ly in 0..CHUNK_Y {
                    if let Some(&b) = self.edits.get(&(ox + lx, ly, oz + lz)) {
                        chunk.set_local(lx, ly, lz, b);
                    }
                }
            }
        }
    }

    pub fn unload_chunk(&mut self, cx: i32, cz: i32) {
        if self.chunks.remove(&(cx, cz)).is_some(){self.dirty_neighbors(cx,cz);}
    }

    pub fn get_block(&self, wx: i32, wy: i32, wz: i32) -> BlockType {
        if self.automation.device_at((wx,wy,wz)).is_some() { return BlockType::AutomationDevice; }
        if wy < 0 || wy >= CHUNK_Y {
            return BlockType::Air;
        }
        let (cx, cz) = world_to_chunk(wx, wz);
        let (lx, lz) = world_to_local(wx, wz);
        match self.chunks.get(&(cx, cz)) {
            Some(chunk) => chunk.get_local(lx, wy, lz),
            None => BlockType::Air,
        }
    }

    pub fn set_block(&mut self, wx: i32, wy: i32, wz: i32, block: BlockType) {
        // Devices are packed transactionally; ordinary edits cannot erase cargo.
        if block==BlockType::AutomationDevice || self.automation.device_at((wx,wy,wz)).is_some() { return; }
        if wy < 0 || wy >= CHUNK_Y {
            return;
        }
        let was = self.get_block(wx, wy, wz);
        if was == BlockType::RedStone && block != BlockType::RedStone {
            self.redstone_positions.retain(|&p| p != (wx, wy, wz));
        } else if block == BlockType::RedStone && was != BlockType::RedStone {
            self.redstone_positions.push((wx, wy, wz));
        }
        self.edits.insert((wx, wy, wz), block);
        let (cx, cz) = world_to_chunk(wx, wz);
        let (lx, lz) = world_to_local(wx, wz);
        if let Some(chunk) = self.chunks.get_mut(&(cx, cz)) {
            chunk.set_local(lx, wy, lz, block);
        }
        // Only chunks within the skylight halo can change (also covers AO and
        // boundary faces). Avoid rebuilding all nine chunks for every edit.
        for z in (wz-8).div_euclid(CHUNK_Z)..=(wz+8).div_euclid(CHUNK_Z) {
            for x in (wx-8).div_euclid(CHUNK_X)..=(wx+8).div_euclid(CHUNK_X) {
                if let Some(neighbor)=self.chunks.get_mut(&(x,z)) {
                    neighbor.dirty=true;
                    *neighbor.sky_cache.get_mut()=None;
                }
            }
        }
    }

    pub fn is_solid(&self, wx: i32, wy: i32, wz: i32) -> bool {
        self.get_block(wx, wy, wz).is_solid()
    }

    /// Call after a block at `start` has just been broken to `Air`. Returns
    /// every additional position (bounded by `MAX_FLOOD_BLOCKS`) that should
    /// become `Water` because it's connected to an existing water source --
    /// the caller is responsible for actually setting those blocks (so it
    /// can replicate each one the same way as any other edit).
    ///
    /// This is a simple bounded flood fill, not real fluid simulation: it
    /// only spreads sideways and downward (never climbing back up through an
    /// open shaft) from a cell that directly touches water in any direction
    /// -- water sitting right above `start` falls in; a newly-dug tunnel
    /// that reaches sideways into a lake wall gets flooded from that point
    /// on.
    pub fn flood_from(&self, start: (i32, i32, i32)) -> Vec<(i32, i32, i32)> {
        if self.get_block(start.0, start.1, start.2) != BlockType::Air {
            return Vec::new();
        }
        let touches_water = NEIGHBOR_OFFSETS.iter().any(|&(dx, dy, dz)| {
            self.get_block(start.0 + dx, start.1 + dy, start.2 + dz) == BlockType::Water
        });
        if !touches_water {
            return Vec::new();
        }

        let mut visited: HashSet<(i32, i32, i32)> = HashSet::new();
        let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
        let mut result = Vec::new();
        visited.insert(start);
        queue.push_back(start);

        while let Some(pos) = queue.pop_front() {
            if result.len() >= MAX_FLOOD_BLOCKS {
                break;
            }
            result.push(pos);
            for &(dx, dy, dz) in NEIGHBOR_OFFSETS.iter() {
                if dy > 0 {
                    continue; // water doesn't climb back up an open shaft
                }
                let next = (pos.0 + dx, pos.1 + dy, pos.2 + dz);
                if visited.insert(next) && self.get_block(next.0, next.1, next.2) == BlockType::Air
                {
                    queue.push_back(next);
                }
            }
        }
        result
    }
}

#[derive(Serialize, Deserialize)]
pub struct WorldSave {
    pub seed: u32,
    pub player_pos: [f32; 3],
    pub player_yaw: f32,
    pub player_pitch: f32,
    pub time_of_day: f32,
    pub edits: Vec<((i32, i32, i32), BlockType)>,
    pub modules: Vec<crate::scripting::ModuleSaveEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompted_surfaces_and_chunk_order_are_consistent() {
        use crate::worldgen::{Shape, Surface, WorldGeneration};
        for (surface, block) in [(Surface::Sand, BlockType::Sand), (Surface::Snow, BlockType::Snow), (Surface::Stone, BlockType::Stone)] {
            let mut world = World::new(42);
            world.generation = WorldGeneration { shape: Shape::Flat, surface, trees: 0, relief: 0, ..Default::default() };
            let first = world.generate_chunk(-1, 0);
            let _neighbor = world.generate_chunk(0, 0);
            let again = world.generate_chunk(-1, 0);
            for x in 0..CHUNK_X { for z in 0..CHUNK_Z {
                assert_eq!(first.get_local(x,25,z), block);
                for y in 0..CHUNK_Y {
                    assert_eq!(first.get_local(x,y,z), again.get_local(x,y,z));
                    assert!(!first.get_local(x,y,z).is_wood());
                }
            }}
        }
    }

    #[test]
    fn flood_from_fills_an_isolated_air_pocket_touching_water() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        // Wall off (6,10,5) on every side except toward the water, so the
        // flood has nowhere else to spread to -- isolates exactly the case
        // under test (one pocket, one water source, nothing beyond it).
        for &(dx, dy, dz) in &[(1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            chunk.set_local(6 + dx, 10 + dy, 5 + dz, BlockType::Stone);
        }
        chunk.set_local(5, 10, 5, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        let result = world.flood_from((6, 10, 5));
        assert_eq!(result, vec![(6, 10, 5)]);
    }

    #[test]
    fn flood_from_does_nothing_without_an_adjacent_water_source() {
        let world = World::new(1);
        // No chunk loaded at all -- get_block reads Air everywhere, so the
        // start cell passes the "is Air" check but never finds water.
        let result = world.flood_from((6, 10, 5));
        assert!(result.is_empty());
    }

    #[test]
    fn flood_from_refuses_to_start_on_a_non_air_cell() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::Water);
        chunk.set_local(6, 10, 5, BlockType::Stone); // not actually broken
        world.chunks.insert((0, 0), chunk);

        assert!(world.flood_from((6, 10, 5)).is_empty());
    }

    #[test]
    fn flood_from_lets_water_fall_from_directly_above() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        // Isolate the pocket again, this time leaving only "up" open --
        // that's where the water sits.
        for &(dx, dy, dz) in &[(1, 0, 0), (-1, 0, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            chunk.set_local(5 + dx, 10 + dy, 5 + dz, BlockType::Stone);
        }
        chunk.set_local(5, 11, 5, BlockType::Water); // sits right above the dig
        world.chunks.insert((0, 0), chunk);

        let result = world.flood_from((5, 10, 5));
        assert_eq!(result, vec![(5, 10, 5)]);
    }

    #[test]
    fn flood_from_spreads_through_a_connected_air_tunnel() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(0, 10, 0, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        // (1,10,0) touches the water directly; (2,10,0) is only reachable
        // by continuing sideways through the already-flooded cell.
        let result = world.flood_from((1, 10, 0));
        assert!(result.contains(&(1, 10, 0)));
        assert!(
            result.contains(&(2, 10, 0)),
            "flood should spread sideways through connected air: {result:?}"
        );
    }

    #[test]
    fn flood_from_does_not_climb_upward_through_an_open_shaft() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        chunk.set_local(5, 10, 5, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        // (6,10,5) touches the water sideways; (6,11,5) is open air directly
        // above it -- water shouldn't defy gravity to climb up into it.
        let result = world.flood_from((6, 10, 5));
        assert!(result.contains(&(6, 10, 5)));
        assert!(
            !result.contains(&(6, 11, 5)),
            "flood shouldn't climb upward into an open shaft: {result:?}"
        );
    }

    #[test]
    fn flood_from_caps_at_max_flood_blocks() {
        let mut world = World::new(1);
        let mut chunk = Chunk::new(0, 0);
        // The rest of the chunk is Air by default -- a huge connected
        // pocket, far more than MAX_FLOOD_BLOCKS, all reachable from one
        // water source.
        chunk.set_local(0, 10, 0, BlockType::Water);
        world.chunks.insert((0, 0), chunk);

        let result = world.flood_from((1, 10, 0));
        assert_eq!(
            result.len(),
            MAX_FLOOD_BLOCKS,
            "flood should stop at the cap instead of filling the whole cavity"
        );
    }
}

#[cfg(test)]
mod resource_generation_tests {
    use super::*;
    #[test]
    fn resources_are_obtainable_and_refined_materials_never_spawn() {
        let mut counts=vec![0u64;crate::voxel::COLLECTIBLE_BLOCKS.len()];
        let mut blocks=0u64;
        let mut soil_ores=0u64;
        for seed in [7,42,2026] {
            let world=World::new(seed);
            for cx in -6..6 {for cz in -6..6 {
                let chunk=world.generate_chunk(cx*7,cz*7);
                for x in 0..CHUNK_X { for z in 0..CHUNK_Z {for y in 0..CHUNK_Y {
                    let block=chunk.get_local(x,y,z);
                    if block==BlockType::Air || block==BlockType::Water {continue;}
                    blocks+=1;
                    if block.id().ends_with("_ore") {
                        let (ox,oz)=chunk.world_origin();
                        let h=world.terrain_height(ox+x,oz+z);
                        assert!(y<h,"ore broke the surface");
                        if y>=h-3 {soil_ores+=1;}
                    }
                    if let Some(i)=crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|&b|b==block) {counts[i]+=1;}
                    if y<BEDROCK_DEPTH {assert_eq!(block,BlockType::Bedrock);}
                    if block.def().cross {assert!(chunk.get_local(x,y-1,z).is_solid());}
                }}}
            }}
        }
        let mut report=String::from("resource,count_in_432_sampled_chunks\n");
        for (i,info) in crate::voxel::resource_catalog::RESOURCES.iter().enumerate() {
            report.push_str(&format!("{},{}\n",info.block.id(),counts[i]));
            if matches!(info.source,"crafted"|"loot"|"cooked") {assert_eq!(counts[i],0,"non-terrain resource {} spawned",info.block.id());}
            else {assert!(counts[i]>0,"natural {} unavailable",info.block.id());}
        }
        let count=|block|counts[crate::voxel::COLLECTIBLE_BLOCKS.iter().position(|&b|b==block).unwrap()];
        assert!(soil_ores>100,"ores never reached buried soil");
        // Baseline for this identical 432-chunk survey was 4,025 iron / 4,492 coal.
        assert!(count(BlockType::IronOre)>8_050);
        assert!(count(BlockType::Coal)>8_984);
        assert!(count(BlockType::IronOre)>count(BlockType::MithrilOre)*3);
        assert!(count(BlockType::Coal)>count(BlockType::Moonstone)*3);
        assert!(count(BlockType::Stone)>blocks/2,"deposits crowded out basic stone");
        std::fs::create_dir_all("target").unwrap();
        std::fs::write("target/resource-distribution.csv",report).unwrap();
    }
    #[test]
    fn generated_resources_are_deterministic_across_load_order() {
        let a=World::new(2026).generate_chunk(-3,5);
        let mut other=World::new(2026); other.ensure_chunk_loaded(2,3);
        let b=other.generate_chunk(-3,5);
        for x in 0..CHUNK_X {for z in 0..CHUNK_Z {for y in 0..CHUNK_Y {
            assert_eq!(a.get_local(x,y,z),b.get_local(x,y,z));
        }}}
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    fn streamed_neighbor_arrival_and_departure_invalidate_boundary_meshes(){
        let mut world=World::new(42);world.ensure_chunk_loaded(0,0);world.chunks.get_mut(&(0,0)).unwrap().dirty=false;
        world.ensure_chunk_loaded(1,0);assert!(world.chunks[&(0,0)].dirty);
        world.chunks.get_mut(&(0,0)).unwrap().dirty=false;world.unload_chunk(1,0);assert!(world.chunks[&(0,0)].dirty);
    }
    #[test]
    #[ignore = "CPU terrain/meshing benchmark; prints exploration costs"]
    fn profile_exploration() {
        use std::time::Instant;
        for center in [0,128,1024] {
            let mut world=World::new(42);
            let start=Instant::now();
            for x in center-5..=center+5 {for z in -5..=5 {world.ensure_chunk_loaded(x,z);}}
            let generation=start.elapsed();let start=Instant::now();let mut triangles=0;
            for chunk in world.chunks.values(){triangles+=super::super::mesher::build_chunk_mesh(&world,chunk).indices.len()/3;}
            let meshing=start.elapsed();let start=Instant::now();
            for z in -5..=5 {world.ensure_chunk_loaded(center+6,z);}
            println!("center_x={} blocks, generate121={:?}, mesh121={:?}, crossing11={:?}, triangles={}",center*16,generation,meshing,start.elapsed(),triangles);
        }
    }
}
