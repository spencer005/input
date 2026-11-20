use {
    super::{Filter, Step},
    crate::{
        backend::DeviceInfo,
        events::{InputEvent, ScrollSource},
        state::Seat,
    },
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
    enabled: bool,
    /// After disengaging while the scroll button is held, swallow its release
    /// so clients do not see a release without a matching press.
    swallow_release: bool,
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
            enabled: true,
            swallow_release: false,
        }
    }

    pub fn with_lock(mut self, enabled: bool) -> Self {
        self.lock_enabled = enabled;
        self
    }

    /// Disengage the filter without removing it.
    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            if self.active {
                self.swallow_release = true;
            }
            self.active = false;
        }
        self.enabled = enabled;
    }

    pub fn reset_runtime_state(&mut self) {
        self.active = false;
        self.swallow_release = false;
    }

    pub fn scroll_button(&self) -> u32 {
        self.scroll_button
    }

    pub fn lock_enabled(&self) -> bool {
        self.lock_enabled
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_active(&self) -> bool {
        self.active
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
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if !self.enabled {
            if self.swallow_release {
                if let InputEvent::PointerButton {
                    button,
                    pressed: false,
                    ..
                } = event
                {
                    if *button == self.scroll_button {
                        self.swallow_release = false;
                        return Step::Suppress;
                    }
                }
            }
            return Step::Keep;
        }

        match event {
            InputEvent::PointerButton {
                button, pressed, ..
            } if *button == self.scroll_button => {
                self.handle_scroll_button(*pressed);
                Step::Suppress
            }
            InputEvent::PointerMotion {
                device_index,
                time_us,
                dx,
                dy,
                ..
            } if self.active => {
                *event = InputEvent::PointerScroll {
                    device_index: *device_index,
                    time_us: *time_us,
                    source: ScrollSource::Continuous,
                    dx: *dx,
                    dy: *dy,
                };
                Step::Keep
            }
            InputEvent::DeviceRemoved { .. } => {
                self.active = false;
                self.swallow_release = false;
                Step::Keep
            }
            _ => Step::Keep,
        }
    }
}
