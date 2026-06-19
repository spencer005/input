use {
    super::{
        Filter, FilterResult,
        config::{TouchpadClickMethod, TouchpadConfig, TouchpadScrollMethod},
    },
    crate::{
        backend::DeviceInfo,
        events::{
            GestureEndData, GestureHoldBeginData, GestureHoldEndData, GestureMoveData,
            GesturePinchBeginData, GesturePinchUpdateData, GestureSwipeData, InputEvent,
            ScrollSource, TouchData, TouchEndData, TouchToolType,
        },
        state::Seat,
    },
    smallvec::smallvec,
    std::f64::consts::PI,
};

const TAP_TIMEOUT_US: u64 = 180_000;
const TAP_THRESHOLD_MM: f64 = 1.0;
const MAX_TOUCH_SLOTS: usize = 10;
const DWT_TIMEOUT_US: u64 = 500_000;
const GESTURE_MOTION_THRESHOLD_MM: f64 = 0.5;
const PINCH_SCALE_THRESHOLD: f64 = 0.05;
const PINCH_ROTATION_THRESHOLD_RAD: f64 = 0.10;
const HOLD_TIMEOUT_US: u64 = 500_000;
const TAP_DRAG_TIMEOUT_US: u64 = 300_000;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

#[derive(Default, Clone, Copy)]
struct TouchState {
    down_x: f64,
    down_y: f64,
    start_x: f64,
    start_y: f64,
    x: f64,
    y: f64,
    active: bool,
    start_time: u64,
    moved: bool,
    is_palm: bool,
    edge_scroll_axis: Option<EdgeScrollAxis>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EdgeScrollAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy)]
struct TapRecord {
    time_us: u64,
    button: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GestureKind {
    Swipe,
    Pinch,
    Hold,
}

#[derive(Clone, Copy)]
struct GestureState {
    kind: GestureKind,
    fingers: usize,
    last_center_x: f64,
    last_center_y: f64,
    initial_distance: f64,
    initial_angle: f64,
}

pub struct TouchpadMotionFilter {
    config: TouchpadConfig,
    slots: [TouchState; MAX_TOUCH_SLOTS],

    pending_dx: f64,
    pending_dy: f64,
    slots_moved: u16,

    fingers_at_start: usize,
    current_gesture_is_tap: bool,
    active_gesture: Option<GestureState>,
    last_tap: Option<TapRecord>,
    tap_drag_button: Option<u32>,
    clickpad_button: Option<u32>,
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
            active_gesture: None,
            last_tap: None,
            tap_drag_button: None,
            clickpad_button: None,
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.slots = [TouchState::default(); MAX_TOUCH_SLOTS];
        self.clear_pending_motion();
        self.fingers_at_start = 0;
        self.current_gesture_is_tap = true;
        self.active_gesture = None;
        self.last_tap = None;
        self.tap_drag_button = None;
        self.clickpad_button = None;
    }

    fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active && !s.is_palm).count()
    }

    fn active_slots(&self) -> ([usize; MAX_TOUCH_SLOTS], usize) {
        let mut slots = [0; MAX_TOUCH_SLOTS];
        let mut count = 0;
        for (idx, slot) in self.slots.iter().enumerate() {
            if slot.active && !slot.is_palm {
                slots[count] = idx;
                count += 1;
            }
        }
        (slots, count)
    }

    fn center_for(
        &self,
        slots: &[usize; MAX_TOUCH_SLOTS],
        count: usize,
        previous: bool,
    ) -> Option<(f64, f64)> {
        if count == 0 {
            return None;
        }

        let mut x = 0.0;
        let mut y = 0.0;
        for idx in slots.iter().take(count) {
            let slot = self.slots[*idx];
            if previous {
                x += slot.start_x;
                y += slot.start_y;
            } else {
                x += slot.x;
                y += slot.y;
            }
        }

        Some((x / count as f64, y / count as f64))
    }

    fn two_finger_geometry(
        &self,
        slots: &[usize; MAX_TOUCH_SLOTS],
        count: usize,
        previous: bool,
    ) -> Option<(f64, f64, f64, f64)> {
        if count != 2 {
            return None;
        }

        let a = self.slots[slots[0]];
        let b = self.slots[slots[1]];
        let (ax, ay) = if previous {
            (a.start_x, a.start_y)
        } else {
            (a.x, a.y)
        };
        let (bx, by) = if previous {
            (b.start_x, b.start_y)
        } else {
            (b.x, b.y)
        };

        let dx = bx - ax;
        let dy = by - ay;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance <= f64::EPSILON {
            return None;
        }

        Some(((ax + bx) * 0.5, (ay + by) * 0.5, distance, dy.atan2(dx)))
    }

    fn normalize_angle_delta(mut angle: f64) -> f64 {
        while angle > PI {
            angle -= PI * 2.0;
        }
        while angle < -PI {
            angle += PI * 2.0;
        }
        angle
    }

    fn reset_frame_starts(&mut self) {
        for slot in &mut self.slots {
            if slot.active {
                slot.start_x = slot.x;
                slot.start_y = slot.y;
            }
        }
    }

    fn clear_pending_motion(&mut self) {
        self.pending_dx = 0.0;
        self.pending_dy = 0.0;
        self.slots_moved = 0;
    }

    fn end_active_gesture(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        cancelled: bool,
        results: &mut FilterResult,
    ) {
        let Some(gesture) = self.active_gesture.take() else {
            return;
        };

        match gesture.kind {
            GestureKind::Swipe => {
                results.push(InputEvent::GestureSwipeEnd(Box::new(GestureEndData {
                    device_index,
                    time_us,
                    cancelled,
                })));
            }
            GestureKind::Pinch => {
                results.push(InputEvent::GesturePinchEnd(Box::new(GestureEndData {
                    device_index,
                    time_us,
                    cancelled,
                })));
            }
            GestureKind::Hold => {
                results.push(InputEvent::GestureHoldEnd(Box::new(GestureHoldEndData {
                    device_index,
                    time_us,
                    fingers: gesture.fingers as u32,
                    cancelled,
                })));
            }
        }
    }

    fn begin_swipe(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        slots: &[usize; MAX_TOUCH_SLOTS],
        count: usize,
        previous_center: (f64, f64),
        results: &mut FilterResult,
    ) {
        self.current_gesture_is_tap = false;
        self.active_gesture = Some(GestureState {
            kind: GestureKind::Swipe,
            fingers: count,
            last_center_x: previous_center.0,
            last_center_y: previous_center.1,
            initial_distance: 0.0,
            initial_angle: 0.0,
        });
        results.push(InputEvent::GestureSwipeBegin(Box::new(GestureSwipeData {
            device_index,
            time_us,
            fingers: count as u32,
        })));
        self.update_active_gesture(device_index, time_us, slots, count, results);
    }

    fn begin_pinch(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        slots: &[usize; MAX_TOUCH_SLOTS],
        count: usize,
        previous_geometry: (f64, f64, f64, f64),
        results: &mut FilterResult,
    ) {
        self.current_gesture_is_tap = false;
        self.active_gesture = Some(GestureState {
            kind: GestureKind::Pinch,
            fingers: count,
            last_center_x: previous_geometry.0,
            last_center_y: previous_geometry.1,
            initial_distance: previous_geometry.2,
            initial_angle: previous_geometry.3,
        });
        results.push(InputEvent::GesturePinchBegin(Box::new(
            GesturePinchBeginData {
                device_index,
                time_us,
                fingers: count as u32,
            },
        )));
        self.update_active_gesture(device_index, time_us, slots, count, results);
    }

    fn begin_hold(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        count: usize,
        center: (f64, f64),
        results: &mut FilterResult,
    ) {
        self.current_gesture_is_tap = false;
        self.active_gesture = Some(GestureState {
            kind: GestureKind::Hold,
            fingers: count,
            last_center_x: center.0,
            last_center_y: center.1,
            initial_distance: 0.0,
            initial_angle: 0.0,
        });
        results.push(InputEvent::GestureHoldBegin(Box::new(
            GestureHoldBeginData {
                device_index,
                time_us,
                fingers: count as u32,
            },
        )));
    }

    fn update_active_gesture(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        slots: &[usize; MAX_TOUCH_SLOTS],
        count: usize,
        results: &mut FilterResult,
    ) -> bool {
        let Some(mut gesture) = self.active_gesture else {
            return false;
        };

        if count == 0 || gesture.fingers != count {
            self.end_active_gesture(device_index, time_us, true, results);
            return true;
        }

        match gesture.kind {
            GestureKind::Swipe => {
                if count < 3 {
                    self.end_active_gesture(device_index, time_us, true, results);
                    return true;
                }
                if let Some((center_x, center_y)) = self.center_for(slots, count, false) {
                    let dx = center_x - gesture.last_center_x;
                    let dy = center_y - gesture.last_center_y;
                    if dx != 0.0 || dy != 0.0 {
                        results.push(InputEvent::GestureSwipeUpdate(Box::new(GestureMoveData {
                            device_index,
                            time_us,
                            dx,
                            dy,
                        })));
                    }
                    gesture.last_center_x = center_x;
                    gesture.last_center_y = center_y;
                    self.active_gesture = Some(gesture);
                }
            }
            GestureKind::Pinch => {
                if count != 2 {
                    self.end_active_gesture(device_index, time_us, true, results);
                    return true;
                }
                if let Some((center_x, center_y, distance, angle)) =
                    self.two_finger_geometry(slots, count, false)
                {
                    let dx = center_x - gesture.last_center_x;
                    let dy = center_y - gesture.last_center_y;
                    let scale = if gesture.initial_distance > 0.0 {
                        distance / gesture.initial_distance
                    } else {
                        1.0
                    };
                    let angle = Self::normalize_angle_delta(angle - gesture.initial_angle);
                    results.push(InputEvent::GesturePinchUpdate(Box::new(
                        GesturePinchUpdateData {
                            device_index,
                            time_us,
                            dx,
                            dy,
                            scale,
                            angle,
                        },
                    )));
                    gesture.last_center_x = center_x;
                    gesture.last_center_y = center_y;
                    self.active_gesture = Some(gesture);
                } else {
                    self.end_active_gesture(device_index, time_us, true, results);
                }
            }
            GestureKind::Hold => {
                self.end_active_gesture(device_index, time_us, true, results);
            }
        }

        true
    }

    fn maybe_begin_hold(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        slots: &[usize; MAX_TOUCH_SLOTS],
        count: usize,
        results: &mut FilterResult,
    ) {
        if self.active_gesture.is_some() || count == 0 {
            return;
        }

        let mut start_time = u64::MAX;
        for idx in slots.iter().take(count) {
            start_time = start_time.min(self.slots[*idx].start_time);
        }

        if time_us.saturating_sub(start_time) >= HOLD_TIMEOUT_US {
            if let Some(center) = self.center_for(slots, count, false) {
                self.begin_hold(device_index, time_us, count, center, results);
            }
        }
    }

    fn take_tap_drag_button(&mut self, time_us: u64) -> Option<u32> {
        if !self.config.tap_drag {
            self.last_tap = None;
            return None;
        }

        let Some(last_tap) = self.last_tap else {
            return None;
        };

        if time_us.saturating_sub(last_tap.time_us) <= TAP_DRAG_TIMEOUT_US {
            self.last_tap = None;
            Some(last_tap.button)
        } else {
            self.last_tap = None;
            None
        }
    }

    fn is_dwt_active(&self, time_us: u64, seat: Option<&Seat>) -> bool {
        if !self.config.dwt {
            return false;
        }
        if let Some(seat) = seat {
            if seat.last_key_time > 0 {
                return time_us.saturating_sub(seat.last_key_time) < DWT_TIMEOUT_US;
            }
        }
        false
    }

    fn is_in_edge_zone(&self, x: f64, y: f64, info: Option<DeviceInfo>) -> bool {
        if let Some(info) = info {
            let width = info.x.max - info.x.min;
            let height = info.y.max - info.y.min;
            if width <= 0.0 || height <= 0.0 {
                return false;
            }

            let edge_x = width * self.config.palm_edge_ratio;
            let edge_y = height * self.config.palm_edge_ratio;

            x < (info.x.min + edge_x)
                || x > (info.x.max - edge_x)
                || y < (info.y.min + edge_y)
                || y > (info.y.max - edge_y)
        } else {
            false
        }
    }

    fn edge_scroll_axis(&self, x: f64, y: f64, info: Option<DeviceInfo>) -> Option<EdgeScrollAxis> {
        if self.config.scroll_method != TouchpadScrollMethod::Edge {
            return None;
        }

        let Some(info) = info else {
            return None;
        };

        let width = info.x.max - info.x.min;
        let height = info.y.max - info.y.min;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        let edge_x = width * self.config.edge_scroll_edge_ratio;
        let edge_y = height * self.config.edge_scroll_edge_ratio;
        if x >= info.x.max - edge_x {
            Some(EdgeScrollAxis::Vertical)
        } else if y >= info.y.max - edge_y {
            Some(EdgeScrollAxis::Horizontal)
        } else {
            None
        }
    }

    fn is_in_thumb_zone(&self, y: f64, info: Option<DeviceInfo>) -> bool {
        let Some(info) = info else {
            return false;
        };

        let height = info.y.max - info.y.min;
        if height <= 0.0 {
            return false;
        }

        y >= info.y.max - height * self.config.thumb_edge_ratio
    }

    fn first_active_touch(&self) -> Option<TouchState> {
        self.slots
            .iter()
            .copied()
            .find(|slot| slot.active && !slot.is_palm)
    }

    fn button_area_button(&self, info: Option<DeviceInfo>) -> u32 {
        let Some(touch) = self.first_active_touch() else {
            return BTN_LEFT;
        };
        let Some(info) = info else {
            return BTN_LEFT;
        };

        let width = info.x.max - info.x.min;
        let height = info.y.max - info.y.min;
        if width <= 0.0 || height <= 0.0 {
            return BTN_LEFT;
        }

        let bottom_edge = info.y.min + height * 0.80;
        if touch.y < bottom_edge {
            return BTN_LEFT;
        }

        let rel_x = (touch.x - info.x.min) / width;
        if rel_x >= 2.0 / 3.0 {
            BTN_RIGHT
        } else if rel_x >= 1.0 / 3.0 {
            BTN_MIDDLE
        } else {
            BTN_LEFT
        }
    }

    fn clickfinger_button(&self) -> u32 {
        match self.active_count() {
            2 => BTN_RIGHT,
            3.. => BTN_MIDDLE,
            _ => BTN_LEFT,
        }
    }

    fn click_method_button(&self, info: Option<DeviceInfo>) -> Option<u32> {
        match self.config.click_method {
            TouchpadClickMethod::None => None,
            TouchpadClickMethod::ButtonAreas => Some(self.button_area_button(info)),
            TouchpadClickMethod::Clickfinger => Some(self.clickfinger_button()),
        }
    }

    fn active_edge_scroll_axis(&self) -> Option<EdgeScrollAxis> {
        self.slots
            .iter()
            .find(|slot| slot.active && !slot.is_palm && slot.edge_scroll_axis.is_some())
            .and_then(|slot| slot.edge_scroll_axis)
    }

    fn get_dist_mm(&self, dx: f64, dy: f64, info: Option<DeviceInfo>) -> f64 {
        if let Some(info) = info {
            let res_x = if info.x.resolution > 0.0 {
                info.x.resolution
            } else {
                1.0
            };
            let res_y = if info.y.resolution > 0.0 {
                info.y.resolution
            } else {
                1.0
            };
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

    /// Enable or disable tap-and-drag recognition in place.
    pub fn set_tap_drag(&mut self, enabled: bool) {
        self.config.tap_drag = enabled;
        if !enabled {
            self.last_tap = None;
        }
    }

    /// Store drag-lock configuration. Full drag-lock behavior is not implemented yet.
    pub fn set_tap_drag_lock(&mut self, enabled: bool) {
        self.config.tap_drag_lock = enabled;
    }

    /// Store the selected click method for future clickpad/clickfinger processing.
    pub fn set_click_method(&mut self, method: TouchpadClickMethod) {
        self.config.click_method = method;
    }

    /// Select the touchpad scroll method for future contacts.
    pub fn set_scroll_method(&mut self, method: TouchpadScrollMethod) {
        self.config.scroll_method = method;
    }
}

impl Filter for TouchpadMotionFilter {
    fn process(
        &mut self,
        event: InputEvent,
        seat: Option<&Seat>,
        info: Option<DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::TouchDown(data) => {
                let TouchData {
                    device_index,
                    time_us,
                    slot,
                    x,
                    y,
                    tool_type,
                    ..
                } = *data;
                let s = slot as usize;
                let mut results = FilterResult::new();
                if !x.is_finite() || !y.is_finite() {
                    return results;
                }
                if s < MAX_TOUCH_SLOTS {
                    let edge_scroll_axis = self.edge_scroll_axis(x, y, info.clone());
                    let active_before = self.active_count();
                    let is_thumb = self.config.thumb_detection
                        && active_before > 0
                        && edge_scroll_axis.is_none()
                        && self.is_in_thumb_zone(y, info.clone());
                    let is_edge_palm = self.config.palm_detection
                        && edge_scroll_axis.is_none()
                        && self.is_in_edge_zone(x, y, info);
                    let is_firmware_palm = self.config.palm_detection
                        && matches!(tool_type, Some(TouchToolType::Palm));
                    let is_palm = is_firmware_palm || is_thumb || is_edge_palm;

                    if active_before == 0 {
                        self.fingers_at_start = 0;
                        self.current_gesture_is_tap = true;
                        self.active_gesture = None;
                    } else if self.active_gesture.is_some() {
                        self.current_gesture_is_tap = false;
                        self.end_active_gesture(device_index, time_us, true, &mut results);
                    }
                    if !is_palm {
                        self.fingers_at_start += 1;
                    }

                    self.slots[s] = TouchState {
                        down_x: x,
                        down_y: y,
                        start_x: x,
                        start_y: y,
                        x,
                        y,
                        active: true,
                        start_time: time_us,
                        moved: false,
                        is_palm,
                        edge_scroll_axis,
                    };

                    if active_before == 0 && !is_palm {
                        if let Some(button) = self.take_tap_drag_button(time_us) {
                            self.current_gesture_is_tap = false;
                            self.tap_drag_button = Some(button);
                            results.push(InputEvent::PointerButton {
                                device_index,
                                time_us,
                                button,
                                pressed: true,
                            });
                        }
                    }
                }
                results
            }
            InputEvent::TouchUp(data) => {
                let TouchEndData {
                    device_index,
                    time_us,
                    slot,
                } = *data;
                let s = slot as usize;
                let mut results = FilterResult::new();

                if s < MAX_TOUCH_SLOTS && self.slots[s].active {
                    let was_palm = self.slots[s].is_palm;
                    self.slots[s].active = false;

                    if was_palm {
                        return results;
                    }

                    if self.active_gesture.is_some() {
                        self.current_gesture_is_tap = false;
                        self.end_active_gesture(device_index, time_us, false, &mut results);
                        return results;
                    }

                    if let Some(button) = self.tap_drag_button {
                        if self.active_count() == 0 {
                            self.current_gesture_is_tap = false;
                            self.tap_drag_button = None;
                            results.push(InputEvent::PointerButton {
                                device_index,
                                time_us,
                                button,
                                pressed: false,
                            });
                            return results;
                        }
                    }

                    let duration = time_us.saturating_sub(self.slots[s].start_time);
                    let dx = self.slots[s].x - self.slots[s].down_x;
                    let dy = self.slots[s].y - self.slots[s].down_y;
                    let dist_mm = self.get_dist_mm(dx, dy, info);

                    let is_tap_like = duration < TAP_TIMEOUT_US
                        && dist_mm < TAP_THRESHOLD_MM
                        && !self.slots[s].moved;
                    if !is_tap_like {
                        self.current_gesture_is_tap = false;
                    }

                    if self.active_count() == 0 {
                        if self.current_gesture_is_tap && self.config.tap_to_click {
                            if !self.is_dwt_active(time_us, seat) {
                                let button = if self.fingers_at_start == 1 {
                                    Some(BTN_LEFT)
                                } else if self.fingers_at_start == 2
                                    && self.config.two_finger_tap_right_click
                                {
                                    Some(BTN_RIGHT)
                                } else if self.fingers_at_start == 3 {
                                    Some(BTN_MIDDLE)
                                } else {
                                    None
                                };

                                if let Some(btn) = button {
                                    if self.config.tap_drag {
                                        self.last_tap = Some(TapRecord {
                                            time_us,
                                            button: btn,
                                        });
                                    }
                                    results.push(InputEvent::PointerButton {
                                        device_index,
                                        time_us,
                                        button: btn,
                                        pressed: true,
                                    });
                                    results.push(InputEvent::PointerButton {
                                        device_index,
                                        time_us,
                                        button: btn,
                                        pressed: false,
                                    });
                                }
                            }
                        }
                    }
                }
                results
            }
            InputEvent::TouchCancel(data) => {
                let TouchEndData {
                    device_index,
                    time_us,
                    slot,
                } = *data;
                let s = slot as usize;
                let mut results = FilterResult::new();

                if s < MAX_TOUCH_SLOTS && self.slots[s].active {
                    let was_palm = self.slots[s].is_palm;
                    self.slots[s] = TouchState::default();
                    self.current_gesture_is_tap = false;
                    self.last_tap = None;
                    self.clear_pending_motion();

                    if !was_palm && self.active_gesture.is_some() {
                        self.end_active_gesture(device_index, time_us, true, &mut results);
                    }

                    if let Some(button) = self.tap_drag_button.take() {
                        results.push(InputEvent::PointerButton {
                            device_index,
                            time_us,
                            button,
                            pressed: false,
                        });
                    }

                    if self.active_count() == 0 {
                        self.fingers_at_start = 0;
                    }
                }

                results
            }
            InputEvent::TouchMotion(data) => {
                let TouchData {
                    device_index: _,
                    time_us: _,
                    slot,
                    x,
                    y,
                    ..
                } = *data;
                let s = slot as usize;
                if !x.is_finite() || !y.is_finite() {
                    return smallvec![];
                }
                if s < MAX_TOUCH_SLOTS && self.slots[s].active && !self.slots[s].is_palm {
                    if !self.slots[s].moved {
                        let dx = x - self.slots[s].down_x;
                        let dy = y - self.slots[s].down_y;
                        let dist_mm = self.get_dist_mm(dx, dy, info);
                        if dist_mm >= TAP_THRESHOLD_MM {
                            self.slots[s].moved = true;
                            self.current_gesture_is_tap = false;
                        }
                    }

                    // Accumulate delta for this frame
                    self.pending_dx += x - self.slots[s].x;
                    self.pending_dy += y - self.slots[s].y;
                    self.slots_moved = self.slots_moved.saturating_add(1);

                    self.slots[s].x = x;
                    self.slots[s].y = y;
                }
                smallvec![]
            }
            InputEvent::PointerButton {
                device_index,
                time_us,
                button,
                pressed,
            } => {
                if pressed {
                    self.current_gesture_is_tap = false;
                    self.last_tap = None;
                }

                if button != BTN_LEFT {
                    return smallvec![InputEvent::PointerButton {
                        device_index,
                        time_us,
                        button,
                        pressed,
                    }];
                }

                if pressed {
                    if let Some(mapped_button) = self.click_method_button(info) {
                        self.clickpad_button = Some(mapped_button);
                        return smallvec![InputEvent::PointerButton {
                            device_index,
                            time_us,
                            button: mapped_button,
                            pressed: true,
                        }];
                    }
                } else if let Some(mapped_button) = self.clickpad_button.take() {
                    return smallvec![InputEvent::PointerButton {
                        device_index,
                        time_us,
                        button: mapped_button,
                        pressed: false,
                    }];
                }

                smallvec![InputEvent::PointerButton {
                    device_index,
                    time_us,
                    button,
                    pressed,
                }]
            }
            InputEvent::Frame {
                device_index,
                time_us,
            } => {
                let mut results = FilterResult::new();

                // Suppress motion while DWT is active
                if self.is_dwt_active(time_us, seat) {
                    if self.active_gesture.is_some() {
                        self.end_active_gesture(device_index, time_us, true, &mut results);
                    }
                    self.clear_pending_motion();
                    self.reset_frame_starts();
                    return results;
                }

                let (active_slots, active_count) = self.active_slots();
                if active_count == 0 {
                    if self.active_gesture.is_some() {
                        self.end_active_gesture(device_index, time_us, true, &mut results);
                    }
                    self.clear_pending_motion();
                    return results;
                }

                if self.slots_moved == 0 {
                    self.maybe_begin_hold(
                        device_index,
                        time_us,
                        &active_slots,
                        active_count,
                        &mut results,
                    );
                    self.reset_frame_starts();
                    return results;
                }

                let pending_dx = self.pending_dx;
                let pending_dy = self.pending_dy;
                self.clear_pending_motion();

                let mut consumed_by_gesture = self.update_active_gesture(
                    device_index,
                    time_us,
                    &active_slots,
                    active_count,
                    &mut results,
                );

                if !consumed_by_gesture && active_count >= 3 {
                    let previous_center = self.center_for(&active_slots, active_count, true);
                    let current_center = self.center_for(&active_slots, active_count, false);
                    if let (Some(previous_center), Some(current_center)) =
                        (previous_center, current_center)
                    {
                        let dx = current_center.0 - previous_center.0;
                        let dy = current_center.1 - previous_center.1;
                        if self.get_dist_mm(dx, dy, info.clone()) >= GESTURE_MOTION_THRESHOLD_MM {
                            self.begin_swipe(
                                device_index,
                                time_us,
                                &active_slots,
                                active_count,
                                previous_center,
                                &mut results,
                            );
                            consumed_by_gesture = true;
                        }
                    }
                }

                if !consumed_by_gesture && active_count == 2 {
                    let previous = self.two_finger_geometry(&active_slots, active_count, true);
                    let current = self.two_finger_geometry(&active_slots, active_count, false);
                    if let (Some(previous), Some(current)) = (previous, current) {
                        let scale_delta = (current.2 / previous.2 - 1.0).abs();
                        let angle_delta = Self::normalize_angle_delta(current.3 - previous.3);
                        if scale_delta >= PINCH_SCALE_THRESHOLD
                            || angle_delta.abs() >= PINCH_ROTATION_THRESHOLD_RAD
                        {
                            self.begin_pinch(
                                device_index,
                                time_us,
                                &active_slots,
                                active_count,
                                previous,
                                &mut results,
                            );
                            consumed_by_gesture = true;
                        }
                    }
                }

                if consumed_by_gesture {
                    self.reset_frame_starts();
                    return results;
                }

                if active_count == 1 {
                    if let Some(axis) = self.active_edge_scroll_axis() {
                        let (mut scroll_dx, mut scroll_dy) = match axis {
                            EdgeScrollAxis::Vertical => (0.0, pending_dy),
                            EdgeScrollAxis::Horizontal => (pending_dx, 0.0),
                        };
                        if self.config.natural_scrolling {
                            scroll_dx = -scroll_dx;
                            scroll_dy = -scroll_dy;
                        }
                        results.push(InputEvent::PointerScroll {
                            device_index,
                            time_us,
                            source: ScrollSource::Finger,
                            dx: scroll_dx,
                            dy: scroll_dy,
                        });
                        self.reset_frame_starts();
                        return results;
                    }
                    results.push(InputEvent::PointerMotion {
                        device_index,
                        time_us,
                        dx: pending_dx,
                        dy: pending_dy,
                        dx_unaccelerated: pending_dx,
                        dy_unaccelerated: pending_dy,
                    });
                } else if active_count == 2
                    && self.config.scroll_method == TouchpadScrollMethod::TwoFinger
                {
                    let (scroll_dx, scroll_dy) = match (
                        self.center_for(&active_slots, active_count, true),
                        self.center_for(&active_slots, active_count, false),
                    ) {
                        (Some(previous_center), Some(current_center)) => (
                            current_center.0 - previous_center.0,
                            current_center.1 - previous_center.1,
                        ),
                        _ => (
                            pending_dx / active_count as f64,
                            pending_dy / active_count as f64,
                        ),
                    };
                    let scroll_dx = if self.config.natural_scrolling {
                        -scroll_dx
                    } else {
                        scroll_dx
                    };
                    let scroll_dy = if self.config.natural_scrolling {
                        -scroll_dy
                    } else {
                        scroll_dy
                    };
                    results.push(InputEvent::PointerScroll {
                        device_index,
                        time_us,
                        source: ScrollSource::Finger,
                        dx: scroll_dx,
                        dy: scroll_dy,
                    });
                }
                self.reset_frame_starts();
                results
            }
            _ => smallvec![event],
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            backend::{AxisInfo, DeviceInfo},
            events::DevIndex,
        },
    };

    fn mock_info() -> DeviceInfo {
        DeviceInfo {
            x: AxisInfo {
                min: 0.0,
                max: 1000.0,
                resolution: 10.0,
            },
            y: AxisInfo {
                min: 0.0,
                max: 1000.0,
                resolution: 10.0,
            },
            ..Default::default()
        }
    }

    fn dev() -> DevIndex {
        DevIndex(0)
    }

    fn plain_touch_data() -> TouchData {
        TouchData {
            device_index: dev(),
            time_us: 0,
            slot: 0,
            x: 0.0,
            y: 0.0,
            pressure: None,
            touch_major: None,
            touch_minor: None,
            tool_type: None,
            tracking_id: None,
        }
    }

    #[test]
    fn test_palm_detection() {
        let config = TouchpadConfig::default();
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 0,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 100,
                slot: 0,
                x: 510.0,
                y: 510.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        let ev = f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 100,
            },
            None,
            Some(info.clone()),
        );
        assert!(!ev.is_empty(), "Center touch should produce motion");

        f.process(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 0,
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 200,
            },
            None,
            Some(info.clone()),
        );

        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 300,
                slot: 0,
                x: 10.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 400,
                slot: 0,
                x: 20.0,
                y: 510.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        let ev_palm = f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 400,
            },
            None,
            Some(info.clone()),
        );
        assert!(
            ev_palm.is_empty(),
            "Edge touch should be detected as palm and ignored"
        );
    }

    #[test]
    fn test_touchpad_complex_gestures() {
        let mut config = TouchpadConfig::default();
        config.natural_scrolling = true;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 0,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 0,
                slot: 1,
                x: 600.0,
                y: 600.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 100,
                slot: 0,
                x: 500.0,
                y: 505.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        let evs = f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 100,
            },
            None,
            Some(info.clone()),
        );

        // Two-finger scroll follows center movement. One of two fingers moved
        // by dy=5, so the center moved by dy=2.5 and natural scrolling reverses it.
        if let Some(InputEvent::PointerScroll { dy, .. }) = evs.get(0) {
            assert_eq!(*dy, -2.5, "Expected scroll dy=-2.5, got {}", dy);
        } else {
            panic!("Expected PointerScroll, got {:?}", evs.get(0));
        }

        f.process(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 0,
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 1,
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 200,
            },
            None,
            Some(info.clone()),
        );

        // Two-finger tap → right click
        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 300,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 305,
                slot: 1,
                x: 600.0,
                y: 600.0,
                ..plain_touch_data()
            })),
            None,
            Some(info.clone()),
        );

        f.process(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 350,
                slot: 0,
            })),
            None,
            Some(info.clone()),
        );
        let evs_tap = f.process(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 355,
                slot: 1,
            })),
            None,
            Some(info.clone()),
        );

        assert_eq!(evs_tap.len(), 2);
        assert!(matches!(
            evs_tap[0],
            InputEvent::PointerButton {
                button: BTN_RIGHT,
                pressed: true,
                ..
            }
        ));
        assert!(matches!(
            evs_tap[1],
            InputEvent::PointerButton {
                button: BTN_RIGHT,
                pressed: false,
                ..
            }
        ));
    }

    #[test]
    fn test_dwt() {
        use crate::state::Seat;
        let config = TouchpadConfig::default();
        let mut f = TouchpadMotionFilter::new(config);
        let mut seat = Seat {
            name: "seat0".into(),
            devices: vec![],
            last_key_time: 1000,
        };
        let info = mock_info();

        f.process(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 1100,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            Some(&seat),
            Some(info.clone()),
        );
        f.process(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 1200,
                slot: 0,
                x: 512.0,
                y: 11.0,
                ..plain_touch_data()
            })),
            Some(&seat),
            Some(info.clone()),
        );
        let ev = f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 1200,
            },
            Some(&seat),
            Some(info.clone()),
        );
        assert!(ev.is_empty(), "Motion should be suppressed by DWT");

        seat.last_key_time = 1000;
        let time_after = 1000 + DWT_TIMEOUT_US + 100;
        f.process(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: time_after,
                slot: 0,
                x: 515.0,
                y: 13.0,
                ..plain_touch_data()
            })),
            Some(&seat),
            Some(info.clone()),
        );
        let ev2 = f.process(
            InputEvent::Frame {
                device_index: dev(),
                time_us: time_after,
            },
            Some(&seat),
            Some(info.clone()),
        );
        assert!(
            !ev2.is_empty(),
            "Motion should NOT be suppressed after DWT timeout"
        );
    }
}
