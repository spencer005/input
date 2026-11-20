use {
    super::{Filter, FilterResult, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const DEFAULT_CHORD_TIMEOUT_US: u64 = 50_000;

#[derive(Clone, Copy)]
struct PendingButton {
    device_index: crate::events::DevIndex,
    button: u32,
    time_us: u64,
}

/// Emulates a middle button press from a near-simultaneous left+right chord.
///
/// The filter delays an isolated left or right press briefly so a matching
/// opposite button can turn the pair into BTN_MIDDLE. Pointer motion or another
/// non-frame event flushes the pending press first, preserving drag ordering.
pub struct MiddleButtonEmulationFilter {
    timeout_us: u64,
    pending: Option<PendingButton>,
    emulating: bool,
    left_down: bool,
    right_down: bool,
    enabled: bool,
    /// After disengaging during a chord, swallow the remaining physical
    /// left/right releases so clients do not see releases without presses.
    swallow_chord_releases: bool,
}

impl MiddleButtonEmulationFilter {
    pub fn new() -> Self {
        Self {
            timeout_us: DEFAULT_CHORD_TIMEOUT_US,
            pending: None,
            emulating: false,
            left_down: false,
            right_down: false,
            enabled: true,
            swallow_chord_releases: false,
        }
    }

    pub fn with_timeout(timeout_us: u64) -> Self {
        Self {
            timeout_us,
            ..Self::new()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Disengage without removing the filter. Context flushes a pending press
    /// and releases an active synthesized middle button before calling this.
    pub fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            if self.emulating {
                self.swallow_chord_releases = true;
            }
            self.emulating = false;
            self.pending = None;
        }
        self.enabled = enabled;
    }

    pub fn reset_runtime_state(&mut self) {
        self.pending = None;
        self.emulating = false;
        self.left_down = false;
        self.right_down = false;
        self.swallow_chord_releases = false;
    }

    fn is_emulated_button(button: u32) -> bool {
        button == BTN_LEFT || button == BTN_RIGHT
    }

    fn other_button_down(&self, button: u32) -> bool {
        match button {
            BTN_LEFT => self.right_down,
            BTN_RIGHT => self.left_down,
            _ => false,
        }
    }

    fn set_button_down(&mut self, button: u32, down: bool) {
        match button {
            BTN_LEFT => self.left_down = down,
            BTN_RIGHT => self.right_down = down,
            _ => {}
        }
    }

    fn button_event(
        device_index: crate::events::DevIndex,
        time_us: u64,
        button: u32,
        pressed: bool,
    ) -> InputEvent {
        InputEvent::PointerButton {
            device_index,
            time_us,
            button,
            pressed,
        }
    }

    fn flush_pending(&mut self, time_us: u64, force: bool, out: &mut FilterResult) {
        let Some(pending) = self.pending else {
            return;
        };
        if force || time_us.saturating_sub(pending.time_us) > self.timeout_us {
            self.pending = None;
            out.push(Self::button_event(
                pending.device_index,
                pending.time_us,
                pending.button,
                true,
            ));
        }
    }

    fn handle_press(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        button: u32,
        out: &mut FilterResult,
    ) {
        let other_was_down = self.other_button_down(button);
        self.set_button_down(button, true);

        if let Some(pending) = self.pending {
            let within_timeout = time_us.saturating_sub(pending.time_us) <= self.timeout_us;
            if pending.button != button && within_timeout {
                self.pending = None;
                self.emulating = true;
                out.push(Self::button_event(device_index, time_us, BTN_MIDDLE, true));
                return;
            }

            self.flush_pending(time_us, true, out);
        }

        if other_was_down {
            out.push(Self::button_event(device_index, time_us, button, true));
        } else {
            self.pending = Some(PendingButton {
                device_index,
                button,
                time_us,
            });
        }
    }

    fn handle_release(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        button: u32,
        out: &mut FilterResult,
    ) {
        self.set_button_down(button, false);

        if let Some(pending) = self.pending {
            if pending.button == button {
                self.pending = None;
                out.push(Self::button_event(
                    pending.device_index,
                    pending.time_us,
                    pending.button,
                    true,
                ));
                out.push(Self::button_event(device_index, time_us, button, false));
                return;
            }
        }

        if self.emulating {
            if !self.left_down && !self.right_down {
                self.emulating = false;
                out.push(Self::button_event(device_index, time_us, BTN_MIDDLE, false));
            }
            return;
        }

        out.push(Self::button_event(device_index, time_us, button, false));
    }
}

impl Filter for MiddleButtonEmulationFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        let mut out = FilterResult::new();

        if self.swallow_chord_releases {
            if let InputEvent::PointerButton {
                button,
                pressed: false,
                ..
            } = event
            {
                if matches!(*button, BTN_LEFT | BTN_RIGHT) {
                    self.set_button_down(*button, false);
                    if !self.left_down && !self.right_down {
                        self.swallow_chord_releases = false;
                    }
                    return Step::Suppress;
                }
            }
        }

        if !self.enabled {
            return Step::Keep;
        }

        match event {
            InputEvent::PointerButton {
                device_index,
                time_us,
                button,
                pressed,
            } if Self::is_emulated_button(*button) => {
                if *pressed {
                    self.handle_press(*device_index, *time_us, *button, &mut out);
                } else {
                    self.handle_release(*device_index, *time_us, *button, &mut out);
                }
                Step::from_result(event, out)
            }
            InputEvent::Frame {
                device_index,
                time_us,
            } => {
                self.flush_pending(*time_us, false, &mut out);
                if out.is_empty() {
                    Step::Keep
                } else {
                    out.push(InputEvent::Frame {
                        device_index: *device_index,
                        time_us: *time_us,
                    });
                    Step::FanOut(Box::new(out))
                }
            }
            other => {
                let time_us = other.time_us();
                self.flush_pending(time_us, true, &mut out);
                if out.is_empty() {
                    Step::Keep
                } else {
                    out.push(other.clone());
                    Step::FanOut(Box::new(out))
                }
            }
        }
    }

    fn dispatch_timeout(&mut self, now_us: u64) -> FilterResult {
        let mut out = FilterResult::new();
        self.flush_pending(now_us, false, &mut out);
        out
    }

    fn next_timeout_us(&self) -> Option<u64> {
        self.pending
            .map(|pending| pending.time_us.saturating_add(self.timeout_us))
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{events::DevIndex, processing::TestFilterExt},
    };

    fn press(time_us: u64, button: u32) -> InputEvent {
        InputEvent::PointerButton {
            device_index: DevIndex(0),
            time_us,
            button,
            pressed: true,
        }
    }

    fn release(time_us: u64, button: u32) -> InputEvent {
        InputEvent::PointerButton {
            device_index: DevIndex(0),
            time_us,
            button,
            pressed: false,
        }
    }

    fn buttons(events: &[InputEvent]) -> Vec<(u32, bool)> {
        events
            .iter()
            .filter_map(|event| match event {
                InputEvent::PointerButton {
                    button, pressed, ..
                } => Some((*button, *pressed)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn chord_within_timeout_emits_only_middle() {
        let mut filter = MiddleButtonEmulationFilter::new();
        assert!(buttons(&filter.run(press(0, BTN_LEFT), None, None)).is_empty());
        let out = filter.run(press(10, BTN_RIGHT), None, None);
        assert_eq!(buttons(&out), vec![(BTN_MIDDLE, true)]);

        let out = filter.run(release(20, BTN_RIGHT), None, None);
        assert!(buttons(&out).is_empty());
        let out = filter.run(release(30, BTN_LEFT), None, None);
        assert_eq!(buttons(&out), vec![(BTN_MIDDLE, false)]);
    }

    #[test]
    fn timer_deadline_flushes_pending_press_without_other_input() {
        let mut filter = MiddleButtonEmulationFilter::new();
        filter.run(press(1_000, BTN_LEFT), None, None);
        assert_eq!(filter.next_timeout_us(), Some(1_000 + 50_000));

        assert!(buttons(&filter.dispatch_timeout(1_001)).is_empty());
        let out = filter.dispatch_timeout(51_001);
        assert_eq!(buttons(&out), vec![(BTN_LEFT, true)]);
        assert_eq!(filter.next_timeout_us(), None);
    }
}
