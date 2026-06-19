use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::smallvec,
};

const TP_MAGIC_SLOWDOWN: f64 = 0.2968;
const NORMALIZED_TOUCHPAD_UNITS_PER_MM: f64 = 10.0;
const MOTION_TIMEOUT: u64 = 1_000_000;
const MAX_VELOCITY_DIFF: f64 = 0.001; // mm/us

#[derive(Clone, Copy, Default)]
struct Tracker {
    dx: f64,
    dy: f64,
    time: u64,
}

pub struct TouchpadFilter {
    trackers: [Tracker; 16],
    cur_tracker: usize,
    last_velocity: f64,
    last_accel_factor: f64,

    threshold: f64,
    upper_threshold: f64,
    speed_factor: f64,
    accel_factor_const: f64,
}

impl TouchpadFilter {
    pub fn new(threshold_mm: f64, speed_adjustment: f64) -> Self {
        let threshold_mm = if threshold_mm.is_finite() && threshold_mm > 0.0 {
            threshold_mm
        } else {
            130.0
        };
        let speed_adjustment = if speed_adjustment.is_finite() {
            speed_adjustment.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let speed_factor = (speed_adjustment + 1.0).powf(2.38) * 0.95 + 0.05;
        let mut f = Self {
            trackers: [Tracker::default(); 16],
            cur_tracker: 0,
            last_velocity: 0.0,
            last_accel_factor: 0.0,
            threshold: threshold_mm,
            upper_threshold: threshold_mm * 4.0,
            speed_factor: speed_factor * TP_MAGIC_SLOWDOWN,
            accel_factor_const: 0.0025 / threshold_mm,
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
            return self.speed_factor;
        }

        let speed_mm_ms = velocity * 1000.0;
        let baseline: f64 = 0.9;
        let factor;

        if speed_mm_ms < 7.0 {
            factor = baseline.min(0.1 * speed_mm_ms + 0.3);
        } else if speed_mm_ms < self.threshold {
            factor = baseline;
        } else {
            let s = speed_mm_ms.min(self.upper_threshold);
            factor = self.accel_factor_const * s * (s - self.threshold) + baseline;
        }

        factor * self.speed_factor
    }

    pub fn reset_runtime_state(&mut self) {
        self.trackers = [Tracker::default(); 16];
        self.cur_tracker = 0;
        self.last_velocity = 0.0;
        self.last_accel_factor = self.accel_profile(0.0);
    }

    fn normalized_resolution(resolution: f64) -> f64 {
        if resolution.is_finite() && resolution > 0.0 {
            resolution
        } else {
            NORMALIZED_TOUCHPAD_UNITS_PER_MM
        }
    }
}

impl Filter for TouchpadFilter {
    fn process(
        &mut self,
        event: InputEvent,
        _seat: Option<&Seat>,
        info: Option<DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::PointerMotion {
                device_index,
                time_us,
                dx,
                dy,
                dx_unaccelerated,
                dy_unaccelerated,
            } => {
                if !dx.is_finite()
                    || !dy.is_finite()
                    || !dx_unaccelerated.is_finite()
                    || !dy_unaccelerated.is_finite()
                {
                    self.reset_runtime_state();
                    return smallvec![];
                }

                let res_x = Self::normalized_resolution(
                    info.as_ref()
                        .map(|i| i.x.resolution)
                        .unwrap_or(NORMALIZED_TOUCHPAD_UNITS_PER_MM),
                );
                let res_y = Self::normalized_resolution(
                    info.as_ref()
                        .map(|i| i.y.resolution)
                        .unwrap_or(NORMALIZED_TOUCHPAD_UNITS_PER_MM),
                );

                let mm_dx = dx / res_x;
                let mm_dy = dy / res_y;
                let norm_dx = mm_dx * NORMALIZED_TOUCHPAD_UNITS_PER_MM;
                let norm_dy = mm_dy * NORMALIZED_TOUCHPAD_UNITS_PER_MM;
                let norm_dx_unaccelerated =
                    dx_unaccelerated / res_x * NORMALIZED_TOUCHPAD_UNITS_PER_MM;
                let norm_dy_unaccelerated =
                    dy_unaccelerated / res_y * NORMALIZED_TOUCHPAD_UNITS_PER_MM;

                if !mm_dx.is_finite()
                    || !mm_dy.is_finite()
                    || !norm_dx.is_finite()
                    || !norm_dy.is_finite()
                    || !norm_dx_unaccelerated.is_finite()
                    || !norm_dy_unaccelerated.is_finite()
                {
                    self.reset_runtime_state();
                    return smallvec![];
                }

                self.cur_tracker = (self.cur_tracker + 1) & 15;
                self.trackers[self.cur_tracker] = Tracker {
                    dx: mm_dx,
                    dy: mm_dy,
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
                    dx_unaccelerated: norm_dx_unaccelerated,
                    dy_unaccelerated: norm_dy_unaccelerated,
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
    use {
        super::*,
        crate::{backend::AxisInfo, events::DevIndex},
    };

    fn mock_info(res: f64) -> DeviceInfo {
        DeviceInfo {
            x: AxisInfo {
                min: 0.0,
                max: 1000.0,
                resolution: res,
            },
            y: AxisInfo {
                min: 0.0,
                max: 1000.0,
                resolution: res,
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_resolution_invariance() {
        let mut f1 = TouchpadFilter::new(130.0, 0.0);
        let mut f2 = TouchpadFilter::new(130.0, 0.0);
        let dev = DevIndex(0);

        let e1 = InputEvent::PointerMotion {
            device_index: dev,
            time_us: 1000,
            dx: 100.0,
            dy: 0.0,
            dx_unaccelerated: 100.0,
            dy_unaccelerated: 0.0,
        };
        let e2 = InputEvent::PointerMotion {
            device_index: dev,
            time_us: 1000,
            dx: 1000.0,
            dy: 0.0,
            dx_unaccelerated: 1000.0,
            dy_unaccelerated: 0.0,
        };

        let r1 = f1.process(e1, None, Some(mock_info(10.0)));
        let r2 = f2.process(e2, None, Some(mock_info(100.0)));

        if let (
            InputEvent::PointerMotion { dx: dx1, .. },
            InputEvent::PointerMotion { dx: dx2, .. },
        ) = (r1[0].clone(), r2[0].clone())
        {
            assert!(
                (dx1 - dx2).abs() < 1e-5,
                "Same physical motion should produce equal output: {} vs {}",
                dx1,
                dx2
            );
        }
    }
}
