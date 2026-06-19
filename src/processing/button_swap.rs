use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::smallvec,
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
        event: InputEvent,
        _seat: Option<&Seat>,
        _info: Option<DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::PointerButton {
                device_index,
                time_us,
                button,
                pressed,
            } => {
                let swapped = match button {
                    0x110 => 0x111, // BTN_LEFT → BTN_RIGHT
                    0x111 => 0x110, // BTN_RIGHT → BTN_LEFT
                    other => other,
                };
                smallvec![InputEvent::PointerButton {
                    device_index,
                    time_us,
                    button: swapped,
                    pressed,
                }]
            }
            InputEvent::Key {
                device_index,
                time_us,
                key,
                pressed,
            } => {
                // Some keys like BTN_0/BTN_1 on pointer devices could also
                // be remapped, but for standard left-handed mode we only
                // swap the primary buttons.
                smallvec![InputEvent::Key {
                    device_index,
                    time_us,
                    key,
                    pressed
                }]
            }
            _ => smallvec![event],
        }
    }
}
