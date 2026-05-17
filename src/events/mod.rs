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

/// Hot-path input event enum, kept compact for cache efficiency, with unaccelerated deltas for raw-input clients.
///
/// Touch and gesture variants are boxed since they are cold paths
/// (mouse/keyboard is the priority). The inline variants cover:
/// PointerMotion, PointerButton, Key, PointerScroll, Frame,
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
        dx: f64,
        dy: f64,
    },
    Frame {
        device_index: DevIndex,
        time_us: u64,
    },
    // Cold-path variants (boxed to keep enum compact)
    TouchDown(Box<TouchData>),
    TouchUp(Box<TouchEndData>),
    TouchMotion(Box<TouchData>),
    GestureSwipeBegin(Box<GestureSwipeData>),
    GestureSwipeUpdate(Box<GestureMoveData>),
    GestureSwipeEnd(Box<GestureEndData>),
    GesturePinchBegin(Box<GesturePinchBeginData>),
    GesturePinchUpdate(Box<GesturePinchUpdateData>),
    GesturePinchEnd(Box<GestureEndData>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub slot: i32,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TouchEndData {
    pub device_index: DevIndex,
    pub time_us: u64,
    pub slot: i32,
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
            InputEvent::Frame { device_index, .. } => *device_index,
            InputEvent::TouchDown(d) => d.device_index,
            InputEvent::TouchUp(d) => d.device_index,
            InputEvent::TouchMotion(d) => d.device_index,
            InputEvent::GestureSwipeBegin(d) => d.device_index,
            InputEvent::GestureSwipeUpdate(d) => d.device_index,
            InputEvent::GestureSwipeEnd(d) => d.device_index,
            InputEvent::GesturePinchBegin(d) => d.device_index,
            InputEvent::GesturePinchUpdate(d) => d.device_index,
            InputEvent::GesturePinchEnd(d) => d.device_index,
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
            InputEvent::Frame { time_us, .. } => *time_us,
            InputEvent::TouchDown(d) => d.time_us,
            InputEvent::TouchUp(d) => d.time_us,
            InputEvent::TouchMotion(d) => d.time_us,
            InputEvent::GestureSwipeBegin(d) => d.time_us,
            InputEvent::GestureSwipeUpdate(d) => d.time_us,
            InputEvent::GestureSwipeEnd(d) => d.time_us,
            InputEvent::GesturePinchBegin(d) => d.time_us,
            InputEvent::GesturePinchUpdate(d) => d.time_us,
            InputEvent::GesturePinchEnd(d) => d.time_us,
        }
    }
}
