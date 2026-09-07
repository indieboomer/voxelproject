use std::collections::HashSet;

use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct Input {
    keys_down: HashSet<KeyCode>,
    pub mouse_delta: (f32, f32),
    pub left_clicked: bool,
    pub right_clicked: bool,
    /// Edge-triggered, like `left_clicked`/`right_clicked` -- set once on
    /// the frame the interact key (E) is pressed, aimed at whatever's under
    /// the crosshair. Distinct from both: unlike a left click it never
    /// breaks a block, and unlike a right click it never places one -- it
    /// only reports "the player deliberately used this" for `on_interact`
    /// to react to (see world_api/schema.yaml).
    pub interact_clicked: bool,
    pub hotbar_select: Option<usize>,
    pub save_requested: bool,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key_event(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => {
                self.keys_down.insert(key);
                match key {
                    KeyCode::Digit1 => self.hotbar_select = Some(0),
                    KeyCode::Digit2 => self.hotbar_select = Some(1),
                    KeyCode::Digit3 => self.hotbar_select = Some(2),
                    KeyCode::Digit4 => self.hotbar_select = Some(3),
                    KeyCode::Digit5 => self.hotbar_select = Some(4),
                    KeyCode::Digit6 => self.hotbar_select = Some(5),
                    KeyCode::Digit7 => self.hotbar_select = Some(6),
                    KeyCode::Digit8 => self.hotbar_select = Some(7),
                    KeyCode::F5 => self.save_requested = true,
                    KeyCode::KeyE => self.interact_clicked = true,
                    _ => {}
                }
            }
            ElementState::Released => {
                self.keys_down.remove(&key);
            }
        }
    }

    pub fn mouse_button_event(&mut self, button: MouseButton, state: ElementState) {
        if state == ElementState::Pressed {
            match button {
                MouseButton::Left => self.left_clicked = true,
                MouseButton::Right => self.right_clicked = true,
                _ => {}
            }
        }
    }

    pub fn is_down(&self, key: KeyCode) -> bool {
        self.keys_down.contains(&key)
    }

    /// Clears held-key state, e.g. when the console opens mid-press so a
    /// stuck WASD key doesn't keep moving the player while typing.
    pub fn release_all(&mut self) {
        self.keys_down.clear();
    }

    /// Clears the per-frame edge-triggered state. Call once per frame after
    /// consuming it.
    pub fn end_frame(&mut self) {
        self.mouse_delta = (0.0, 0.0);
        self.left_clicked = false;
        self.right_clicked = false;
        self.interact_clicked = false;
        self.hotbar_select = None;
        self.save_requested = false;
    }
}
