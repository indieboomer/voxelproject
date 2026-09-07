use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Weather {
    Sunny = 0,
    Rain = 1,
    Mist = 2,
    Storm = 3,
    Windy = 4,
}

/// How heavily each weather is weighted when `WeatherState` picks the next
/// one -- see `WeatherState::pick_next`. Sunny is deliberately the
/// heaviest so it reads as the default/baseline weather, the way most real
/// climates spend more days clear than in any single kind of bad weather.
const WEATHER_WEIGHTS: [(Weather, u32); 5] = [
    (Weather::Sunny, 45),
    (Weather::Rain, 20),
    (Weather::Mist, 15),
    (Weather::Windy, 12),
    (Weather::Storm, 8),
];

impl Weather {
    pub fn name(self) -> &'static str {
        match self {
            Weather::Sunny => "sunny",
            Weather::Rain => "rain",
            Weather::Mist => "mist",
            Weather::Storm => "storm",
            Weather::Windy => "windy",
        }
    }

    pub fn from_name(name: &str) -> Option<Weather> {
        match name.to_ascii_lowercase().as_str() {
            // "clear" is kept as an accepted alias for "sunny" -- the name
            // this weather went by before the 5-weather system existed --
            // so an old rule/save that still says api.set_weather("clear")
            // keeps working instead of silently doing nothing.
            "sunny" | "clear" => Some(Weather::Sunny),
            "rain" => Some(Weather::Rain),
            "mist" => Some(Weather::Mist),
            "storm" => Some(Weather::Storm),
            "windy" => Some(Weather::Windy),
            _ => None,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Weather {
        match v {
            1 => Weather::Rain,
            2 => Weather::Mist,
            3 => Weather::Storm,
            4 => Weather::Windy,
            _ => Weather::Sunny,
        }
    }

    /// Multiplies the grass/leaf wind-sway amplitude (see `shader.wgsl`'s
    /// `vs_main`, driven by `CameraUniform.light_params.w` in `app.rs`) --
    /// storm and windy visibly whip vegetation harder than the sunny/rain
    /// baseline; mist is calmer than baseline (still, hazy air).
    pub fn wind_strength(self) -> f32 {
        match self {
            Weather::Sunny | Weather::Rain => 1.0,
            Weather::Mist => 0.5,
            Weather::Windy => 2.0,
            Weather::Storm => 2.5,
        }
    }

    /// Whether this weather renders the rain-particle effect. Storm is
    /// treated as heavy rain plus extra wind rather than a separate dry
    /// effect -- a storm with no rain falling would just look like windy
    /// with a different name.
    pub fn has_rain_particles(self) -> bool {
        matches!(self, Weather::Rain | Weather::Storm)
    }

    /// Whether this weather can strike lightning -- see
    /// `App::update_lightning`. Storm only; plain windy has no rain or
    /// lightning, just stronger gusts.
    pub fn has_lightning(self) -> bool {
        matches!(self, Weather::Storm)
    }

    /// How much of the sky dome the procedural cloud layer covers, 0
    /// (none) to 1 (fully overcast) -- see sky.wgsl's `cloud_density`/
    /// `App::render`'s `weather_fx.y`. Only storm gets heavy cover for
    /// now; every other kind (including rain) renders a clear sky above
    /// its own effect, left at 0 rather than guessed at since it wasn't
    /// asked for.
    pub fn cloud_coverage(self) -> f32 {
        match self {
            Weather::Storm => 0.85,
            _ => 0.0,
        }
    }

    /// `(min, max)` seconds a stretch of this weather lasts once picked.
    /// Storms are short and intense; sunny stretches run the longest since
    /// it's the baseline weather. Everything else sits in between.
    fn stretch_range(self) -> (f32, f32) {
        match self {
            Weather::Sunny => (120.0, 300.0),
            Weather::Rain | Weather::Mist | Weather::Windy => (60.0, 150.0),
            Weather::Storm => (30.0, 90.0),
        }
    }
}

/// Host-authoritative weather cycle: on a randomized timer, the next
/// weather is picked with sunny weighted heaviest (see `WEATHER_WEIGHTS`),
/// independent of the day/night cycle. A Lua rule can still force a change
/// via `api.set_weather`, which just resets this timer using the newly-set
/// weather's own `stretch_range`.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherState {
    pub current: Weather,
    timer: f32,
    rng: u64,
}

impl WeatherState {
    pub fn new(seed: u32) -> Self {
        let mut state = Self {
            current: Weather::Sunny,
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
        let (min, max) = self.current.stretch_range();
        min + self.next_f32() * (max - min)
    }

    /// Weighted-random pick from `WEATHER_WEIGHTS` -- can land back on the
    /// current weather (a real sky doesn't reliably change just because a
    /// timer expired), which is exactly what makes "sunny is most common"
    /// actually show up as multiple consecutive sunny stretches rather
    /// than a forced, evenly-spaced rotation through all five.
    fn pick_next(&mut self) -> Weather {
        let total: u64 = WEATHER_WEIGHTS.iter().map(|&(_, w)| w as u64).sum();
        let mut roll = self.next_u64() % total;
        for &(kind, weight) in WEATHER_WEIGHTS.iter() {
            if roll < weight as u64 {
                return kind;
            }
            roll -= weight as u64;
        }
        unreachable!("roll is always < total by construction")
    }

    pub fn update(&mut self, dt: f32) {
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.current = self.pick_next();
            self.timer = self.next_stretch();
        }
    }

    /// Called when a Lua rule sets the weather directly.
    pub fn set(&mut self, weather: Weather) {
        self.current = weather;
        self.timer = self.next_stretch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_WEATHERS: [Weather; 5] = [
        Weather::Sunny,
        Weather::Rain,
        Weather::Mist,
        Weather::Storm,
        Weather::Windy,
    ];

    #[test]
    fn name_and_from_name_round_trip_for_every_weather() {
        for w in ALL_WEATHERS {
            assert_eq!(Weather::from_name(w.name()), Some(w));
        }
    }

    #[test]
    fn from_name_accepts_clear_as_a_backward_compatible_alias_for_sunny() {
        assert_eq!(Weather::from_name("clear"), Some(Weather::Sunny));
        assert_eq!(Weather::from_name("CLEAR"), Some(Weather::Sunny));
    }

    #[test]
    fn from_name_is_case_insensitive_and_rejects_unknown_names() {
        assert_eq!(Weather::from_name("STORM"), Some(Weather::Storm));
        assert_eq!(Weather::from_name("Windy"), Some(Weather::Windy));
        assert_eq!(Weather::from_name("blizzard"), None);
    }

    #[test]
    fn to_u8_and_from_u8_round_trip_for_every_weather_and_default_to_sunny() {
        for w in ALL_WEATHERS {
            assert_eq!(Weather::from_u8(w.to_u8()), w);
        }
        assert_eq!(Weather::from_u8(255), Weather::Sunny);
    }

    #[test]
    fn storm_and_windy_increase_wind_strength_above_baseline_mist_reduces_it() {
        let baseline = Weather::Sunny.wind_strength();
        assert_eq!(Weather::Rain.wind_strength(), baseline);
        assert!(Weather::Storm.wind_strength() > baseline);
        assert!(Weather::Windy.wind_strength() > baseline);
        assert!(Weather::Storm.wind_strength() > Weather::Windy.wind_strength());
        assert!(Weather::Mist.wind_strength() < baseline);
    }

    #[test]
    fn only_rain_and_storm_render_rain_particles() {
        assert!(Weather::Rain.has_rain_particles());
        assert!(Weather::Storm.has_rain_particles());
        assert!(!Weather::Sunny.has_rain_particles());
        assert!(!Weather::Mist.has_rain_particles());
        assert!(!Weather::Windy.has_rain_particles());
    }

    #[test]
    fn only_storm_has_lightning() {
        assert!(Weather::Storm.has_lightning());
        assert!(!Weather::Sunny.has_lightning());
        assert!(!Weather::Rain.has_lightning());
        assert!(!Weather::Mist.has_lightning());
        assert!(!Weather::Windy.has_lightning());
    }

    #[test]
    fn only_storm_has_heavy_cloud_coverage() {
        assert!(Weather::Storm.cloud_coverage() > 0.5);
        for w in [Weather::Sunny, Weather::Rain, Weather::Mist, Weather::Windy] {
            assert_eq!(w.cloud_coverage(), 0.0, "expected {} to have no cloud cover yet", w.name());
        }
    }

    #[test]
    fn storm_stretches_are_shorter_than_sunny_stretches() {
        let (storm_min, storm_max) = Weather::Storm.stretch_range();
        let (sunny_min, sunny_max) = Weather::Sunny.stretch_range();
        assert!(storm_max <= sunny_min, "expected every storm stretch to be shorter than every sunny stretch");
        assert!(storm_min > 0.0 && sunny_max > storm_max);
    }

    #[test]
    fn new_weather_state_starts_sunny() {
        let state = WeatherState::new(1);
        assert_eq!(state.current, Weather::Sunny);
    }

    #[test]
    fn set_applies_immediately_and_resets_the_timer_to_a_positive_stretch() {
        let mut state = WeatherState::new(1);
        state.set(Weather::Storm);
        assert_eq!(state.current, Weather::Storm);
        assert!(state.timer > 0.0);
    }

    /// Statistical sanity check on `pick_next`'s weighting: over many
    /// picks, sunny should come up noticeably more often than storm (the
    /// lightest weight), matching WEATHER_WEIGHTS' 45 vs. 8. Uses a large
    /// sample and a generous margin so this doesn't flake.
    #[test]
    fn pick_next_favors_sunny_over_storm_across_many_picks() {
        let mut state = WeatherState::new(7);
        let mut sunny_count = 0u32;
        let mut storm_count = 0u32;
        for _ in 0..5000 {
            match state.pick_next() {
                Weather::Sunny => sunny_count += 1,
                Weather::Storm => storm_count += 1,
                _ => {}
            }
        }
        assert!(
            sunny_count > storm_count * 3,
            "expected sunny (weight 45) to be picked well over 3x as often as storm (weight 8) \
             across 5000 picks; got sunny={sunny_count}, storm={storm_count}"
        );
    }

    #[test]
    fn update_does_not_change_weather_before_the_timer_expires() {
        let mut state = WeatherState::new(1);
        let starting = state.current;
        // Sunny's shortest possible stretch is 120s; a single small dt
        // can't possibly exhaust it.
        state.update(0.01);
        assert_eq!(state.current, starting);
    }

    #[test]
    fn update_picks_a_new_weather_once_the_timer_expires() {
        let mut state = WeatherState::new(1);
        state.timer = 0.01;
        state.update(1.0);
        // A new weather was picked and a fresh positive stretch started --
        // this doesn't assert *which* weather (that's pick_next's job,
        // covered above), just that the timer mechanism actually fires.
        assert!(state.timer > 0.0);
    }
}
