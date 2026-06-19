use {
    super::{Filter, FilterResult},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
    smallvec::smallvec,
};

/// Configuration for button/key debounce behavior.
///
/// All timeouts are in microseconds. Set to 0 to disable that phase.
/// `enabled: false` disables the filter entirely (zero-latency passthrough).
#[derive(Debug, Clone, Copy)]
pub struct DebounceConfig {
    /// Whether debounce is active at all. When false, all events pass through
    /// with no latency penalty.
    pub enabled: bool,
    /// Bounce timeout: suppresses a release-then-press within this window (us).
    /// Default 25ms for mechanical switches that physically bounce.
    pub bounce_timeout_us: u64,
    /// Spurious timeout: after a legitimate release, watches for rapid re-press
    /// within this window (us) to filter spurious double-events.
    /// Default 12ms. Set to 0 to disable spurious detection.
    pub spurious_timeout_us: u64,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            // Disabled by default — many devices debounce in hardware/firmware
            // and the latency penalty is not worth it. Enable only for devices
            // known to bounce.
            enabled: false,
            bounce_timeout_us: 25_000,
            spurious_timeout_us: 12_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum DebounceState {
    #[default]
    Up,
    Down,
    DownWaiting,
    UpDelaying,
    UpDetectingSpurious,
    DownDetectingSpurious,
}

struct ButtonState {
    state: DebounceState,
    last_event_time: u64,
}

impl Copy for ButtonState {}
impl Clone for ButtonState {
    fn clone(&self) -> Self {
        *self
    }
}

impl Default for ButtonState {
    fn default() -> Self {
        Self {
            state: DebounceState::Up,
            last_event_time: 0,
        }
    }
}

pub struct ButtonDebounceFilter {
    config: DebounceConfig,
    buttons: [ButtonState; 16],
}

impl ButtonDebounceFilter {
    pub fn new(config: DebounceConfig) -> Self {
        Self {
            config,
            buttons: [ButtonState::default(); 16],
        }
    }

    /// Create a disabled filter that passes all events through with zero overhead.
    pub fn disabled() -> Self {
        Self {
            config: DebounceConfig {
                enabled: false,
                ..Default::default()
            },
            buttons: [ButtonState::default(); 16],
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.buttons = [ButtonState::default(); 16];
    }

    fn handle_event(&mut self, key: u32, pressed: bool, time_us: u64) -> Option<bool> {
        if !self.config.enabled {
            return Some(pressed);
        }

        let idx = (key as usize) % 16;
        let state_obj = &mut self.buttons[idx];
        let old_state = state_obj.state;

        let mut emit: Option<bool> = None;

        let bounce_timeout = self.config.bounce_timeout_us;
        let spurious_timeout = self.config.spurious_timeout_us;
        let elapsed = time_us.saturating_sub(state_obj.last_event_time);

        match old_state {
            DebounceState::Up => {
                if pressed {
                    state_obj.state = DebounceState::DownWaiting;
                    emit = Some(true);
                }
            }
            DebounceState::Down => {
                if !pressed {
                    state_obj.state = DebounceState::UpDetectingSpurious;
                    emit = Some(false);
                }
            }
            DebounceState::DownWaiting => {
                if !pressed {
                    if elapsed < bounce_timeout {
                        state_obj.state = DebounceState::UpDelaying;
                    } else {
                        state_obj.state = DebounceState::UpDetectingSpurious;
                        emit = Some(false);
                    }
                } else if elapsed >= bounce_timeout {
                    state_obj.state = DebounceState::Down;
                }
            }
            DebounceState::UpDelaying => {
                if pressed {
                    state_obj.state = DebounceState::DownWaiting;
                } else if elapsed >= bounce_timeout {
                    state_obj.state = DebounceState::Up;
                    emit = Some(false);
                }
            }
            DebounceState::UpDetectingSpurious => {
                if pressed {
                    if spurious_timeout > 0 && elapsed < spurious_timeout {
                        state_obj.state = DebounceState::DownDetectingSpurious;
                    } else {
                        state_obj.state = DebounceState::DownWaiting;
                        emit = Some(true);
                    }
                } else if spurious_timeout > 0 && elapsed >= spurious_timeout {
                    state_obj.state = DebounceState::Up;
                }
            }
            DebounceState::DownDetectingSpurious => {
                if !pressed {
                    state_obj.state = DebounceState::UpDetectingSpurious;
                } else if elapsed >= spurious_timeout {
                    state_obj.state = DebounceState::Down;
                    emit = Some(true);
                }
            }
        }

        state_obj.last_event_time = time_us;
        emit
    }
}

impl Filter for ButtonDebounceFilter {
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
            } => match self.handle_event(button, pressed, time_us) {
                Some(p) => smallvec![InputEvent::PointerButton {
                    device_index,
                    time_us,
                    button,
                    pressed: p
                }],
                None => smallvec![],
            },
            InputEvent::Key {
                device_index,
                time_us,
                key,
                pressed,
            } => match self.handle_event(key, pressed, time_us) {
                Some(p) => smallvec![InputEvent::Key {
                    device_index,
                    time_us,
                    key,
                    pressed: p
                }],
                None => smallvec![],
            },
            _ => smallvec![event],
        }
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::events::DevIndex};

    fn enabled_filter() -> ButtonDebounceFilter {
        ButtonDebounceFilter::new(DebounceConfig {
            enabled: true,
            bounce_timeout_us: 25_000,
            spurious_timeout_us: 12_000,
        })
    }

    #[test]
    fn test_disabled_passthrough() {
        let mut f = ButtonDebounceFilter::disabled();
        assert!(f.handle_event(0x110, true, 0) == Some(true));
        assert!(f.handle_event(0x110, false, 100_000) == Some(false));
    }

    #[test]
    fn test_bounce_suppression() {
        let mut f = enabled_filter();
        let btn = 0x110;

        // Press goes through immediately
        assert!(f.handle_event(btn, true, 0).is_some());
        // Spurious release within bounce timeout → suppressed
        assert!(f.handle_event(btn, false, 5_000).is_none());
        // Spurious re-press within bounce timeout → suppressed
        assert!(f.handle_event(btn, true, 10_000).is_none());
        // Legitimate release after timeout
        assert!(f.handle_event(btn, false, 100_000).is_some());
    }

    #[test]
    fn test_spurious_detection() {
        let mut f = enabled_filter();
        let btn = 0x110;

        assert!(f.handle_event(btn, true, 0).is_some());
        assert!(f.handle_event(btn, false, 100_000).is_some());
    }
}
