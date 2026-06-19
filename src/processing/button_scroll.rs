use {
    super::{Filter, FilterResult},
    crate::{
        backend::DeviceInfo,
        events::{InputEvent, ScrollSource},
        state::Seat,
    },
    smallvec::smallvec,
};

const BTN_MIDDLE: u32 = 0x112;

/// Converts relative pointer motion into continuous scroll while a button is held.
///
/// The scroll button is swallowed by design: a configured pointing-stick scroll
/// button should not also reach clients as an ordinary click.
pub struct ButtonScrollFilter {
    scroll_button: u32,
    active: bool,
    lock_enabled: bool,
}

impl ButtonScrollFilter {
    pub fn middle_button() -> Self {
        Self::new(BTN_MIDDLE)
    }

    pub fn new(scroll_button: u32) -> Self {
        Self {
            scroll_button,
            active: false,
            lock_enabled: false,
        }
    }

    pub fn with_lock(mut self, enabled: bool) -> Self {
        self.lock_enabled = enabled;
        self
    }

    pub fn reset_runtime_state(&mut self) {
        self.active = false;
    }

    fn handle_scroll_button(&mut self, pressed: bool) {
        if self.lock_enabled {
            if pressed {
                self.active = !self.active;
            }
        } else {
            self.active = pressed;
        }
    }
}

impl Filter for ButtonScrollFilter {
    fn process(
        &mut self,
        event: InputEvent,
        _seat: Option<&Seat>,
        _info: Option<DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::PointerButton {
                button, pressed, ..
            } if button == self.scroll_button => {
                self.handle_scroll_button(pressed);
                smallvec![]
            }
            InputEvent::PointerMotion {
                device_index,
                time_us,
                dx,
                dy,
                ..
            } if self.active => {
                smallvec![InputEvent::PointerScroll {
                    device_index,
                    time_us,
                    source: ScrollSource::Continuous,
                    dx,
                    dy,
                }]
            }
            InputEvent::DeviceRemoved { .. } => {
                self.active = false;
                smallvec![event]
            }
            other => smallvec![other],
        }
    }
}
