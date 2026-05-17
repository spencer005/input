#[derive(Debug, Clone, Copy)]
pub struct TouchpadConfig {
    pub tap_to_click: bool,
    pub natural_scrolling: bool,
    pub two_finger_tap_right_click: bool,
    pub dwt: bool,
    pub palm_detection: bool,
    pub palm_edge_ratio: f64, // e.g. 0.05 for 5% edge zone
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scrolling: false,
            two_finger_tap_right_click: true,
            dwt: true,
            palm_detection: true,
            palm_edge_ratio: 0.05,
        }
    }
}
