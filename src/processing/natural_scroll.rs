use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

/// Inverts scroll direction for natural/reverse scrolling.
/// When enabled, vertical and horizontal scroll deltas are negated.
/// This works for both mouse wheel scroll and touchpad scroll events.
pub struct NaturalScrollFilter {
    enabled: bool,
}

impl NaturalScrollFilter {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Toggle natural scrolling in place.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Filter for NaturalScrollFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if self.enabled {
            if let InputEvent::PointerScroll { dx, dy, .. } = event {
                *dx = -*dx;
                *dy = -*dy;
            }
        }
        Step::Keep
    }
}
