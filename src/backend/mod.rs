pub mod evdev;

use {
    crate::{
        events::{InputEvent, SwitchKind},
        state::DeviceId,
    },
    std::{io, os::unix::io::OwnedFd},
};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BusType {
    #[default]
    Unknown,
    Usb,
    Bluetooth,
    I2c,
    Ps2,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AxisInfo {
    pub min: f64,
    pub max: f64,
    pub resolution: f64,
}

/// Device capability flags — used to auto-assign filter chains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceCaps(pub u32);

impl DeviceCaps {
    pub const KEYBOARD: Self = DeviceCaps(1 << 0);
    pub const POINTER: Self = DeviceCaps(1 << 1);
    pub const TOUCHPAD: Self = DeviceCaps(1 << 2);
    pub const TOUCHSCREEN: Self = DeviceCaps(1 << 3);
    pub const TABLET: Self = DeviceCaps(1 << 4);
    pub const TABLET_PAD: Self = DeviceCaps(1 << 5);
    pub const SWITCH: Self = DeviceCaps(1 << 6);
    /// Touchpad with software buttons under the pad surface
    /// (`INPUT_PROP_BUTTONPAD`).
    pub const CLICKPAD: Self = DeviceCaps(1 << 7);
    /// Pointing stick (`INPUT_PROP_POINTING_STICK`).
    pub const POINTING_STICK: Self = DeviceCaps(1 << 8);

    pub fn contains(self, other: DeviceCaps) -> bool {
        self.0 & other.0 != 0
    }

    pub fn insert(&mut self, other: DeviceCaps) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: DeviceCaps) {
        self.0 &= !other.0;
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeviceInfo {
    pub name: String,
    pub x: AxisInfo,
    pub y: AxisInfo,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus: BusType,
    pub capabilities: DeviceCaps,
    pub supported_switches: Vec<SwitchKind>,
    pub switch_states: Vec<(SwitchKind, bool)>,
    /// True when the device reports `INPUT_PROP_BUTTONPAD`, i.e. the touchpad
    /// surface itself is the physical click mechanism.
    pub is_clickpad: bool,
    /// True when the device reports `INPUT_PROP_TOPBUTTONPAD`.
    pub is_top_buttonpad: bool,
    /// Conservative internal-device classification derived from the bus type
    /// (I2C/PS2). Used by switch and DWT policy; profiles may override it.
    pub internal: bool,
    /// Initial LED state reported by the kernel, when available.
    pub led_state: crate::policy::LedState,
}

pub struct DeviceInfoRef<'a> {
    pub name: &'a str,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus: BusType,
    pub capabilities: DeviceCaps,
    pub supported_switches: &'a [SwitchKind],
    pub switch_states: &'a [(SwitchKind, bool)],
    pub seat: &'a str,
    pub is_clickpad: bool,
    pub internal: bool,
    pub led_state: crate::policy::LedState,
    /// Device axis ranges and hardware resolution, as reported by evdev.
    pub x: AxisInfo,
    pub y: AxisInfo,
}

pub trait Backend {
    fn initialize(&mut self) -> io::Result<()>;
    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()>;
    fn add_device_fd(&mut self, id: DeviceId, fd: OwnedFd, name: &str) -> io::Result<DeviceInfo>;
    fn remove_device(&mut self, _id: DeviceId) -> io::Result<()> {
        Ok(())
    }
    /// Update the kernel LED state (caps/num/scroll lock) for a device.
    ///
    /// Backends that cannot write LEDs may leave the default no-op
    /// implementation. Compositors should call this instead of applying LED
    /// policy themselves.
    fn set_led_state(&mut self, _id: DeviceId, _state: crate::policy::LedState) -> io::Result<()> {
        Ok(())
    }
    fn get_fd(&self) -> std::os::unix::io::RawFd;
}
