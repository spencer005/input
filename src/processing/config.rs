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
        }
    }
}
