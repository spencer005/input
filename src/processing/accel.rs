use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::smallvec,
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
                smallvec![InputEvent::PointerMotion {
                    device_index,
                    time_us,
                    dx: dx * self.factor,
                    dy: dy * self.factor,
                    dx_unaccelerated: dx_unaccelerated,
                    dy_unaccelerated: dy_unaccelerated,
                }]
            }
            _ => smallvec![event],
        }
    }
}
