use {
    super::{
        Filter, FilterResult, Step,
        config::{
            ClickfingerButtonMap, TapButtonMap, TouchpadClickMethod, TouchpadConfig,
            TouchpadScrollMethod,
        },
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
    std::f64::consts::PI,
};

const MAX_TOUCH_SLOTS: usize = 10;

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

    /// Logical button held by drag lock after the drag contact lifted.
    drag_lock_button: Option<u32>,
    /// Optional absolute deadline after which the drag-locked button is
    /// released without another contact.
    drag_lock_deadline_us: Option<u64>,
    /// Contact that is currently continuing a drag-locked drag, if any.
    drag_lock_contact: Option<usize>,
    /// Device index owning the drag-locked button, for timeout release.
    drag_lock_device: Option<crate::events::DevIndex>,
    /// Once a two-finger sequence has produced scrolling, it stays a scroll
    /// sequence; a later spacing change must not promote it to a pinch.
    two_finger_scroll_latched: bool,
}

impl TouchpadMotionFilter {
    pub fn new(config: TouchpadConfig) -> Self {
        Self {
            config: config.sanitized(),
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
            drag_lock_button: None,
            drag_lock_deadline_us: None,
            drag_lock_contact: None,
            drag_lock_device: None,
            two_finger_scroll_latched: false,
        }
    }

    /// Current effective configuration, including runtime and profile updates.
    pub fn config(&self) -> &TouchpadConfig {
        &self.config
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
        self.drag_lock_button = None;
        self.drag_lock_deadline_us = None;
        self.drag_lock_contact = None;
        self.drag_lock_device = None;
        self.two_finger_scroll_latched = false;
    }

    fn clickpad(&self, info: Option<&DeviceInfo>) -> bool {
        self.config
            .clickpad_override
            .unwrap_or_else(|| info.is_some_and(|i| i.is_clickpad))
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

        if time_us.saturating_sub(start_time) >= self.config.hold_timeout_us {
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

        let timeout = self.config.tap_drag_timeout_us;
        if time_us.saturating_sub(last_tap.time_us) <= timeout {
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
                return time_us.saturating_sub(seat.last_key_time) < self.config.dwt_timeout_us;
            }
        }
        false
    }

    fn is_dwtp_active(&self, time_us: u64, seat: Option<&Seat>) -> bool {
        if !self.config.dwtp {
            return false;
        }
        if let Some(seat) = seat {
            if seat.last_trackpoint_time > 0 {
                return time_us.saturating_sub(seat.last_trackpoint_time)
                    < self.config.dwtp_timeout_us;
            }
        }
        false
    }

    fn is_stylus_active(&self, seat: Option<&Seat>) -> bool {
        self.config.stylus_arbitration && seat.is_some_and(|seat| seat.stylus_in_proximity)
    }

    fn is_pointer_suppressed(&self, time_us: u64, seat: Option<&Seat>) -> bool {
        self.is_dwt_active(time_us, seat)
            || self.is_dwtp_active(time_us, seat)
            || self.is_stylus_active(seat)
    }

    /// Button produced by a tap with `fingers` contacts, or `None` when the
    /// configured map does not produce a button for that count.
    fn tap_button_for_fingers(&self, fingers: usize) -> Option<u32> {
        if fingers == 0 || fingers > self.config.max_tap_fingers as usize {
            return None;
        }

        match fingers {
            1 => Some(BTN_LEFT),
            2 if self.config.two_finger_tap_right_click => {
                Some(self.config.tap_button_map.two_finger_button())
            }
            2 => None,
            3 => Some(self.config.tap_button_map.three_finger_button()),
            _ => None,
        }
    }

    /// Release the drag-locked logical button, if one is held.
    fn release_drag_lock(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        results: &mut FilterResult,
    ) {
        self.drag_lock_deadline_us = None;
        self.drag_lock_contact = None;
        self.drag_lock_device = None;
        if let Some(button) = self.drag_lock_button.take() {
            self.current_gesture_is_tap = false;
            results.push(InputEvent::PointerButton {
                device_index,
                time_us,
                button,
                pressed: false,
            });
        }
    }

    /// Release a drag-locked button whose configured timeout has elapsed.
    fn expire_drag_lock(
        &mut self,
        device_index: crate::events::DevIndex,
        time_us: u64,
        results: &mut FilterResult,
    ) {
        if let Some(deadline) = self.drag_lock_deadline_us {
            if time_us >= deadline {
                self.release_drag_lock(device_index, time_us, results);
            }
        }
    }

    fn is_in_edge_zone(&self, x: f64, y: f64, info: Option<&DeviceInfo>) -> bool {
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

    fn edge_scroll_axis(
        &self,
        x: f64,
        y: f64,
        info: Option<&DeviceInfo>,
    ) -> Option<EdgeScrollAxis> {
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

    fn is_in_thumb_zone(&self, y: f64, info: Option<&DeviceInfo>) -> bool {
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

    fn button_area_button(&self, info: Option<&DeviceInfo>) -> u32 {
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

        let bottom_edge = info.y.max - height * self.config.button_area_bottom_ratio;
        if touch.y < bottom_edge {
            return BTN_LEFT;
        }

        let rel_x = (touch.x - info.x.min) / width;
        let area_button = if rel_x >= self.config.button_area_right_ratio {
            BTN_RIGHT
        } else if rel_x >= self.config.button_area_left_ratio {
            BTN_MIDDLE
        } else {
            BTN_LEFT
        };

        // The main area above always stays the primary (left) click area.
        // Left-handed mode swaps only the explicit bottom-left/bottom-right
        // software button areas; middle stays middle. This matches libinput's
        // clickpad soft-button policy.
        if self.config.left_handed {
            match area_button {
                BTN_LEFT => BTN_RIGHT,
                BTN_RIGHT => BTN_LEFT,
                other => other,
            }
        } else {
            area_button
        }
    }

    fn clickfinger_button(&self) -> u32 {
        self.config
            .clickfinger_button_map
            .button_for_fingers(self.active_count())
    }

    fn click_method_button(&self, info: Option<&DeviceInfo>) -> Option<u32> {
        if !self.clickpad(info.clone()) {
            return None;
        }

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

    fn get_dist_mm(&self, dx: f64, dy: f64, info: Option<&DeviceInfo>) -> f64 {
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

    /// Enable or disable drag lock.
    ///
    /// Disabling drag lock while a logical button is held schedules its release
    /// on the next processed event rather than leaving the button stuck.
    pub fn set_tap_drag_lock(&mut self, enabled: bool) {
        self.config.tap_drag_lock = enabled;
        if !enabled && self.drag_lock_button.is_some() {
            self.drag_lock_deadline_us = Some(0);
        }
    }

    /// Select the tap button map.
    pub fn set_tap_button_map(&mut self, map: TapButtonMap) {
        self.config.tap_button_map = map;
    }

    /// Select the clickfinger button map.
    pub fn set_clickfinger_button_map(&mut self, map: ClickfingerButtonMap) {
        self.config.clickfinger_button_map = map;
    }

    /// Replace the tap timing and movement thresholds.
    pub fn set_tap_timing(
        &mut self,
        tap_timeout_us: u64,
        tap_move_threshold_mm: f64,
        tap_drag_timeout_us: u64,
        drag_lock_timeout_us: u64,
    ) {
        self.config.tap_timeout_us = tap_timeout_us;
        if tap_move_threshold_mm.is_finite() && tap_move_threshold_mm > 0.0 {
            self.config.tap_move_threshold_mm = tap_move_threshold_mm;
        }
        self.config.tap_drag_timeout_us = tap_drag_timeout_us;
        self.config.drag_lock_timeout_us = drag_lock_timeout_us;
        if drag_lock_timeout_us > 0 && self.drag_lock_button.is_some() {
            self.drag_lock_deadline_us = None;
        }
    }

    /// Set the maximum number of fingers allowed to produce a tap.
    pub fn set_max_tap_fingers(&mut self, fingers: u8) {
        self.config.max_tap_fingers = fingers.clamp(1, 3);
    }

    /// Set the selected click method for future clickpad/clickfinger processing.
    pub fn set_click_method(&mut self, method: TouchpadClickMethod) {
        self.config.click_method = method;
    }

    /// Override clickpad detection for this device.
    pub fn set_clickpad_override(&mut self, clickpad: Option<bool>) {
        self.config.clickpad_override = clickpad;
    }

    /// Select the touchpad scroll method for future contacts.
    pub fn set_scroll_method(&mut self, method: TouchpadScrollMethod) {
        self.config.scroll_method = method;
    }

    /// Apply the touchpad-specific left-handed clickpad policy.
    pub fn set_left_handed(&mut self, enabled: bool) {
        self.config.left_handed = enabled;
    }

    /// Configure raw pressure/size palm thresholds and the thumb pressure
    /// threshold. `None` disables the corresponding heuristic.
    pub fn set_palm_thresholds(
        &mut self,
        pressure: Option<f64>,
        size: Option<f64>,
        thumb_pressure: Option<f64>,
    ) {
        self.config.palm_pressure_threshold = pressure.filter(|v| v.is_finite() && *v > 0.0);
        self.config.palm_size_threshold = size.filter(|v| v.is_finite() && *v > 0.0);
        self.config.thumb_pressure_threshold = thumb_pressure.filter(|v| v.is_finite() && *v > 0.0);
    }

    /// Replace the disable-while-typing timeout.
    pub fn set_dwt_timeout(&mut self, timeout_us: u64) {
        self.config.dwt_timeout_us = timeout_us;
    }

    /// Enable or disable disable-while-trackpointing and its timeout.
    pub fn set_dwtp(&mut self, enabled: bool, timeout_us: u64) {
        self.config.dwtp = enabled;
        self.config.dwtp_timeout_us = timeout_us;
    }

    /// Enable or disable stylus-vs-touchpad arbitration.
    pub fn set_stylus_arbitration(&mut self, enabled: bool) {
        self.config.stylus_arbitration = enabled;
    }

    /// Replace gesture timing and thresholds.
    pub fn set_gesture_config(
        &mut self,
        hold_timeout_us: u64,
        motion_threshold_mm: f64,
        pinch_scale_threshold: f64,
        pinch_rotation_threshold_rad: f64,
    ) {
        self.config.hold_timeout_us = hold_timeout_us;
        if motion_threshold_mm.is_finite() && motion_threshold_mm > 0.0 {
            self.config.gesture_motion_threshold_mm = motion_threshold_mm;
        }
        if pinch_scale_threshold.is_finite() && pinch_scale_threshold > 0.0 {
            self.config.pinch_scale_threshold = pinch_scale_threshold;
        }
        if pinch_rotation_threshold_rad.is_finite() && pinch_rotation_threshold_rad > 0.0 {
            self.config.pinch_rotation_threshold_rad = pinch_rotation_threshold_rad;
        }
    }

    /// Apply profile defaults to the live configuration. Fields not set by the
    /// profile keep their current value.
    pub fn apply_profile(&mut self, profile: &crate::profile::TouchpadProfile) {
        profile.apply(&mut self.config);
        self.config = self.config.sanitized();
    }
}

impl Filter for TouchpadMotionFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> Step {
        if let InputEvent::TouchMotion(data) = event {
            let slot = data.slot as usize;
            let x = data.x;
            let y = data.y;
            let tool_type = data.tool_type;
            let device_index = data.device_index;
            let time_us = data.time_us;
            if !x.is_finite() || !y.is_finite() {
                return Step::Suppress;
            }
            if slot < MAX_TOUCH_SLOTS && self.slots[slot].active && !self.slots[slot].is_palm {
                if self.config.palm_detection && matches!(tool_type, Some(TouchToolType::Palm)) {
                    self.slots[slot].is_palm = true;
                    self.fingers_at_start = self.fingers_at_start.saturating_sub(1);
                    self.current_gesture_is_tap = false;
                    self.last_tap = None;
                    self.clear_pending_motion();
                    if self.active_count() == 0 && self.active_gesture.is_some() {
                        let mut results = FilterResult::new();
                        self.end_active_gesture(device_index, time_us, true, &mut results);
                        return Step::from_result(event, results);
                    }
                    return Step::Suppress;
                }

                if !self.slots[slot].moved {
                    let dx = x - self.slots[slot].down_x;
                    let dy = y - self.slots[slot].down_y;
                    let dist_mm = self.get_dist_mm(dx, dy, info);
                    if dist_mm >= self.config.tap_move_threshold_mm {
                        self.slots[slot].moved = true;
                        self.current_gesture_is_tap = false;
                    }
                }

                self.pending_dx += x - self.slots[slot].x;
                self.pending_dy += y - self.slots[slot].y;
                self.slots_moved = self.slots_moved.saturating_add(1);
                self.slots[slot].x = x;
                self.slots[slot].y = y;
            }
            return Step::Suppress;
        }

        if let InputEvent::PointerButton {
            device_index,
            time_us,
            button,
            pressed,
        } = event
        {
            let device_index = *device_index;
            let time_us = *time_us;
            let button = *button;
            let pressed = *pressed;

            if pressed {
                self.current_gesture_is_tap = false;
                self.last_tap = None;
            }

            if button != BTN_LEFT {
                let out_button = if self.config.left_handed {
                    match button {
                        BTN_RIGHT => BTN_LEFT,
                        _ => button,
                    }
                } else {
                    button
                };
                *event = InputEvent::PointerButton {
                    device_index,
                    time_us,
                    button: out_button,
                    pressed,
                };
                return Step::Keep;
            }

            if pressed {
                if let Some(mapped_button) = self.click_method_button(info) {
                    self.clickpad_button = Some(mapped_button);
                    *event = InputEvent::PointerButton {
                        device_index,
                        time_us,
                        button: mapped_button,
                        pressed: true,
                    };
                    return Step::Keep;
                }
            } else if let Some(mapped_button) = self.clickpad_button.take() {
                *event = InputEvent::PointerButton {
                    device_index,
                    time_us,
                    button: mapped_button,
                    pressed: false,
                };
                return Step::Keep;
            }

            let out_button = if self.config.left_handed {
                BTN_RIGHT
            } else {
                BTN_LEFT
            };
            *event = InputEvent::PointerButton {
                device_index,
                time_us,
                button: out_button,
                pressed,
            };
            return Step::Keep;
        }

        if matches!(event, InputEvent::DeviceRemoved { .. }) {
            self.reset_runtime_state();
            return Step::Keep;
        }

        if let InputEvent::Frame {
            device_index,
            time_us,
        } = event
        {
            let device_index = *device_index;
            let time_us = *time_us;
            return self.process_frame(event, device_index, time_us, seat, info);
        }

        let device_index = event.device_index();
        let time_us = event.time_us();
        let owned = std::mem::replace(
            event,
            InputEvent::Frame {
                device_index,
                time_us,
            },
        );

        let mut results = FilterResult::new();
        self.expire_drag_lock(device_index, time_us, &mut results);
        results.extend(self.process_inner(owned, seat, info));
        Step::from_result(event, results)
    }

    fn dispatch_timeout(&mut self, now_us: u64) -> FilterResult {
        let mut results = FilterResult::new();
        if let Some(device_index) = self.drag_lock_device {
            self.expire_drag_lock(device_index, now_us, &mut results);
        }
        results
    }

    fn next_timeout_us(&self) -> Option<u64> {
        self.drag_lock_deadline_us
    }
}

impl TouchpadMotionFilter {
    fn process_frame(
        &mut self,
        event: &mut InputEvent,
        device_index: crate::events::DevIndex,
        time_us: u64,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> Step {
        let mut results = FilterResult::new();
        self.expire_drag_lock(device_index, time_us, &mut results);

        if self.is_pointer_suppressed(time_us, seat) {
            if self.active_gesture.is_some() {
                self.end_active_gesture(device_index, time_us, true, &mut results);
            }
            self.clear_pending_motion();
            self.reset_frame_starts();
            return Step::from_result(event, results);
        }

        let (active_slots, active_count) = self.active_slots();
        if active_count != 2 {
            self.two_finger_scroll_latched = false;
        }
        if active_count == 0 {
            if self.active_gesture.is_some() {
                self.end_active_gesture(device_index, time_us, true, &mut results);
            }
            self.clear_pending_motion();
            return Step::from_result(event, results);
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
            return Step::from_result(event, results);
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
            if let (Some(previous_center), Some(current_center)) = (previous_center, current_center)
            {
                let dx = current_center.0 - previous_center.0;
                let dy = current_center.1 - previous_center.1;
                if self.get_dist_mm(dx, dy, info) >= self.config.gesture_motion_threshold_mm {
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

        if !consumed_by_gesture && active_count == 2 && !self.two_finger_scroll_latched {
            let previous = self.two_finger_geometry(&active_slots, active_count, true);
            let current = self.two_finger_geometry(&active_slots, active_count, false);
            if let (Some(previous), Some(current)) = (previous, current) {
                let scale_delta = (current.2 / previous.2 - 1.0).abs();
                let angle_delta = Self::normalize_angle_delta(current.3 - previous.3);
                if scale_delta >= self.config.pinch_scale_threshold
                    || angle_delta.abs() >= self.config.pinch_rotation_threshold_rad
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
            return Step::from_result(event, results);
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
                if scroll_dx != 0.0 || scroll_dy != 0.0 {
                    self.current_gesture_is_tap = false;
                    self.last_tap = None;
                }
                results.push(InputEvent::PointerScroll {
                    device_index,
                    time_us,
                    source: ScrollSource::Finger,
                    dx: scroll_dx,
                    dy: scroll_dy,
                });
                self.reset_frame_starts();
                return Step::from_result(event, results);
            }
            results.push(InputEvent::PointerMotion {
                device_index,
                time_us,
                dx: pending_dx,
                dy: pending_dy,
                dx_unaccelerated: pending_dx,
                dy_unaccelerated: pending_dy,
            });
        } else if active_count == 2 && self.config.scroll_method == TouchpadScrollMethod::TwoFinger
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
            if scroll_dx != 0.0 || scroll_dy != 0.0 {
                self.current_gesture_is_tap = false;
                self.last_tap = None;
                self.two_finger_scroll_latched = true;
            }
            results.push(InputEvent::PointerScroll {
                device_index,
                time_us,
                source: ScrollSource::Finger,
                dx: scroll_dx,
                dy: scroll_dy,
            });
        }
        self.reset_frame_starts();
        Step::from_result(event, results)
    }

    fn process_inner(
        &mut self,
        event: InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::TouchDown(data) => {
                let TouchData {
                    device_index,
                    time_us,
                    slot,
                    x,
                    y,
                    pressure,
                    touch_major,
                    tool_type,
                    ..
                } = *data;
                let s = slot as usize;
                let mut results = FilterResult::new();
                if !x.is_finite() || !y.is_finite() {
                    return results;
                }
                if s < MAX_TOUCH_SLOTS {
                    let edge_scroll_axis = self.edge_scroll_axis(x, y, info);
                    let active_before = self.active_count();
                    let is_thumb = self.config.thumb_detection
                        && active_before > 0
                        && edge_scroll_axis.is_none()
                        && self.is_in_thumb_zone(y, info)
                        && self
                            .config
                            .thumb_pressure_threshold
                            .is_none_or(|threshold| pressure.is_none_or(|p| p >= threshold));
                    let is_edge_palm = self.config.palm_detection
                        && edge_scroll_axis.is_none()
                        && self.is_in_edge_zone(x, y, info);
                    let is_firmware_palm = self.config.palm_detection
                        && matches!(tool_type, Some(TouchToolType::Palm));
                    let is_pressure_palm = self.config.palm_detection
                        && self
                            .config
                            .palm_pressure_threshold
                            .is_some_and(|threshold| pressure.is_some_and(|p| p >= threshold));
                    let is_size_palm = self.config.palm_detection
                        && self.config.palm_size_threshold.is_some_and(|threshold| {
                            touch_major.is_some_and(|major| major >= threshold)
                        });
                    let is_palm = is_firmware_palm
                        || is_thumb
                        || is_edge_palm
                        || is_pressure_palm
                        || is_size_palm;

                    if active_before == 0 {
                        self.fingers_at_start = 0;
                        self.current_gesture_is_tap = true;
                        self.active_gesture = None;
                    } else if self.active_gesture.is_some() && !is_palm {
                        // A palm contact must not cancel an active gesture; it
                        // is ignored instead.
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
                        if self.drag_lock_button.is_some() {
                            self.drag_lock_contact = Some(s);
                            self.current_gesture_is_tap = false;
                        } else if let Some(button) = self.take_tap_drag_button(time_us) {
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

                    if self.drag_lock_contact == Some(s) {
                        self.drag_lock_contact = self
                            .slots
                            .iter()
                            .position(|slot| slot.active && !slot.is_palm);
                        if self.active_count() == 0 {
                            self.release_drag_lock(device_index, time_us, &mut results);
                        }
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
                            if self.config.tap_drag_lock {
                                self.drag_lock_button = Some(button);
                                self.drag_lock_device = Some(device_index);
                                self.drag_lock_deadline_us = (self.config.drag_lock_timeout_us > 0)
                                    .then(|| {
                                        time_us.saturating_add(self.config.drag_lock_timeout_us)
                                    });
                            } else {
                                results.push(InputEvent::PointerButton {
                                    device_index,
                                    time_us,
                                    button,
                                    pressed: false,
                                });
                            }
                            return results;
                        }
                    }

                    let duration = time_us.saturating_sub(self.slots[s].start_time);
                    let dx = self.slots[s].x - self.slots[s].down_x;
                    let dy = self.slots[s].y - self.slots[s].down_y;
                    let dist_mm = self.get_dist_mm(dx, dy, info);

                    let is_tap_like = duration < self.config.tap_timeout_us
                        && dist_mm < self.config.tap_move_threshold_mm
                        && !self.slots[s].moved;
                    if !is_tap_like {
                        self.current_gesture_is_tap = false;
                    }

                    if self.active_count() == 0 {
                        if self.current_gesture_is_tap && self.config.tap_to_click {
                            if !self.is_pointer_suppressed(time_us, seat) {
                                let button = self.tap_button_for_fingers(self.fingers_at_start);

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

                    if self.drag_lock_contact == Some(s) {
                        self.drag_lock_contact = None;
                    }
                    if self.active_count() == 0 {
                        self.release_drag_lock(device_index, time_us, &mut results);
                        self.fingers_at_start = 0;
                    }
                }

                results
            }
            _ => smallvec::smallvec![event],
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
            processing::TestFilterExt,
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

        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 0,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 100,
                slot: 0,
                x: 510.0,
                y: 510.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        let ev = f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 100,
            },
            None,
            Some(&info),
        );
        assert!(!ev.is_empty(), "Center touch should produce motion");

        f.run(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 0,
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 200,
            },
            None,
            Some(&info),
        );

        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 300,
                slot: 0,
                x: 10.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 400,
                slot: 0,
                x: 20.0,
                y: 510.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        let ev_palm = f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 400,
            },
            None,
            Some(&info),
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

        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 0,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 0,
                slot: 1,
                x: 600.0,
                y: 600.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 100,
                slot: 0,
                x: 500.0,
                y: 505.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        let evs = f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 100,
            },
            None,
            Some(&info),
        );

        // Two-finger scroll follows center movement. One of two fingers moved
        // by dy=5, so the center moved by dy=2.5 and natural scrolling reverses it.
        if let Some(InputEvent::PointerScroll { dy, .. }) = evs.get(0) {
            assert_eq!(*dy, -2.5, "Expected scroll dy=-2.5, got {}", dy);
        } else {
            panic!("Expected PointerScroll, got {:?}", evs.get(0));
        }

        f.run(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 0,
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 1,
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 200,
            },
            None,
            Some(&info),
        );

        // Two-finger tap → right click
        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 300,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );
        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 305,
                slot: 1,
                x: 600.0,
                y: 600.0,
                ..plain_touch_data()
            })),
            None,
            Some(&info),
        );

        f.run(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 350,
                slot: 0,
            })),
            None,
            Some(&info),
        );
        let evs_tap = f.run(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 355,
                slot: 1,
            })),
            None,
            Some(&info),
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
    fn test_edge_scroll_does_not_click_on_release() {
        let mut config = TouchpadConfig::default();
        config.scroll_method = TouchpadScrollMethod::Edge;
        let mut filter = TouchpadMotionFilter::new(config);
        let touch = |time_us, y| TouchData {
            time_us,
            x: 990.0,
            y,
            ..plain_touch_data()
        };
        filter.run(
            InputEvent::TouchDown(Box::new(touch(0, 500.0))),
            None,
            Some(&mock_info()),
        );
        filter.run(
            InputEvent::TouchMotion(Box::new(touch(100, 505.0))),
            None,
            Some(&mock_info()),
        );
        let scroll = filter.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 100,
            },
            None,
            Some(&mock_info()),
        );
        assert!(
            scroll
                .iter()
                .any(|event| matches!(event, InputEvent::PointerScroll { dy, .. } if *dy != 0.0))
        );
        let release = filter.run(
            InputEvent::TouchUp(Box::new(TouchEndData {
                device_index: dev(),
                time_us: 200,
                slot: 0,
            })),
            None,
            Some(&mock_info()),
        );
        assert!(
            release
                .iter()
                .all(|event| !matches!(event, InputEvent::PointerButton { .. }))
        );
    }

    fn touch_down(time_us: u64, slot: i32, x: f64, y: f64) -> InputEvent {
        InputEvent::TouchDown(Box::new(TouchData {
            time_us,
            slot,
            x,
            y,
            ..plain_touch_data()
        }))
    }

    fn touch_motion(time_us: u64, slot: i32, x: f64, y: f64) -> InputEvent {
        InputEvent::TouchMotion(Box::new(TouchData {
            time_us,
            slot,
            x,
            y,
            ..plain_touch_data()
        }))
    }

    fn touch_up(time_us: u64, slot: i32) -> InputEvent {
        InputEvent::TouchUp(Box::new(TouchEndData {
            device_index: dev(),
            time_us,
            slot,
        }))
    }

    fn frame(time_us: u64) -> InputEvent {
        InputEvent::Frame {
            device_index: dev(),
            time_us,
        }
    }

    fn button_events(events: &[InputEvent]) -> Vec<(u32, bool)> {
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
    fn test_tap_button_map_remaps_two_and_three_finger_taps() {
        let mut config = TouchpadConfig::default();
        config.tap_button_map = TapButtonMap::LeftMiddleRight;
        config.tap_drag = false;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        for (count, expected) in [(2usize, BTN_MIDDLE), (3, BTN_RIGHT)] {
            for slot in 0..count {
                f.run(
                    touch_down(0, slot as i32, 500.0 + slot as f64 * 20.0, 500.0),
                    None,
                    Some(&info),
                );
            }
            let mut events = FilterResult::new();
            for slot in (0..count).rev() {
                events.extend(f.run(touch_up(20, slot as i32), None, Some(&info)));
            }
            assert_eq!(
                button_events(&events),
                vec![(expected, true), (expected, false)],
                "unexpected tap output for {count} fingers"
            );
        }
    }

    #[test]
    fn test_tap_finger_count_limit_suppresses_three_finger_tap() {
        let mut config = TouchpadConfig::default();
        config.max_tap_fingers = 2;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        for slot in 0..3 {
            f.run(
                touch_down(0, slot, 500.0 + slot as f64 * 20.0, 500.0),
                None,
                Some(&info),
            );
        }
        let mut events = FilterResult::new();
        for slot in (0..3).rev() {
            events.extend(f.run(touch_up(20, slot), None, Some(&info)));
        }
        assert!(button_events(&events).is_empty());
    }

    #[test]
    fn test_tap_timeout_is_configurable() {
        let mut config = TouchpadConfig::default();
        config.tap_timeout_us = 100;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        let events = f.run(touch_up(150, 0), None, Some(&info));
        assert!(
            button_events(&events).is_empty(),
            "tap held past the configured timeout must not click"
        );
    }

    #[test]
    fn test_tap_move_threshold_is_configurable() {
        let mut config = TouchpadConfig::default();
        config.tap_move_threshold_mm = 0.5;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        // 10 device units at 10 units/mm is 1mm, above the configured 0.5mm.
        f.run(touch_motion(10, 0, 510.0, 500.0), None, Some(&info));
        let events = f.run(touch_up(20, 0), None, Some(&info));
        assert!(button_events(&events).is_empty());
    }

    #[test]
    fn test_drag_lock_holds_button_until_next_tap() {
        let mut config = TouchpadConfig::default();
        config.tap_drag_lock = true;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        // Initial tap.
        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        let tap = f.run(touch_up(50, 0), None, Some(&info));
        assert_eq!(
            button_events(&tap),
            vec![(BTN_LEFT, true), (BTN_LEFT, false)]
        );

        // Second touch starts a drag.
        f.run(touch_down(100, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_motion(150, 0, 520.0, 500.0), None, Some(&info));
        f.run(frame(150), None, Some(&info));

        // Lifting the drag contact leaves the logical button held.
        let lift = f.run(touch_up(200, 0), None, Some(&info));
        assert!(
            button_events(&lift).is_empty(),
            "drag lock must not release on lift"
        );

        // The next contact unlocks the button on release.
        f.run(touch_down(300, 0, 500.0, 500.0), None, Some(&info));
        let unlock = f.run(touch_up(350, 0), None, Some(&info));
        assert_eq!(button_events(&unlock), vec![(BTN_LEFT, false)]);
    }

    #[test]
    fn test_drag_lock_timeout_releases_button() {
        let mut config = TouchpadConfig::default();
        config.tap_drag_lock = true;
        config.drag_lock_timeout_us = 1_000;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_up(50, 0), None, Some(&info));
        f.run(touch_down(100, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_motion(150, 0, 520.0, 500.0), None, Some(&info));
        f.run(frame(150), None, Some(&info));
        let lift = f.run(touch_up(200, 0), None, Some(&info));
        assert!(button_events(&lift).is_empty());

        // Deadline is 200 + 1000us; the next event after it releases the
        // button before processing its own output.
        let expired = f.run(frame(1_300), None, Some(&info));
        assert_eq!(button_events(&expired), vec![(BTN_LEFT, false)]);
    }

    #[test]
    fn test_clickpad_detection_gates_software_buttons() {
        let mut config = TouchpadConfig::default();
        config.click_method = TouchpadClickMethod::ButtonAreas;
        let mut f = TouchpadMotionFilter::new(config);
        let mut info = mock_info();
        info.is_clickpad = false;

        f.run(touch_down(0, 0, 900.0, 900.0), None, Some(&info));
        let raw = f.run(
            InputEvent::PointerButton {
                device_index: dev(),
                time_us: 10,
                button: BTN_LEFT,
                pressed: true,
            },
            None,
            Some(&info),
        );
        assert_eq!(
            button_events(&raw),
            vec![(BTN_LEFT, true)],
            "non-clickpad physical buttons must pass through unmapped"
        );

        info.is_clickpad = true;
        let mapped = f.run(
            InputEvent::PointerButton {
                device_index: dev(),
                time_us: 20,
                button: BTN_LEFT,
                pressed: true,
            },
            None,
            Some(&info),
        );
        assert_eq!(button_events(&mapped), vec![(BTN_RIGHT, true)]);
    }

    #[test]
    fn test_left_handed_swaps_only_bottom_button_areas() {
        let mut config = TouchpadConfig::default();
        config.click_method = TouchpadClickMethod::ButtonAreas;
        config.left_handed = true;
        let mut f = TouchpadMotionFilter::new(config);
        let mut info = mock_info();
        info.is_clickpad = true;

        let press = |f: &mut TouchpadMotionFilter, x: f64, y: f64, t: u64| {
            f.run(touch_down(t, 0, x, y), None, Some(&info));
            let out = f.run(
                InputEvent::PointerButton {
                    device_index: dev(),
                    time_us: t + 1,
                    button: BTN_LEFT,
                    pressed: true,
                },
                None,
                Some(&info),
            );
            f.run(touch_up(t + 2, 0), None, Some(&info));
            button_events(&out)
        };

        assert_eq!(press(&mut f, 900.0, 900.0, 0), vec![(BTN_LEFT, true)]);
        assert_eq!(press(&mut f, 100.0, 900.0, 100), vec![(BTN_RIGHT, true)]);
        assert_eq!(press(&mut f, 500.0, 900.0, 200), vec![(BTN_MIDDLE, true)]);
        // Main area stays the primary click area.
        assert_eq!(press(&mut f, 900.0, 100.0, 300), vec![(BTN_LEFT, true)]);
    }

    #[test]
    fn test_left_handed_swaps_physical_touchpad_buttons() {
        let mut config = TouchpadConfig::default();
        config.left_handed = true;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        let send = |f: &mut TouchpadMotionFilter, button, pressed| {
            let events = f.run(
                InputEvent::PointerButton {
                    device_index: dev(),
                    time_us: 10,
                    button,
                    pressed,
                },
                None,
                Some(&info),
            );
            button_events(&events)
        };

        assert_eq!(send(&mut f, BTN_LEFT, true), vec![(BTN_RIGHT, true)]);
        assert_eq!(send(&mut f, BTN_LEFT, false), vec![(BTN_RIGHT, false)]);
        assert_eq!(send(&mut f, BTN_RIGHT, true), vec![(BTN_LEFT, true)]);
        assert_eq!(send(&mut f, BTN_MIDDLE, true), vec![(BTN_MIDDLE, true)]);
    }

    #[test]
    fn test_clickfinger_button_map() {
        let mut config = TouchpadConfig::default();
        config.click_method = TouchpadClickMethod::Clickfinger;
        config.clickfinger_button_map = ClickfingerButtonMap::LeftMiddleRight;
        let mut f = TouchpadMotionFilter::new(config);
        let mut info = mock_info();
        info.is_clickpad = true;

        for (count, expected) in [(1usize, BTN_LEFT), (2, BTN_MIDDLE), (3, BTN_RIGHT)] {
            for slot in 0..count {
                f.run(
                    touch_down(0, slot as i32, 500.0 + slot as f64 * 20.0, 500.0),
                    None,
                    Some(&info),
                );
            }
            let events = f.run(
                InputEvent::PointerButton {
                    device_index: dev(),
                    time_us: 10,
                    button: BTN_LEFT,
                    pressed: true,
                },
                None,
                Some(&info),
            );
            assert_eq!(button_events(&events), vec![(expected, true)]);
            f.run(
                InputEvent::PointerButton {
                    device_index: dev(),
                    time_us: 11,
                    button: BTN_LEFT,
                    pressed: false,
                },
                None,
                Some(&info),
            );
            for slot in 0..count {
                f.run(touch_up(20, slot as i32), None, Some(&info));
            }
        }
    }

    #[test]
    fn test_pressure_and_size_palm_thresholds() {
        let mut config = TouchpadConfig::default();
        config.palm_pressure_threshold = Some(50.0);
        config.palm_size_threshold = Some(60.0);
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        let mut palm = plain_touch_data();
        palm.time_us = 0;
        palm.x = 500.0;
        palm.y = 500.0;
        palm.pressure = Some(70.0);
        f.run(InputEvent::TouchDown(Box::new(palm)), None, Some(&info));
        f.run(touch_motion(10, 0, 520.0, 500.0), None, Some(&info));
        let events = f.run(frame(10), None, Some(&info));
        assert!(events.is_empty(), "high-pressure contact must be a palm");

        let mut finger = plain_touch_data();
        finger.time_us = 100;
        finger.x = 500.0;
        finger.y = 500.0;
        finger.pressure = Some(30.0);
        finger.touch_major = Some(20.0);
        f.run(InputEvent::TouchDown(Box::new(finger)), None, Some(&info));
        f.run(touch_motion(110, 0, 520.0, 500.0), None, Some(&info));
        let events = f.run(frame(110), None, Some(&info));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, InputEvent::PointerMotion { .. })),
            "contact under both thresholds must remain a finger"
        );
    }

    #[test]
    fn test_late_firmware_palm_reclassification_suppresses_motion() {
        let mut f = TouchpadMotionFilter::new(TouchpadConfig::default());
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        let mut motion = plain_touch_data();
        motion.time_us = 10;
        motion.x = 520.0;
        motion.y = 500.0;
        motion.tool_type = Some(TouchToolType::Palm);
        f.run(InputEvent::TouchMotion(Box::new(motion)), None, Some(&info));
        let events = f.run(frame(10), None, Some(&info));
        assert!(
            events.is_empty(),
            "late palm reclassification must suppress the contact"
        );

        let up = f.run(touch_up(20, 0), None, Some(&info));
        assert!(button_events(&up).is_empty(), "palm must not tap");
    }

    #[test]
    fn test_two_finger_scroll_does_not_promote_to_pinch() {
        let mut f = TouchpadMotionFilter::new(TouchpadConfig::default());
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_down(0, 1, 600.0, 500.0), None, Some(&info));

        // Normal parallel scroll establishes the two-finger scroll lock.
        f.run(touch_motion(10, 0, 505.0, 500.0), None, Some(&info));
        f.run(touch_motion(10, 1, 605.0, 500.0), None, Some(&info));
        let first = f.run(frame(10), None, Some(&info));
        assert!(
            first
                .iter()
                .any(|event| matches!(event, InputEvent::PointerScroll { .. }))
        );

        // A large spacing change afterwards must not turn the same gesture
        // into a pinch.
        f.run(touch_motion(20, 1, 750.0, 500.0), None, Some(&info));
        let second = f.run(frame(20), None, Some(&info));
        assert!(
            second.iter().all(|event| !matches!(
                event,
                InputEvent::GesturePinchBegin(_) | InputEvent::GesturePinchUpdate(_)
            )),
            "scroll-latched two-finger gesture must not become a pinch"
        );
        assert!(
            second
                .iter()
                .any(|event| matches!(event, InputEvent::PointerScroll { .. }))
        );
    }

    #[test]
    fn test_drag_lock_timeout_is_dispatchable_without_input() {
        let mut config = TouchpadConfig::default();
        config.tap_drag_lock = true;
        config.drag_lock_timeout_us = 1_000;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_up(50, 0), None, Some(&info));
        f.run(touch_down(100, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_motion(150, 0, 520.0, 500.0), None, Some(&info));
        f.run(frame(150), None, Some(&info));
        assert!(button_events(&f.run(touch_up(200, 0), None, Some(&info))).is_empty());

        assert_eq!(f.next_timeout_us(), Some(1_200));
        assert!(button_events(&f.dispatch_timeout(1_199)).is_empty());
        assert_eq!(
            button_events(&f.dispatch_timeout(1_200)),
            vec![(BTN_LEFT, false)]
        );
        assert_eq!(f.next_timeout_us(), None);
    }

    #[test]
    fn test_dwtp_suppresses_touchpad_while_trackpoint_active() {
        let mut f = TouchpadMotionFilter::new(TouchpadConfig::default());
        let info = mock_info();
        let mut seat = Seat {
            last_trackpoint_time: 1_000,
            ..Default::default()
        };

        f.run(touch_down(1_100, 0, 500.0, 500.0), Some(&seat), Some(&info));
        f.run(
            touch_motion(1_200, 0, 520.0, 500.0),
            Some(&seat),
            Some(&info),
        );
        let suppressed = f.run(frame(1_200), Some(&seat), Some(&info));
        assert!(suppressed.is_empty(), "DWTP must suppress touchpad motion");

        seat.last_trackpoint_time = 1_000;
        f.run(
            touch_motion(
                1_000 + TouchpadConfig::default().dwtp_timeout_us + 100,
                0,
                540.0,
                500.0,
            ),
            Some(&seat),
            Some(&info),
        );
        let restored = f.run(
            frame(1_000 + TouchpadConfig::default().dwtp_timeout_us + 100),
            Some(&seat),
            Some(&info),
        );
        assert!(!restored.is_empty(), "DWTP must expire");
    }

    #[test]
    fn test_stylus_proximity_suppresses_touchpad() {
        let mut f = TouchpadMotionFilter::new(TouchpadConfig::default());
        let info = mock_info();
        let seat = Seat {
            stylus_in_proximity: true,
            ..Default::default()
        };

        f.run(touch_down(100, 0, 500.0, 500.0), Some(&seat), Some(&info));
        f.run(touch_motion(110, 0, 520.0, 500.0), Some(&seat), Some(&info));
        assert!(f.run(frame(110), Some(&seat), Some(&info)).is_empty());

        let released = Seat::default();
        f.run(
            touch_motion(200, 0, 540.0, 500.0),
            Some(&released),
            Some(&info),
        );
        assert!(
            !f.run(frame(200), Some(&released), Some(&info)).is_empty(),
            "touchpad input resumes after stylus proximity ends"
        );
    }

    #[test]
    fn test_configurable_hold_timeout() {
        let mut config = TouchpadConfig::default();
        config.hold_timeout_us = 50_000;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        let events = f.run(frame(60_000), None, Some(&info));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, InputEvent::GestureHoldBegin(_)))
        );
    }

    #[test]
    fn test_configurable_pinch_threshold() {
        let mut config = TouchpadConfig::default();
        config.pinch_scale_threshold = 0.5;
        let mut f = TouchpadMotionFilter::new(config);
        let info = mock_info();

        f.run(touch_down(0, 0, 500.0, 500.0), None, Some(&info));
        f.run(touch_down(0, 1, 600.0, 500.0), None, Some(&info));
        // 20% spacing change is below the configured 50% pinch threshold.
        f.run(touch_motion(10, 1, 620.0, 500.0), None, Some(&info));
        let events = f.run(frame(10), None, Some(&info));
        assert!(
            events.iter().all(|event| !matches!(
                event,
                InputEvent::GesturePinchBegin(_) | InputEvent::GesturePinchUpdate(_)
            )),
            "pinch threshold must suppress a small spacing change"
        );
    }

    #[test]
    fn test_palm_joining_active_gesture_does_not_cancel_it() {
        let mut f = TouchpadMotionFilter::new(TouchpadConfig::default());
        let info = mock_info();

        for slot in 0..3 {
            f.run(
                touch_down(0, slot, 500.0 + slot as f64 * 20.0, 500.0),
                None,
                Some(&info),
            );
        }
        for slot in 0..3 {
            f.run(
                touch_motion(10, slot, 520.0 + slot as f64 * 20.0, 500.0),
                None,
                Some(&info),
            );
        }
        let started = f.run(frame(10), None, Some(&info));
        assert!(
            started
                .iter()
                .any(|event| matches!(event, InputEvent::GestureSwipeBegin(_)))
        );

        let mut palm = plain_touch_data();
        palm.time_us = 20;
        palm.slot = 3;
        palm.x = 500.0;
        palm.y = 500.0;
        palm.tool_type = Some(TouchToolType::Palm);
        let palm_events = f.run(InputEvent::TouchDown(Box::new(palm)), None, Some(&info));
        assert!(
            palm_events
                .iter()
                .all(|event| !matches!(event, InputEvent::GestureSwipeEnd(_))),
            "palm contact must not cancel an active swipe"
        );

        for slot in 0..3 {
            f.run(
                touch_motion(30, slot, 540.0 + slot as f64 * 20.0, 500.0),
                None,
                Some(&info),
            );
        }
        let continued = f.run(frame(30), None, Some(&info));
        assert!(
            continued
                .iter()
                .any(|event| matches!(event, InputEvent::GestureSwipeUpdate(_)))
        );
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
            ..Default::default()
        };
        let info = mock_info();

        f.run(
            InputEvent::TouchDown(Box::new(TouchData {
                device_index: dev(),
                time_us: 1100,
                slot: 0,
                x: 500.0,
                y: 500.0,
                ..plain_touch_data()
            })),
            Some(&seat),
            Some(&info),
        );
        f.run(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: 1200,
                slot: 0,
                x: 512.0,
                y: 11.0,
                ..plain_touch_data()
            })),
            Some(&seat),
            Some(&info),
        );
        let ev = f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: 1200,
            },
            Some(&seat),
            Some(&info),
        );
        assert!(ev.is_empty(), "Motion should be suppressed by DWT");

        seat.last_key_time = 1000;
        let time_after = 1000 + TouchpadConfig::default().dwt_timeout_us + 100;
        f.run(
            InputEvent::TouchMotion(Box::new(TouchData {
                device_index: dev(),
                time_us: time_after,
                slot: 0,
                x: 515.0,
                y: 13.0,
                ..plain_touch_data()
            })),
            Some(&seat),
            Some(&info),
        );
        let ev2 = f.run(
            InputEvent::Frame {
                device_index: dev(),
                time_us: time_after,
            },
            Some(&seat),
            Some(&info),
        );
        assert!(
            !ev2.is_empty(),
            "Motion should NOT be suppressed after DWT timeout"
        );
    }
}
