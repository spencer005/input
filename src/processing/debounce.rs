use {
    super::{Filter, FilterResult, Step},
    crate::{
        backend::DeviceInfo,
        events::{DevIndex, InputEvent},
        state::Seat,
    },
};

/// Linux input codes with debounce state. `KEY_MAX` is 0x2ff, so this covers
/// every key and button code. Codes above this range pass through without
/// debounce rather than aliasing another code.
const MAX_DEBOUNCE_CODES: usize = 0x300;

/// Configuration for button/key debounce behavior.
///
/// All timeouts are in microseconds. Set to 0 to disable that phase.
/// Each input code is tracked independently; changing one button does not flush
/// other buttons' deferral state.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DebouncedEventKind {
    #[default]
    Button,
    Key,
}

#[derive(Debug, Clone, Copy)]
struct ButtonState {
    state: DebounceState,
    last_event_time: u64,
    device_index: DevIndex,
    code: u32,
    kind: DebouncedEventKind,
}

impl Default for ButtonState {
    fn default() -> Self {
        Self {
            state: DebounceState::Up,
            last_event_time: 0,
            device_index: DevIndex(0),
            code: 0,
            kind: DebouncedEventKind::Button,
        }
    }
}

pub struct ButtonDebounceFilter {
    config: DebounceConfig,
    /// One independent state per input code. This is heap-allocated because
    /// `FilterKind` is stored inline in a small vector; embedding the full
    /// table would make every filter chain large.
    buttons: Box<[ButtonState]>,
}

impl ButtonDebounceFilter {
    pub fn new(config: DebounceConfig) -> Self {
        let mut filter = Self {
            config,
            buttons: vec![ButtonState::default(); MAX_DEBOUNCE_CODES].into_boxed_slice(),
        };
        filter.reset_runtime_state();
        filter
    }

    /// Create a disabled filter that passes all events through with zero overhead.
    pub fn disabled() -> Self {
        Self {
            config: DebounceConfig {
                enabled: false,
                ..Default::default()
            },
            buttons: vec![ButtonState::default(); MAX_DEBOUNCE_CODES].into_boxed_slice(),
        }
    }

    pub fn reset_runtime_state(&mut self) {
        for state in self.buttons.iter_mut() {
            *state = ButtonState::default();
        }
    }

    /// Update configuration while preserving in-flight debounce state.
    ///
    /// Pending deferred transitions keep their existing deadlines; callers that
    /// disable debounce should flush pending deadlines through the context
    /// timeout path so no release is lost.
    pub fn set_config(&mut self, config: DebounceConfig) {
        self.config = config;
    }

    pub fn config(&self) -> DebounceConfig {
        self.config
    }

    /// Mark a code as already held when debounce is enabled at runtime.
    ///
    /// Without this, enabling debounce while a button is held would observe
    /// only the later release and drop it as an unknown transition.
    pub fn seed_down(&mut self, code: u32, is_key: bool) {
        let Some(index) = Self::state_index(code) else {
            return;
        };
        let state = &mut self.buttons[index];
        if state.state == DebounceState::Up {
            state.state = DebounceState::Down;
            state.last_event_time = 0;
            state.code = code;
            state.kind = if is_key {
                DebouncedEventKind::Key
            } else {
                DebouncedEventKind::Button
            };
        }
    }

    fn state_index(code: u32) -> Option<usize> {
        let index = code as usize;
        (index < MAX_DEBOUNCE_CODES).then_some(index)
    }

    fn deadline_for(&self, state: &ButtonState) -> Option<u64> {
        match state.state {
            DebounceState::UpDelaying => Some(
                state
                    .last_event_time
                    .saturating_add(self.config.bounce_timeout_us),
            ),
            DebounceState::DownDetectingSpurious if self.config.spurious_timeout_us > 0 => Some(
                state
                    .last_event_time
                    .saturating_add(self.config.spurious_timeout_us),
            ),
            _ => None,
        }
    }

    fn handle_event(
        &mut self,
        device_index: DevIndex,
        key: u32,
        pressed: bool,
        time_us: u64,
        kind: DebouncedEventKind,
    ) -> Option<bool> {
        if !self.config.enabled {
            return Some(pressed);
        }

        let Some(idx) = Self::state_index(key) else {
            // Unknown/extended code: never alias another code's state.
            return Some(pressed);
        };
        let state_obj = &mut self.buttons[idx];
        state_obj.device_index = device_index;
        state_obj.code = key;
        state_obj.kind = kind;
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
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        match event {
            InputEvent::PointerButton {
                device_index,
                time_us,
                button,
                pressed,
            } => match self.handle_event(
                *device_index,
                *button,
                *pressed,
                *time_us,
                DebouncedEventKind::Button,
            ) {
                Some(emitted) => {
                    *pressed = emitted;
                    Step::Keep
                }
                None => Step::Suppress,
            },
            InputEvent::Key {
                device_index,
                time_us,
                key,
                pressed,
            } => match self.handle_event(
                *device_index,
                *key,
                *pressed,
                *time_us,
                DebouncedEventKind::Key,
            ) {
                Some(emitted) => {
                    *pressed = emitted;
                    Step::Keep
                }
                None => Step::Suppress,
            },
            _ => Step::Keep,
        }
    }

    fn dispatch_timeout(&mut self, now_us: u64) -> FilterResult {
        let bounce_timeout = self.config.bounce_timeout_us;
        let spurious_timeout = self.config.spurious_timeout_us;
        let mut out = FilterResult::new();
        for state in self.buttons.iter_mut() {
            let deadline = match state.state {
                DebounceState::UpDelaying => {
                    Some(state.last_event_time.saturating_add(bounce_timeout))
                }
                DebounceState::DownDetectingSpurious if spurious_timeout > 0 => {
                    Some(state.last_event_time.saturating_add(spurious_timeout))
                }
                _ => None,
            };
            let Some(deadline) = deadline else { continue };
            if now_us < deadline {
                continue;
            }
            match state.state {
                DebounceState::UpDelaying => {
                    state.state = DebounceState::Up;
                    state.last_event_time = now_us;
                    out.push(if state.kind == DebouncedEventKind::Key {
                        InputEvent::Key {
                            device_index: state.device_index,
                            time_us: now_us,
                            key: state.code,
                            pressed: false,
                        }
                    } else {
                        InputEvent::PointerButton {
                            device_index: state.device_index,
                            time_us: now_us,
                            button: state.code,
                            pressed: false,
                        }
                    });
                }
                DebounceState::DownDetectingSpurious => {
                    state.state = DebounceState::Down;
                    state.last_event_time = now_us;
                    out.push(if state.kind == DebouncedEventKind::Key {
                        InputEvent::Key {
                            device_index: state.device_index,
                            time_us: now_us,
                            key: state.code,
                            pressed: true,
                        }
                    } else {
                        InputEvent::PointerButton {
                            device_index: state.device_index,
                            time_us: now_us,
                            button: state.code,
                            pressed: true,
                        }
                    });
                }
                _ => {}
            }
        }
        out
    }

    fn next_timeout_us(&self) -> Option<u64> {
        self.buttons
            .iter()
            .filter_map(|state| self.deadline_for(state))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::processing::TestFilterExt};

    fn enabled_filter() -> ButtonDebounceFilter {
        ButtonDebounceFilter::new(DebounceConfig {
            enabled: true,
            bounce_timeout_us: 25_000,
            spurious_timeout_us: 12_000,
        })
    }

    fn handle(
        filter: &mut ButtonDebounceFilter,
        code: u32,
        pressed: bool,
        time_us: u64,
    ) -> Option<bool> {
        filter.handle_event(
            DevIndex(0),
            code,
            pressed,
            time_us,
            DebouncedEventKind::Button,
        )
    }

    fn handle_key(
        filter: &mut ButtonDebounceFilter,
        code: u32,
        pressed: bool,
        time_us: u64,
    ) -> Option<bool> {
        filter.handle_event(DevIndex(0), code, pressed, time_us, DebouncedEventKind::Key)
    }

    #[test]
    fn test_disabled_passthrough() {
        let mut f = ButtonDebounceFilter::disabled();
        assert_eq!(handle(&mut f, 0x110, true, 0), Some(true));
        assert_eq!(handle(&mut f, 0x110, false, 100_000), Some(false));
    }

    #[test]
    fn test_bounce_suppression() {
        let mut f = enabled_filter();
        let btn = 0x110;

        // Press goes through immediately
        assert!(handle(&mut f, btn, true, 0).is_some());
        // Spurious release within bounce timeout → suppressed
        assert!(handle(&mut f, btn, false, 5_000).is_none());
        // Spurious re-press within bounce timeout → suppressed
        assert!(handle(&mut f, btn, true, 10_000).is_none());
        // Legitimate release after timeout
        assert!(handle(&mut f, btn, false, 100_000).is_some());
    }

    #[test]
    fn test_spurious_detection() {
        let mut f = enabled_filter();
        let btn = 0x110;

        assert!(handle(&mut f, btn, true, 0).is_some());
        assert!(handle(&mut f, btn, false, 100_000).is_some());
    }

    #[test]
    fn test_deferred_release_is_deadline_driven() {
        let mut f = enabled_filter();
        let btn = 0x110;

        assert_eq!(handle(&mut f, btn, true, 0), Some(true));
        assert_eq!(handle(&mut f, btn, false, 10_000), None);

        // Release was deferred; the deadline is release time + bounce timeout.
        assert_eq!(f.next_timeout_us(), Some(35_000));
        assert!(f.dispatch_timeout(34_999).is_empty());

        let events = f.dispatch_timeout(35_000);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 35_000,
                button: 0x110,
                pressed: false,
            }
        ));
        assert_eq!(f.next_timeout_us(), None);
    }

    #[test]
    fn test_deferred_key_release_is_deadline_driven() {
        let mut f = enabled_filter();
        let key = 30; // KEY_A

        assert_eq!(handle_key(&mut f, key, true, 0), Some(true));
        assert_eq!(handle_key(&mut f, key, false, 5_000), None);
        let events = f.dispatch_timeout(30_000);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::Key {
                key: 30,
                pressed: false,
                ..
            }
        ));
    }

    #[test]
    fn test_codes_do_not_alias_debounce_state() {
        let mut f = enabled_filter();

        // 16 and 32 collide under the old `% 16` implementation.
        assert_eq!(handle(&mut f, 16, true, 0), Some(true));
        assert_eq!(handle(&mut f, 32, true, 1_000), Some(true));

        // Releasing one must not consume or disturb the other's state.
        assert_eq!(handle(&mut f, 16, false, 30_000), Some(false));
        assert_eq!(handle(&mut f, 32, false, 31_000), Some(false));
    }

    #[test]
    fn test_spurious_re_press_is_deadline_driven() {
        let mut f = enabled_filter();

        assert_eq!(handle(&mut f, 0x110, true, 0), Some(true));
        assert_eq!(handle(&mut f, 0x110, false, 100_000), Some(false));
        // Re-press inside the spurious window is held back.
        assert_eq!(handle(&mut f, 0x110, true, 105_000), None);
        assert_eq!(f.next_timeout_us(), Some(117_000));

        let events = f.dispatch_timeout(117_000);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                pressed: true,
                button: 0x110,
                ..
            }
        ));
    }

    #[test]
    fn multiple_buttons_debounce_independently() {
        let mut f = enabled_filter();

        assert_eq!(handle(&mut f, 0x110, true, 0), Some(true)); // left
        assert_eq!(handle(&mut f, 0x111, true, 1_000), Some(true)); // right
        assert_eq!(handle(&mut f, 0x110, false, 2_000), None); // left deferred
        assert_eq!(handle(&mut f, 0x111, false, 3_000), None); // right deferred

        // One timer dispatch flushes both independently tracked releases.
        let events = f.dispatch_timeout(30_000);
        let mut buttons: Vec<(u32, bool)> = events
            .iter()
            .filter_map(|event| match event {
                InputEvent::PointerButton {
                    button, pressed, ..
                } => Some((*button, *pressed)),
                _ => None,
            })
            .collect();
        buttons.sort_unstable();
        assert_eq!(buttons, vec![(0x110, false), (0x111, false)]);
    }

    #[test]
    fn out_of_range_codes_pass_through() {
        let mut f = enabled_filter();
        assert_eq!(
            handle(&mut f, MAX_DEBOUNCE_CODES as u32 + 5, true, 0),
            Some(true)
        );
    }
}
