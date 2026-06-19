use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::smallvec,
};

#[derive(Default)]
struct HysteresisState {
    pending_dx: f64,
    pending_dy: f64,
    pending_dx_unaccelerated: f64,
    pending_dy_unaccelerated: f64,
}

pub struct HysteresisFilter {
    threshold_sq: f64,
    state: HysteresisState,
}

impl HysteresisFilter {
    pub fn new(threshold: f64) -> Self {
        let threshold = if threshold.is_finite() && threshold > 0.0 {
            threshold
        } else {
            0.0
        };
        Self {
            threshold_sq: threshold * threshold,
            state: HysteresisState::default(),
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.state = HysteresisState::default();
    }
}

impl Filter for HysteresisFilter {
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

                let state = &mut self.state;
                if !state.pending_dx.is_finite()
                    || !state.pending_dy.is_finite()
                    || !state.pending_dx_unaccelerated.is_finite()
                    || !state.pending_dy_unaccelerated.is_finite()
                {
                    *state = HysteresisState::default();
                }

                state.pending_dx += dx;
                state.pending_dy += dy;
                state.pending_dx_unaccelerated += dx_unaccelerated;
                state.pending_dy_unaccelerated += dy_unaccelerated;

                if !state.pending_dx.is_finite()
                    || !state.pending_dy.is_finite()
                    || !state.pending_dx_unaccelerated.is_finite()
                    || !state.pending_dy_unaccelerated.is_finite()
                {
                    *state = HysteresisState::default();
                    return smallvec![];
                }

                let dist_sq =
                    state.pending_dx * state.pending_dx + state.pending_dy * state.pending_dy;
                if !dist_sq.is_finite() {
                    *state = HysteresisState::default();
                    return smallvec![];
                }

                if dist_sq >= self.threshold_sq {
                    let dx = state.pending_dx;
                    let dy = state.pending_dy;
                    let dx_unaccelerated = state.pending_dx_unaccelerated;
                    let dy_unaccelerated = state.pending_dy_unaccelerated;
                    *state = HysteresisState::default();
                    smallvec![InputEvent::PointerMotion {
                        device_index,
                        time_us,
                        dx,
                        dy,
                        dx_unaccelerated,
                        dy_unaccelerated,
                    }]
                } else {
                    smallvec![]
                }
            }
            InputEvent::DeviceRemoved { .. } => {
                self.reset_runtime_state();
                smallvec![event]
            }
            _ => smallvec![event],
        }
    }
}
