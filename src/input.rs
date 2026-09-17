use std::collections::HashSet;

use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct Input {
    keys_down: HashSet<KeyCode>,
    pub mouse_delta: (f32, f32),
    pub left_clicked: bool,
    pub left_released: bool,
    pub right_clicked: bool,
    /// Edge-triggered, like `left_clicked`/`right_clicked` -- set once on
    /// the frame the interact key (F) is pressed, aimed at whatever's under
    /// the crosshair. Distinct from both: unlike a left click it never
    /// breaks a block, and unlike a right click it never places one -- it
    /// only reports "the player deliberately used this" for `on_interact`
    /// to react to (see world_api/schema.yaml).
    pub interact_clicked: bool,
    pub hotbar_select: Option<usize>,
    pub save_requested: bool,
    pub gesture: Option<crate::player_animation::Clip>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gesture_keys_are_edges_and_clear_with_gameplay_input() {
        let mut input = Input::new();
        for (key, clip) in [
            (KeyCode::Comma, crate::player_animation::Clip::Dance),
            (KeyCode::Period, crate::player_animation::Clip::Angry),
            (KeyCode::Slash, crate::player_animation::Clip::Jump),
        ] {
            input.key_event(key, ElementState::Pressed);
            assert_eq!(input.gesture, Some(clip));
            input.end_frame();
            input.key_event(key, ElementState::Pressed);
            assert_eq!(input.gesture, None);
            input.key_event(key, ElementState::Released);
            input.key_event(key, ElementState::Pressed);
            assert_eq!(input.gesture, Some(clip));
            input.release_all();
            assert_eq!(input.gesture, None);
        }
    }
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key_event(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => {
                let first_press = self.keys_down.insert(key);
                if first_press {
                    self.gesture = match key {
                        KeyCode::Comma => Some(crate::player_animation::Clip::Dance),
                        KeyCode::Period => Some(crate::player_animation::Clip::Angry),
                        KeyCode::Slash => Some(crate::player_animation::Clip::Jump),
                        _ => self.gesture,
                    };
                }
                match key {
                    KeyCode::Digit1 => self.hotbar_select = Some(0),
                    KeyCode::Digit2 => self.hotbar_select = Some(1),
                    KeyCode::Digit3 => self.hotbar_select = Some(2),
                    KeyCode::Digit4 => self.hotbar_select = Some(3),
                    KeyCode::Digit5 => self.hotbar_select = Some(4),
                    KeyCode::Digit6 => self.hotbar_select = Some(5),
                    KeyCode::Digit7 => self.hotbar_select = Some(6),
                    KeyCode::Digit8 => self.hotbar_select = Some(7),
                    KeyCode::Digit9 => self.hotbar_select = Some(8),
                    KeyCode::F5 => self.save_requested = true,
                    KeyCode::KeyF => self.interact_clicked = true,
                    _ => {}
                }
            }
            ElementState::Released => {
                self.keys_down.remove(&key);
            }
        }
    }

    pub fn mouse_button_event(&mut self, button: MouseButton, state: ElementState) {
        if state == ElementState::Released && button == MouseButton::Left {
            self.left_released = true;
        }
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
        self.gesture = None;
    }

    /// Clears the per-frame edge-triggered state. Call once per frame after
    /// consuming it.
    pub fn end_frame(&mut self) {
        self.mouse_delta = (0.0, 0.0);
        self.left_clicked = false;
        self.left_released = false;
        self.right_clicked = false;
        self.interact_clicked = false;
        self.hotbar_select = None;
        self.save_requested = false;
        self.gesture = None;
    }
}
