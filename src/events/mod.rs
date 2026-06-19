use std::fmt;

/// Compact device index used in events for cache efficiency.
/// Maps to full DeviceId via Context lookup when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DevIndex(pub u32);

impl fmt::Display for DevIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dev{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwitchKind {
    Lid,
    TabletMode,
    Unknown(u32),
}

impl SwitchKind {
    pub fn from_linux_code(code: u32) -> Self {
        match code {
            0 => Self::Lid,
            1 => Self::TabletMode,
            code => Self::Unknown(code),
        }
    }

    pub fn linux_code(self) -> u32 {
        match self {
            Self::Lid => 0,
            Self::TabletMode => 1,
            Self::Unknown(code) => code,
        }
    }
}

impl fmt::Display for SwitchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lid => f.write_str("lid"),
            Self::TabletMode => f.write_str("tablet-mode"),
            Self::Unknown(code) => write!(f, "unknown({code})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollSource {
    Wheel,
    Finger,
    Continuous,
}

/// Hot-path input event enum, kept compact for cache efficiency, with unaccelerated deltas for raw-input clients.
///
/// Touch, gesture, and tablet variants are boxed since they are cold paths
/// (mouse/keyboard is the priority). The inline variants cover:
/// PointerMotion, PointerButton, Key, PointerScroll, SwitchToggle, Frame,
/// DeviceAdded, DeviceRemoved.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    DeviceAdded {
        device_index: DevIndex,
    },
    DeviceRemoved {
        device_index: DevIndex,
    },
    PointerMotion {
        device_index: DevIndex,
        time_us: u64,
        dx: f64,
        dy: f64,
        dx_unaccelerated: f64,
        dy_unaccelerated: f64,
    },
    PointerButton {
        device_index: DevIndex,
        time_us: u64,
        button: u32,
        pressed: bool,
    },
    Key {
        device_index: DevIndex,
        time_us: u64,
        key: u32,
        pressed: bool,
    },
    PointerScroll {
        device_index: DevIndex,
        time_us: u64,
        source: ScrollSource,
        dx: f64,
        dy: f64,
    },
    SwitchToggle {
        device_index: DevIndex,
        time_us: u64,
        switch: SwitchKind,
        enabled: bool,
    },
    Frame {
        device_index: DevIndex,
        time_us: u64,
    },
    // Cold-path variants (boxed to keep enum compact)
    TouchDown(Box<TouchData>),
    TouchUp(Box<TouchEndData>),
    TouchMotion(Box<TouchData>),
    TouchCancel(Box<TouchEndData>),
    GestureSwipeBegin(Box<GestureSwipeData>),
    GestureSwipeUpdate(Box<GestureMoveData>),
    GestureSwipeEnd(Box<GestureEndData>),
    GesturePinchBegin(Box<GesturePinchBeginData>),
    GesturePinchUpdate(Box<GesturePinchUpdateData>),
    GesturePinchEnd(Box<GestureEndData>),
    GestureHoldBegin(Box<GestureHoldBeginData>),
    GestureHoldEnd(Box<GestureHoldEndData>),
    TabletToolProximity(Box<TabletToolProximityData>),
    TabletToolAxis(Box<TabletToolAxisData>),
    TabletToolTip(Box<TabletToolTipData>),
    TabletToolButton(Box<TabletToolButtonData>),
    TabletPadButton(Box<TabletPadButtonData>),
    TabletPadRing(Box<TabletPadValueData>),
    TabletPadStrip(Box<TabletPadValueData>),
    TabletPadDial(Box<TabletPadDialData>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub slot: i32,
    pub x: f64,
    pub y: f64,
    pub pressure: Option<f64>,
    pub touch_major: Option<f64>,
    pub touch_minor: Option<f64>,
    pub tool_type: Option<TouchToolType>,
    pub tracking_id: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchEndData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub slot: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchToolType {
    Finger,
    Pen,
    Palm,
    Dial,
    Unknown(i32),
}

impl TouchToolType {
    pub fn from_linux_value(value: i32) -> Self {
        match value {
            0 => Self::Finger,
            1 => Self::Pen,
            2 => Self::Palm,
            10 => Self::Dial,
            value => Self::Unknown(value),
        }
    }

    pub fn linux_value(self) -> i32 {
        match self {
            Self::Finger => 0,
            Self::Pen => 1,
            Self::Palm => 2,
            Self::Dial => 10,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureSwipeData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub fingers: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureMoveData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub dx: f64,
    pub dy: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureEndData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GesturePinchBeginData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub fingers: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GesturePinchUpdateData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    pub angle: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureHoldBeginData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub fingers: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GestureHoldEndData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub fingers: u32,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletToolProximityData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub x: f64,
    pub y: f64,
    pub in_range: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletToolAxisData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletToolTipData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub down: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletToolButtonData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub button: u32,
    pub pressed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletPadButtonData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub index: u32,
    pub pressed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletPadValueData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub index: u32,
    pub position: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletPadDialData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub index: u32,
    pub delta: f64,
}

impl InputEvent {
    /// Returns the device index for any event variant.
    pub fn device_index(&self) -> DevIndex {
        match self {
            InputEvent::DeviceAdded { device_index } => *device_index,
            InputEvent::DeviceRemoved { device_index } => *device_index,
            InputEvent::PointerMotion { device_index, .. } => *device_index,
            InputEvent::PointerButton { device_index, .. } => *device_index,
            InputEvent::Key { device_index, .. } => *device_index,
            InputEvent::PointerScroll { device_index, .. } => *device_index,
            InputEvent::SwitchToggle { device_index, .. } => *device_index,
            InputEvent::Frame { device_index, .. } => *device_index,
            InputEvent::TouchDown(d) => d.device_index,
            InputEvent::TouchUp(d) => d.device_index,
            InputEvent::TouchMotion(d) => d.device_index,
            InputEvent::TouchCancel(d) => d.device_index,
            InputEvent::GestureSwipeBegin(d) => d.device_index,
            InputEvent::GestureSwipeUpdate(d) => d.device_index,
            InputEvent::GestureSwipeEnd(d) => d.device_index,
            InputEvent::GesturePinchBegin(d) => d.device_index,
            InputEvent::GesturePinchUpdate(d) => d.device_index,
            InputEvent::GesturePinchEnd(d) => d.device_index,
            InputEvent::GestureHoldBegin(d) => d.device_index,
            InputEvent::GestureHoldEnd(d) => d.device_index,
            InputEvent::TabletToolProximity(d) => d.device_index,
            InputEvent::TabletToolAxis(d) => d.device_index,
            InputEvent::TabletToolTip(d) => d.device_index,
            InputEvent::TabletToolButton(d) => d.device_index,
            InputEvent::TabletPadButton(d) => d.device_index,
            InputEvent::TabletPadRing(d) => d.device_index,
            InputEvent::TabletPadStrip(d) => d.device_index,
            InputEvent::TabletPadDial(d) => d.device_index,
        }
    }

    /// Returns the timestamp in microseconds for any event variant.
    pub fn time_us(&self) -> u64 {
        match self {
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } => 0,
            InputEvent::PointerMotion { time_us, .. } => *time_us,
            InputEvent::PointerButton { time_us, .. } => *time_us,
            InputEvent::Key { time_us, .. } => *time_us,
            InputEvent::PointerScroll { time_us, .. } => *time_us,
            InputEvent::SwitchToggle { time_us, .. } => *time_us,
            InputEvent::Frame { time_us, .. } => *time_us,
            InputEvent::TouchDown(d) => d.time_us,
            InputEvent::TouchUp(d) => d.time_us,
            InputEvent::TouchMotion(d) => d.time_us,
            InputEvent::TouchCancel(d) => d.time_us,
            InputEvent::GestureSwipeBegin(d) => d.time_us,
            InputEvent::GestureSwipeUpdate(d) => d.time_us,
            InputEvent::GestureSwipeEnd(d) => d.time_us,
            InputEvent::GesturePinchBegin(d) => d.time_us,
            InputEvent::GesturePinchUpdate(d) => d.time_us,
            InputEvent::GesturePinchEnd(d) => d.time_us,
            InputEvent::GestureHoldBegin(d) => d.time_us,
            InputEvent::GestureHoldEnd(d) => d.time_us,
            InputEvent::TabletToolProximity(d) => d.time_us,
            InputEvent::TabletToolAxis(d) => d.time_us,
            InputEvent::TabletToolTip(d) => d.time_us,
            InputEvent::TabletToolButton(d) => d.time_us,
            InputEvent::TabletPadButton(d) => d.time_us,
            InputEvent::TabletPadRing(d) => d.time_us,
            InputEvent::TabletPadStrip(d) => d.time_us,
            InputEvent::TabletPadDial(d) => d.time_us,
        }
    }
}
