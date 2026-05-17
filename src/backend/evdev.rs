use super::{Backend, DeviceInfo, AxisInfo, BusType, DeviceCaps};
use crate::events::{InputEvent, DevIndex, TouchData, TouchEndData};
use crate::state::DeviceId;
use std::io;
use std::os::unix::io::{OwnedFd, AsRawFd, RawFd};
use evdev::{Device, AbsoluteAxisCode, RelativeAxisCode};
use rustix::event::epoll;
use std::collections::HashMap;
use std::time::SystemTime;
use slotmap::Key;

const MAX_SLOTS: usize = 16;

#[derive(Default, Clone, Copy)]
struct Slot {
    active: bool,
    x: f64,
    y: f64,
    dirty: bool,
    changed: bool,
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
}

pub struct EvdevBackend {
    devices: HashMap<DeviceId, Device>,
    states: HashMap<DeviceId, PendingState>,
    epoll_fd: OwnedFd,
}

impl EvdevBackend {
    pub fn new() -> io::Result<Self> {
        let epoll_fd = epoll::create(epoll::CreateFlags::CLOEXEC)
            .map_err(|e| io::Error::from(e))?;

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

        // Use the caller-provided name; the evdev name ioctl may differ from
        // the udev/sysfs name that jay already knows.
        info.name = name.to_string();
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
                evdev::EventType::KEY => caps.insert(DeviceCaps::KEYBOARD),
                evdev::EventType::RELATIVE => caps.insert(DeviceCaps::POINTER),
                _ => {}
            }
        }
        // A touchpad has both ABS and KEY, a touchscreen only has ABS
        let has_abs_mt = device.get_absinfo()
            .is_ok_and(|mut iter| iter.any(|(code, _)| {
                matches!(code, AbsoluteAxisCode::ABS_MT_POSITION_X | AbsoluteAxisCode::ABS_MT_POSITION_Y)
            }));
        if has_abs_mt {
            if caps.contains(DeviceCaps::KEYBOARD) {
                caps.insert(DeviceCaps::TOUCHPAD);
            } else {
                caps.insert(DeviceCaps::TOUCHSCREEN);
            }
        }
        info.capabilities = caps;

        if let Ok(absinfo_iter) = device.get_absinfo() {
            for (code, abs) in absinfo_iter {
                match code {
                    AbsoluteAxisCode::ABS_X | AbsoluteAxisCode::ABS_MT_POSITION_X => {
                        if info.x.max == 0.0 {
                            info.x = AxisInfo {
                                min: abs.minimum() as f64,
                                max: abs.maximum() as f64,
                                resolution: abs.resolution() as f64,
                            };
                        }
                    }
                    AbsoluteAxisCode::ABS_Y | AbsoluteAxisCode::ABS_MT_POSITION_Y => {
                        if info.y.max == 0.0 {
                            info.y = AxisInfo {
                                min: abs.minimum() as f64,
                                max: abs.maximum() as f64,
                                resolution: abs.resolution() as f64,
                            };
                        }
                    }
                    _ => {}
                }
            }
        }

        if !id.is_null() {
            let id_u64 = id.data().as_ffi();
            epoll::add(
                &self.epoll_fd,
                &device,
                epoll::EventData::new_u64(id_u64),
                epoll::EventFlags::IN,
            ).map_err(|e| io::Error::from(e))?;

            self.devices.insert(id, device);
            self.states.insert(id, PendingState::default());
        }

        Ok(info)
    }

    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
        let mut epoll_events = epoll::EventVec::with_capacity(32);

        epoll::wait(&self.epoll_fd, &mut epoll_events, 0)
            .map_err(|e| io::Error::from(e))?;

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
            self.states.remove(&device_id);
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
    let time_us = ev.timestamp()
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
            if code == 47 { // ABS_MT_SLOT
                let slot = ev.value() as usize;
                if slot < MAX_SLOTS {
                    state.current_slot = slot;
                }
            } else if code == 57 { // ABS_MT_TRACKING_ID
                let slot = &mut state.slots[state.current_slot];
                let id = ev.value();
                if id == -1 {
                    if slot.active {
                        slot.active = false;
                        slot.changed = true;
                    }
                } else {
                    if !slot.active {
                        slot.active = true;
                        slot.changed = true;
                    }
                }
            } else if code == 53 { // ABS_MT_POSITION_X
                state.slots[state.current_slot].x = ev.value() as f64;
                state.slots[state.current_slot].dirty = true;
            } else if code == 54 { // ABS_MT_POSITION_Y
                state.slots[state.current_slot].y = ev.value() as f64;
                state.slots[state.current_slot].dirty = true;
            }
        }
        evdev::EventType::KEY => {
            // BTN_LEFT=0x110 through BTN_TASK=0x14f are pointer/touchpad buttons.
            // Keyboard keys use different codes (KEY_ESC=1, KEY_A=30, etc.)
            // so this range cleanly separates mouse buttons from keyboard keys.
            let code = ev.code() as u32;
            if code >= 0x110 && code <= 0x14f {
                callback(device_id, InputEvent::PointerButton {
                    device_index: dev_idx,
                    time_us,
                    button: code,
                    pressed: ev.value() != 0,
                });
            } else {
                callback(device_id, InputEvent::Key {
                    device_index: dev_idx,
                    time_us,
                    key: code,
                    pressed: ev.value() != 0,
                });
            }
        }
        evdev::EventType::SYNCHRONIZATION => {
            if ev.code() == 0 { // SYN_REPORT
                if state.dirty {
                    callback(device_id, InputEvent::PointerMotion {
                        device_index: dev_idx,
                        time_us,
                        dx: state.dx,
                        dy: state.dy,
                        dx_unaccelerated: state.dx,
                        dy_unaccelerated: state.dy,
                    });
                    state.dx = 0.0;
                    state.dy = 0.0;
                    state.dirty = false;
                }

                if state.scroll_dirty {
                    let final_dw = if state.dw_hi_res != 0.0 { state.dw_hi_res } else { state.dw * 120.0 };
                    let final_dh = if state.dh_hi_res != 0.0 { state.dh_hi_res } else { state.dh * 120.0 };

                    if final_dw != 0.0 || final_dh != 0.0 {
                        callback(device_id, InputEvent::PointerScroll {
                            device_index: dev_idx,
                            time_us,
                            dx: final_dh / 120.0,
                            // evdev REL_WHEEL positive is wheel-up; Wayland scroll
                            // convention is positive for down, so invert here.
                            dy: -final_dw / 120.0,
                        });
                    }
                    state.dw = 0.0; state.dw_hi_res = 0.0;
                    state.dh = 0.0; state.dh_hi_res = 0.0;
                    state.scroll_dirty = false;
                }

                for (i, slot) in state.slots.iter_mut().enumerate() {
                    if slot.changed {
                        if slot.active {
                            callback(device_id, InputEvent::TouchDown(Box::new(TouchData {
                                device_index: dev_idx,
                                time_us,
                                slot: i as i32,
                                x: slot.x,
                                y: slot.y,
                            })));
                        } else {
                            callback(device_id, InputEvent::TouchUp(Box::new(TouchEndData {
                                device_index: dev_idx,
                                time_us,
                                slot: i as i32,
                            })));
                        }
                        slot.changed = false;
                    } else if slot.active && slot.dirty {
                        callback(device_id, InputEvent::TouchMotion(Box::new(TouchData {
                            device_index: dev_idx,
                            time_us,
                            slot: i as i32,
                            x: slot.x,
                            y: slot.y,
                        })));
                    }
                    slot.dirty = false;
                }

                callback(device_id, InputEvent::Frame { device_index: dev_idx, time_us });
            }
        }
        _ => {}
    }
}
