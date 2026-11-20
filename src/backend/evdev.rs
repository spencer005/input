use {
    super::{AxisInfo, Backend, BusType, DeviceCaps, DeviceInfo},
    crate::{
        events::{
            DevIndex, InputEvent, ScrollSource, SwitchKind, TouchData, TouchEndData, TouchToolType,
        },
        state::DeviceId,
    },
    evdev::{
        AbsoluteAxisCode, Device, KeyCode, LedCode, PropType, RelativeAxisCode, SynchronizationCode,
    },
    rustix::event::epoll,
    slotmap::Key,
    std::{
        collections::HashMap,
        io,
        os::unix::io::{AsRawFd, OwnedFd, RawFd},
        time::SystemTime,
    },
};

const MAX_SLOTS: usize = 16;
/// `_IOC(_IOC_READ, 'E', 0x0a, (1 + MAX_SLOTS) * 4)`: EVIOCGMTSLOTS.
const EVIOCGMTSLOTS: libc::c_ulong =
    ((2u64 << 30) | ((4 * (1 + MAX_SLOTS as u64)) << 16) | ((b'E' as u64) << 8) | 0x0a)
        as libc::c_ulong;
const BTN_MISC_START: u32 = 0x100;
const BTN_CODE_END: u32 = 0x15f;
const BTN_LEFT: u32 = 0x110;
const BTN_TASK: u32 = 0x117;
const BTN_0: u32 = 0x100;
const BTN_9: u32 = 0x109;
const EV_SYN: u16 = 0x00;
const EV_LED: u16 = 0x11;
const SYN_REPORT: u16 = 0x00;
const BTN_TOUCH: u32 = 0x14a;
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DeviceKind {
    #[default]
    Other,
    TabletTool,
    TabletPad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    PointerButton,
    Key,
    TabletToolButton,
    TabletPadButton,
}

#[derive(Default, Clone, Copy)]
struct Slot {
    active: bool,
    x: f64,
    y: f64,
    pressure: Option<f64>,
    touch_major: Option<f64>,
    touch_minor: Option<f64>,
    tool_type: Option<TouchToolType>,
    tracking_id: Option<i32>,
    dirty: bool,
    changed: bool,
}

impl Slot {
    fn clear_contact_metadata(&mut self) {
        self.pressure = None;
        self.touch_major = None;
        self.touch_minor = None;
        self.tool_type = None;
        self.tracking_id = None;
    }
}

#[derive(Default, Clone, Copy)]
struct SingleTouchState {
    active: bool,
    x: f64,
    y: f64,
    pressure: Option<f64>,
    touch_major: Option<f64>,
    touch_minor: Option<f64>,
    tool_type: Option<TouchToolType>,
    have_x: bool,
    have_y: bool,
    dirty: bool,
    pending_active: Option<bool>,
}

impl SingleTouchState {
    fn clear_contact_metadata(&mut self) {
        self.pressure = None;
        self.touch_major = None;
        self.touch_minor = None;
        self.tool_type = None;
    }
}

#[derive(Default, Clone, Copy)]
struct TabletToolState {
    in_range: bool,
    tool: Option<crate::events::TabletToolKind>,
    pending_proximity: Option<bool>,
    tip_down: bool,
    pending_tip: Option<bool>,
    x: f64,
    y: f64,
    pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
    axes_dirty: bool,
}

#[derive(Default, Clone, Copy)]
struct TabletPadState {
    ring: f64,
    strip: f64,
    ring_dirty: bool,
    strip_dirty: bool,
    dial_delta: f64,
}

#[derive(Default)]
struct PendingState {
    device_kind: DeviceKind,
    /// Device exposes ABS_MT position axes; enables EVIOCGMTSLOTS resync.
    mt_enabled: bool,
    tablet: TabletToolState,
    pad: TabletPadState,

    dx: f64,
    dy: f64,
    dirty: bool,

    // Scroll state
    dw: f64,
    dw_hi_res: f64,
    dh: f64,
    dh_hi_res: f64,
    /// Whether a high-resolution wheel axis was actually reported this frame.
    /// Presence, not the accumulated value, selects the representation.
    dw_hi_res_seen: bool,
    dh_hi_res_seen: bool,
    scroll_dirty: bool,

    /// Key/button codes emitted as pressed, with the event class needed to
    /// synthesize a release after SYN_DROPPED.
    keys_down: HashMap<u32, KeyKind>,

    current_slot: usize,
    slots: [Slot; MAX_SLOTS],

    switch_states: HashMap<SwitchKind, bool>,
    pending_switches: Vec<(SwitchKind, bool)>,

    single_touch_enabled: bool,
    single_touch: SingleTouchState,
}

pub struct EvdevBackend {
    devices: HashMap<DeviceId, Device>,
    states: HashMap<DeviceId, PendingState>,
    epoll_fd: OwnedFd,
}

fn is_button_code(code: u32) -> bool {
    (BTN_MISC_START..=BTN_CODE_END).contains(&code)
}

/// Select the v120 representation for a wheel axis.
///
/// A high-resolution axis wins when the kernel actually reported it in this
/// frame, even if the accumulated value is zero (for example a +120/-120 pair).
fn scroll_v120(legacy: f64, hi_res: f64, hi_res_seen: bool) -> f64 {
    if hi_res_seen { hi_res } else { legacy * 120.0 }
}

fn is_pointer_button_code(code: u32) -> bool {
    (BTN_LEFT..=BTN_TASK).contains(&code)
}

fn is_tablet_tool_proximity_code(code: u32) -> bool {
    matches!(code, 0x140..=0x144 | 0x146 | 0x147)
}

fn is_tablet_pad_button_code(code: u32) -> bool {
    (BTN_0..=BTN_9).contains(&code)
}

impl EvdevBackend {
    pub fn new() -> io::Result<Self> {
        let epoll_fd =
            epoll::create(epoll::CreateFlags::CLOEXEC).map_err(|e| io::Error::from(e))?;

        Ok(Self {
            devices: HashMap::new(),
            states: HashMap::new(),
            epoll_fd,
        })
    }
}

impl Backend for EvdevBackend {
    fn initialize(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn get_fd(&self) -> RawFd {
        self.epoll_fd.as_raw_fd()
    }

    fn add_device_fd(&mut self, id: DeviceId, fd: OwnedFd, name: &str) -> io::Result<DeviceInfo> {
        let device = Device::from_fd(fd)?;

        let mut info = DeviceInfo::default();

        // Prefer the evdev-reported name when available. During hotplug, sysfs
        // can briefly be generic while the evdev fd already has the real name.
        info.name = device
            .name()
            .filter(|name| !name.is_empty())
            .unwrap_or(name)
            .to_string();
        let input_id = device.input_id();
        info.vendor_id = input_id.vendor();
        info.product_id = input_id.product();
        info.bus = match input_id.bus_type() {
            evdev::BusType::BUS_USB => BusType::Usb,
            evdev::BusType::BUS_BLUETOOTH => BusType::Bluetooth,
            evdev::BusType::BUS_I2C => BusType::I2c,
            evdev::BusType::BUS_I8042 => BusType::Ps2,
            _ => BusType::Unknown,
        };
        info.internal = matches!(info.bus, BusType::I2c | BusType::Ps2);

        let properties = device.properties();
        info.is_clickpad = properties.contains(PropType::BUTTONPAD);
        info.is_top_buttonpad = properties.contains(PropType::TOPBUTTONPAD);
        let is_pointing_stick = properties.contains(PropType::POINTING_STICK);
        if let Ok(leds) = device.get_led_state() {
            info.led_state = crate::policy::LedState {
                caps_lock: leds.contains(LedCode::LED_CAPSL),
                num_lock: leds.contains(LedCode::LED_NUML),
                scroll_lock: leds.contains(LedCode::LED_SCROLLL),
            };
        }

        // Detect device capabilities from evdev event types
        let mut caps = DeviceCaps::default();
        for ev_type in device.supported_events() {
            match ev_type {
                evdev::EventType::RELATIVE => caps.insert(DeviceCaps::POINTER),
                evdev::EventType::SWITCH => caps.insert(DeviceCaps::SWITCH),
                _ => {}
            }
        }
        if device.supported_keys().is_some_and(|keys| {
            // Linux input button codes occupy BTN_MISC..BTN_GEAR_UP
            // (0x100..=0x15f). Treat only non-button codes as keyboard keys
            // so BTN_TOUCH/BTN_TOOL_* do not make direct-touch devices look
            // like touchpads/keyboards, while consumer keys above BTN_* still
            // count as keys.
            keys.iter().any(|key| !is_button_code(key.0 as u32))
        }) {
            caps.insert(DeviceCaps::KEYBOARD);
        }
        let mut initial_switch_states = HashMap::new();
        if caps.contains(DeviceCaps::SWITCH) {
            if let Some(switches) = device.supported_switches() {
                let mut supported_codes = Vec::new();
                for code in switches.iter() {
                    let switch = SwitchKind::from_linux_code(code.0 as u32);
                    info.supported_switches.push(switch);
                    supported_codes.push((code, switch));
                }

                if info.supported_switches.is_empty() {
                    caps.remove(DeviceCaps::SWITCH);
                } else {
                    match device.get_switch_state() {
                        Ok(active_switches) => {
                            for (code, switch) in supported_codes {
                                let enabled = active_switches.contains(code);
                                info.switch_states.push((switch, enabled));
                                initial_switch_states.insert(switch, enabled);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "evdev: could not read initial switch state for {:?}: {}",
                                id,
                                e
                            );
                        }
                    }
                }
            } else {
                caps.remove(DeviceCaps::SWITCH);
            }
        }
        // MT devices with touchpad tool markers are treated as touchpads.
        // Direct MT devices, plus the conservative single-touch fallback
        // below, are reported as touchscreens.
        let has_abs_mt = device.get_absinfo().is_ok_and(|mut iter| {
            iter.any(|(code, _)| {
                matches!(
                    code,
                    AbsoluteAxisCode::ABS_MT_POSITION_X | AbsoluteAxisCode::ABS_MT_POSITION_Y
                )
            })
        });
        let supported_absolute_axes = device.supported_absolute_axes();
        let has_abs_xy = supported_absolute_axes.is_some_and(|axes| {
            axes.contains(AbsoluteAxisCode::ABS_X) && axes.contains(AbsoluteAxisCode::ABS_Y)
        });
        let has_abs_wheel =
            supported_absolute_axes.is_some_and(|axes| axes.contains(AbsoluteAxisCode::ABS_WHEEL));
        let (has_btn_touch, has_tablet_tool, has_touchpad_tool) = device
            .supported_keys()
            .map(|keys| {
                (
                    keys.contains(KeyCode::BTN_TOUCH),
                    keys.contains(KeyCode::BTN_TOOL_PEN)
                        || keys.contains(KeyCode::BTN_TOOL_RUBBER)
                        || keys.contains(KeyCode::BTN_TOOL_BRUSH)
                        || keys.contains(KeyCode::BTN_TOOL_PENCIL)
                        || keys.contains(KeyCode::BTN_TOOL_AIRBRUSH)
                        || keys.contains(KeyCode::BTN_TOOL_MOUSE)
                        || keys.contains(KeyCode::BTN_TOOL_LENS)
                        || keys.contains(KeyCode::BTN_STYLUS)
                        || keys.contains(KeyCode::BTN_STYLUS2),
                    keys.contains(KeyCode::BTN_TOOL_FINGER)
                        || keys.contains(KeyCode::BTN_TOOL_DOUBLETAP)
                        || keys.contains(KeyCode::BTN_TOOL_TRIPLETAP)
                        || keys.contains(KeyCode::BTN_TOOL_QUADTAP)
                        || keys.contains(KeyCode::BTN_TOOL_QUINTTAP),
                )
            })
            .unwrap_or((false, false, false));

        let pad_button_count = device
            .supported_keys()
            .map(|keys| {
                (BTN_0..=BTN_9)
                    .filter(|code| keys.contains(KeyCode(*code as u16)))
                    .count()
            })
            .unwrap_or(0);

        if has_tablet_tool && has_abs_xy && !has_abs_mt {
            caps.insert(DeviceCaps::TABLET);
        }
        let is_tablet_pad =
            !has_tablet_tool && !has_abs_mt && (pad_button_count > 0 || has_abs_wheel);
        if is_tablet_pad {
            caps.insert(DeviceCaps::TABLET_PAD);
            caps.remove(DeviceCaps::KEYBOARD);
        }

        let single_touch_enabled = has_abs_xy
            && has_btn_touch
            && !has_abs_mt
            && !has_tablet_tool
            && !caps.contains(DeviceCaps::POINTER);
        if has_abs_mt {
            if has_touchpad_tool && !has_tablet_tool {
                caps.insert(DeviceCaps::TOUCHPAD);
            } else {
                caps.insert(DeviceCaps::TOUCHSCREEN);
            }
        } else if single_touch_enabled {
            caps.insert(DeviceCaps::TOUCHSCREEN);
            caps.remove(DeviceCaps::KEYBOARD);
        }
        if info.is_clickpad {
            caps.insert(DeviceCaps::CLICKPAD);
        }
        if is_pointing_stick {
            caps.insert(DeviceCaps::POINTING_STICK);
        }
        info.capabilities = caps;

        let mut abs_x = None;
        let mut abs_y = None;
        let mut mt_x = None;
        let mut mt_y = None;
        if let Ok(absinfo_iter) = device.get_absinfo() {
            for (code, abs) in absinfo_iter {
                let axis = AxisInfo {
                    min: abs.minimum() as f64,
                    max: abs.maximum() as f64,
                    resolution: abs.resolution() as f64,
                };
                match code {
                    AbsoluteAxisCode::ABS_X => abs_x = Some(axis),
                    AbsoluteAxisCode::ABS_Y => abs_y = Some(axis),
                    AbsoluteAxisCode::ABS_MT_POSITION_X => mt_x = Some(axis),
                    AbsoluteAxisCode::ABS_MT_POSITION_Y => mt_y = Some(axis),
                    _ => {}
                }
            }
        }
        if has_abs_mt {
            info.x = mt_x.or(abs_x).unwrap_or_default();
            info.y = mt_y.or(abs_y).unwrap_or_default();
        } else {
            info.x = abs_x.or(mt_x).unwrap_or_default();
            info.y = abs_y.or(mt_y).unwrap_or_default();
        }

        if !id.is_null() {
            let id_u64 = id.data().as_ffi();
            epoll::add(
                &self.epoll_fd,
                &device,
                epoll::EventData::new_u64(id_u64),
                epoll::EventFlags::IN,
            )
            .map_err(|e| io::Error::from(e))?;

            let device_kind = if caps.contains(DeviceCaps::TABLET) {
                DeviceKind::TabletTool
            } else if caps.contains(DeviceCaps::TABLET_PAD) {
                DeviceKind::TabletPad
            } else {
                DeviceKind::Other
            };

            self.devices.insert(id, device);
            self.states.insert(
                id,
                PendingState {
                    device_kind,
                    mt_enabled: has_abs_mt,
                    switch_states: initial_switch_states,
                    single_touch_enabled,
                    ..Default::default()
                },
            );
        }

        Ok(info)
    }

    fn remove_device(&mut self, id: DeviceId) -> io::Result<()> {
        if let Some(device) = self.devices.remove(&id) {
            let _ = epoll::delete(&self.epoll_fd, &device);
        }
        self.states.remove(&id);
        Ok(())
    }

    fn set_led_state(&mut self, id: DeviceId, state: crate::policy::LedState) -> io::Result<()> {
        let Some(device) = self.devices.get(&id) else {
            return Ok(());
        };

        write_input_event(device, EV_LED, LedCode::LED_NUML.0, state.num_lock as i32)?;
        write_input_event(device, EV_LED, LedCode::LED_CAPSL.0, state.caps_lock as i32)?;
        write_input_event(
            device,
            EV_LED,
            LedCode::LED_SCROLLL.0,
            state.scroll_lock as i32,
        )?;
        write_input_event(device, EV_SYN, SYN_REPORT, 0)?;
        Ok(())
    }

    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
        let mut epoll_events = Vec::<epoll::Event>::with_capacity(32);
        let no_wait = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        epoll::wait(
            &self.epoll_fd,
            rustix::buffer::spare_capacity(&mut epoll_events),
            Some(&no_wait),
        )
        .map_err(io::Error::from)?;

        let mut dead_devices = Vec::new();
        for event in &epoll_events {
            let id_u64 = event.data.u64();
            let device_id = DeviceId::from(slotmap::KeyData::from_ffi(id_u64));

            if let Some(device) = self.devices.get_mut(&device_id) {
                let state = self.states.entry(device_id).or_default();

                match fetch_and_process(device, device_id, state, callback) {
                    Ok(Some(time_us)) => {
                        // The event iterator's mutable borrow has ended, so the
                        // keystate query can run.
                        resync_key_state(device, device_id, time_us, state, callback);
                    }
                    Ok(None) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        tracing::warn!(
                            "evdev: removing device {:?} after fetch_events error: {}",
                            device_id,
                            e
                        );
                        dead_devices.push(device_id);
                    }
                }
            }
        }

        for device_id in dead_devices {
            if let Some(device) = self.devices.remove(&device_id) {
                let _ = epoll::delete(&self.epoll_fd, &device);
            }
            if let Some(mut state) = self.states.remove(&device_id) {
                let time_us = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64;
                cancel_active_touches(device_id, DevIndex(0), time_us, &mut state, callback);
            }
            // Context remaps this to the device's real DevIndex.
            callback(
                device_id,
                InputEvent::DeviceRemoved {
                    device_index: DevIndex(0),
                },
            );
        }
        Ok(())
    }
}

#[cfg(target_pointer_width = "64")]
const INPUT_EVENT_TIME_BYTES: usize = 16; // struct timeval { i64 sec, i64 usec }
#[cfg(not(target_pointer_width = "64"))]
const INPUT_EVENT_TIME_BYTES: usize = 8; // struct timeval { i32 sec, i32 usec }

/// Write one raw `input_event` to an evdev fd.
///
/// The evdev crate exposes reads but no LED write API. Linux evdev accepts
/// `EV_LED` events written to the device fd; timestamps are left zero because
/// the kernel stamps injected LED events itself.
fn write_input_event(device: &Device, event_type: u16, code: u16, value: i32) -> io::Result<()> {
    let offset = INPUT_EVENT_TIME_BYTES;
    let mut buf = [0u8; 24];
    buf[offset..offset + 2].copy_from_slice(&event_type.to_ne_bytes());
    buf[offset + 2..offset + 4].copy_from_slice(&code.to_ne_bytes());
    buf[offset + 4..offset + 8].copy_from_slice(&value.to_ne_bytes());

    let fd = unsafe { rustix::fd::BorrowedFd::borrow_raw(device.as_raw_fd()) };
    rustix::io::write(fd, &buf[..offset + 8]).map_err(io::Error::from)?;
    Ok(())
}

fn fetch_and_process(
    device: &mut Device,
    device_id: DeviceId,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) -> io::Result<Option<u64>> {
    let mut dropped_time_us = None;
    for ev in device.fetch_events()? {
        if ev.event_type() == evdev::EventType::SYNCHRONIZATION
            && ev.code() == SynchronizationCode::SYN_DROPPED.0
        {
            dropped_time_us = Some(
                ev.timestamp()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64,
            );
        }
        process_evdev_event(device_id, &ev, state, callback);
    }
    Ok(dropped_time_us)
}

fn track_key(state: &mut PendingState, code: u32, pressed: bool, kind: KeyKind) {
    if pressed {
        state.keys_down.insert(code, kind);
    } else {
        state.keys_down.remove(&code);
    }
}

fn classify_key(state: &PendingState, code: u32) -> Option<KeyKind> {
    if is_pointer_button_code(code) {
        Some(KeyKind::PointerButton)
    } else if state.device_kind == DeviceKind::TabletTool
        && (code == BTN_STYLUS || code == BTN_STYLUS2)
    {
        Some(KeyKind::TabletToolButton)
    } else if state.device_kind == DeviceKind::TabletPad && is_tablet_pad_button_code(code) {
        Some(KeyKind::TabletPadButton)
    } else if is_button_code(code) {
        // Contact/tool markers are represented by touch/tablet state, not as
        // independent keys.
        None
    } else {
        Some(KeyKind::Key)
    }
}

fn resync_key_state(
    device: &Device,
    device_id: DeviceId,
    time_us: u64,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    let kernel_keys = match device.get_key_state() {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(
                "evdev: could not resynchronize key state after SYN_DROPPED: {}",
                error
            );
            return;
        }
    };

    let mut kernel_codes = std::collections::HashSet::new();
    for code in kernel_keys.iter() {
        kernel_codes.insert(code.0 as u32);
    }

    // Release anything we believe is down that the kernel says is up. Sort for
    // deterministic event order.
    let mut releases: Vec<(u32, KeyKind)> = state
        .keys_down
        .iter()
        .filter(|(code, _)| !kernel_codes.contains(code))
        .map(|(code, kind)| (*code, *kind))
        .collect();
    releases.sort_by_key(|(code, _)| *code);
    for (code, kind) in releases {
        let dev_idx = DevIndex(0);
        let event = match kind {
            KeyKind::PointerButton => InputEvent::PointerButton {
                device_index: dev_idx,
                time_us,
                button: code,
                pressed: false,
            },
            KeyKind::Key => InputEvent::Key {
                device_index: dev_idx,
                time_us,
                key: code,
                pressed: false,
            },
            KeyKind::TabletToolButton => {
                InputEvent::TabletToolButton(Box::new(crate::events::TabletToolButtonData {
                    device_index: dev_idx,
                    time_us,
                    button: code,
                    pressed: false,
                }))
            }
            KeyKind::TabletPadButton => {
                InputEvent::TabletPadButton(Box::new(crate::events::TabletPadButtonData {
                    device_index: dev_idx,
                    time_us,
                    index: code.saturating_sub(BTN_0),
                    pressed: false,
                }))
            }
        };
        callback(device_id, event);
        state.keys_down.remove(&code);
    }

    // Press anything the kernel holds that we do not know about. This
    // reconstructs state lost across the dropped-event boundary.
    let mut presses: Vec<u32> = kernel_codes
        .iter()
        .copied()
        .filter(|code| !state.keys_down.contains_key(code))
        .collect();
    presses.sort_unstable();

    for code in presses {
        if state.keys_down.contains_key(&code) {
            continue;
        }
        let Some(kind) = classify_key(state, code) else {
            continue;
        };
        let dev_idx = DevIndex(0);
        let event = match kind {
            KeyKind::PointerButton => InputEvent::PointerButton {
                device_index: dev_idx,
                time_us,
                button: code,
                pressed: true,
            },
            KeyKind::Key => InputEvent::Key {
                device_index: dev_idx,
                time_us,
                key: code,
                pressed: true,
            },
            KeyKind::TabletToolButton => {
                InputEvent::TabletToolButton(Box::new(crate::events::TabletToolButtonData {
                    device_index: dev_idx,
                    time_us,
                    button: code,
                    pressed: true,
                }))
            }
            KeyKind::TabletPadButton => {
                InputEvent::TabletPadButton(Box::new(crate::events::TabletPadButtonData {
                    device_index: dev_idx,
                    time_us,
                    index: code.saturating_sub(BTN_0),
                    pressed: true,
                }))
            }
        };
        callback(device_id, event);
        state.keys_down.insert(code, kind);
    }

    resync_absolute_state(device, device_id, time_us, &kernel_codes, state, callback);
}

fn get_mt_values(device: &Device, code: u16) -> io::Result<[i32; MAX_SLOTS]> {
    let mut values = [0i32; 1 + MAX_SLOTS];
    values[0] = code as i32;
    let result = unsafe { libc::ioctl(device.as_raw_fd(), EVIOCGMTSLOTS, values.as_mut_ptr()) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut out = [0i32; MAX_SLOTS];
    out.copy_from_slice(&values[1..]);
    Ok(out)
}

fn mt_slots_from_values(
    tracking: &[i32; MAX_SLOTS],
    x: &[i32; MAX_SLOTS],
    y: &[i32; MAX_SLOTS],
    pressure: &[i32; MAX_SLOTS],
    touch_major: &[i32; MAX_SLOTS],
    touch_minor: &[i32; MAX_SLOTS],
    tool_type: &[i32; MAX_SLOTS],
) -> [Slot; MAX_SLOTS] {
    let mut slots = [Slot::default(); MAX_SLOTS];
    for (index, slot) in slots.iter_mut().enumerate() {
        if tracking[index] < 0 {
            continue;
        }
        *slot = Slot {
            active: true,
            x: x[index] as f64,
            y: y[index] as f64,
            pressure: Some(pressure[index] as f64),
            touch_major: Some(touch_major[index] as f64),
            touch_minor: Some(touch_minor[index] as f64),
            tool_type: Some(TouchToolType::from_linux_value(tool_type[index])),
            tracking_id: Some(tracking[index]),
            dirty: false,
            changed: false,
        };
    }
    slots
}

fn resync_absolute_state(
    device: &Device,
    device_id: DeviceId,
    time_us: u64,
    kernel_codes: &std::collections::HashSet<u32>,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    let abs = match device.get_abs_state() {
        Ok(abs) => abs,
        Err(error) => {
            tracing::warn!(
                "evdev: could not resynchronize absolute state after SYN_DROPPED: {}",
                error
            );
            return;
        }
    };
    let dev_idx = DevIndex(0);

    match state.device_kind {
        DeviceKind::TabletTool => {
            let in_range = kernel_codes
                .iter()
                .any(|code| is_tablet_tool_proximity_code(*code));
            if !in_range {
                state.tablet = TabletToolState::default();
                return;
            }

            let x = abs[AbsoluteAxisCode::ABS_X.0 as usize].value as f64;
            let y = abs[AbsoluteAxisCode::ABS_Y.0 as usize].value as f64;
            let pressure = abs[AbsoluteAxisCode::ABS_PRESSURE.0 as usize].value as f64;
            let tilt_x = abs[AbsoluteAxisCode::ABS_TILT_X.0 as usize].value as f64;
            let tilt_y = abs[AbsoluteAxisCode::ABS_TILT_Y.0 as usize].value as f64;

            if !state.tablet.in_range {
                state.tablet.in_range = true;
                callback(
                    device_id,
                    InputEvent::TabletToolProximity(Box::new(
                        crate::events::TabletToolProximityData {
                            device_index: dev_idx,
                            time_us,
                            x,
                            y,
                            in_range: true,
                            tool: state.tablet.tool,
                        },
                    )),
                );
            }

            state.tablet.x = x;
            state.tablet.y = y;
            state.tablet.pressure = pressure;
            state.tablet.tilt_x = tilt_x;
            state.tablet.tilt_y = tilt_y;
            state.tablet.axes_dirty = false;
            state.tablet.pending_proximity = None;
            state.tablet.pending_tip = None;

            callback(
                device_id,
                InputEvent::TabletToolAxis(Box::new(crate::events::TabletToolAxisData {
                    device_index: dev_idx,
                    time_us,
                    x,
                    y,
                    pressure,
                    tilt_x,
                    tilt_y,
                })),
            );

            let tip_down = kernel_codes.contains(&BTN_TOUCH);
            if tip_down != state.tablet.tip_down {
                state.tablet.tip_down = tip_down;
                callback(
                    device_id,
                    InputEvent::TabletToolTip(Box::new(crate::events::TabletToolTipData {
                        device_index: dev_idx,
                        time_us,
                        down: tip_down,
                    })),
                );
            }
        }
        DeviceKind::TabletPad => {
            let ring = abs[AbsoluteAxisCode::ABS_WHEEL.0 as usize].value as f64;
            let strip = abs[AbsoluteAxisCode::ABS_MISC.0 as usize].value as f64;
            state.pad = TabletPadState::default();
            callback(
                device_id,
                InputEvent::TabletPadRing(Box::new(crate::events::TabletPadValueData {
                    device_index: dev_idx,
                    time_us,
                    index: 0,
                    position: ring,
                })),
            );
            callback(
                device_id,
                InputEvent::TabletPadStrip(Box::new(crate::events::TabletPadValueData {
                    device_index: dev_idx,
                    time_us,
                    index: 0,
                    position: strip,
                })),
            );
        }
        DeviceKind::Other => {
            if state.mt_enabled {
                let result = (|| -> io::Result<_> {
                    Ok(mt_slots_from_values(
                        &get_mt_values(device, 57)?, // ABS_MT_TRACKING_ID
                        &get_mt_values(device, 53)?, // ABS_MT_POSITION_X
                        &get_mt_values(device, 54)?, // ABS_MT_POSITION_Y
                        &get_mt_values(device, 58)?, // ABS_MT_PRESSURE
                        &get_mt_values(device, 48)?, // ABS_MT_TOUCH_MAJOR
                        &get_mt_values(device, 49)?, // ABS_MT_TOUCH_MINOR
                        &get_mt_values(device, 55)?, // ABS_MT_TOOL_TYPE
                    ))
                })();

                match result {
                    Ok(slots) => {
                        state.slots = slots;
                        state.current_slot = 0;
                        for (index, slot) in state.slots.iter().enumerate() {
                            if !slot.active {
                                continue;
                            }
                            callback(
                                device_id,
                                InputEvent::TouchDown(Box::new(TouchData {
                                    device_index: dev_idx,
                                    time_us,
                                    slot: index as i32,
                                    x: slot.x,
                                    y: slot.y,
                                    pressure: slot.pressure,
                                    touch_major: slot.touch_major,
                                    touch_minor: slot.touch_minor,
                                    tool_type: slot.tool_type,
                                    tracking_id: slot.tracking_id,
                                })),
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            "evdev: could not resynchronize MT slots after SYN_DROPPED: {}",
                            error
                        );
                    }
                }
            } else if state.single_touch_enabled {
                let tip = kernel_codes.contains(&BTN_TOUCH);
                if tip {
                    let x = abs[AbsoluteAxisCode::ABS_X.0 as usize].value as f64;
                    let y = abs[AbsoluteAxisCode::ABS_Y.0 as usize].value as f64;
                    let pressure = abs[AbsoluteAxisCode::ABS_PRESSURE.0 as usize].value as f64;
                    state.single_touch = SingleTouchState {
                        active: true,
                        x,
                        y,
                        pressure: Some(pressure),
                        have_x: true,
                        have_y: true,
                        ..Default::default()
                    };
                    callback(
                        device_id,
                        InputEvent::TouchDown(Box::new(TouchData {
                            device_index: dev_idx,
                            time_us,
                            slot: 0,
                            x,
                            y,
                            pressure: Some(pressure),
                            touch_major: None,
                            touch_minor: None,
                            tool_type: None,
                            tracking_id: None,
                        })),
                    );
                } else {
                    state.single_touch = SingleTouchState::default();
                }
            }
        }
    }
}

fn process_evdev_event(
    device_id: DeviceId,
    ev: &evdev::InputEvent,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    let time_us = ev
        .timestamp()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    // Use a placeholder DevIndex(0) — Context::dispatch will remap it
    let dev_idx = DevIndex(0);

    match ev.event_type() {
        evdev::EventType::RELATIVE => {
            let code = ev.code();
            if state.device_kind == DeviceKind::TabletPad && code == RelativeAxisCode::REL_DIAL.0 {
                state.pad.dial_delta += ev.value() as f64;
            } else if code == RelativeAxisCode::REL_X.0 {
                state.dx += ev.value() as f64;
                state.dirty = true;
            } else if code == RelativeAxisCode::REL_Y.0 {
                state.dy += ev.value() as f64;
                state.dirty = true;
            } else if code == RelativeAxisCode::REL_WHEEL.0 {
                state.dw += ev.value() as f64;
                state.scroll_dirty = true;
            } else if code == RelativeAxisCode::REL_WHEEL_HI_RES.0 {
                state.dw_hi_res += ev.value() as f64;
                state.dw_hi_res_seen = true;
                state.scroll_dirty = true;
            } else if code == RelativeAxisCode::REL_HWHEEL.0 {
                state.dh += ev.value() as f64;
                state.scroll_dirty = true;
            } else if code == RelativeAxisCode::REL_HWHEEL_HI_RES.0 {
                state.dh_hi_res += ev.value() as f64;
                state.dh_hi_res_seen = true;
                state.scroll_dirty = true;
            }
        }
        evdev::EventType::ABSOLUTE => {
            let code = ev.code();
            if state.device_kind == DeviceKind::TabletTool {
                if code == AbsoluteAxisCode::ABS_X.0 {
                    state.tablet.x = ev.value() as f64;
                    state.tablet.axes_dirty = true;
                } else if code == AbsoluteAxisCode::ABS_Y.0 {
                    state.tablet.y = ev.value() as f64;
                    state.tablet.axes_dirty = true;
                } else if code == AbsoluteAxisCode::ABS_PRESSURE.0 {
                    state.tablet.pressure = ev.value() as f64;
                    state.tablet.axes_dirty = true;
                } else if code == AbsoluteAxisCode::ABS_TILT_X.0 {
                    state.tablet.tilt_x = ev.value() as f64;
                    state.tablet.axes_dirty = true;
                } else if code == AbsoluteAxisCode::ABS_TILT_Y.0 {
                    state.tablet.tilt_y = ev.value() as f64;
                    state.tablet.axes_dirty = true;
                }
            } else if state.device_kind == DeviceKind::TabletPad {
                if code == AbsoluteAxisCode::ABS_WHEEL.0 {
                    state.pad.ring = ev.value() as f64;
                    state.pad.ring_dirty = true;
                } else if code == AbsoluteAxisCode::ABS_MISC.0 {
                    state.pad.strip = ev.value() as f64;
                    state.pad.strip_dirty = true;
                }
            } else if code == 47 {
                // ABS_MT_SLOT
                let slot = ev.value() as usize;
                if slot < MAX_SLOTS {
                    state.current_slot = slot;
                }
            } else if code == 57 {
                // ABS_MT_TRACKING_ID
                let slot = &mut state.slots[state.current_slot];
                let id = ev.value();
                if id == -1 {
                    if slot.active {
                        slot.active = false;
                        slot.changed = true;
                    }
                    slot.clear_contact_metadata();
                } else {
                    slot.tracking_id = Some(id);
                    if !slot.active {
                        slot.active = true;
                        slot.changed = true;
                    }
                }
            } else if code == 53 {
                // ABS_MT_POSITION_X
                state.slots[state.current_slot].x = ev.value() as f64;
                state.slots[state.current_slot].dirty = true;
            } else if code == 54 {
                // ABS_MT_POSITION_Y
                state.slots[state.current_slot].y = ev.value() as f64;
                state.slots[state.current_slot].dirty = true;
            } else if code == 58 {
                // ABS_MT_PRESSURE
                let slot = &mut state.slots[state.current_slot];
                slot.pressure = Some(ev.value() as f64);
                slot.dirty = true;
            } else if code == 48 {
                // ABS_MT_TOUCH_MAJOR
                let slot = &mut state.slots[state.current_slot];
                slot.touch_major = Some(ev.value() as f64);
                slot.dirty = true;
            } else if code == 49 {
                // ABS_MT_TOUCH_MINOR
                let slot = &mut state.slots[state.current_slot];
                slot.touch_minor = Some(ev.value() as f64);
                slot.dirty = true;
            } else if code == 55 {
                // ABS_MT_TOOL_TYPE
                let slot = &mut state.slots[state.current_slot];
                slot.tool_type = Some(TouchToolType::from_linux_value(ev.value()));
                slot.dirty = true;
            } else if state.single_touch_enabled && code == AbsoluteAxisCode::ABS_X.0 {
                state.single_touch.x = ev.value() as f64;
                state.single_touch.have_x = true;
                if state.single_touch.active || state.single_touch.pending_active == Some(true) {
                    state.single_touch.dirty = true;
                }
            } else if state.single_touch_enabled && code == AbsoluteAxisCode::ABS_Y.0 {
                state.single_touch.y = ev.value() as f64;
                state.single_touch.have_y = true;
                if state.single_touch.active || state.single_touch.pending_active == Some(true) {
                    state.single_touch.dirty = true;
                }
            } else if state.single_touch_enabled && code == AbsoluteAxisCode::ABS_PRESSURE.0 {
                state.single_touch.pressure = Some(ev.value() as f64);
                if state.single_touch.active || state.single_touch.pending_active == Some(true) {
                    state.single_touch.dirty = true;
                }
            } else if state.single_touch_enabled && code == AbsoluteAxisCode::ABS_TOOL_WIDTH.0 {
                state.single_touch.touch_major = Some(ev.value() as f64);
                if state.single_touch.active || state.single_touch.pending_active == Some(true) {
                    state.single_touch.dirty = true;
                }
            }
        }
        evdev::EventType::KEY => {
            // BTN_LEFT=0x110 through BTN_TASK=0x117 are pointer/touchpad buttons.
            // BTN_TOOL_* and BTN_TOUCH live later in the button range and are
            // contact metadata, not client-visible pointer buttons.
            // Keyboard keys use different codes (KEY_ESC=1, KEY_A=30, etc.)
            // so this range cleanly separates mouse buttons from keyboard keys.
            let code = ev.code() as u32;
            if state.device_kind == DeviceKind::TabletTool {
                if code == BTN_TOUCH {
                    state.tablet.pending_tip = Some(ev.value() != 0);
                } else if code == BTN_STYLUS || code == BTN_STYLUS2 {
                    let pressed = ev.value() != 0;
                    track_key(state, code, pressed, KeyKind::TabletToolButton);
                    callback(
                        device_id,
                        InputEvent::TabletToolButton(Box::new(
                            crate::events::TabletToolButtonData {
                                device_index: dev_idx,
                                time_us,
                                button: code,
                                pressed,
                            },
                        )),
                    );
                } else if is_tablet_tool_proximity_code(code) {
                    let pressed = ev.value() != 0;
                    if pressed {
                        state.tablet.tool = crate::events::TabletToolKind::from_linux_code(code);
                    } else {
                        state.tablet.tool = None;
                    }
                    state.tablet.pending_proximity = Some(pressed);
                } else if !is_button_code(code) {
                    let pressed = ev.value() != 0;
                    track_key(state, code, pressed, KeyKind::Key);
                    callback(
                        device_id,
                        InputEvent::Key {
                            device_index: dev_idx,
                            time_us,
                            key: code,
                            pressed,
                        },
                    );
                }
            } else if state.device_kind == DeviceKind::TabletPad {
                if is_tablet_pad_button_code(code) {
                    let pressed = ev.value() != 0;
                    track_key(state, code, pressed, KeyKind::TabletPadButton);
                    callback(
                        device_id,
                        InputEvent::TabletPadButton(Box::new(crate::events::TabletPadButtonData {
                            device_index: dev_idx,
                            time_us,
                            index: code - BTN_0,
                            pressed,
                        })),
                    );
                } else if !is_button_code(code) {
                    let pressed = ev.value() != 0;
                    track_key(state, code, pressed, KeyKind::Key);
                    callback(
                        device_id,
                        InputEvent::Key {
                            device_index: dev_idx,
                            time_us,
                            key: code,
                            pressed,
                        },
                    );
                }
            } else if state.single_touch_enabled && code == BTN_TOUCH {
                state.single_touch.pending_active = Some(ev.value() != 0);
            } else if is_pointer_button_code(code) {
                let pressed = ev.value() != 0;
                track_key(state, code, pressed, KeyKind::PointerButton);
                callback(
                    device_id,
                    InputEvent::PointerButton {
                        device_index: dev_idx,
                        time_us,
                        button: code,
                        pressed,
                    },
                );
            } else if is_button_code(code) {
                // Other button-range codes describe tools/contacts/pads here.
                // They are handled by ABS_MT state or future tablet plumbing.
            } else {
                let pressed = ev.value() != 0;
                track_key(state, code, pressed, KeyKind::Key);
                callback(
                    device_id,
                    InputEvent::Key {
                        device_index: dev_idx,
                        time_us,
                        key: code,
                        pressed,
                    },
                );
            }
        }
        evdev::EventType::SWITCH => {
            let code = ev.code();
            queue_switch(
                state,
                SwitchKind::from_linux_code(code as u32),
                ev.value() != 0,
            );
        }
        evdev::EventType::SYNCHRONIZATION => {
            if ev.code() == SynchronizationCode::SYN_REPORT.0 {
                // SYN_REPORT
                if state.dirty {
                    callback(
                        device_id,
                        InputEvent::PointerMotion {
                            device_index: dev_idx,
                            time_us,
                            dx: state.dx,
                            dy: state.dy,
                            dx_unaccelerated: state.dx,
                            dy_unaccelerated: state.dy,
                        },
                    );
                    state.dx = 0.0;
                    state.dy = 0.0;
                    state.dirty = false;
                }

                if state.scroll_dirty {
                    let final_dw = scroll_v120(state.dw, state.dw_hi_res, state.dw_hi_res_seen);
                    let final_dh = scroll_v120(state.dh, state.dh_hi_res, state.dh_hi_res_seen);

                    if final_dw != 0.0 || final_dh != 0.0 {
                        callback(
                            device_id,
                            InputEvent::PointerScroll {
                                device_index: dev_idx,
                                time_us,
                                source: ScrollSource::Wheel,
                                dx: final_dh / 120.0,
                                // evdev REL_WHEEL positive is wheel-up; Wayland scroll
                                // convention is positive for down, so invert here.
                                dy: -final_dw / 120.0,
                            },
                        );
                    }
                    state.dw = 0.0;
                    state.dw_hi_res = 0.0;
                    state.dh = 0.0;
                    state.dh_hi_res = 0.0;
                    state.dw_hi_res_seen = false;
                    state.dh_hi_res_seen = false;
                    state.scroll_dirty = false;
                }

                for (i, slot) in state.slots.iter_mut().enumerate() {
                    if slot.changed {
                        if slot.active {
                            callback(
                                device_id,
                                InputEvent::TouchDown(Box::new(TouchData {
                                    device_index: dev_idx,
                                    time_us,
                                    slot: i as i32,
                                    x: slot.x,
                                    y: slot.y,
                                    pressure: slot.pressure,
                                    touch_major: slot.touch_major,
                                    touch_minor: slot.touch_minor,
                                    tool_type: slot.tool_type,
                                    tracking_id: slot.tracking_id,
                                })),
                            );
                        } else {
                            callback(
                                device_id,
                                InputEvent::TouchUp(Box::new(TouchEndData {
                                    device_index: dev_idx,
                                    time_us,
                                    slot: i as i32,
                                })),
                            );
                        }
                        slot.changed = false;
                    } else if slot.active && slot.dirty {
                        callback(
                            device_id,
                            InputEvent::TouchMotion(Box::new(TouchData {
                                device_index: dev_idx,
                                time_us,
                                slot: i as i32,
                                x: slot.x,
                                y: slot.y,
                                pressure: slot.pressure,
                                touch_major: slot.touch_major,
                                touch_minor: slot.touch_minor,
                                tool_type: slot.tool_type,
                                tracking_id: slot.tracking_id,
                            })),
                        );
                    }
                    slot.dirty = false;
                }

                flush_single_touch(device_id, dev_idx, time_us, state, callback);
                flush_tablet_tool(device_id, dev_idx, time_us, state, callback);
                flush_tablet_pad(device_id, dev_idx, time_us, state, callback);
                flush_switches(device_id, dev_idx, time_us, state, callback);

                callback(
                    device_id,
                    InputEvent::Frame {
                        device_index: dev_idx,
                        time_us,
                    },
                );
            } else if ev.code() == SynchronizationCode::SYN_DROPPED.0 {
                cancel_active_touches(device_id, dev_idx, time_us, state, callback);
            }
        }
        _ => {}
    }
}

fn queue_switch(state: &mut PendingState, switch: SwitchKind, enabled: bool) {
    for pending in &mut state.pending_switches {
        if pending.0 == switch {
            pending.1 = enabled;
            return;
        }
    }
    state.pending_switches.push((switch, enabled));
}

fn flush_switches(
    device_id: DeviceId,
    dev_idx: DevIndex,
    time_us: u64,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    let pending_switches = std::mem::take(&mut state.pending_switches);
    for (switch, enabled) in pending_switches {
        if state.switch_states.get(&switch).copied() == Some(enabled) {
            continue;
        }
        state.switch_states.insert(switch, enabled);
        callback(
            device_id,
            InputEvent::SwitchToggle {
                device_index: dev_idx,
                time_us,
                switch,
                enabled,
            },
        );
    }
}

fn cancel_active_touches(
    device_id: DeviceId,
    dev_idx: DevIndex,
    time_us: u64,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    for (i, slot) in state.slots.iter_mut().enumerate() {
        if slot.active {
            callback(
                device_id,
                InputEvent::TouchCancel(Box::new(TouchEndData {
                    device_index: dev_idx,
                    time_us,
                    slot: i as i32,
                })),
            );
        }
        *slot = Slot::default();
    }

    state.current_slot = 0;

    // Drop partially accumulated relative/scroll state from before the
    // dropped-event boundary. Keys/buttons remain owned by the context's
    // tracked state and are reconciled by later real events.
    state.dx = 0.0;
    state.dy = 0.0;
    state.dirty = false;
    state.dw = 0.0;
    state.dw_hi_res = 0.0;
    state.dh = 0.0;
    state.dh_hi_res = 0.0;
    state.dw_hi_res_seen = false;
    state.dh_hi_res_seen = false;
    state.scroll_dirty = false;
    state.pending_switches.clear();

    if state.single_touch.active {
        callback(
            device_id,
            InputEvent::TouchCancel(Box::new(TouchEndData {
                device_index: dev_idx,
                time_us,
                slot: 0,
            })),
        );
    }
    state.single_touch = SingleTouchState::default();

    if state.tablet.in_range {
        if state.tablet.tip_down {
            callback(
                device_id,
                InputEvent::TabletToolTip(Box::new(crate::events::TabletToolTipData {
                    device_index: dev_idx,
                    time_us,
                    down: false,
                })),
            );
        }
        callback(
            device_id,
            InputEvent::TabletToolProximity(Box::new(crate::events::TabletToolProximityData {
                device_index: dev_idx,
                time_us,
                x: state.tablet.x,
                y: state.tablet.y,
                in_range: false,
                tool: None,
            })),
        );
    }
    state.tablet = TabletToolState::default();
    state.pad = TabletPadState::default();
}

fn flush_tablet_tool(
    device_id: DeviceId,
    dev_idx: DevIndex,
    time_us: u64,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    if state.device_kind != DeviceKind::TabletTool {
        return;
    }

    let tablet = &mut state.tablet;

    if let Some(in_range) = tablet.pending_proximity.take() {
        if in_range != tablet.in_range {
            tablet.in_range = in_range;
            if !in_range {
                if tablet.tip_down {
                    tablet.tip_down = false;
                    callback(
                        device_id,
                        InputEvent::TabletToolTip(Box::new(crate::events::TabletToolTipData {
                            device_index: dev_idx,
                            time_us,
                            down: false,
                        })),
                    );
                }
                tablet.pending_tip = None;
                tablet.axes_dirty = false;
            }
            callback(
                device_id,
                InputEvent::TabletToolProximity(Box::new(crate::events::TabletToolProximityData {
                    device_index: dev_idx,
                    time_us,
                    x: tablet.x,
                    y: tablet.y,
                    in_range,
                    tool: tablet.tool,
                })),
            );
        }
    }

    if !tablet.in_range {
        tablet.axes_dirty = false;
        tablet.pending_tip = None;
        return;
    }

    if tablet.axes_dirty {
        tablet.axes_dirty = false;
        callback(
            device_id,
            InputEvent::TabletToolAxis(Box::new(crate::events::TabletToolAxisData {
                device_index: dev_idx,
                time_us,
                x: tablet.x,
                y: tablet.y,
                pressure: tablet.pressure,
                tilt_x: tablet.tilt_x,
                tilt_y: tablet.tilt_y,
            })),
        );
    }

    if let Some(down) = tablet.pending_tip.take() {
        if down != tablet.tip_down {
            tablet.tip_down = down;
            callback(
                device_id,
                InputEvent::TabletToolTip(Box::new(crate::events::TabletToolTipData {
                    device_index: dev_idx,
                    time_us,
                    down,
                })),
            );
        }
    }
}

fn flush_tablet_pad(
    device_id: DeviceId,
    dev_idx: DevIndex,
    time_us: u64,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    if state.device_kind != DeviceKind::TabletPad {
        return;
    }

    let pad = &mut state.pad;
    if pad.ring_dirty {
        pad.ring_dirty = false;
        callback(
            device_id,
            InputEvent::TabletPadRing(Box::new(crate::events::TabletPadValueData {
                device_index: dev_idx,
                time_us,
                index: 0,
                position: pad.ring,
            })),
        );
    }
    if pad.strip_dirty {
        pad.strip_dirty = false;
        callback(
            device_id,
            InputEvent::TabletPadStrip(Box::new(crate::events::TabletPadValueData {
                device_index: dev_idx,
                time_us,
                index: 0,
                position: pad.strip,
            })),
        );
    }
    if pad.dial_delta != 0.0 {
        let delta = std::mem::take(&mut pad.dial_delta);
        callback(
            device_id,
            InputEvent::TabletPadDial(Box::new(crate::events::TabletPadDialData {
                device_index: dev_idx,
                time_us,
                index: 0,
                delta,
            })),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(state: &mut PendingState) -> Vec<InputEvent> {
        let mut events = Vec::new();
        let id = DeviceId::null();
        let mut callback = |_id: DeviceId, event: InputEvent| events.push(event);
        flush_tablet_tool(id, DevIndex(0), 10, state, &mut callback);
        flush_tablet_pad(id, DevIndex(0), 10, state, &mut callback);
        flush_switches(id, DevIndex(0), 10, state, &mut callback);
        events
    }

    #[test]
    fn tablet_tool_emits_proximity_axis_and_tip_in_order() {
        let mut state = PendingState {
            device_kind: DeviceKind::TabletTool,
            ..Default::default()
        };
        state.tablet.pending_proximity = Some(true);
        state.tablet.x = 100.0;
        state.tablet.y = 200.0;
        state.tablet.pressure = 300.0;
        state.tablet.tilt_x = 4.0;
        state.tablet.tilt_y = -5.0;
        state.tablet.axes_dirty = true;
        state.tablet.pending_tip = Some(true);

        let events = collect(&mut state);
        assert!(matches!(
            events[0],
            InputEvent::TabletToolProximity(ref data) if data.in_range
        ));
        assert!(matches!(
            events[1],
            InputEvent::TabletToolAxis(ref data)
                if data.x == 100.0 && data.pressure == 300.0 && data.tilt_y == -5.0
        ));
        assert!(matches!(
            events[2],
            InputEvent::TabletToolTip(ref data) if data.down
        ));
        assert!(state.tablet.in_range);
    }

    #[test]
    fn tablet_tool_proximity_out_stops_axis_output() {
        let mut state = PendingState {
            device_kind: DeviceKind::TabletTool,
            ..Default::default()
        };
        state.tablet.in_range = true;
        state.tablet.pending_proximity = Some(false);
        state.tablet.axes_dirty = true;
        state.tablet.pending_tip = Some(false);

        let events = collect(&mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::TabletToolProximity(ref data) if !data.in_range
        ));
        assert!(!state.tablet.in_range);
        assert!(!state.tablet.axes_dirty);
    }

    #[test]
    fn tablet_pad_emits_ring_strip_and_dial() {
        let mut state = PendingState {
            device_kind: DeviceKind::TabletPad,
            ..Default::default()
        };
        state.pad.ring = 180.0;
        state.pad.ring_dirty = true;
        state.pad.strip = 42.0;
        state.pad.strip_dirty = true;
        state.pad.dial_delta = 3.0;

        let events = collect(&mut state);
        assert!(matches!(
            events[0],
            InputEvent::TabletPadRing(ref data) if data.position == 180.0
        ));
        assert!(matches!(
            events[1],
            InputEvent::TabletPadStrip(ref data) if data.position == 42.0
        ));
        assert!(matches!(
            events[2],
            InputEvent::TabletPadDial(ref data) if data.delta == 3.0
        ));
        assert_eq!(state.pad.dial_delta, 0.0);
    }

    #[test]
    fn key_classification_skips_contact_markers() {
        let mut state = PendingState::default();
        assert_eq!(classify_key(&state, 0x110), Some(KeyKind::PointerButton));
        assert_eq!(classify_key(&state, 30), Some(KeyKind::Key));
        // BTN_TOOL_PEN is contact metadata, not a client-visible key.
        assert_eq!(classify_key(&state, 0x140), None);

        state.device_kind = DeviceKind::TabletTool;
        assert_eq!(
            classify_key(&state, BTN_STYLUS),
            Some(KeyKind::TabletToolButton)
        );

        state.device_kind = DeviceKind::TabletPad;
        assert_eq!(classify_key(&state, BTN_0), Some(KeyKind::TabletPadButton));

        track_key(&mut state, 0x110, true, KeyKind::PointerButton);
        assert!(state.keys_down.contains_key(&0x110));
        track_key(&mut state, 0x110, false, KeyKind::PointerButton);
        assert!(!state.keys_down.contains_key(&0x110));
    }

    #[test]
    fn high_resolution_scroll_presence_selects_representation() {
        // Legacy only: one wheel click is 120 v120 units.
        assert_eq!(scroll_v120(1.0, 0.0, false), 120.0);
        // High-resolution present, including a zero accumulated value.
        assert_eq!(scroll_v120(1.0, 0.0, true), 0.0);
        assert_eq!(scroll_v120(1.0, 240.0, true), 240.0);
    }

    #[test]
    fn switch_state_seeding_suppresses_duplicate() {
        let mut state = PendingState::default();
        let switch = SwitchKind::Lid;
        state.switch_states.insert(switch, true);
        queue_switch(&mut state, switch, true);
        assert!(collect(&mut state).is_empty(), "same state is suppressed");
        queue_switch(&mut state, switch, false);
        let events = collect(&mut state);
        assert!(matches!(
            events[0],
            InputEvent::SwitchToggle { enabled: false, .. }
        ));
    }
}

fn flush_single_touch(
    device_id: DeviceId,
    dev_idx: DevIndex,
    time_us: u64,
    state: &mut PendingState,
    callback: &mut dyn FnMut(DeviceId, InputEvent),
) {
    if !state.single_touch_enabled {
        return;
    }

    let touch = &mut state.single_touch;
    let have_position = touch.have_x && touch.have_y;
    let pending_active = touch.pending_active.take();

    match pending_active {
        Some(true) if !touch.active => {
            if have_position {
                touch.active = true;
                touch.dirty = false;
                callback(
                    device_id,
                    InputEvent::TouchDown(Box::new(TouchData {
                        device_index: dev_idx,
                        time_us,
                        slot: 0,
                        x: touch.x,
                        y: touch.y,
                        pressure: touch.pressure,
                        touch_major: touch.touch_major,
                        touch_minor: touch.touch_minor,
                        tool_type: touch.tool_type,
                        tracking_id: None,
                    })),
                );
            } else {
                touch.pending_active = Some(true);
            }
        }
        Some(false) if touch.active => {
            touch.active = false;
            touch.dirty = false;
            callback(
                device_id,
                InputEvent::TouchUp(Box::new(TouchEndData {
                    device_index: dev_idx,
                    time_us,
                    slot: 0,
                })),
            );
            touch.clear_contact_metadata();
        }
        Some(false) => {
            touch.active = false;
            touch.dirty = false;
            touch.clear_contact_metadata();
        }
        _ if touch.active && touch.dirty && have_position => {
            touch.dirty = false;
            callback(
                device_id,
                InputEvent::TouchMotion(Box::new(TouchData {
                    device_index: dev_idx,
                    time_us,
                    slot: 0,
                    x: touch.x,
                    y: touch.y,
                    pressure: touch.pressure,
                    touch_major: touch.touch_major,
                    touch_minor: touch.touch_minor,
                    tool_type: touch.tool_type,
                    tracking_id: None,
                })),
            );
        }
        _ => {
            touch.dirty = false;
        }
    }
}
