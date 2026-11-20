use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

/// Swaps BTN_LEFT (0x110) and BTN_RIGHT (0x111) for left-handed mode.
/// Passes all other events through unchanged.
pub struct ButtonSwapFilter;

impl ButtonSwapFilter {
    pub fn new() -> Self {
        Self
    }
}

impl Filter for ButtonSwapFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if let InputEvent::PointerButton { button, .. } = event {
            *button = match *button {
                0x110 => 0x111,
                0x111 => 0x110,
                other => other,
            };
        }
        Step::Keep
    }
}
