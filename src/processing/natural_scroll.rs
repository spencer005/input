use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::smallvec,
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
}

impl Filter for NaturalScrollFilter {
    fn process(
        &mut self,
        event: InputEvent,
        _seat: Option<&Seat>,
        _info: Option<DeviceInfo>,
    ) -> FilterResult {
        if !self.enabled {
            return smallvec![event];
        }

        match event {
            InputEvent::PointerScroll {
                device_index,
                time_us,
                source,
                dx,
                dy,
            } => {
                smallvec![InputEvent::PointerScroll {
                    device_index,
                    time_us,
                    source,
                    dx: -dx,
                    dy: -dy,
                }]
            }
            _ => smallvec![event],
        }
    }
}
