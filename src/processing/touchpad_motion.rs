use crate::events::{InputEvent, TouchData, TouchEndData};
use crate::state::Seat;
use crate::backend::DeviceInfo;
use super::{Filter, FilterResult};
use super::config::TouchpadConfig;
use smallvec::smallvec;

const TAP_TIMEOUT_US: u64 = 180_000;
const TAP_THRESHOLD_MM: f64 = 1.0;
const MAX_TOUCH_SLOTS: usize = 10;
const DWT_TIMEOUT_US: u64 = 500_000;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

#[derive(Default, Clone, Copy)]
struct TouchState {
    start_x: f64,
    start_y: f64,
    x: f64,
    y: f64,
    active: bool,
    start_time: u64,
    moved: bool,
    is_palm: bool,
}

pub struct TouchpadMotionFilter {
    config: TouchpadConfig,
    slots: [TouchState; MAX_TOUCH_SLOTS],

    pending_dx: f64,
    pending_dy: f64,
    slots_moved: u16,

    fingers_at_start: usize,
    current_gesture_is_tap: bool,
}

impl TouchpadMotionFilter {
    pub fn new(config: TouchpadConfig) -> Self {
        Self {
            config,
            slots: [TouchState::default(); MAX_TOUCH_SLOTS],
            pending_dx: 0.0,
            pending_dy: 0.0,
            slots_moved: 0,
            fingers_at_start: 0,
            current_gesture_is_tap: true,
        }
    }

    fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active && !s.is_palm).count()
    }

    fn is_dwt_active(&self, time_us: u64, seat: Option<&Seat>) -> bool {
        if !self.config.dwt { return false; }
        if let Some(seat) = seat {
            if seat.last_key_time > 0 && time_us > seat.last_key_time {
                return time_us - seat.last_key_time < DWT_TIMEOUT_US;
            }
        }
        false
    }

    fn is_in_edge_zone(&self, x: f64, y: f64, info: Option<DeviceInfo>) -> bool {
        if let Some(info) = info {
            let width = info.x.max - info.x.min;
            let height = info.y.max - info.y.min;
            if width <= 0.0 || height <= 0.0 { return false; }

            let edge_x = width * self.config.palm_edge_ratio;
            let edge_y = height * self.config.palm_edge_ratio;

            x < (info.x.min + edge_x) || x > (info.x.max - edge_x) ||
            y < (info.y.min + edge_y) || y > (info.y.max - edge_y)
        } else {
            false
        }
    }

    fn get_dist_mm(&self, dx: f64, dy: f64, info: Option<DeviceInfo>) -> f64 {
        if let Some(info) = info {
            let res_x = if info.x.resolution > 0.0 { info.x.resolution } else { 1.0 };
            let res_y = if info.y.resolution > 0.0 { info.y.resolution } else { 1.0 };
            let mm_x = dx / res_x;
            let mm_y = dy / res_y;
            (mm_x * mm_x + mm_y * mm_y).sqrt()
        } else {
            (dx * dx + dy * dy).sqrt() / 10.0
        }
    }

    /// Set natural scrolling direction in place.
    pub fn set_natural_scrolling(&mut self, enabled: bool) {
        self.config.natural_scrolling = enabled;
    }

    /// Enable or disable tap-to-click in place.
    pub fn set_tap_to_click(&mut self, enabled: bool) {
        self.config.tap_to_click = enabled;
    }

    /// Enable or disable two-finger tap right-click in place.
    pub fn set_two_finger_tap_right_click(&mut self, enabled: bool) {
        self.config.two_finger_tap_right_click = enabled;
    }
}

impl Filter for TouchpadMotionFilter {
    fn process(&mut self, event: InputEvent, seat: Option<&Seat>, info: Option<DeviceInfo>) -> FilterResult {
        match event {
            InputEvent::TouchDown(data) => {
                let TouchData { device_index: _, time_us, slot, x, y } = *data;
                let s = slot as usize;
                if s < MAX_TOUCH_SLOTS {
                    let is_palm = self.config.palm_detection && self.is_in_edge_zone(x, y, info);

                    if self.active_count() == 0 {
                        self.fingers_at_start = 0;
                        self.current_gesture_is_tap = true;
                    }
                    if !is_palm {
                        self.fingers_at_start += 1;
                    }

                    self.slots[s] = TouchState {
                        start_x: x, start_y: y, x, y,
                        active: true, start_time: time_us,
                        moved: false, is_palm,
                    };
                }
                smallvec![]
            }
            InputEvent::TouchUp(data) => {
                let TouchEndData { device_index, time_us, slot } = *data;
                let s = slot as usize;
                let mut results = FilterResult::new();

                if s < MAX_TOUCH_SLOTS && self.slots[s].active {
                    self.slots[s].active = false;

                    if self.slots[s].is_palm {
                        return results;
                    }

                    let duration = time_us - self.slots[s].start_time;
                    let dx = self.slots[s].x - self.slots[s].start_x;
                    let dy = self.slots[s].y - self.slots[s].start_y;
                    let dist_mm = self.get_dist_mm(dx, dy, info);

                    let is_tap_like = duration < TAP_TIMEOUT_US && dist_mm < TAP_THRESHOLD_MM && !self.slots[s].moved;
                    if !is_tap_like {
                        self.current_gesture_is_tap = false;
                    }

                    if self.active_count() == 0 {
                        if self.current_gesture_is_tap && self.config.tap_to_click {
                            if !self.is_dwt_active(time_us, seat) {
                                let button = if self.fingers_at_start == 1 {
                                    Some(BTN_LEFT)
                                } else if self.fingers_at_start == 2 && self.config.two_finger_tap_right_click {
                                    Some(BTN_RIGHT)
                                } else {
                                    None
                                };

                                if let Some(btn) = button {
                                    results.push(InputEvent::PointerButton {
                                        device_index, time_us, button: btn, pressed: true,
                                    });
                                    results.push(InputEvent::PointerButton {
                                        device_index, time_us, button: btn, pressed: false,
                                    });
                                }
                            }
                        }
                    }
                }
                results
            }
            InputEvent::TouchMotion(data) => {
                let TouchData { device_index: _, time_us: _, slot, x, y, .. } = *data;
                let s = slot as usize;
                if s < MAX_TOUCH_SLOTS && self.slots[s].active && !self.slots[s].is_palm {
                    if !self.slots[s].moved {
                        let dx = x - self.slots[s].start_x;
                        let dy = y - self.slots[s].start_y;
                        let dist_mm = self.get_dist_mm(dx, dy, info);
                        if dist_mm >= TAP_THRESHOLD_MM {
                            self.slots[s].moved = true;
                            self.current_gesture_is_tap = false;
                        }
                    }

                    // Accumulate delta for this frame
                    self.pending_dx += x - self.slots[s].x;
                    self.pending_dy += y - self.slots[s].y;
                    self.slots_moved += 1;

                    self.slots[s].x = x;
                    self.slots[s].y = y;
                }
                smallvec![]
            }
            InputEvent::Frame { device_index, time_us } => {
                let mut results = FilterResult::new();

                // Suppress motion while DWT is active
                if self.is_dwt_active(time_us, seat) {
                    self.pending_dx = 0.0;
                    self.pending_dy = 0.0;
                    self.slots_moved = 0;
                    return results;
                }

                if self.slots_moved == 0 {
                    // No motion this frame; still need to reset start positions
                    for slot in &mut self.slots {
                        if slot.active {
                            slot.start_x = slot.x;
                            slot.start_y = slot.y;
                        }
                    }
                    return results;
                }

                let pending_dx = self.pending_dx;
                let pending_dy = self.pending_dy;
                let moved_slots = self.slots_moved;
                self.pending_dx = 0.0;
                self.pending_dy = 0.0;
                self.slots_moved = 0;

                // Reset start positions for next frame
                for slot in &mut self.slots {
                    if slot.active {
                        slot.start_x = slot.x;
                        slot.start_y = slot.y;
                    }
                }

                let count = self.active_count();
                if count == 0 {
                    return results;
                }

                if count == 1 {
                    results.push(InputEvent::PointerMotion {
                        device_index,
                        time_us,
                        dx: pending_dx,
                        dy: pending_dy,
                        dx_unaccelerated: pending_dx,
                        dy_unaccelerated: pending_dy,
                    });
                } else if count == 2 {
                    let scroll_dx = if self.config.natural_scrolling { -pending_dx } else { pending_dx };
                    let scroll_dy = if self.config.natural_scrolling { -pending_dy } else { pending_dy };
                    results.push(InputEvent::PointerScroll {
                        device_index,
                        time_us,
                        dx: scroll_dx / moved_slots as f64,
                        dy: scroll_dy / moved_slots as f64,
                    });
                }
                results
            }
            _ => smallvec![event],
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::DevIndex;
    use crate::backend::{DeviceInfo, AxisInfo};

    fn mock_info() -> DeviceInfo {
        DeviceInfo {
            x: AxisInfo { min: 0.0, max: 1000.0, resolution: 10.0 },
            y: AxisInfo { min: 0.0, max: 1000.0, resolution: 10.0 },
            ..Default::default()
        }
    }

    fn dev() -> DevIndex { DevIndex(0) }

    #[test]
    fn test_palm_detection() {
        let config = TouchpadConfig::default();
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 0, slot: 0, x: 500.0, y: 500.0 })), None, Some(info.clone()));
        f.process(InputEvent::TouchMotion(Box::new(TouchData { device_index: dev(), time_us: 100, slot: 0, x: 510.0, y: 510.0 })), None, Some(info.clone()));
        let ev = f.process(InputEvent::Frame { device_index: dev(), time_us: 100 }, None, Some(info.clone()));
        assert!(!ev.is_empty(), "Center touch should produce motion");

        f.process(InputEvent::TouchUp(Box::new(TouchEndData { device_index: dev(), time_us: 200, slot: 0 })), None, Some(info.clone()));
        f.process(InputEvent::Frame { device_index: dev(), time_us: 200 }, None, Some(info.clone()));

        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 300, slot: 0, x: 10.0, y: 500.0 })), None, Some(info.clone()));
        f.process(InputEvent::TouchMotion(Box::new(TouchData { device_index: dev(), time_us: 400, slot: 0, x: 20.0, y: 510.0 })), None, Some(info.clone()));
        let ev_palm = f.process(InputEvent::Frame { device_index: dev(), time_us: 400 }, None, Some(info.clone()));
        assert!(ev_palm.is_empty(), "Edge touch should be detected as palm and ignored");
    }

    #[test]
    fn test_touchpad_complex_gestures() {
        let mut config = TouchpadConfig::default();
        config.natural_scrolling = true;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 0, slot: 0, x: 500.0, y: 500.0 })), None, Some(info.clone()));
        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 0, slot: 1, x: 600.0, y: 600.0 })), None, Some(info.clone()));
        f.process(InputEvent::TouchMotion(Box::new(TouchData { device_index: dev(), time_us: 100, slot: 0, x: 500.0, y: 505.0 })), None, Some(info.clone()));
        let evs = f.process(InputEvent::Frame { device_index: dev(), time_us: 100 }, None, Some(info.clone()));

        // Two-finger scroll: slot 0 moved by dy=5, natural scrolling reverses it
        if let Some(InputEvent::PointerScroll { dy, .. }) = evs.get(0) {
            assert_eq!(*dy, -5.0, "Expected scroll dy=-5.0, got {}", dy);
        } else {
            panic!("Expected PointerScroll, got {:?}", evs.get(0));
        }

        f.process(InputEvent::TouchUp(Box::new(TouchEndData { device_index: dev(), time_us: 200, slot: 0 })), None, Some(info.clone()));
        f.process(InputEvent::TouchUp(Box::new(TouchEndData { device_index: dev(), time_us: 200, slot: 1 })), None, Some(info.clone()));
        f.process(InputEvent::Frame { device_index: dev(), time_us: 200 }, None, Some(info.clone()));

        // Two-finger tap → right click
        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 300, slot: 0, x: 500.0, y: 500.0 })), None, Some(info.clone()));
        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 305, slot: 1, x: 600.0, y: 600.0 })), None, Some(info.clone()));

        f.process(InputEvent::TouchUp(Box::new(TouchEndData { device_index: dev(), time_us: 350, slot: 0 })), None, Some(info.clone()));
        let evs_tap = f.process(InputEvent::TouchUp(Box::new(TouchEndData { device_index: dev(), time_us: 355, slot: 1 })), None, Some(info.clone()));

        assert_eq!(evs_tap.len(), 2);
        assert!(matches!(evs_tap[0], InputEvent::PointerButton { button: BTN_RIGHT, pressed: true, .. }));
        assert!(matches!(evs_tap[1], InputEvent::PointerButton { button: BTN_RIGHT, pressed: false, .. }));
    }

    #[test]
    fn test_dwt() {
        use crate::state::Seat;
        let config = TouchpadConfig::default();
        let mut f = TouchpadMotionFilter::new(config);
        let mut seat = Seat { name: "seat0".into(), devices: vec![], last_key_time: 1000 };
        let info = mock_info();

        f.process(InputEvent::TouchDown(Box::new(TouchData { device_index: dev(), time_us: 1100, slot: 0, x: 500.0, y: 500.0 })), Some(&seat), Some(info.clone()));
        f.process(InputEvent::TouchMotion(Box::new(TouchData { device_index: dev(), time_us: 1200, slot: 0, x: 512.0, y: 11.0 })), Some(&seat), Some(info.clone()));
        let ev = f.process(InputEvent::Frame { device_index: dev(), time_us: 1200 }, Some(&seat), Some(info.clone()));
        assert!(ev.is_empty(), "Motion should be suppressed by DWT");

        seat.last_key_time = 1000;
        let time_after = 1000 + DWT_TIMEOUT_US + 100;
        f.process(InputEvent::TouchMotion(Box::new(TouchData { device_index: dev(), time_us: time_after, slot: 0, x: 515.0, y: 13.0 })), Some(&seat), Some(info.clone()));
        let ev2 = f.process(InputEvent::Frame { device_index: dev(), time_us: time_after }, Some(&seat), Some(info.clone()));
        assert!(!ev2.is_empty(), "Motion should NOT be suppressed after DWT timeout");
    }
}
