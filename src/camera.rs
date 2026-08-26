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

impl Camera {
    pub fn new(position: Vec3, aspect: f32) -> Self {
        Self {
            position,
            yaw: -90f32.to_radians(),
            pitch: 0.0,
            aspect,
            fovy: 70f32.to_radians(),
            znear: 0.1,
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
