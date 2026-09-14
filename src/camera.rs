use glam::{Mat4, Vec3};

pub struct Camera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_plane_stays_inside_head_clearance_on_wide_displays() {
        for aspect in [16. / 9., 21. / 9., 32. / 9.] {
            let camera = Camera::new(Vec3::ZERO, aspect);
            let half_height = camera.znear * (camera.fovy * 0.5).tan();
            let half_width = half_height * aspect;
            // The bounding sphere covers every pitch and yaw, including
            // looking diagonally up while touching a cave ceiling.
            let radius = Vec3::new(half_width, half_height, camera.znear).length();
            assert!(radius < 1.8 - camera.eye_position().y);
        }
    }
}

impl Camera {
    pub fn new(position: Vec3, aspect: f32) -> Self {
        Self {
            position,
            yaw: -90f32.to_radians(),
            pitch: 0.0,
            aspect,
            fovy: 70f32.to_radians(),
            // Keep the whole near plane inside the collider, even under a
            // low ceiling on an ultrawide screen (eyes have 0.18 clearance).
            znear: 0.05,
            zfar: 400.0,
        }
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.sin() * self.pitch.cos(),
        )
        .normalize()
    }

    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize()
    }

    pub fn eye_position(&self) -> Vec3 {
        // player position is feet position; eyes are offset up.
        self.position + Vec3::new(0.0, 1.62, 0.0)
    }

    pub fn view_proj(&self) -> Mat4 {
        let eye = self.eye_position();
        let view = Mat4::look_to_rh(eye, self.forward(), Vec3::Y);
        let proj = Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar);
        proj * view
    }
}
