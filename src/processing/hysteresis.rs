 use crate::events::InputEvent;
use crate::state::Seat;
use crate::backend::DeviceInfo;
use super::{Filter, FilterResult};
use smallvec::smallvec;

#[derive(Default)]
struct HysteresisState {
    last_x: f64,
    last_y: f64,
    initialized: bool,
}

pub struct HysteresisFilter {
    threshold_sq: f64,
    state: HysteresisState,
}

impl HysteresisFilter {
    pub fn new(threshold: f64) -> Self {
        Self {
            threshold_sq: threshold * threshold,
            state: HysteresisState::default(),
        }
    }
}

impl Filter for HysteresisFilter {
    fn process(&mut self, event: InputEvent, _seat: Option<&Seat>, _info: Option<DeviceInfo>) -> FilterResult {
        match event {
             InputEvent::PointerMotion { device_index: _, time_us: _, dx, dy, .. } => {
                let state = &mut self.state;
                if !state.initialized {
                    state.initialized = true;
                    state.last_x = dx;
                    state.last_y = dy;
                    return smallvec![event];
                }

                let diff_x = dx - state.last_x;
                let diff_y = dy - state.last_y;
                let dist_sq = diff_x * diff_x + diff_y * diff_y;

                if dist_sq > self.threshold_sq {
                    state.last_x = dx;
                    state.last_y = dy;
                    smallvec![event]
                } else {
                    smallvec![]
                }
            }
            _ => smallvec![event],
        }
    }
}
