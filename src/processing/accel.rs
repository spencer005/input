use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

pub struct ConstantAccelFilter {
    factor: f64,
}

impl ConstantAccelFilter {
    pub fn new(factor: f64) -> Self {
        Self { factor }
    }
}

impl Filter for ConstantAccelFilter {
    #[inline]
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if let InputEvent::PointerMotion { dx, dy, .. } = event {
            *dx *= self.factor;
            *dy *= self.factor;
        }
        Step::Keep
    }
}
