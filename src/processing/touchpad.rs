 use crate::events::InputEvent;
use crate::state::Seat;
use crate::backend::DeviceInfo;
use super::{Filter, FilterResult};
use smallvec::smallvec;

const TP_MAGIC_SLOWDOWN: f64 = 0.2968;
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
    cumulative_dx: f64,
    cumulative_dy: f64,
    last_velocity: f64,
    last_accel_factor: f64,

    threshold: f64,
    upper_threshold: f64,
    speed_factor: f64,
    accel_factor_const: f64,
}

impl TouchpadFilter {
    pub fn new(threshold_mm: f64, speed_adjustment: f64) -> Self {
        let speed_factor = (speed_adjustment + 1.0).powf(2.38) * 0.95 + 0.05;
        let mut f = Self {
            trackers: [Tracker::default(); 16],
            cur_tracker: 0,
            cumulative_dx: 0.0,
            cumulative_dy: 0.0,
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

        for offset in 1..16 {
            let index = (cur + 16 - offset) & 15;
            let tracker = &self.trackers[index];

            if tracker.time == 0 || tracker.time > time { break; }
            let tdelta = time - tracker.time + 1;
            if tdelta > MOTION_TIMEOUT { break; }

            let dx = self.cumulative_dx - tracker.dx;
            let dy = self.cumulative_dy - tracker.dy;
            let velocity = (dx * dx + dy * dy).sqrt() / tdelta as f64;

            if initial_velocity == 0.0 || offset <= 2 {
                result = velocity;
                initial_velocity = velocity;
            } else {
                if (initial_velocity - velocity).abs() > MAX_VELOCITY_DIFF { break; }
                result = velocity;
            }
        }
        result
    }

    #[inline(always)]
    fn accel_profile(&self, velocity: f64) -> f64 {
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
}

impl Filter for TouchpadFilter {
    fn process(&mut self, event: InputEvent, _seat: Option<&Seat>, info: Option<DeviceInfo>) -> FilterResult {
        match event {
            InputEvent::PointerMotion { device_index, time_us, dx, dy, dx_unaccelerated, dy_unaccelerated } => {
                let res_x = info.as_ref().map(|i| i.x.resolution).unwrap_or(10.0).max(1.0);
                let res_y = info.as_ref().map(|i| i.y.resolution).unwrap_or(10.0).max(1.0);

                let mm_dx = dx / res_x;
                let mm_dy = dy / res_y;

                self.cumulative_dx += mm_dx;
                self.cumulative_dy += mm_dy;

                self.cur_tracker = (self.cur_tracker + 1) & 15;
                self.trackers[self.cur_tracker] = Tracker {
                    dx: self.cumulative_dx,
                    dy: self.cumulative_dy,
                    time: time_us,
                };

                let velocity = self.calculate_velocity(time_us);

                let current_accel_factor = self.accel_profile(velocity);
                let mid_accel_factor = self.accel_profile((self.last_velocity + velocity) * 0.5);

                let accel_factor = (current_accel_factor + self.last_accel_factor + 4.0 * mid_accel_factor) * (1.0 / 6.0);

                self.last_velocity = velocity;
                self.last_accel_factor = current_accel_factor;

                smallvec![InputEvent::PointerMotion {
                    device_index,
                    time_us,
                    dx: (dx * accel_factor) as f64,
                    dy: (dy * accel_factor) as f64,
                    dx_unaccelerated,
                    dy_unaccelerated,
                }]
            }
            _ => smallvec![event],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::AxisInfo;
     use crate::events::DevIndex;

    fn mock_info(res: f64) -> DeviceInfo {
        DeviceInfo {
            x: AxisInfo { min: 0.0, max: 1000.0, resolution: res },
            y: AxisInfo { min: 0.0, max: 1000.0, resolution: res },
            ..Default::default()
        }
    }

    #[test]
    fn test_resolution_invariance() {
        let mut f1 = TouchpadFilter::new(130.0, 0.0);
        let mut f2 = TouchpadFilter::new(130.0, 0.0);
        let dev = DevIndex(0);

        let e1 = InputEvent::PointerMotion { device_index: dev, time_us: 1000, dx: 100.0, dy: 0.0, dx_unaccelerated: 100.0, dy_unaccelerated: 0.0 };
        let e2 = InputEvent::PointerMotion { device_index: dev, time_us: 1000, dx: 1000.0, dy: 0.0, dx_unaccelerated: 1000.0, dy_unaccelerated: 0.0 };

        let r1 = f1.process(e1, None, Some(mock_info(10.0)));
        let r2 = f2.process(e2, None, Some(mock_info(100.0)));

         if let (InputEvent::PointerMotion { dx: dx1, .. }, InputEvent::PointerMotion { dx: dx2, .. }) = (r1[0].clone(), r2[0].clone()) {
            let ratio1 = dx1 / 100.0;
            let ratio2 = dx2 / 1000.0;
            assert!((ratio1 - ratio2).abs() < 1e-5, "Accel ratios differ: {} vs {}", ratio1, ratio2);
        }
    }
}
