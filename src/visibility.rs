//! Conservative chunk culling for separate camera and sun frusta.
use glam::{Mat4, Vec3, Vec4};
pub fn within_terrain_range(cell: (i32, i32), center: (i32, i32), radius: i32) -> bool {
    (cell.0 - center.0).abs() <= radius && (cell.1 - center.1).abs() <= radius
}
pub struct Frustum {
    planes: [Vec4; 6],
}
impl Frustum {
    pub fn new(matrix: Mat4) -> Self {
        let m = matrix.transpose();
        let [x, y, z, w] = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        // wgpu clip volume: -w <= x,y <= w and 0 <= z <= w.
        Self {
            planes: [w + x, w - x, w + y, w - y, z, w - z],
        }
    }
    pub fn chunk(&self, cx: i32, cz: i32) -> bool {
        use crate::voxel::chunk::{CHUNK_X, CHUNK_Y, CHUNK_Z};
        let min = Vec3::new(
            (cx * CHUNK_X) as f32 - 1.0,
            -1.0,
            (cz * CHUNK_Z) as f32 - 1.0,
        );
        let max = min
            + Vec3::new(
                CHUNK_X as f32 + 2.0,
                CHUNK_Y as f32 + 2.0,
                CHUNK_Z as f32 + 2.0,
            );
        self.planes.iter().all(|p| {
            let v = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            p.truncate().dot(v) + p.w >= 0.0
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn creatures_share_terrain_boundary_after_travel() {
        for center in [(0,0),(-100,83),(1000,-900)] {
            assert!(within_terrain_range((center.0+5,center.1-5),center,5));
            assert!(!within_terrain_range((center.0+6,center.1),center,5));
            assert!(!within_terrain_range((center.0,center.1-6),center,5));
        }
    }
    #[test]
    fn retained_ring_does_not_expand_camera_draws_after_travel() {
        for center in [0, 128, 1024] {
            let eye = Vec3::new(center as f32 * 16.0 + 8.0, 30.0, 8.0);
            let f = Frustum::new(
                Mat4::perspective_rh(70f32.to_radians(), 16.0 / 9.0, 0.1, 400.0)
                    * Mat4::look_to_rh(eye, Vec3::NEG_Z, Vec3::Y),
            );
            let visible = (center - 5..=center + 5)
                .flat_map(|x| (-5..=5).map(move |z| (x, z)))
                .filter(|&(x, z)| f.chunk(x, z))
                .count();
            println!(
                "center={} camera draws={visible} vs 225 retained chunks",
                center * 16
            );
            assert!((20..70).contains(&visible));
        }
    }
    #[test]
    fn chunks_behind_camera_are_culled_but_near_plane_intersections_survive() {
        let eye = Vec3::new(8.0, 24.0, 8.0);
        let f = Frustum::new(
            Mat4::perspective_rh(70f32.to_radians(), 1.6, 0.1, 160.0)
                * Mat4::look_to_rh(eye, Vec3::NEG_Z, Vec3::Y),
        );
        assert!(f.chunk(0, 0));
        assert!(f.chunk(0, -3));
        assert!(!f.chunk(0, 3));
        assert!(!f.chunk(30, -3));
        assert!(!f.chunk(0, -30));
    }
    #[test]
    fn sun_frustum_keeps_off_camera_shadow_casters() {
        let f = Frustum::new(
            Mat4::orthographic_rh(-80.0, 80.0, -80.0, 80.0, 0.1, 300.0)
                * Mat4::look_at_rh(
                    Vec3::new(8.0, 120.0, 8.0),
                    Vec3::new(8.0, 0.0, 8.0),
                    Vec3::Z,
                ),
        );
        assert!(f.chunk(0, 3));
        assert!(!f.chunk(50, 0));
    }
}
