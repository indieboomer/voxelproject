//! Reuse inactive device geometry; active mechanisms and feedback still animate.
use crate::{
    automation::{Activity, Cell, Device, Kind, State},
    voxel::mesher::MeshData,
};
type Stamp = (Kind, u8, crate::automation::Config, bool);
#[derive(Default)]
pub struct Cache(std::collections::BTreeMap<Cell, ([Option<Stamp>; 7], MeshData)>);
impl Cache {
    pub fn retain(&mut self, state: &State) {
        self.0.retain(|cell, _| state.devices.contains_key(cell));
    }
    pub fn mesh(&mut self, d: &Device, time: f32, state: &State) -> MeshData {
        if d.activity == Activity::Working || d.signal > 0 {
            self.0.remove(&d.cell);
            return crate::automation_mesh::device(d, time, None, state);
        }
        let (x, y, z) = d.cell;
        let cells = [
            (x, y, z),
            (x - 1, y, z),
            (x + 1, y, z),
            (x, y - 1, z),
            (x, y + 1, z),
            (x, y, z - 1),
            (x, y, z + 1),
        ];
        let stamp = cells.map(|p| {
            state
                .devices
                .get(&p)
                .map(|d| (d.kind, d.rotation, d.config.clone(), d.open()))
        });
        if self.0.get(&d.cell).is_none_or(|(old, _)| *old != stamp) {
            self.0.insert(
                d.cell,
                (stamp, crate::automation_mesh::device(d, 0., None, state)),
            );
        }
        let mesh = &self.0[&d.cell].1;
        MeshData {
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_matches(cache: &mut Cache, state: &State, cell: Cell, time: f32) {
        let d = &state.devices[&cell];
        let actual = cache.mesh(d, time, state);
        let expected = crate::automation_mesh::device(d, time, None, state);
        assert_eq!(actual.indices, expected.indices);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&actual.vertices),
            bytemuck::cast_slice::<_, u8>(&expected.vertices)
        );
    }
    #[test]
    fn cached_geometry_tracks_config_neighbors_and_animation() {
        let mut state = State::default();
        let cell = (0, 0, 0);
        let mut cache = Cache::default();
        state
            .devices
            .insert(cell, Device::new(Kind::Valve, cell, 0));
        assert_matches(&mut cache, &state, cell, 0.);
        state.devices.get_mut(&cell).unwrap().rotation = 1;
        state.devices.get_mut(&cell).unwrap().config.valve_matter = true;
        assert_matches(&mut cache, &state, cell, 1.);
        state
            .devices
            .insert((1, 0, 0), Device::new(Kind::Channel, (1, 0, 0), 0));
        assert_matches(&mut cache, &state, cell, 1.);
        state.devices.get_mut(&cell).unwrap().activity = Activity::Working;
        assert_matches(&mut cache, &state, cell, 1.);
        assert_matches(&mut cache, &state, cell, 2.);
        state.devices.clear();
        cache.retain(&state);
        assert!(cache.0.is_empty());
    }
    #[test]
    #[ignore = "manual CPU presentation benchmark"]
    fn profile_static_props() {
        let mut state = State::default();
        for x in 0..8 {
            for z in 0..8 {
                let cell = (x * 2, 0, z * 2);
                state
                    .devices
                    .insert(cell, Device::new(Kind::Workshop, cell, 0));
            }
        }
        let mut cache = Cache::default();
        for d in state.devices.values() {
            cache.mesh(d, 0., &state);
        }
        for cached in [false, true] {
            let start = std::time::Instant::now();
            for _ in 0..100 {
                for d in state.devices.values() {
                    std::hint::black_box(if cached {
                        cache.mesh(d, 0., &state)
                    } else {
                        crate::automation_mesh::device(d, 0., None, &state)
                    });
                }
            }
            println!(
                "64 static workshops cached={cached}: {:.3} ms/frame",
                start.elapsed().as_secs_f64() * 10.
            );
        }
    }
}
