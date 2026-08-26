use serde::{Deserialize, Serialize};

/// How long, on average, a stretch of weather lasts before it might change.
const MIN_STRETCH_SECS: f32 = 90.0;
const MAX_STRETCH_SECS: f32 = 240.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Weather {
    Clear = 0,
    Rain = 1,
}

impl Weather {
    pub fn name(self) -> &'static str {
        match self {
            Weather::Clear => "clear",
            Weather::Rain => "rain",
        }
    }

    pub fn from_name(name: &str) -> Option<Weather> {
        match name.to_ascii_lowercase().as_str() {
            "clear" => Some(Weather::Clear),
            "rain" => Some(Weather::Rain),
            _ => None,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Weather {
        match v {
            1 => Weather::Rain,
            _ => Weather::Clear,
        }
    }
}

/// Host-authoritative weather cycle: clear/rain alternate on a randomized
/// timer, independent of the day/night cycle. A Lua rule can still force a
/// change via `api.set_weather`, which just resets this timer.
pub struct WeatherState {
    pub current: Weather,
    timer: f32,
    rng: u64,
}

impl WeatherState {
    pub fn new(seed: u32) -> Self {
        let mut state = Self {
            current: Weather::Clear,
            timer: 0.0,
            rng: (seed as u64) | 1,
        };
        state.timer = state.next_stretch();
        state
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn next_stretch(&mut self) -> f32 {
        MIN_STRETCH_SECS + self.next_f32() * (MAX_STRETCH_SECS - MIN_STRETCH_SECS)
    }

    pub fn update(&mut self, dt: f32) {
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.current = match self.current {
                Weather::Clear => Weather::Rain,
                Weather::Rain => Weather::Clear,
            };
            self.timer = self.next_stretch();
        }
    }

    /// Called when a Lua rule sets the weather directly.
    pub fn set(&mut self, weather: Weather) {
        self.current = weather;
        self.timer = self.next_stretch();
    }
}
