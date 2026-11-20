use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
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
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        let InputEvent::PointerMotion {
            dx,
            dy,
            dx_unaccelerated,
            dy_unaccelerated,
            ..
        } = event
        else {
            return Step::Keep;
        };

        let state = &mut self.state;
        state.pending_dx += *dx;
        state.pending_dy += *dy;
        state.pending_dx_unaccelerated += *dx_unaccelerated;
        state.pending_dy_unaccelerated += *dy_unaccelerated;

        let dist_sq = state.pending_dx * state.pending_dx + state.pending_dy * state.pending_dy;
        if !dist_sq.is_finite() {
            *state = HysteresisState::default();
            return Step::Suppress;
        }

        if dist_sq >= self.threshold_sq {
            *dx = state.pending_dx;
            *dy = state.pending_dy;
            *dx_unaccelerated = state.pending_dx_unaccelerated;
            *dy_unaccelerated = state.pending_dy_unaccelerated;
            *state = HysteresisState::default();
            Step::Keep
        } else {
            Step::Suppress
        }
    }
}
