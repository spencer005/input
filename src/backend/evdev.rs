use {
    super::{AxisInfo, Backend, BusType, DeviceCaps, DeviceInfo},
    crate::{
        events::{
            DevIndex, InputEvent, ScrollSource, SwitchKind, TouchData, TouchEndData, TouchToolType,
        },
        state::DeviceId,
    },
    evdev::{AbsoluteAxisCode, Device, KeyCode, RelativeAxisCode, SynchronizationCode},
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
const BTN_MISC_START: u32 = 0x100;
const BTN_CODE_END: u32 = 0x15f;
const BTN_LEFT: u32 = 0x110;
const BTN_TASK: u32 = 0x117;

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

#[derive(Default)]
struct PendingState {
    dx: f64,
    dy: f64,
    dirty: bool,

    // Scroll state
    dw: f64,
    dw_hi_res: f64,
    dh: f64,
    dh_hi_res: f64,
    scroll_dirty: bool,

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

fn is_pointer_button_code(code: u32) -> bool {
    (BTN_LEFT..=BTN_TASK).contains(&code)
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
        let has_abs_xy = device.supported_absolute_axes().is_some_and(|axes| {
            axes.contains(AbsoluteAxisCode::ABS_X) && axes.contains(AbsoluteAxisCode::ABS_Y)
        });
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

            self.devices.insert(id, device);
            self.states.insert(
                id,
                PendingState {
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

    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
        let mut epoll_events = epoll::EventVec::with_capacity(32);

        epoll::wait(&self.epoll_fd, &mut epoll_events, 0).map_err(|e| io::Error::from(e))?;

        let mut dead_devices = Vec::new();
        for event in &epoll_events {
            let id_u64 = event.data.u64();
            let device_id = DeviceId::from(slotmap::KeyData::from_ffi(id_u64));

            if let Some(device) = self.devices.get_mut(&device_id) {
                let state = self.states.entry(device_id).or_default();

                match device.fetch_events() {
                    Ok(ev_iter) => {
                        for ev in ev_iter {
                            process_evdev_event(device_id, &ev, state, callback);
                        }
                    }
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
            if code == RelativeAxisCode::REL_X.0 {
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
                state.scroll_dirty = true;
            } else if code == RelativeAxisCode::REL_HWHEEL.0 {
                state.dh += ev.value() as f64;
                state.scroll_dirty = true;
            } else if code == RelativeAxisCode::REL_HWHEEL_HI_RES.0 {
                state.dh_hi_res += ev.value() as f64;
                state.scroll_dirty = true;
            }
        }
        evdev::EventType::ABSOLUTE => {
            let code = ev.code();
            if code == 47 {
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
            }
        }
        evdev::EventType::KEY => {
            // BTN_LEFT=0x110 through BTN_TASK=0x117 are pointer/touchpad buttons.
            // BTN_TOOL_* and BTN_TOUCH live later in the button range and are
            // contact metadata, not client-visible pointer buttons.
            // Keyboard keys use different codes (KEY_ESC=1, KEY_A=30, etc.)
            // so this range cleanly separates mouse buttons from keyboard keys.
            let code = ev.code() as u32;
            if state.single_touch_enabled && code == KeyCode::BTN_TOUCH.0 as u32 {
                state.single_touch.pending_active = Some(ev.value() != 0);
            } else if is_pointer_button_code(code) {
                callback(
                    device_id,
                    InputEvent::PointerButton {
                        device_index: dev_idx,
                        time_us,
                        button: code,
                        pressed: ev.value() != 0,
                    },
                );
            } else if is_button_code(code) {
                // Other button-range codes describe tools/contacts/pads here.
                // They are handled by ABS_MT state or future tablet plumbing.
            } else {
                callback(
                    device_id,
                    InputEvent::Key {
                        device_index: dev_idx,
                        time_us,
                        key: code,
                        pressed: ev.value() != 0,
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
                    let final_dw = if state.dw_hi_res != 0.0 {
                        state.dw_hi_res
                    } else {
                        state.dw * 120.0
                    };
                    let final_dh = if state.dh_hi_res != 0.0 {
                        state.dh_hi_res
                    } else {
                        state.dh * 120.0
                    };

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
