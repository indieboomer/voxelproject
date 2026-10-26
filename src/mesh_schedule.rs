//! Time-sliced chunk meshing: chunks are built one block layer at a time
//! within a per-frame budget, nearest first, so a costly chunk never stalls a
//! frame. The old mesh stays visible until the new one is complete.

use std::time::{Duration, Instant};

use crate::voxel::mesher::{build_chunk_mesh_prepared, Detail, MeshData, MeshPrep};
use crate::voxel::World;

/// Block layers meshed per step: one, so the budget is checked after every
/// layer (a busy surface layer costs up to about 3 ms in shaped mode).
const SLICE_LAYERS: i32 = 1;

struct Job {
    key: (i32, i32),
    detail: Detail,
    layer: i32,
    mesh: MeshData,
    prep: MeshPrep,
}

#[derive(Default)]
pub struct MeshScheduler {
    job: Option<Job>,
}

impl MeshScheduler {
    /// No chunk build in progress.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.job.is_none()
    }

    /// Forget an in-progress build of `key` (it was rebuilt another way).
    pub fn cancel(&mut self, key: (i32, i32)) {
        if self.job.as_ref().is_some_and(|job| job.key == key) {
            self.job = None;
        }
    }

    /// Spends up to `budget` on mesh work and returns finished chunk meshes.
    /// A job always runs to completion: if its chunk is re-dirtied meanwhile
    /// (neighbors streaming in), the dirty flag stays set and the chunk is
    /// rebuilt again later. Restarting instead would starve every other
    /// chunk while the world is still loading.
    pub fn step(
        &mut self,
        world: &mut World,
        center: (i32, i32),
        in_range: impl Fn(&(i32, i32)) -> bool,
        detail_for: impl Fn((i32, i32)) -> Detail,
        budget: Duration,
    ) -> Vec<((i32, i32), Detail, MeshData)> {
        let start = Instant::now();
        let mut finished = Vec::new();
        while start.elapsed() < budget {
            if self.job.is_none() {
                let next = world
                    .chunks
                    .iter()
                    .filter(|(k, c)| c.dirty && in_range(k))
                    .map(|(k, _)| *k)
                    .min_by_key(|&(x, z)| ((x - center.0).pow(2) + (z - center.1).pow(2), x, z));
                let Some(key) = next else { break };
                world.chunks.get_mut(&key).unwrap().dirty = false;
                let prep = MeshPrep::new(world, &world.chunks[&key]);
                let mesh = MeshData { vertices: Vec::new(), indices: Vec::new() };
                self.job = Some(Job { key, detail: detail_for(key), layer: 0, mesh, prep });
            }
            let job = self.job.as_mut().unwrap();
            let Some(chunk) = world.chunks.get(&job.key).filter(|_| in_range(&job.key)) else {
                // Unloaded or out of range meanwhile; rebuild later if needed.
                if let Some(chunk) = world.chunks.get_mut(&job.key) {
                    chunk.dirty = true;
                }
                self.job = None;
                continue;
            };
            let end = (job.layer + SLICE_LAYERS).min(chunk.stored_height());
            job.mesh.extend(build_chunk_mesh_prepared(world, chunk, job.detail, job.layer..end, &job.prep));
            job.layer = end;
            if end >= chunk.stored_height() {
                let job = self.job.take().unwrap();
                finished.push((job.key, job.detail, job.mesh));
            }
        }
        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Streams chunks in around the origin like the game does (up to two new
    /// chunks a frame, each dirtying its loaded neighbors) and checks every
    /// chunk gets a mesh. Restarting re-dirtied jobs used to starve this.
    #[test]
    fn every_chunk_gets_meshed_while_neighbors_keep_streaming_in() {
        let mut world = World::new(42);
        world.terrain_shaping = true;
        let mut pending: Vec<(i32, i32)> = (-3..=3).flat_map(|x| (-3..=3).map(move |z| (x, z))).collect();
        pending.sort_by_key(|&(x, z)| x * x + z * z);
        let mut scheduler = MeshScheduler::default();
        let mut meshed = std::collections::HashSet::new();
        for _frame in 0..2000 {
            for _ in 0..2 {
                if !pending.is_empty() {
                    let (x, z) = pending.remove(0);
                    world.ensure_chunk_loaded(x, z);
                }
            }
            let done = scheduler.step(&mut world, (0, 0), |_| true, |_| Detail::Full, Duration::from_millis(3));
            meshed.extend(done.into_iter().map(|(k, _, _)| k));
            if pending.is_empty() && scheduler.idle() && world.chunks.values().all(|c| !c.dirty) {
                break;
            }
        }
        assert_eq!(meshed.len(), 49, "every loaded chunk was meshed");
        assert!(world.chunks.values().all(|c| !c.dirty), "nothing left waiting");
    }
}
