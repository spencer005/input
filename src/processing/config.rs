#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadClickMethod {
    None,
    ButtonAreas,
    Clickfinger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchpadScrollMethod {
    TwoFinger,
    Edge,
}

/// Mapping from tap finger count to logical pointer button.
///
/// `LeftRightMiddle` is the libinput default: one finger is a left click, two
/// fingers are a right click, and three fingers are a middle click.
/// `LeftMiddleRight` follows the alternative libinput tap map: one finger is a
/// left click, two fingers are a middle click, and three fingers are a right
/// click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapButtonMap {
    LeftRightMiddle,
    LeftMiddleRight,
}

impl TapButtonMap {
    /// Button for a two-finger tap.
    pub fn two_finger_button(self) -> u32 {
        match self {
            TapButtonMap::LeftRightMiddle => 0x111, // BTN_RIGHT
            TapButtonMap::LeftMiddleRight => 0x112, // BTN_MIDDLE
        }
    }

    /// Button for a three-finger tap.
    pub fn three_finger_button(self) -> u32 {
        match self {
            TapButtonMap::LeftRightMiddle => 0x112, // BTN_MIDDLE
            TapButtonMap::LeftMiddleRight => 0x111, // BTN_RIGHT
        }
    }
}

/// Mapping from clickfinger finger count to logical pointer button.
///
/// Clickfinger maps are independent from the generic left-handed setting, as in
/// libinput. `LeftRightMiddle` maps one/two/three or more fingers to
/// left/right/middle; `LeftMiddleRight` maps them to left/middle/right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickfingerButtonMap {
    LeftRightMiddle,
    LeftMiddleRight,
}

impl ClickfingerButtonMap {
    pub fn button_for_fingers(self, fingers: usize) -> u32 {
        match (self, fingers) {
            (_, 0 | 1) => 0x110,                                 // BTN_LEFT
            (ClickfingerButtonMap::LeftRightMiddle, 2) => 0x111, // BTN_RIGHT
            (ClickfingerButtonMap::LeftRightMiddle, _) => 0x112, // BTN_MIDDLE
            (ClickfingerButtonMap::LeftMiddleRight, 2) => 0x112, // BTN_MIDDLE
            (ClickfingerButtonMap::LeftMiddleRight, _) => 0x111, // BTN_RIGHT
        }
    }
}

/// Per-device touchpad configuration.
///
/// All timings are in microseconds and all distances are in millimetres at the
/// input-rs normalization layer (the same units used by tap recognition).
#[derive(Debug, Clone, Copy)]
pub struct TouchpadConfig {
    pub tap_to_click: bool,
    pub natural_scrolling: bool,
    pub two_finger_tap_right_click: bool,
    pub tap_drag: bool,
    pub tap_drag_lock: bool,
    pub click_method: TouchpadClickMethod,
    pub scroll_method: TouchpadScrollMethod,
    pub edge_scroll_edge_ratio: f64,
    pub dwt: bool,
    pub palm_detection: bool,
    pub palm_edge_ratio: f64, // e.g. 0.05 for 5% edge zone
    pub thumb_detection: bool,
    pub thumb_edge_ratio: f64,

    // --- Tap button and timing configuration ---
    /// Map used for one/two/three finger taps.
    pub tap_button_map: TapButtonMap,
    /// Map used for one/two/three finger clickfinger clicks.
    pub clickfinger_button_map: ClickfingerButtonMap,
    /// Maximum duration of a tap, in microseconds.
    pub tap_timeout_us: u64,
    /// Maximum movement allowed for a contact to still count as a tap, in mm.
    pub tap_move_threshold_mm: f64,
    /// Maximum delay between a tap and the start of tap-and-drag, in
    /// microseconds.
    pub tap_drag_timeout_us: u64,
    /// Time after lifting a drag-locked contact before the logical button is
    /// released automatically. Zero disables the timeout, so unlocking happens
    /// on the next tap.
    pub drag_lock_timeout_us: u64,
    /// Maximum finger count that may produce a tap. Devices that cannot track
    /// three fingers should set this to 2.
    pub max_tap_fingers: u8,

    // --- Clickpad geometry / policy ---
    /// Override whether the device is treated as a clickpad. `None` uses the
    /// backend-detected `DeviceInfo::is_clickpad` value.
    pub clickpad_override: Option<bool>,
    /// Fraction of the touchpad height occupied by software button areas,
    /// measured from the bottom edge.
    pub button_area_bottom_ratio: f64,
    /// Left boundary of the middle software button area, as a fraction of
    /// touchpad width.
    pub button_area_left_ratio: f64,
    /// Right boundary of the middle software button area, as a fraction of
    /// touchpad width.
    pub button_area_right_ratio: f64,
    /// Left-handed clickpad policy. Swaps the bottom-left and bottom-right
    /// software button areas. Tap and clickfinger maps are not rewritten by
    /// this setting, matching libinput.
    pub left_handed: bool,

    // --- Palm / thumb / DWT policy ---
    /// Raw `ABS_MT_PRESSURE` value above which a contact is treated as a palm.
    /// `None` disables pressure-based palm detection.
    pub palm_pressure_threshold: Option<f64>,
    /// Raw `ABS_MT_TOUCH_MAJOR` value above which a contact is treated as a
    /// palm. `None` disables size-based palm detection.
    pub palm_size_threshold: Option<f64>,
    /// Raw pressure value above which a resting thumb candidate is ignored.
    /// `None` uses position-only thumb detection.
    pub thumb_pressure_threshold: Option<f64>,
    /// Disable-while-typing suppression window after the last internal keyboard
    /// event, in microseconds.
    pub dwt_timeout_us: u64,
    /// Suppress touchpad pointer motion while a grouped trackpoint is active.
    pub dwtp: bool,
    /// Disable-while-trackpointing suppression window, in microseconds.
    pub dwtp_timeout_us: u64,
    /// How long fingers must remain stationary before a hold gesture starts.
    pub hold_timeout_us: u64,
    /// Minimum center movement, in mm, to start a swipe gesture.
    pub gesture_motion_threshold_mm: f64,
    /// Relative spacing change required to start a pinch.
    pub pinch_scale_threshold: f64,
    /// Relative angle change, in radians, required to start a pinch.
    pub pinch_rotation_threshold_rad: f64,
    /// Suppress touchpad input while a stylus is in proximity over the same
    /// seat.
    pub stylus_arbitration: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scrolling: false,
            two_finger_tap_right_click: true,
            tap_drag: true,
            tap_drag_lock: false,
            click_method: TouchpadClickMethod::None,
            scroll_method: TouchpadScrollMethod::TwoFinger,
            edge_scroll_edge_ratio: 0.07,
            dwt: true,
            palm_detection: true,
            palm_edge_ratio: 0.05,
            thumb_detection: false,
            thumb_edge_ratio: 0.20,
            tap_button_map: TapButtonMap::LeftRightMiddle,
            clickfinger_button_map: ClickfingerButtonMap::LeftRightMiddle,
            tap_timeout_us: 180_000,
            tap_move_threshold_mm: 1.0,
            tap_drag_timeout_us: 300_000,
            drag_lock_timeout_us: 0,
            max_tap_fingers: 3,
            clickpad_override: None,
            button_area_bottom_ratio: 0.20,
            button_area_left_ratio: 1.0 / 3.0,
            button_area_right_ratio: 2.0 / 3.0,
            left_handed: false,
            palm_pressure_threshold: None,
            palm_size_threshold: None,
            thumb_pressure_threshold: None,
            dwt_timeout_us: 500_000,
            dwtp: true,
            dwtp_timeout_us: 500_000,
            stylus_arbitration: true,
            hold_timeout_us: 500_000,
            gesture_motion_threshold_mm: 0.5,
            pinch_scale_threshold: 0.05,
            pinch_rotation_threshold_rad: 0.10,
        }
    }
}

impl TouchpadConfig {
    /// Return a copy with invalid numeric values replaced by the safe default.
    ///
    /// Configuration normally arrives from the compositor, so malformed values
    /// must never panic or silently produce non-finite motion. Invalid values
    /// are clamped or replaced; callers that need diagnostics can validate
    /// before installing a configuration.
    pub fn sanitized(self) -> Self {
        let default = Self::default();
        let sanitize_ratio = |value: f64, fallback: f64| {
            if value.is_finite() && (0.0..=1.0).contains(&value) {
                value
            } else {
                fallback
            }
        };

        let mut out = self;
        out.palm_edge_ratio = sanitize_ratio(out.palm_edge_ratio, default.palm_edge_ratio);
        out.thumb_edge_ratio = sanitize_ratio(out.thumb_edge_ratio, default.thumb_edge_ratio);
        out.edge_scroll_edge_ratio =
            sanitize_ratio(out.edge_scroll_edge_ratio, default.edge_scroll_edge_ratio);
        out.button_area_bottom_ratio = sanitize_ratio(
            out.button_area_bottom_ratio,
            default.button_area_bottom_ratio,
        );
        out.button_area_left_ratio =
            sanitize_ratio(out.button_area_left_ratio, default.button_area_left_ratio);
        out.button_area_right_ratio =
            sanitize_ratio(out.button_area_right_ratio, default.button_area_right_ratio);
        if out.button_area_left_ratio >= out.button_area_right_ratio {
            out.button_area_left_ratio = default.button_area_left_ratio;
            out.button_area_right_ratio = default.button_area_right_ratio;
        }
        if !out.tap_move_threshold_mm.is_finite() || out.tap_move_threshold_mm <= 0.0 {
            out.tap_move_threshold_mm = default.tap_move_threshold_mm;
        }
        if out.max_tap_fingers == 0 {
            out.max_tap_fingers = default.max_tap_fingers;
        }
        out.max_tap_fingers = out.max_tap_fingers.min(3);

        let sanitize_threshold = |value: Option<f64>| value.filter(|v| v.is_finite() && *v > 0.0);
        out.palm_pressure_threshold = sanitize_threshold(out.palm_pressure_threshold);
        out.palm_size_threshold = sanitize_threshold(out.palm_size_threshold);
        out.thumb_pressure_threshold = sanitize_threshold(out.thumb_pressure_threshold);
        if !out.gesture_motion_threshold_mm.is_finite() || out.gesture_motion_threshold_mm <= 0.0 {
            out.gesture_motion_threshold_mm = default.gesture_motion_threshold_mm;
        }
        if !out.pinch_scale_threshold.is_finite() || out.pinch_scale_threshold <= 0.0 {
            out.pinch_scale_threshold = default.pinch_scale_threshold;
        }
        if !out.pinch_rotation_threshold_rad.is_finite() || out.pinch_rotation_threshold_rad <= 0.0
        {
            out.pinch_rotation_threshold_rad = default.pinch_rotation_threshold_rad;
        }
        out
    }
}
