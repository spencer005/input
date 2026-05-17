use slotmap::{SlotMap, new_key_type};
use bitvec::prelude::*;
use std::collections::VecDeque;
use std::os::unix::io::OwnedFd;
use crate::backend::{Backend, DeviceInfo, DeviceInfoRef, DeviceCaps};
use crate::processing::{FilterKind, FilterChain};
use crate::processing::mouse::{MouseFilter, MouseAccelProfile};
use crate::processing::debounce::DebounceConfig;
use crate::processing::touchpad::TouchpadFilter;
use crate::processing::touchpad_motion::TouchpadMotionFilter;
use crate::processing::config::TouchpadConfig;
use crate::events::{InputEvent, DevIndex};

new_key_type! {
    pub struct DeviceId;
    pub struct SeatId;
    pub struct GroupId;
}

pub struct Device {
    pub name: String,
    pub seat: SeatId,
    pub group: Option<GroupId>,
    pub filters: FilterChain,
    pub info: DeviceInfo,
    /// Compact index used in events for cache efficiency.
    pub index: DevIndex,
    pub buttons_pressed: BitArray<[u64; 12], Lsb0>,
    /// When true, all events from this device are silently dropped.
    /// This implements libinput's "send-events" disabled mode.
    pub disabled: bool,
}


pub struct Seat {
    pub name: String,
    pub devices: Vec<DeviceId>,
    pub last_key_time: u64,
}

pub struct Context {
    pub devices: SlotMap<DeviceId, Device>,
    pub seats: SlotMap<SeatId, Seat>,
    pub groups: SlotMap<GroupId, DeviceGroup>,
    pub backend: Box<dyn Backend>,
    event_queue: VecDeque<InputEvent>,
    next_device_index: u32,
}

pub struct DeviceGroup {
    pub name: String,
    pub devices: Vec<DeviceId>,
}

 pub struct DeviceHandle<'a> {
     _id: DeviceId,
     _ctx: &'a mut Context,
 }

impl<'a> DeviceHandle<'a> {
    pub fn set_natural_scroll(&mut self, _enabled: bool) {
    }
}

impl Context {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            devices: SlotMap::with_key(),
            seats: SlotMap::with_key(),
            groups: SlotMap::with_key(),
            backend,
            event_queue: VecDeque::with_capacity(64),
            next_device_index: 0,
        }
    }

    fn alloc_device_index(&mut self) -> DevIndex {
        let idx = self.next_device_index;
        self.next_device_index += 1;
        DevIndex(idx)
    }

    pub fn device_info(&self, id: DeviceId) -> Option<DeviceInfoRef<'_>> {
        let dev = self.devices.get(id)?;
        let seat = self.seats.get(dev.seat)?;
        Some(DeviceInfoRef {
            name: &dev.name,
            vendor_id: dev.info.vendor_id,
            product_id: dev.info.product_id,
            bus: dev.info.bus,
            capabilities: dev.info.capabilities,
            seat: &seat.name,
        })
    }

    pub fn device(&mut self, id: DeviceId) -> Option<DeviceHandle<'_>> {
        if self.devices.contains_key(id) {
            Some(DeviceHandle { _id: id, _ctx: self })
        } else {
            None
        }
    }

    pub fn devices(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.devices.keys()
    }

    /// Look up a DeviceId by its compact DevIndex.
    pub fn device_id_by_index(&self, index: DevIndex) -> Option<DeviceId> {
        self.devices.iter()
            .find(|(_, d)| d.index == index)
            .map(|(id, _)| id)
    }

    /// Look up the compact DevIndex for a DeviceId.
    pub fn device_index_by_id(&self, id: DeviceId) -> Option<DevIndex> {
        self.devices.get(id).map(|d| d.index)
    }
    pub fn create_seat(&mut self, name: &str) -> SeatId {
        self.seats.insert(Seat {
            name: name.to_string(),
            devices: Vec::new(),
            last_key_time: 0,
        })
    }

    /// Add a device using an FD obtained from logind (jay handles discovery).
    /// The backend creates the evdev Device, probes info, and registers
    /// for epoll — all in a single call with the real DeviceId.
    pub fn add_device_fd(&mut self, name: &str, seat_id: SeatId, fd: OwnedFd) -> DeviceId {
        let dev_index = self.alloc_device_index();

        // We need to register the device with a real DeviceId, but we don't
        // have one yet (it's allocated by SlotMap::insert). We'll insert a
        // placeholder first, then pass the real id + fd to the backend.
        let dev_id = self.devices.insert(Device {
            name: name.to_string(),
            seat: seat_id,
            group: None,
            filters: FilterChain::new(),
            info: DeviceInfo::default(),
            index: dev_index,
            buttons_pressed: BitArray::ZERO,
            disabled: false,
        });

        // Now we have the real DeviceId. Pass it along with the FD.
        // The backend opens the device, probes info, and adds to epoll.
        let info = self.backend.add_device_fd(dev_id, fd, name).unwrap_or_default();
        self.devices.get_mut(dev_id).unwrap().info = info;

        if let Some(seat) = self.seats.get_mut(seat_id) {
            seat.devices.push(dev_id);
        }

        self.event_queue.push_back(InputEvent::DeviceAdded { device_index: dev_index });

        dev_id
    }

    pub fn dispatch(&mut self) -> std::io::Result<()> {
        let devices = &mut self.devices;
        let seats = &mut self.seats;
        let queue = &mut self.event_queue;

        let mut last_id: Option<DeviceId> = None;
        let mut last_dev_ptr: *mut Device = std::ptr::null_mut();

        self.backend.dispatch(&mut |id, event| {
            let dev = if Some(id) == last_id {
                unsafe { &mut *last_dev_ptr }
            } else {
                if let Some(d) = devices.get_mut(id) {
                    last_id = Some(id);
                    last_dev_ptr = d as *mut Device;
                    d
                } else {
                    return;
                }
            };

            // If device is disabled, silently drop all events
            if dev.disabled {
                return;
            }

            // Remap device_id in event to our compact index
            let device_index = dev.index;

            if let InputEvent::Key { time_us, pressed: true, .. } = &event {
                if let Some(seat) = seats.get_mut(dev.seat) {
                    seat.last_key_time = *time_us;
                }
            }

            let seat_ref = seats.get(dev.seat);
            let info = dev.info.clone();

            // Remap the event's device_id to device_index
            let remapped = remap_event_device(event, device_index);
            let filtered_events = dev.filters.process(remapped, seat_ref, Some(info));
            for filtered in filtered_events {
                match &filtered {
                    InputEvent::Key { key, pressed, .. } | InputEvent::PointerButton { button: key, pressed, .. } => {
                        let k = *key as usize;
                        if k < 768 {
                            dev.buttons_pressed.set(k, *pressed);
                        }
                    }
                    _ => {}
                }
                queue.push_back(filtered);
            }
        })
    }

    pub fn next_event(&mut self) -> Option<InputEvent> {

        self.event_queue.pop_front()
    }

    /// Get the epoll file descriptor for polling.
    /// Returns -1 if no backend is available.
    pub fn fd(&self) -> std::os::unix::io::RawFd {
        self.backend.get_fd()
    }

    /// Initialize the backend. Call this after construction.
    pub fn initialize(&mut self) -> std::io::Result<()> {
        self.backend.initialize()
    }

    /// Auto-configure filters for a device based on its capabilities.
    ///
    /// Mouse:   flat profile by default (no acceleration), optional hysteresis.
    /// Touchpad: adaptive acceleration + motion filter + tap-to-click + debounce.
    /// Keyboard: debounce (disabled by default — most keyboards self-debounce).
    pub fn configure_device_filters(&mut self, id: DeviceId) {
        let caps = match self.devices.get(id) {
            Some(d) => d.info.capabilities,
            None => return,
        };

        if caps.contains(DeviceCaps::POINTER) && !caps.contains(DeviceCaps::TOUCHPAD) {
            // Plain mouse — flat profile, DPI normalization only
            if let Some(dev) = self.devices.get_mut(id) {
                let dpi = 1000; // Default; caller can reconfigure
                dev.filters.add(FilterKind::Mouse(Box::new(MouseFilter::flat(dpi))));
            }
        }

        if caps.contains(DeviceCaps::TOUCHPAD) {
            if let Some(dev) = self.devices.get_mut(id) {
                dev.filters.add(FilterKind::TouchpadMotion(Box::new(
                    TouchpadMotionFilter::new(TouchpadConfig::default())
                )));
                dev.filters.add(FilterKind::Touchpad(Box::new(
                    TouchpadFilter::new(130.0, 0.0)
                )));
            }
        }

        // Debounce is disabled by default for all devices (zero-latency passthrough).
        // Enable explicitly for devices known to bounce:
        //   ButtonDebounceFilter::new(DebounceConfig { enabled: true, .. })
    }
}

/// Remap the raw DeviceId in an event from the backend to our compact DevIndex.
fn remap_event_device(event: InputEvent, device_index: DevIndex) -> InputEvent {
    match event {
            InputEvent::PointerMotion { device_index: _, time_us, dx, dy, dx_unaccelerated, dy_unaccelerated } =>
            InputEvent::PointerMotion { device_index, time_us, dx, dy, dx_unaccelerated, dy_unaccelerated },
        InputEvent::PointerButton { device_index: _, time_us, button, pressed } =>
            InputEvent::PointerButton { device_index, time_us, button, pressed },
        InputEvent::Key { device_index: _, time_us, key, pressed } =>
            InputEvent::Key { device_index, time_us, key, pressed },
        InputEvent::PointerScroll { device_index: _, time_us, dx, dy } =>
            InputEvent::PointerScroll { device_index, time_us, dx, dy },
        InputEvent::Frame { device_index: _, time_us } =>
            InputEvent::Frame { device_index, time_us },
        InputEvent::DeviceAdded { device_index: _ } =>
            InputEvent::DeviceAdded { device_index },
        InputEvent::DeviceRemoved { device_index: _ } =>
            InputEvent::DeviceRemoved { device_index },
        InputEvent::TouchDown(mut d) => { d.device_index = device_index; InputEvent::TouchDown(d) }
        InputEvent::TouchUp(mut d) => { d.device_index = device_index; InputEvent::TouchUp(d) }
        InputEvent::TouchMotion(mut d) => { d.device_index = device_index; InputEvent::TouchMotion(d) }
        InputEvent::GestureSwipeBegin(mut d) => { d.device_index = device_index; InputEvent::GestureSwipeBegin(d) }
        InputEvent::GestureSwipeUpdate(mut d) => { d.device_index = device_index; InputEvent::GestureSwipeUpdate(d) }
        InputEvent::GestureSwipeEnd(mut d) => { d.device_index = device_index; InputEvent::GestureSwipeEnd(d) }
        InputEvent::GesturePinchBegin(mut d) => { d.device_index = device_index; InputEvent::GesturePinchBegin(d) }
        InputEvent::GesturePinchUpdate(mut d) => { d.device_index = device_index; InputEvent::GesturePinchUpdate(d) }
        InputEvent::GesturePinchEnd(mut d) => { d.device_index = device_index; InputEvent::GesturePinchEnd(d) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, DeviceInfo};
    use crate::events::InputEvent;

    struct MockBackend {
        events: Vec<(DeviceId, InputEvent)>,
    }

    impl Backend for MockBackend {
        fn initialize(&mut self) -> std::io::Result<()> { Ok(()) }
        fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> std::io::Result<()> {
            for (id, ev) in self.events.drain(..) {
                callback(id, ev);
            }
            Ok(())
        }
        fn add_device_fd(&mut self, _id: DeviceId, _fd: std::os::unix::io::OwnedFd, _name: &str) -> std::io::Result<DeviceInfo> {
            Ok(DeviceInfo::default())
        }
        fn get_fd(&self) -> std::os::unix::io::RawFd { -1 }
    }

    #[test]
    fn test_architecture() {
        let mut ctx = Context::new(Box::new(MockBackend { events: vec![] }));
        let seat_id = ctx.create_seat("seat0");

        let dev_id = ctx.add_device_fd("Mouse0", seat_id, std::fs::File::open("/dev/null").unwrap().into());
        assert!(matches!(ctx.next_event(), Some(InputEvent::DeviceAdded { .. })));

        let dev_index = ctx.devices.get(dev_id).unwrap().index;
        let backend = MockBackend {
            events: vec![
                (dev_id, InputEvent::PointerMotion { device_index: DevIndex(0), time_us: 100, dx: 10.0, dy: 5.0, dx_unaccelerated: 10.0, dy_unaccelerated: 5.0 })
            ],
        };

        ctx.backend = Box::new(backend);
        ctx.dispatch().unwrap();

        match ctx.next_event() {
            Some(InputEvent::PointerMotion { dx, dy, .. }) => {
                assert_eq!(dx, 10.0);
                assert_eq!(dy, 5.0);
            }
            _ => panic!("Expected PointerMotion"),
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime configuration methods
// ---------------------------------------------------------------------------
// These delegate to FilterChain methods which handle variant dispatch
// and in-place filter replacement internally.
// ---------------------------------------------------------------------------

impl Context {
    /// Change the DPI normalization for a mouse device.
    pub fn set_mouse_dpi(&mut self, id: DeviceId, dpi: i32) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_mouse_dpi(dpi);
        }
    }

    /// Change the acceleration profile for a mouse device.
    pub fn set_mouse_profile(&mut self, id: DeviceId, profile: MouseAccelProfile) {
        if let Some(dev) = self.devices.get_mut(id) {
            tracing::debug!("set_mouse_profile: id={:?} profile={:?}", id, profile);
            dev.filters.set_mouse_profile(profile);
        } else {
            tracing::warn!("set_mouse_profile: device id {:?} not found", id);
        }
    }

    /// Change the speed adjustment for a mouse device.
    /// Speed is in [-1.0, 1.0]; 0.0 is the default.
    pub fn set_mouse_speed_adjustment(&mut self, id: DeviceId, speed: f64) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_mouse_speed_adjustment(speed);
            tracing::debug!("set_mouse_speed_adjustment: id={:?} speed={}", id, speed);
        } else {
            tracing::warn!("set_mouse_speed_adjustment: device id {:?} not found", id);
        }
    }

    /// Enable or disable natural scrolling for a touchpad device.
    pub fn set_natural_scroll(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_natural_scroll(enabled);
        }
    }

    /// Enable or disable natural scrolling for a mouse/pointer device.
    /// This adds/removes a NaturalScrollFilter that inverts scroll deltas.
    pub fn set_natural_scroll_mouse(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_natural_scroll_mouse(enabled);
        }
    }

    /// Enable or disable tap-to-click for a touchpad device.
    pub fn set_tap_to_click(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_tap_to_click(enabled);
        }
    }

    /// Enable or disable two-finger tap right-click for a touchpad device.
    pub fn set_two_finger_tap_right_click(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_two_finger_tap_right_click(enabled);
        }
    }

    /// Set left-handed mode for a device.
    pub fn set_left_handed(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_left_handed(enabled);
        }
    }

    /// Set debounce configuration.
    pub fn set_debounce(&mut self, id: DeviceId, config: DebounceConfig) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_debounce(config);
        }
    }

    /// Enable or disable a device. When disabled, all events are silently dropped.
    /// This implements libinput's "send-events" disabled mode.
    pub fn set_device_enabled(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.disabled = !enabled;
        }
    }

    /// Returns whether a device is enabled (not disabled).
    pub fn is_device_enabled(&self, id: DeviceId) -> bool {
        self.devices.get(id).map(|dev| !dev.disabled).unwrap_or(true)
    }
}
