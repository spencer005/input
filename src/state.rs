use {
    crate::{
        backend::{Backend, DeviceCaps, DeviceInfo, DeviceInfoRef},
        events::{DevIndex, InputEvent, TouchEndData},
        processing::{
            FilterChain, FilterKind,
            config::TouchpadConfig,
            debounce::DebounceConfig,
            mouse::{MouseAccelProfile, MouseFilter},
            touchpad::TouchpadFilter,
            touchpad_motion::TouchpadMotionFilter,
        },
    },
    bitvec::prelude::*,
    slotmap::{SlotMap, new_key_type},
    std::{collections::VecDeque, os::unix::io::OwnedFd, time::SystemTime},
};

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
    pub keys_pressed: BitArray<[u64; 12], Lsb0>,
    pub touches_active: BitArray<[u64; 1], Lsb0>,
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
    pub fn set_natural_scroll(&mut self, _enabled: bool) {}
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
            supported_switches: &dev.info.supported_switches,
            switch_states: &dev.info.switch_states,
            seat: &seat.name,
        })
    }

    pub fn device(&mut self, id: DeviceId) -> Option<DeviceHandle<'_>> {
        if self.devices.contains_key(id) {
            Some(DeviceHandle {
                _id: id,
                _ctx: self,
            })
        } else {
            None
        }
    }

    pub fn devices(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.devices.keys()
    }

    /// Look up a DeviceId by its compact DevIndex.
    pub fn device_id_by_index(&self, index: DevIndex) -> Option<DeviceId> {
        self.devices
            .iter()
            .find(|(_, d)| d.index == index)
            .map(|(id, _)| id)
    }

    /// Look up the compact DevIndex for a DeviceId.
    pub fn device_index_by_id(&self, id: DeviceId) -> Option<DevIndex> {
        self.devices.get(id).map(|d| d.index)
    }

    /// Remove a device by compact DevIndex. This is used by higher layers that
    /// receive monitor-driven removal notifications before the evdev backend
    /// observes the same fd disappearing.
    pub fn remove_device_by_index(&mut self, index: DevIndex) -> bool {
        let Some(id) = self.device_id_by_index(index) else {
            return false;
        };
        self.remove_device(id)
    }

    /// Remove a device from the input-rs context, release tracked state, reset
    /// stateful filters, and emit one DeviceRemoved notification.
    pub fn remove_device(&mut self, id: DeviceId) -> bool {
        let Some(mut dev) = self.take_device_record(id) else {
            return false;
        };

        let _ = self.backend.remove_device(id);

        let device_index = dev.index;
        let cleanup = release_tracked_device_state(&mut dev, now_us());
        self.event_queue.extend(cleanup);
        self.event_queue
            .push_back(InputEvent::DeviceRemoved { device_index });
        true
    }

    fn take_device_record(&mut self, id: DeviceId) -> Option<Device> {
        let dev = self.devices.remove(id)?;
        if let Some(seat) = self.seats.get_mut(dev.seat) {
            seat.devices.retain(|device| *device != id);
        }
        Some(dev)
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
            keys_pressed: BitArray::ZERO,
            touches_active: BitArray::ZERO,
            disabled: false,
        });

        // Now we have the real DeviceId. Pass it along with the FD.
        // The backend opens the device, probes info, and adds to epoll.
        let info = self
            .backend
            .add_device_fd(dev_id, fd, name)
            .unwrap_or_default();
        if let Some(dev) = self.devices.get_mut(dev_id) {
            if !info.name.is_empty() {
                dev.name.clone_from(&info.name);
            }
            dev.info = info;
        }

        if let Some(seat) = self.seats.get_mut(seat_id) {
            seat.devices.push(dev_id);
        }

        self.event_queue.push_back(InputEvent::DeviceAdded {
            device_index: dev_index,
        });

        dev_id
    }

    pub fn dispatch(&mut self) -> std::io::Result<()> {
        let mut removed_devices = Vec::new();

        let result = {
            let devices = &mut self.devices;
            let seats = &mut self.seats;
            let queue = &mut self.event_queue;

            let mut last_id: Option<DeviceId> = None;
            let mut last_dev_ptr: *mut Device = std::ptr::null_mut();

            self.backend.dispatch(&mut |id, event| {
                let backend_device_removed = matches!(event, InputEvent::DeviceRemoved { .. });
                let dev = if Some(id) == last_id {
                    unsafe { &mut *last_dev_ptr }
                } else if let Some(d) = devices.get_mut(id) {
                    last_id = Some(id);
                    last_dev_ptr = d as *mut Device;
                    d
                } else {
                    return;
                };

                // If device is disabled, silently drop normal events. Removal
                // still has to clear the context record.
                if dev.disabled && !backend_device_removed {
                    return;
                }

                // Remap device_id in event to our compact index
                let device_index = dev.index;

                if let InputEvent::Key {
                    time_us,
                    pressed: true,
                    ..
                } = &event
                {
                    if let Some(seat) = seats.get_mut(dev.seat) {
                        seat.last_key_time = *time_us;
                    }
                }

                let seat_ref = seats.get(dev.seat);
                let info = dev.info.clone();

                if backend_device_removed {
                    let cleanup = release_tracked_device_state(dev, now_us());
                    queue.extend(cleanup);
                }

                // Remap the event's device_id to device_index
                let remapped = remap_event_device(event, device_index);
                let filtered_events = dev.filters.process(remapped, seat_ref, Some(info));
                for filtered in filtered_events {
                    match &filtered {
                        InputEvent::Key { key, pressed, .. } => {
                            let k = *key as usize;
                            if k < 768 {
                                dev.keys_pressed.set(k, *pressed);
                            }
                        }
                        InputEvent::PointerButton {
                            button, pressed, ..
                        } => {
                            let k = *button as usize;
                            if k < 768 {
                                dev.buttons_pressed.set(k, *pressed);
                            }
                        }
                        InputEvent::TouchDown(data) => {
                            if data.slot >= 0 && data.slot < 64 {
                                dev.touches_active.set(data.slot as usize, true);
                            }
                        }
                        InputEvent::TouchUp(data) | InputEvent::TouchCancel(data) => {
                            if data.slot >= 0 && data.slot < 64 {
                                dev.touches_active.set(data.slot as usize, false);
                            }
                        }
                        _ => {}
                    }
                    queue.push_back(filtered);
                }

                if backend_device_removed {
                    removed_devices.push(id);
                }
            })
        };

        for id in removed_devices {
            let _ = self.take_device_record(id);
        }

        result
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
                dev.filters
                    .add(FilterKind::Mouse(Box::new(MouseFilter::flat(dpi))));
            }
        }

        if caps.contains(DeviceCaps::TOUCHPAD) {
            if let Some(dev) = self.devices.get_mut(id) {
                dev.filters.add(FilterKind::TouchpadMotion(Box::new(
                    TouchpadMotionFilter::new(TouchpadConfig::default()),
                )));
                dev.filters
                    .add(FilterKind::Touchpad(Box::new(TouchpadFilter::new(
                        130.0, 0.0,
                    ))));
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
        InputEvent::PointerMotion {
            device_index: _,
            time_us,
            dx,
            dy,
            dx_unaccelerated,
            dy_unaccelerated,
        } => InputEvent::PointerMotion {
            device_index,
            time_us,
            dx,
            dy,
            dx_unaccelerated,
            dy_unaccelerated,
        },
        InputEvent::PointerButton {
            device_index: _,
            time_us,
            button,
            pressed,
        } => InputEvent::PointerButton {
            device_index,
            time_us,
            button,
            pressed,
        },
        InputEvent::Key {
            device_index: _,
            time_us,
            key,
            pressed,
        } => InputEvent::Key {
            device_index,
            time_us,
            key,
            pressed,
        },
        InputEvent::PointerScroll {
            device_index: _,
            time_us,
            source,
            dx,
            dy,
        } => InputEvent::PointerScroll {
            device_index,
            time_us,
            source,
            dx,
            dy,
        },
        InputEvent::SwitchToggle {
            device_index: _,
            time_us,
            switch,
            enabled,
        } => InputEvent::SwitchToggle {
            device_index,
            time_us,
            switch,
            enabled,
        },
        InputEvent::Frame {
            device_index: _,
            time_us,
        } => InputEvent::Frame {
            device_index,
            time_us,
        },
        InputEvent::DeviceAdded { device_index: _ } => InputEvent::DeviceAdded { device_index },
        InputEvent::DeviceRemoved { device_index: _ } => InputEvent::DeviceRemoved { device_index },
        InputEvent::TouchDown(mut d) => {
            d.device_index = device_index;
            InputEvent::TouchDown(d)
        }
        InputEvent::TouchUp(mut d) => {
            d.device_index = device_index;
            InputEvent::TouchUp(d)
        }
        InputEvent::TouchMotion(mut d) => {
            d.device_index = device_index;
            InputEvent::TouchMotion(d)
        }
        InputEvent::TouchCancel(mut d) => {
            d.device_index = device_index;
            InputEvent::TouchCancel(d)
        }
        InputEvent::GestureSwipeBegin(mut d) => {
            d.device_index = device_index;
            InputEvent::GestureSwipeBegin(d)
        }
        InputEvent::GestureSwipeUpdate(mut d) => {
            d.device_index = device_index;
            InputEvent::GestureSwipeUpdate(d)
        }
        InputEvent::GestureSwipeEnd(mut d) => {
            d.device_index = device_index;
            InputEvent::GestureSwipeEnd(d)
        }
        InputEvent::GesturePinchBegin(mut d) => {
            d.device_index = device_index;
            InputEvent::GesturePinchBegin(d)
        }
        InputEvent::GesturePinchUpdate(mut d) => {
            d.device_index = device_index;
            InputEvent::GesturePinchUpdate(d)
        }
        InputEvent::GesturePinchEnd(mut d) => {
            d.device_index = device_index;
            InputEvent::GesturePinchEnd(d)
        }
        InputEvent::GestureHoldBegin(mut d) => {
            d.device_index = device_index;
            InputEvent::GestureHoldBegin(d)
        }
        InputEvent::GestureHoldEnd(mut d) => {
            d.device_index = device_index;
            InputEvent::GestureHoldEnd(d)
        }
        InputEvent::TabletToolProximity(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletToolProximity(d)
        }
        InputEvent::TabletToolAxis(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletToolAxis(d)
        }
        InputEvent::TabletToolTip(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletToolTip(d)
        }
        InputEvent::TabletToolButton(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletToolButton(d)
        }
        InputEvent::TabletPadButton(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletPadButton(d)
        }
        InputEvent::TabletPadRing(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletPadRing(d)
        }
        InputEvent::TabletPadStrip(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletPadStrip(d)
        }
        InputEvent::TabletPadDial(mut d) => {
            d.device_index = device_index;
            InputEvent::TabletPadDial(d)
        }
    }
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn release_tracked_device_state(dev: &mut Device, time_us: u64) -> Vec<InputEvent> {
    let mut cleanup = Vec::new();
    let device_index = dev.index;

    for key in 0..768 {
        if dev.keys_pressed[key] {
            cleanup.push(InputEvent::Key {
                device_index,
                time_us,
                key: key as u32,
                pressed: false,
            });
            dev.keys_pressed.set(key, false);
        }
    }

    for button in 0..768 {
        if dev.buttons_pressed[button] {
            cleanup.push(InputEvent::PointerButton {
                device_index,
                time_us,
                button: button as u32,
                pressed: false,
            });
            dev.buttons_pressed.set(button, false);
        }
    }

    for slot in 0..64 {
        if dev.touches_active[slot] {
            cleanup.push(InputEvent::TouchCancel(Box::new(TouchEndData {
                device_index,
                time_us,
                slot: slot as i32,
            })));
            dev.touches_active.set(slot, false);
        }
    }

    dev.filters.reset_runtime_state();
    cleanup
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            backend::{Backend, DeviceInfo},
            events::InputEvent,
        },
    };

    struct MockBackend {
        events: Vec<(DeviceId, InputEvent)>,
    }

    impl Backend for MockBackend {
        fn initialize(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn dispatch(
            &mut self,
            callback: &mut dyn FnMut(DeviceId, InputEvent),
        ) -> std::io::Result<()> {
            for (id, ev) in self.events.drain(..) {
                callback(id, ev);
            }
            Ok(())
        }
        fn add_device_fd(
            &mut self,
            _id: DeviceId,
            _fd: std::os::unix::io::OwnedFd,
            _name: &str,
        ) -> std::io::Result<DeviceInfo> {
            Ok(DeviceInfo::default())
        }
        fn get_fd(&self) -> std::os::unix::io::RawFd {
            -1
        }
    }

    #[test]
    fn test_architecture() {
        let mut ctx = Context::new(Box::new(MockBackend { events: vec![] }));
        let seat_id = ctx.create_seat("seat0");

        let dev_id = ctx.add_device_fd(
            "Mouse0",
            seat_id,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::DeviceAdded { .. })
        ));

        let dev_index = ctx.devices.get(dev_id).unwrap().index;
        let backend = MockBackend {
            events: vec![(
                dev_id,
                InputEvent::PointerMotion {
                    device_index: DevIndex(0),
                    time_us: 100,
                    dx: 10.0,
                    dy: 5.0,
                    dx_unaccelerated: 10.0,
                    dy_unaccelerated: 5.0,
                },
            )],
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

    /// Enable or disable tap-and-drag for a touchpad device.
    pub fn set_tap_drag(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_tap_drag(enabled);
        }
    }

    /// Enable or disable tap drag lock for a touchpad device.
    pub fn set_tap_drag_lock(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_tap_drag_lock(enabled);
        }
    }

    /// Select a touchpad click method.
    pub fn set_click_method(
        &mut self,
        id: DeviceId,
        method: crate::processing::config::TouchpadClickMethod,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_click_method(method);
        }
    }

    /// Select a touchpad scroll method.
    pub fn set_scroll_method(
        &mut self,
        id: DeviceId,
        method: crate::processing::config::TouchpadScrollMethod,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_scroll_method(method);
        }
    }

    /// Enable, disable, or replace button scrolling for a pointer device.
    pub fn set_button_scroll(
        &mut self,
        id: DeviceId,
        enabled: bool,
        button: u32,
        lock_enabled: bool,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_button_scroll(enabled, button, lock_enabled);
        }
    }

    /// Set an absolute-device calibration matrix.
    pub fn set_calibration_matrix(
        &mut self,
        id: DeviceId,
        matrix: crate::processing::absolute_transform::CalibrationMatrix,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_calibration_matrix(matrix);
        }
    }

    /// Set left-handed mode for a device.
    pub fn set_left_handed(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_left_handed(enabled);
        }
    }

    /// Enable or disable middle-button emulation for a pointer device.
    pub fn set_middle_button_emulation(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_middle_button_emulation(enabled);
        }
    }

    /// Set debounce configuration.
    pub fn set_debounce(&mut self, id: DeviceId, config: DebounceConfig) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_debounce(config);
        }
    }

    /// Enable or disable a device. Disabling first releases tracked keys/buttons
    /// and cancels tracked touches, then future events are silently dropped.
    /// This implements libinput's "send-events" disabled mode.
    pub fn set_device_enabled(&mut self, id: DeviceId, enabled: bool) {
        let mut cleanup = Vec::new();
        if let Some(dev) = self.devices.get_mut(id) {
            if !enabled && !dev.disabled {
                cleanup = release_tracked_device_state(dev, now_us());
            }
            dev.disabled = !enabled;
        }
        self.event_queue.extend(cleanup);
    }

    /// Returns whether a device is enabled (not disabled).
    pub fn is_device_enabled(&self, id: DeviceId) -> bool {
        self.devices
            .get(id)
            .map(|dev| !dev.disabled)
            .unwrap_or(true)
    }
}
