use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::SmallVec,
};

const MOTION_TIMEOUT: u64 = 1_000_000;
const MAX_VELOCITY_DIFF: f64 = 0.001; // units/us

/// Velocity is estimated from samples in this time window, independent of the
/// device polling rate (up to 8 kHz with the 128-entry ring).
const VELOCITY_WINDOW_US: u64 = 16_000;
const TRACKER_SAMPLES: usize = 128;
const TRACKER_MASK: usize = TRACKER_SAMPLES - 1;

/// Normalized counts per inch used by the DPI normalization layer.
const NORMALIZED_COUNTS_PER_INCH: f64 = 1000.0;
/// Conversion from normalized counts/µs to mm/s at 1000 counts per inch.
const MM_PER_SECOND_PER_COUNT_PER_US: f64 = 25.4 / NORMALIZED_COUNTS_PER_INCH * 1_000_000.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MouseConfigError {
    EmptyCustomCurve,
    NonFiniteCustomPoint { index: usize },
    NonPositiveCustomFactor { index: usize },
    NonMonotonicCustomSpeed { index: usize },
    MoreThanEightCustomPoints,
}

impl std::fmt::Display for MouseConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MouseConfigError::EmptyCustomCurve => {
                f.write_str("custom acceleration curve has no points")
            }
            MouseConfigError::NonFiniteCustomPoint { index } => {
                write!(f, "custom acceleration point {index} is not finite")
            }
            MouseConfigError::NonPositiveCustomFactor { index } => {
                write!(
                    f,
                    "custom acceleration point {index} has a non-positive factor"
                )
            }
            MouseConfigError::NonMonotonicCustomSpeed { index } => {
                write!(
                    f,
                    "custom acceleration speed {index} is not strictly increasing"
                )
            }
            MouseConfigError::MoreThanEightCustomPoints => {
                f.write_str("custom acceleration curve has more than 8 points")
            }
        }
    }
}

impl std::error::Error for MouseConfigError {}

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
    /// Validate the configuration, especially custom acceleration curves.
    ///
    /// Custom points are `(speed_mm_per_s, factor)` and must be finite, have a
    /// positive factor, and use strictly increasing speeds.
    pub fn validate(&self) -> Result<(), MouseConfigError> {
        let MouseAccelProfile::Custom(points) = &self.profile else {
            return Ok(());
        };

        if points.is_empty() {
            return Err(MouseConfigError::EmptyCustomCurve);
        }
        if points.len() > 8 {
            return Err(MouseConfigError::MoreThanEightCustomPoints);
        }

        for (index, (speed, factor)) in points.iter().enumerate() {
            if !speed.is_finite() || !factor.is_finite() || *speed < 0.0 {
                return Err(MouseConfigError::NonFiniteCustomPoint { index });
            }
            if *factor <= 0.0 {
                return Err(MouseConfigError::NonPositiveCustomFactor { index });
            }
            if index > 0 && *speed <= points[index - 1].0 {
                return Err(MouseConfigError::NonMonotonicCustomSpeed { index });
            }
        }
        Ok(())
    }

    pub fn validated(self) -> Result<Self, MouseConfigError> {
        self.validate()?;
        Ok(self)
    }

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
    trackers: [Tracker; TRACKER_SAMPLES],
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

    /// Construct a filter, rejecting an invalid custom acceleration curve.
    pub fn try_with_config(config: MouseConfig) -> Result<Self, MouseConfigError> {
        config.validate()?;
        Ok(Self::from_validated_config(config))
    }

    /// Construct a filter from configuration.
    ///
    /// Invalid custom curves are rejected with a diagnostic and fall back to
    /// the flat profile. Use [`MouseFilter::try_with_config`] when the caller
    /// wants the validation error.
    pub fn with_config(config: MouseConfig) -> Self {
        match config.validate() {
            Ok(()) => Self::from_validated_config(config),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "input-rs: invalid mouse acceleration curve; using flat profile"
                );
                Self::from_validated_config(MouseConfig {
                    profile: MouseAccelProfile::Flat,
                    ..config
                })
            }
        }
    }

    fn from_validated_config(config: MouseConfig) -> Self {
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
            trackers: [Tracker::default(); TRACKER_SAMPLES],
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
        let mut accepted = 0usize;

        for offset in 1..TRACKER_SAMPLES {
            let index = (cur + TRACKER_SAMPLES - offset) & TRACKER_MASK;
            let tracker = &self.trackers[index];

            if tracker.time == 0 || tracker.time > time {
                break;
            }

            let age = time.saturating_sub(tracker.time);
            if age > MOTION_TIMEOUT {
                break;
            }

            // Normally stop at the time window. Low-polling-rate devices may
            // have only one sample inside it; accept that first interval before
            // applying the window limit so velocity does not collapse to zero.
            if age > VELOCITY_WINDOW_US && accepted > 0 {
                break;
            }

            // The tracker at `index` is older than the newest sample; add the
            // displacement of the sample immediately newer than it so the
            // velocity for the newest movement is not dropped.
            let delta_index = (cur + TRACKER_SAMPLES - offset + 1) & TRACKER_MASK;
            let delta = &self.trackers[delta_index];
            dx += delta.dx;
            dy += delta.dy;
            accepted += 1;

            let tdelta = age + 1;
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
                // Piecewise-linear interpolation over (speed_mm_per_s, factor)
                // points. Trackers store normalized counts/µs, so convert to
                // physical mm/s before selecting a curve point.
                let speed = velocity * MM_PER_SECOND_PER_COUNT_PER_US;
                if !speed.is_finite() {
                    return 1.0;
                }
                // The curve was validated at construction; the hot path only
                // performs the lookup/interpolation.
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
        self.trackers = [Tracker::default(); TRACKER_SAMPLES];
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
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        let time_us = event.time_us();
        let InputEvent::PointerMotion {
            dx,
            dy,
            dx_unaccelerated,
            dy_unaccelerated,
            ..
        } = event
        else {
            if matches!(event, InputEvent::DeviceRemoved { .. }) {
                self.reset_runtime_state();
            }
            return Step::Keep;
        };

        let norm_dx = *dx * self.dpi_factor;
        let norm_dy = *dy * self.dpi_factor;
        if !norm_dx.is_finite() || !norm_dy.is_finite() {
            self.reset_runtime_state();
            return Step::Suppress;
        }

        let (out_dx, out_dy) = if self.profile == MouseAccelProfile::Flat {
            let factor = self.accel_profile(0.0);
            (norm_dx * factor, norm_dy * factor)
        } else {
            self.cur_tracker = (self.cur_tracker + 1) & TRACKER_MASK;
            self.trackers[self.cur_tracker] = Tracker {
                dx: norm_dx,
                dy: norm_dy,
                time: time_us,
            };

            let velocity = self.calculate_velocity(time_us);
            let current_accel_factor = self.accel_profile(velocity);
            let mid_accel_factor = self.accel_profile((self.last_velocity + velocity) * 0.5);
            let accel_factor =
                (current_accel_factor + self.last_accel_factor + 4.0 * mid_accel_factor)
                    * (1.0 / 6.0);
            if !accel_factor.is_finite() {
                self.reset_runtime_state();
                return Step::Suppress;
            }
            self.last_velocity = velocity;
            self.last_accel_factor = current_accel_factor;
            (norm_dx * accel_factor, norm_dy * accel_factor)
        };

        if !out_dx.is_finite() || !out_dy.is_finite() {
            self.reset_runtime_state();
            return Step::Suppress;
        }

        *dx = out_dx;
        *dy = out_dy;
        *dx_unaccelerated = norm_dx;
        *dy_unaccelerated = norm_dy;
        Step::Keep
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::processing::TestFilterExt};

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
        let result = filter.run(
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
        let result = filter_400.run(
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

        let result = filter.run(
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
        let result = filter_slow.run(
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
        let result = filter_fast.run(
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
    fn velocity_is_stable_across_polling_rates() {
        fn build(samples: usize, step_us: u64) -> MouseFilter {
            let mut filter = MouseFilter::new(1000, 0.0);
            let delta = step_us as f64 / 1000.0; // same physical speed
            for index in 0..samples {
                filter.trackers[index] = Tracker {
                    dx: delta,
                    dy: 0.0,
                    time: 1_000 + index as u64 * step_us,
                };
            }
            filter.cur_tracker = samples - 1;
            filter
        }

        let low = build(16, 1_000); // 1 kHz, 15 ms
        let high = build(128, 125); // 8 kHz, ~15.9 ms
        let low_velocity = low.calculate_velocity(1_000 + 15 * 1_000);
        let high_velocity = high.calculate_velocity(1_000 + 127 * 125);

        let relative = (low_velocity - high_velocity).abs() / low_velocity.max(1e-12);
        assert!(low_velocity > 0.0);
        assert!(
            relative < 0.01,
            "velocity should be polling-rate independent: low={low_velocity}, high={high_velocity}"
        );
    }

    #[test]
    fn low_polling_rate_still_produces_a_velocity() {
        let mut filter = MouseFilter::new(1000, 0.0);
        // 30 Hz polling: the previous sample is older than the 16 ms window,
        // but one interval must still be accepted.
        filter.trackers[0] = Tracker {
            dx: 0.0,
            dy: 0.0,
            time: 1_000,
        };
        filter.trackers[1] = Tracker {
            dx: 3.0,
            dy: 4.0,
            time: 34_000,
        };
        filter.cur_tracker = 1;

        let velocity = filter.calculate_velocity(34_000);
        assert!(velocity > 0.0);
        assert!((velocity - 5.0 / 33_001.0).abs() < 1e-12);
    }

    #[test]
    fn velocity_estimate_includes_the_newest_movement_sample() {
        let mut filter = MouseFilter::new(1000, 0.0);
        // Two samples: 1 normalized count at t=1ms and 1 at t=2ms.
        filter.trackers[0] = Tracker {
            dx: 1.0,
            dy: 0.0,
            time: 1_000,
        };
        filter.trackers[1] = Tracker {
            dx: 1.0,
            dy: 0.0,
            time: 2_000,
        };
        filter.cur_tracker = 1;

        let velocity = filter.calculate_velocity(2_000);
        assert!(
            velocity > 0.0,
            "velocity must include the newest sample's displacement"
        );
        assert!((velocity - 1.0 / 1_001.0).abs() < 1e-12);
    }

    #[test]
    fn custom_curve_validation_rejects_invalid_configurations() {
        use smallvec::smallvec;

        let base = MouseConfig {
            dpi: 1000,
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Custom(smallvec![]),
        };
        assert_eq!(base.validate(), Err(MouseConfigError::EmptyCustomCurve));

        let unordered = MouseConfig {
            profile: MouseAccelProfile::Custom(smallvec![(10.0, 1.0), (5.0, 2.0)]),
            ..base.clone()
        };
        assert_eq!(
            unordered.validate(),
            Err(MouseConfigError::NonMonotonicCustomSpeed { index: 1 })
        );

        let negative_factor = MouseConfig {
            profile: MouseAccelProfile::Custom(smallvec![(0.0, 1.0), (10.0, -1.0)]),
            ..base.clone()
        };
        assert_eq!(
            negative_factor.validate(),
            Err(MouseConfigError::NonPositiveCustomFactor { index: 1 })
        );

        let non_finite = MouseConfig {
            profile: MouseAccelProfile::Custom(smallvec![(0.0, 1.0), (f64::NAN, 2.0)]),
            ..base
        };
        assert_eq!(
            non_finite.validate(),
            Err(MouseConfigError::NonFiniteCustomPoint { index: 1 })
        );
    }

    #[test]
    fn custom_curve_speed_axis_is_physical_mm_per_second() {
        use smallvec::smallvec;

        // Velocity is measured in normalized counts/µs. At 1000 counts/inch,
        // 0.01 counts/µs is 254 mm/s, which must land on the curve point.
        let config = MouseConfig {
            dpi: 1000,
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Custom(smallvec![(0.0, 1.0), (254.0, 3.0)]),
        };
        let filter = MouseFilter::try_with_config(config).unwrap();
        assert!((filter.accel_profile(0.01) - 3.0).abs() < 1e-12);
        assert!((filter.accel_profile(0.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_custom_curve_falls_back_without_panicking() {
        use smallvec::smallvec;

        let mut filter = MouseFilter::with_config(MouseConfig {
            dpi: 1000,
            speed_adjustment: 0.0,
            profile: MouseAccelProfile::Custom(smallvec![(10.0, 1.0), (5.0, 2.0)]),
        });
        let result = filter.run(
            InputEvent::PointerMotion {
                device_index: crate::events::DevIndex(0),
                time_us: 1000,
                dx: 5.0,
                dy: 5.0,
                dx_unaccelerated: 5.0,
                dy_unaccelerated: 5.0,
            },
            None,
            None,
        );
        assert_eq!(result.len(), 1);
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
        let result = filter.run(
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
            let result = filter.run(
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
