use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::{SmallVec, smallvec},
};

const MOTION_TIMEOUT: u64 = 1_000_000;
const MAX_VELOCITY_DIFF: f64 = 0.001; // units/us

/// Configuration for mouse acceleration profile.
///
/// For a "flat" profile (no acceleration, just DPI scaling), use
/// `MouseConfig::flat(dpi)` which sets accel=1.0 with only a DPI factor.
///
/// For adaptive profiles, adjust `accel` and `threshold` as needed.
#[derive(Debug, Clone)]
pub struct MouseConfig {
    /// DPI of the mouse sensor. Used to normalize motion units.
    pub dpi: i32,
    /// Speed adjustment from -1.0 (slowest) to 1.0 (fastest).
    /// Affects threshold and acceleration curve.
    pub speed_adjustment: f64,
    /// The acceleration profile to use.
    pub profile: MouseAccelProfile,
}

/// Acceleration profile for mouse motion.
///
/// `Flat` applies no acceleration — just DPI normalization.
/// `Adaptive` uses the built-in threshold/incline curve.
/// `Custom` uses a user-provided piecewise-linear speed→factor curve.
#[derive(Debug, Clone, PartialEq)]
pub enum MouseAccelProfile {
    /// Flat: no acceleration, constant factor of 1.0 (only DPI scaling applied).
    Flat,
    /// Adaptive: uses the threshold/incline acceleration curve.
    Adaptive,
    /// Custom: piecewise-linear curve defined by (speed, factor) points.
    /// Speed values are in mm/s and must be monotonically increasing.
    /// Factor values are the acceleration multiplier at each speed.
    /// Linear interpolation is used between points.
    Custom(SmallVec<(f64, f64), 8>),
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            dpi: 1000,
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Flat,
        }
    }
}

impl MouseConfig {
    /// Create a flat (no-acceleration) config for the given DPI.
    pub fn flat(dpi: i32) -> Self {
        Self {
            dpi: normalize_dpi(dpi),
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Flat,
        }
    }

    /// Create a config from an existing MouseFilter's settings,
    /// changing only the fields that need updating.
    pub fn default_from_existing(filter: &MouseFilter) -> Self {
        Self {
            dpi: filter.dpi(),
            speed_adjustment: filter.speed_adjustment(),
            profile: filter.profile(),
        }
    }
}

fn normalize_dpi(dpi: i32) -> i32 {
    if dpi > 0 { dpi } else { 1000 }
}

pub fn parse_mouse_dpi_property(value: &str) -> Option<i32> {
    let mut fallback = None;
    let mut selected = None;
    let mut saw_token = false;

    for raw in value.split(|c: char| c.is_ascii_whitespace() || c == ',') {
        if raw.is_empty() {
            continue;
        }
        saw_token = true;

        let (is_selected, token) = match raw.strip_prefix('*') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };
        if token.is_empty() || token.contains('*') {
            return None;
        }

        let dpi_part = match token.split_once('@') {
            Some((dpi, rate)) => {
                if parse_positive_i32(rate).is_none() {
                    return None;
                }
                dpi
            }
            None => token,
        };

        let dpi = parse_positive_i32(dpi_part)?;
        if is_selected && selected.is_none() {
            selected = Some(dpi);
        }
        fallback = Some(dpi);
    }

    if !saw_token {
        return None;
    }

    selected.or(fallback)
}

fn parse_positive_i32(value: &str) -> Option<i32> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match value.parse::<i32>() {
        Ok(value) if value > 0 => Some(value),
        _ => None,
    }
}

#[derive(Clone, Copy, Default)]
struct Tracker {
    dx: f64,
    dy: f64,
    time: u64,
}

pub struct MouseFilter {
    trackers: [Tracker; 16],
    cur_tracker: usize,
    last_velocity: f64,
    last_accel_factor: f64,

    profile: MouseAccelProfile,
    speed_adjustment: f64,
    // Adaptive-only constants (pre-calculated)
    threshold: f64,
    accel: f64,
    incline_factor: f64,
    // DPI normalization
    dpi: i32,
    dpi_factor: f64,
}

impl MouseFilter {
    pub fn new(dpi: i32, speed_adjustment: f64) -> Self {
        Self::with_config(MouseConfig {
            dpi,
            speed_adjustment,
            profile: MouseAccelProfile::Adaptive,
        })
    }

    /// Create a mouse filter with flat (no acceleration) profile.
    /// Only applies DPI normalization.
    pub fn flat(dpi: i32) -> Self {
        Self::with_config(MouseConfig::flat(dpi))
    }

    pub fn with_config(config: MouseConfig) -> Self {
        let dpi = normalize_dpi(config.dpi);
        let dpi_factor = 1000.0 / dpi as f64;
        let speed_adjustment = normalize_speed_adjustment(config.speed_adjustment);

        let default_threshold = 0.4 / 1000.0;
        let threshold = match &config.profile {
            MouseAccelProfile::Flat | MouseAccelProfile::Custom(_) => 0.0,
            MouseAccelProfile::Adaptive => {
                (default_threshold - (0.25 / 1000.0) * speed_adjustment).max(0.2 / 1000.0)
            }
        };
        let accel = 2.0 + speed_adjustment * 1.5;
        let incline = 1.1 + speed_adjustment * 0.75;

        let mut f = Self {
            trackers: [Tracker::default(); 16],
            cur_tracker: 0,
            last_velocity: 0.0,
            last_accel_factor: 1.0,
            profile: config.profile,
            speed_adjustment,
            threshold,
            accel,
            incline_factor: incline * 1000.0,
            dpi,
            dpi_factor,
        };
        f.last_accel_factor = f.accel_profile(0.0);
        f
    }

    fn calculate_velocity(&self, time: u64) -> f64 {
        let mut result = 0.0;
        let mut initial_velocity = 0.0;
        let cur = self.cur_tracker;
        let mut dx = 0.0;
        let mut dy = 0.0;

        for offset in 1..16 {
            let index = (cur + 16 - offset) & 15;
            let tracker = &self.trackers[index];

            if tracker.time == 0 || tracker.time > time {
                break;
            }
            if offset > 1 {
                let delta_index = (cur + 16 - offset + 1) & 15;
                let delta = &self.trackers[delta_index];
                dx += delta.dx;
                dy += delta.dy;
            }

            let tdelta = time.saturating_sub(tracker.time) + 1;
            if tdelta > MOTION_TIMEOUT {
                break;
            }

            let velocity = (dx * dx + dy * dy).sqrt() / tdelta as f64;

            if initial_velocity == 0.0 || offset <= 2 {
                result = velocity;
                initial_velocity = velocity;
            } else {
                if (initial_velocity - velocity).abs() > MAX_VELOCITY_DIFF {
                    break;
                }
                result = velocity;
            }
        }
        result
    }

    #[inline(always)]
    fn accel_profile(&self, velocity: f64) -> f64 {
        if !velocity.is_finite() {
            return 1.0;
        }

        match &self.profile {
            MouseAccelProfile::Flat => {
                // Flat profile should be speed-independent (no adaptive accel),
                // but still honor user sensitivity strongly across the full range.
                // Keep this a simple linear scale so -0.96 behaves close to "very slow".
                1.0 + self.speed_adjustment
            }
            MouseAccelProfile::Adaptive => {
                let speed_ms = velocity * 1000.0;
                let factor;

                if speed_ms < 0.07 {
                    factor = 10.0 * speed_ms + 0.3;
                } else if velocity < self.threshold {
                    factor = 1.0;
                } else {
                    factor = self.incline_factor * (velocity - self.threshold) + 1.0;
                }

                factor.min(self.accel)
            }
            MouseAccelProfile::Custom(points) => {
                // Piecewise-linear interpolation over (speed_mm_per_s, factor) points.
                // Speed values are in mm/s (velocity * 1000 converts from units/us to mm/s).
                let speed = velocity * 1000.0;
                if !speed.is_finite() || points.is_empty() {
                    return 1.0;
                }
                if points
                    .iter()
                    .any(|(point_speed, factor)| !point_speed.is_finite() || !factor.is_finite())
                {
                    return 1.0;
                }
                // Below first point: use its factor
                if speed <= points[0].0 {
                    return points[0].1;
                }
                // Above last point: use last factor
                if speed >= points[points.len() - 1].0 {
                    return points[points.len() - 1].1;
                }
                // Linear interpolation between bracketing points
                for i in 0..points.len() - 1 {
                    if speed >= points[i].0 && speed < points[i + 1].0 {
                        let t = (speed - points[i].0) / (points[i + 1].0 - points[i].0);
                        return points[i].1 + t * (points[i + 1].1 - points[i].1);
                    }
                }
                // Fallback (should not reach here)
                points[points.len() - 1].1
            }
        }
    }

    /// Get the current DPI setting.
    pub fn dpi(&self) -> i32 {
        self.dpi
    }

    /// Get the current speed adjustment.
    pub fn speed_adjustment(&self) -> f64 {
        self.speed_adjustment
    }

    /// Get the current acceleration profile.
    pub fn profile(&self) -> MouseAccelProfile {
        self.profile.clone()
    }

    pub fn reset_runtime_state(&mut self) {
        self.trackers = [Tracker::default(); 16];
        self.cur_tracker = 0;
        self.last_velocity = 0.0;
        self.last_accel_factor = self.accel_profile(0.0);
    }
}

fn normalize_speed_adjustment(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

impl Filter for MouseFilter {
    fn process(
        &mut self,
        event: InputEvent,
        _seat: Option<&Seat>,
        _info: Option<DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::PointerMotion {
                device_index,
                time_us,
                dx,
                dy,
                dx_unaccelerated: _,
                dy_unaccelerated: _,
            } => {
                let norm_dx = dx * self.dpi_factor;
                let norm_dy = dy * self.dpi_factor;
                if !norm_dx.is_finite() || !norm_dy.is_finite() {
                    self.reset_runtime_state();
                    return smallvec![];
                }

                // Flat profile fast path: skip velocity tracking and acceleration.
                // Only DPI normalization + constant factor needed. No velocity
                // tracking required for the flat profile since accel is speed-independent.
                if self.profile == MouseAccelProfile::Flat {
                    let factor = self.accel_profile(0.0);
                    let out_dx = norm_dx * factor;
                    let out_dy = norm_dy * factor;
                    if !out_dx.is_finite() || !out_dy.is_finite() {
                        self.reset_runtime_state();
                        return smallvec![];
                    }
                    return smallvec![InputEvent::PointerMotion {
                        device_index,
                        time_us,
                        dx: out_dx,
                        dy: out_dy,
                        dx_unaccelerated: norm_dx,
                        dy_unaccelerated: norm_dy,
                    }];
                }

                self.cur_tracker = (self.cur_tracker + 1) & 15;
                self.trackers[self.cur_tracker] = Tracker {
                    dx: norm_dx,
                    dy: norm_dy,
                    time: time_us,
                };

                let velocity = self.calculate_velocity(time_us);

                let current_accel_factor = self.accel_profile(velocity);
                let mid_accel_factor = self.accel_profile((self.last_velocity + velocity) * 0.5);

                // Simpson's rule for smooth acceleration transitions
                let accel_factor =
                    (current_accel_factor + self.last_accel_factor + 4.0 * mid_accel_factor)
                        * (1.0 / 6.0);
                if !accel_factor.is_finite() {
                    self.reset_runtime_state();
                    return smallvec![];
                }

                self.last_velocity = velocity;
                self.last_accel_factor = current_accel_factor;
                let out_dx = norm_dx * accel_factor;
                let out_dy = norm_dy * accel_factor;
                if !out_dx.is_finite() || !out_dy.is_finite() {
                    self.reset_runtime_state();
                    return smallvec![];
                }

                smallvec![InputEvent::PointerMotion {
                    device_index,
                    time_us,
                    dx: out_dx,
                    dy: out_dy,
                    dx_unaccelerated: norm_dx,
                    dy_unaccelerated: norm_dy,
                }]
            }
            InputEvent::DeviceRemoved { .. } => {
                self.reset_runtime_state();
                smallvec![event]
            }
            _ => smallvec![event],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mouse_dpi_property() {
        let cases = [
            ("450 *1800 3200", Some(1800)),
            ("*450 1800 3200", Some(450)),
            ("450 1800 *3200", Some(3200)),
            ("450 1800 3200", Some(3200)),
            ("450 1800 failboat", None),
            ("450 1800 *failboat", None),
            ("0 450 1800 *3200", None),
            ("450@37 1800@12 *3200@6", Some(3200)),
            ("450@125 *1800@125 3200@125", Some(1800)),
            ("*this @string fails", None),
            ("12@34 *45@", None),
            ("12@a *45@", None),
            ("12@a *45@25", None),
            ("                                      * 12, 450, 800", None),
            (
                "                                      *12, 450, 800",
                Some(12),
            ),
            ("*12, *450, 800", Some(12)),
            ("*-23412, 450, 800", None),
            ("112@125, 450@125, 800@125, 900@-125", None),
            ("", None),
            ("   ", None),
            ("* ", None),
        ];

        for (value, expected) in cases {
            assert_eq!(parse_mouse_dpi_property(value), expected, "value={value:?}");
        }
    }

    #[test]
    fn test_non_positive_dpi_defaults_to_1000() {
        assert_eq!(MouseFilter::flat(0).dpi(), 1000);
        assert_eq!(MouseFilter::flat(-400).dpi(), 1000);

        let filter = MouseFilter::with_config(MouseConfig {
            dpi: 0,
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Adaptive,
        });
        assert_eq!(filter.dpi(), 1000);
    }

    #[test]
    fn test_flat_profile_returns_normalized_deltas() {
        let mut filter = MouseFilter::flat(1000);
        let dev = crate::events::DevIndex(0);

        // With dpi_factor = 1000/1000 = 1.0, output should equal input
        let result = filter.process(
            InputEvent::PointerMotion {
                device_index: dev,
                time_us: 1000,
                dx: 5.0,
                dy: -3.0,
                dx_unaccelerated: 5.0,
                dy_unaccelerated: -3.0,
            },
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        match &result[0] {
            InputEvent::PointerMotion {
                dx,
                dy,
                dx_unaccelerated,
                dy_unaccelerated,
                ..
            } => {
                assert!((dx - 5.0).abs() < 1e-10, "flat(1000): dx={dx}");
                assert!((dy - (-3.0)).abs() < 1e-10, "flat(1000): dy={dy}");
                assert!(
                    (dx_unaccelerated - 5.0).abs() < 1e-10,
                    "flat(1000): dx_unaccel={dx_unaccelerated}"
                );
                assert!(
                    (dy_unaccelerated - (-3.0)).abs() < 1e-10,
                    "flat(1000): dy_unaccel={dy_unaccelerated}"
                );
            }
            _ => panic!("expected PointerMotion"),
        }

        // With dpi_factor = 1000/400 = 2.5, output should be input * 2.5
        let mut filter_400 = MouseFilter::flat(400);
        let result = filter_400.process(
            InputEvent::PointerMotion {
                device_index: dev,
                time_us: 1000,
                dx: 1.0,
                dy: 1.0,
                dx_unaccelerated: 1.0,
                dy_unaccelerated: 1.0,
            },
            None,
            None,
        );
        match &result[0] {
            InputEvent::PointerMotion {
                dx,
                dy,
                dx_unaccelerated,
                dy_unaccelerated,
                ..
            } => {
                assert!((dx - 2.5).abs() < 1e-10, "flat(400): dx={dx}");
                assert!((dy - 2.5).abs() < 1e-10, "flat(400): dy={dy}");
                assert!(
                    (dx_unaccelerated - 2.5).abs() < 1e-10,
                    "flat(400): dx_unaccel={dx_unaccelerated}"
                );
                assert!(
                    (dy_unaccelerated - 2.5).abs() < 1e-10,
                    "flat(400): dy_unaccel={dy_unaccelerated}"
                );
            }
            _ => panic!("expected PointerMotion"),
        }
    }

    #[test]
    fn test_flat_profile_preserves_non_event_types() {
        let mut filter = MouseFilter::flat(1000);
        let dev = crate::events::DevIndex(0);

        let result = filter.process(
            InputEvent::Key {
                device_index: dev,
                time_us: 1000,
                key: 0x110,
                pressed: true,
            },
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        match &result[0] {
            InputEvent::Key { .. } => {}
            _ => panic!("expected Key event to pass through"),
        }
    }

    #[test]
    fn test_flat_profile_with_speed_adjustment() {
        // Flat profile with negative speed: should scale down movement
        // At speed=-0.96, factor = 1.0 + (-0.96) = 0.04
        let mut filter_slow = MouseFilter::with_config(MouseConfig {
            dpi: 1000,
            speed_adjustment: -0.96,
            profile: MouseAccelProfile::Flat,
        });
        let dev = crate::events::DevIndex(0);
        let result = filter_slow.process(
            InputEvent::PointerMotion {
                device_index: dev,
                time_us: 1000,
                dx: 10.0,
                dy: 10.0,
                dx_unaccelerated: 10.0,
                dy_unaccelerated: 10.0,
            },
            None,
            None,
        );
        match &result[0] {
            InputEvent::PointerMotion {
                dx,
                dy,
                dx_unaccelerated,
                dy_unaccelerated,
                ..
            } => {
                let expected = 10.0 * 0.04;
                assert!(
                    (dx - expected).abs() < 1e-10,
                    "flat(-0.96): dx={dx}, expected={expected}"
                );
                assert!(
                    (dy - expected).abs() < 1e-10,
                    "flat(-0.96): dy={dy}, expected={expected}"
                );
                assert!(
                    (dx_unaccelerated - 10.0).abs() < 1e-10,
                    "flat(-0.96): unaccel dx={dx_unaccelerated}"
                );
            }
            _ => panic!("expected PointerMotion"),
        }

        // Flat profile with positive speed: should scale up movement
        // At speed=+0.5, factor = 1.0 + 0.5 = 1.5
        let mut filter_fast = MouseFilter::with_config(MouseConfig {
            dpi: 1000,
            speed_adjustment: 0.5,
            profile: MouseAccelProfile::Flat,
        });
        let result = filter_fast.process(
            InputEvent::PointerMotion {
                device_index: dev,
                time_us: 2000,
                dx: 10.0,
                dy: 10.0,
                dx_unaccelerated: 10.0,
                dy_unaccelerated: 10.0,
            },
            None,
            None,
        );
        match &result[0] {
            InputEvent::PointerMotion { dx, dy, .. } => {
                let expected = 10.0 * 1.5;
                assert!(
                    (dx - expected).abs() < 1e-10,
                    "flat(+0.5): dx={dx}, expected={expected}"
                );
                assert!(
                    (dy - expected).abs() < 1e-10,
                    "flat(+0.5): dy={dy}, expected={expected}"
                );
            }
            _ => panic!("expected PointerMotion"),
        }
    }

    #[test]
    fn test_custom_profile_piecewise_linear() {
        use smallvec::smallvec;

        // Custom curve: (0 mm/s, 0.5), (10 mm/s, 1.0), (50 mm/s, 2.0)
        // Below 10mm/s: factor ramps from 0.5 to 1.0
        // Between 10-50mm/s: factor ramps from 1.0 to 2.0
        let points = smallvec![(0.0, 0.5), (10.0, 1.0), (50.0, 2.0)];
        let config = MouseConfig {
            dpi: 1000,
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Custom(points),
        };
        let mut filter = MouseFilter::with_config(config);
        let dev = crate::events::DevIndex(0);

        // At very low speed, expect factor close to 0.5 (first point)
        let result = filter.process(
            InputEvent::PointerMotion {
                device_index: dev,
                time_us: 1000,
                dx: 0.1,
                dy: 0.1,
                dx_unaccelerated: 0.1,
                dy_unaccelerated: 0.1,
            },
            None,
            None,
        );
        assert_eq!(result.len(), 1);
        // The factor should be around 0.5 for very slow motion
    }

    #[test]
    fn test_adaptive_profile_unaccelerated_differs_from_accelerated() {
        let mut filter = MouseFilter::new(1000, 0.0);
        let dev = crate::events::DevIndex(0);

        // Feed several fast motions to push past the threshold so acceleration kicks in
        for i in 0..20 {
            let result = filter.process(
                InputEvent::PointerMotion {
                    device_index: dev,
                    time_us: 1000 + i as u64 * 1000,
                    dx: 100.0,
                    dy: 100.0,
                    dx_unaccelerated: 100.0,
                    dy_unaccelerated: 100.0,
                },
                None,
                None,
            );
            assert_eq!(result.len(), 1);
            if i > 5 {
                // Once we're past the threshold, accelerated should exceed unaccelerated
                if let InputEvent::PointerMotion {
                    dx,
                    dy,
                    dx_unaccelerated,
                    dy_unaccelerated,
                    ..
                } = &result[0]
                {
                    // At high speed, accel_factor > 1.0, so |dx| > |dx_unaccelerated|
                    assert!(
                        dx.abs() > dx_unaccelerated.abs() || dx_unaccelerated.abs() < 1e-10,
                        "accelerated dx={dx} should exceed unaccelerated dx_unaccel={dx_unaccelerated} at high speed"
                    );
                }
            }
        }
    }
}
