use {
    crate::{
        backend::{Backend, DeviceCaps, DeviceInfo, DeviceInfoRef},
        events::{DevIndex, InputEvent, SwitchKind, TouchEndData},
        policy::DisableReasons,
        processing::{
            FilterChain, FilterKind, FilterResult,
            config::TouchpadConfig,
            debounce::DebounceConfig,
            mouse::{MouseAccelProfile, MouseConfig, MouseFilter},
            touchpad::TouchpadFilter,
            touchpad_motion::TouchpadMotionFilter,
        },
        profile::{DeviceProfile, ProfileMatch, match_profile},
    },
    bitvec::prelude::*,
    slotmap::{Key, SlotMap, new_key_type},
    std::{
        collections::{HashMap, VecDeque},
        os::unix::io::OwnedFd,
        time::SystemTime,
    },
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
    /// This implements libinput's "send-events" disabled mode and is the
    /// composition of the individual reasons below.
    pub disabled: bool,
    /// Individual disable reasons. Keeping these separate means one policy
    /// cannot accidentally re-enable a device that another policy disabled.
    pub disable_reasons: DisableReasons,
    /// Name of the explicitly selected or automatically matched profile.
    pub profile: Option<String>,
}

impl Device {
    /// Recompute the effective `disabled` flag from the individual reasons.
    pub fn refresh_disabled(&mut self) {
        self.disabled = self.disable_reasons.any();
    }

    fn set_disable_reason(&mut self, reason: DisableReason, disabled: bool) -> bool {
        let was_disabled = self.disabled;
        match reason {
            DisableReason::User => self.disable_reasons.user = disabled,
            DisableReason::Switch => self.disable_reasons.switch = disabled,
            DisableReason::ExternalMouse => self.disable_reasons.external_mouse = disabled,
            DisableReason::Temporary => self.disable_reasons.temporary = disabled,
        }
        self.refresh_disabled();
        !was_disabled && self.disabled
    }
}

/// Reason a device is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisableReason {
    User,
    Switch,
    ExternalMouse,
    Temporary,
}

/// Policy applied to internal devices while a tablet-mode switch is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchPolicy {
    /// Disable internal touchpads in tablet mode.
    pub disable_internal_touchpads: bool,
    /// Disable internal pointing devices (mice/trackpoints) in tablet mode.
    pub disable_internal_pointers: bool,
    /// Disable internal keyboards in tablet mode. Off by default because some
    /// convertible keyboards remain usable in tablet mode.
    pub disable_internal_keyboards: bool,
    /// Disable internal tablet devices in tablet mode.
    pub disable_internal_tablets: bool,
}

impl Default for SwitchPolicy {
    fn default() -> Self {
        Self {
            disable_internal_touchpads: true,
            disable_internal_pointers: true,
            disable_internal_keyboards: false,
            disable_internal_tablets: false,
        }
    }
}

#[derive(Default)]
pub struct Seat {
    pub name: String,
    pub devices: Vec<DeviceId>,
    /// Timestamp of the last internal-keyboard key press, used by DWT.
    pub last_key_time: u64,
    /// Timestamp of the last grouped-trackpoint motion, used by DWTP.
    pub last_trackpoint_time: u64,
    /// Whether a stylus is currently in proximity on this seat.
    pub stylus_in_proximity: bool,
}

pub struct Context {
    pub devices: SlotMap<DeviceId, Device>,
    pub seats: SlotMap<SeatId, Seat>,
    pub groups: SlotMap<GroupId, DeviceGroup>,
    pub backend: Box<dyn Backend>,
    event_queue: VecDeque<InputEvent>,
    next_device_index: u32,
    /// Switch-driven policy for internal devices.
    pub switch_policy: SwitchPolicy,
    /// Last tablet-mode state reported by each switch device.
    tablet_mode_sources: HashMap<DeviceId, bool>,
    /// When true (default), only internal keyboards update the seat DWT
    /// timestamp. Set to false to preserve the older behavior where any
    /// keyboard suppresses the touchpad.
    pub dwt_internal_only: bool,
    /// Explicit, user-visible device profiles. Later profiles with the same
    /// name replace earlier ones.
    pub profiles: Vec<DeviceProfile>,
    /// When true, internal touchpads are disabled while an external pointer is
    /// present. Disabled by default, matching libinput's opt-in setting.
    pub disable_internal_on_external_mouse: bool,
    /// Whether an external (non-internal) pointer is currently present.
    pub external_mouse_present: bool,
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
            switch_policy: SwitchPolicy::default(),
            tablet_mode_sources: HashMap::new(),
            dwt_internal_only: true,
            profiles: Vec::new(),
            disable_internal_on_external_mouse: false,
            external_mouse_present: false,
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
            is_clickpad: dev.info.is_clickpad,
            internal: dev.info.internal,
            led_state: dev.info.led_state,
            x: dev.info.x,
            y: dev.info.y,
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
        let was_switch = dev.info.capabilities.contains(DeviceCaps::SWITCH);
        let seat = dev.seat;
        let cleanup = release_tracked_device_state(&mut dev, now_us());
        self.event_queue.extend(cleanup);
        self.event_queue
            .push_back(InputEvent::DeviceRemoved { device_index });

        if was_switch {
            self.tablet_mode_sources.remove(&id);
            self.apply_tablet_mode_policy(seat);
        }
        self.refresh_external_mouse_policy();
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
            last_trackpoint_time: 0,
            stylus_in_proximity: false,
        })
    }

    /// Add a device using an FD obtained from logind (jay handles discovery).
    /// The backend creates the evdev Device, probes info, and registers
    /// for epoll — all in a single call with the real DeviceId.
    ///
    /// Prefer [`Context::try_add_device_fd`] when the caller can handle a
    /// backend registration failure. This compatibility wrapper logs the error
    /// and returns `DeviceId::null()`, without adding a phantom device.
    pub fn add_device_fd(&mut self, name: &str, seat_id: SeatId, fd: OwnedFd) -> DeviceId {
        match self.try_add_device_fd(name, seat_id, fd) {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(device = name, %error, "input-rs: device add failed");
                DeviceId::null()
            }
        }
    }

    /// Transactionally add a device, returning the backend registration error
    /// without leaving a placeholder device or emitting `DeviceAdded`.
    pub fn try_add_device_fd(
        &mut self,
        name: &str,
        seat_id: SeatId,
        fd: OwnedFd,
    ) -> std::io::Result<DeviceId> {
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
            disable_reasons: DisableReasons::default(),
            profile: None,
        });

        // Now we have the real DeviceId. Pass it along with the FD.
        // The backend opens the device, probes info, and adds to epoll.
        let info = match self.backend.add_device_fd(dev_id, fd, name) {
            Ok(info) => info,
            Err(error) => {
                self.devices.remove(dev_id);
                return Err(error);
            }
        };
        if let Some(dev) = self.devices.get_mut(dev_id) {
            if !info.name.is_empty() {
                dev.name.clone_from(&info.name);
            }
            dev.info = info;
        }

        self.assign_device_profile(dev_id);

        if let Some(seat) = self.seats.get_mut(seat_id) {
            seat.devices.push(dev_id);
        }

        self.event_queue.push_back(InputEvent::DeviceAdded {
            device_index: dev_index,
        });

        self.seed_initial_switch_state(dev_id);
        self.apply_switch_policy_to_device(dev_id);
        self.refresh_external_mouse_policy();
        Ok(dev_id)
    }

    pub fn dispatch(&mut self) -> std::io::Result<()> {
        let mut removed_devices = Vec::new();
        let mut switch_updates = Vec::new();
        let mut removed_switch_sources = Vec::new();
        let dwt_internal_only = self.dwt_internal_only;

        let result = {
            let devices = &mut self.devices;
            let seats = &mut self.seats;
            let queue = &mut self.event_queue;

            let mut last_id: Option<DeviceId> = None;
            let mut last_dev_ptr: *mut Device = std::ptr::null_mut();
            let mut filtered_events = FilterResult::new();

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

                if backend_device_removed && dev.info.capabilities.contains(DeviceCaps::SWITCH) {
                    removed_switch_sources.push((id, dev.seat));
                }

                if let InputEvent::SwitchToggle {
                    switch: SwitchKind::TabletMode,
                    enabled,
                    ..
                } = &event
                {
                    switch_updates.push((id, *enabled));
                }

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
                    let counts_for_dwt = !dwt_internal_only || dev.info.internal;
                    if counts_for_dwt {
                        if let Some(seat) = seats.get_mut(dev.seat) {
                            seat.last_key_time = *time_us;
                        }
                    }
                }

                if let InputEvent::PointerMotion { time_us, .. } = &event {
                    if dev.info.capabilities.contains(DeviceCaps::POINTING_STICK) {
                        if let Some(seat) = seats.get_mut(dev.seat) {
                            seat.last_trackpoint_time = *time_us;
                        }
                    }
                }

                if let InputEvent::TabletToolProximity(data) = &event {
                    if let Some(seat) = seats.get_mut(dev.seat) {
                        seat.stylus_in_proximity = data.in_range;
                    }
                }

                let seat_ref = seats.get(dev.seat);

                if backend_device_removed {
                    let cleanup = release_tracked_device_state(dev, now_us());
                    queue.extend(cleanup);
                }

                // Remap the event's device_id to device_index
                let remapped = remap_event_device(event, device_index);
                dev.filters
                    .process_into(remapped, seat_ref, Some(&dev.info), &mut filtered_events);
                for filtered in filtered_events.drain(..) {
                    record_filtered_event(dev, &filtered);
                    queue.push_back(filtered);
                }

                if backend_device_removed {
                    removed_devices.push(id);
                }
            })
        };

        let had_removals = !removed_devices.is_empty();
        for id in removed_devices {
            let tablet_seat = self.devices.get(id).and_then(|dev| {
                dev.info
                    .capabilities
                    .contains(DeviceCaps::TABLET)
                    .then_some(dev.seat)
            });
            if let Some(seat_id) = tablet_seat {
                if let Some(seat) = self.seats.get_mut(seat_id) {
                    seat.stylus_in_proximity = false;
                }
            }
            let _ = self.take_device_record(id);
        }

        // External-mouse presence can only change when a device is removed;
        // the add path refreshes it separately.
        if had_removals {
            self.refresh_external_mouse_policy();
        }

        for (id, enabled) in switch_updates {
            self.update_tablet_mode_source(id, enabled);
        }
        for (id, seat) in removed_switch_sources {
            // Clearing only the removed source's contribution mirrors the
            // documented policy: other switches in the same seat keep their
            // state, and policy falls back to inactive when none remain.
            self.tablet_mode_sources.remove(&id);
            self.apply_tablet_mode_policy(seat);
        }

        result
    }

    /// Add or replace a user-visible device profile. Profiles are matched on
    /// the next device add; explicit selection applies to an existing device.
    pub fn add_device_profile(&mut self, profile: DeviceProfile) {
        self.profiles.retain(|p| p.name != profile.name);
        self.profiles.push(profile);
    }

    /// Remove a device profile by name. Returns whether it existed.
    pub fn remove_device_profile(&mut self, name: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|profile| profile.name != name);
        self.profiles.len() != before
    }

    /// The profile name currently assigned to a device.
    pub fn device_profile_name(&self, id: DeviceId) -> Option<&str> {
        self.devices.get(id)?.profile.as_deref()
    }

    /// Explicitly select a profile for an existing device. Explicit selection
    /// ignores the profile's match keys, as documented. Returns false if the
    /// device or profile does not exist.
    pub fn select_device_profile(&mut self, id: DeviceId, name: &str) -> bool {
        let Some(profile) = self.profiles.iter().find(|p| p.name == name).cloned() else {
            return false;
        };
        let Some(dev) = self.devices.get_mut(id) else {
            return false;
        };

        if let Some(internal) = profile.internal {
            dev.info.internal = internal;
        }
        dev.profile = Some(name.to_string());
        dev.filters.apply_touchpad_profile(&profile.touchpad);
        if let Some(dpi) = profile.pointer.dpi {
            dev.filters.set_mouse_dpi(dpi);
        }
        if let Some(speed) = profile.pointer.speed_adjustment {
            dev.filters.set_mouse_speed_adjustment(speed);
        }
        if let Some(adaptive) = profile.pointer.adaptive {
            dev.filters.set_mouse_profile(if adaptive {
                MouseAccelProfile::Adaptive
            } else {
                MouseAccelProfile::Flat
            });
        }
        if let Some(angle) = profile.pointer.rotation_degrees {
            let _ = dev.filters.set_rotation(angle);
        }
        if let Some(natural) = profile.pointer.natural_scrolling {
            dev.filters.set_natural_scroll_mouse(natural);
        }
        if let Some(x) = profile.pointer.scroll_scale_x {
            let y = profile.pointer.scroll_scale_y.unwrap_or(1.0);
            dev.filters.set_scroll_scale(x, y);
        } else if let Some(y) = profile.pointer.scroll_scale_y {
            dev.filters.set_scroll_scale(1.0, y);
        }
        if let Some(map) = profile.pointer.button_map.clone() {
            dev.filters.set_button_map(map);
        }

        // An internal/external override can change grouped policy.
        self.apply_switch_policy_to_device(id);
        self.refresh_external_mouse_policy();
        true
    }

    /// Assign the best automatic profile after a device was added.
    fn assign_device_profile(&mut self, id: DeviceId) {
        let Some(info) = self.devices.get(id).map(|dev| dev.info.clone()) else {
            return;
        };

        let matched = match match_profile(self.profiles.iter(), &info) {
            ProfileMatch::Matched(name) => {
                let internal = self
                    .profiles
                    .iter()
                    .find(|profile| profile.name == name)
                    .and_then(|profile| profile.internal);
                (Some(name), internal)
            }
            ProfileMatch::Ambiguous(names) => {
                tracing::warn!(
                    "device {:?} matched multiple equally specific profiles {:?}; using defaults",
                    id,
                    names
                );
                (None, None)
            }
            ProfileMatch::NoMatch => (None, None),
        };

        if let Some(dev) = self.devices.get_mut(id) {
            if let Some(internal) = matched.1 {
                dev.info.internal = internal;
            }
            dev.profile = matched.0;
        }
    }

    /// Seed tablet-mode policy from a switch device's initial kernel state.
    fn seed_initial_switch_state(&mut self, id: DeviceId) {
        let Some(dev) = self.devices.get(id) else {
            return;
        };
        let tablet_mode = dev
            .info
            .switch_states
            .iter()
            .find(|(switch, _)| *switch == SwitchKind::TabletMode)
            .map(|(_, active)| *active);
        if let Some(active) = tablet_mode {
            self.tablet_mode_sources.insert(id, active);
        }
    }

    /// Update the tablet-mode contribution of one switch device and reapply
    /// policy for its seat.
    fn update_tablet_mode_source(&mut self, id: DeviceId, enabled: bool) {
        let Some(seat) = self.devices.get(id).map(|dev| dev.seat) else {
            return;
        };
        self.tablet_mode_sources.insert(id, enabled);
        self.apply_tablet_mode_policy(seat);
    }

    fn tablet_mode_active(&self, seat: SeatId) -> bool {
        self.tablet_mode_sources.iter().any(|(id, active)| {
            *active && self.devices.get(*id).is_some_and(|dev| dev.seat == seat)
        })
    }

    /// Apply the current tablet-mode state to all internal devices in `seat`.
    ///
    /// Only the switch disable reason is touched here; a device the user
    /// disabled, or one disabled because an external mouse is present, stays
    /// disabled when tablet mode ends.
    fn apply_tablet_mode_policy(&mut self, seat: SeatId) {
        let active = self.tablet_mode_active(seat);
        let policy = self.switch_policy;
        let Context {
            devices,
            event_queue,
            ..
        } = self;
        let time_us = now_us();

        for (_, dev) in devices.iter_mut() {
            if dev.seat != seat || !dev.info.internal {
                continue;
            }

            let caps = dev.info.capabilities;
            let should_disable = active
                && ((policy.disable_internal_touchpads && caps.contains(DeviceCaps::TOUCHPAD))
                    || (policy.disable_internal_pointers
                        && caps.contains(DeviceCaps::POINTER)
                        && !caps.contains(DeviceCaps::TOUCHPAD))
                    || (policy.disable_internal_keyboards && caps.contains(DeviceCaps::KEYBOARD))
                    || (policy.disable_internal_tablets && caps.contains(DeviceCaps::TABLET)));

            let transitioned = dev.set_disable_reason(DisableReason::Switch, should_disable);
            if transitioned && dev.disabled {
                event_queue.extend(release_tracked_device_state(dev, time_us));
            }
        }
    }

    /// Apply tablet-mode policy to a device that was just added.
    fn apply_switch_policy_to_device(&mut self, id: DeviceId) {
        let Some(seat) = self.devices.get(id).map(|dev| dev.seat) else {
            return;
        };
        self.apply_tablet_mode_policy(seat);
    }

    /// Change the switch-driven internal-device policy and reapply it to every
    /// existing seat.
    pub fn set_switch_policy(&mut self, policy: SwitchPolicy) {
        self.switch_policy = policy;
        let seats: Vec<SeatId> = self.seats.keys().collect();
        for seat in seats {
            self.apply_tablet_mode_policy(seat);
        }
    }

    /// Test-only and diagnostic helper: whether tablet mode is currently
    /// active on a seat.
    pub fn is_tablet_mode_active(&self, seat: SeatId) -> bool {
        self.tablet_mode_active(seat)
    }

    /// Enable or disable the "disable internal touchpad while an external
    /// mouse is present" policy. Off by default.
    pub fn set_disable_internal_on_external_mouse(&mut self, enabled: bool) {
        self.disable_internal_on_external_mouse = enabled;
        self.refresh_external_mouse_policy();
    }

    /// Recompute external-mouse presence and apply it as a disable reason.
    /// This is separate from user and switch reasons so ending either policy
    /// cannot re-enable a device still disabled by the other.
    fn refresh_external_mouse_policy(&mut self) {
        let present = self.devices.values().any(|dev| {
            !dev.info.internal
                && dev.info.capabilities.contains(DeviceCaps::POINTER)
                && !dev.info.capabilities.contains(DeviceCaps::TOUCHPAD)
        });
        self.external_mouse_present = present;

        let enable_reason = self.disable_internal_on_external_mouse && present;
        let Context {
            devices,
            event_queue,
            ..
        } = self;
        let time_us = now_us();
        for (_, dev) in devices.iter_mut() {
            if !dev.info.internal || !dev.info.capabilities.contains(DeviceCaps::TOUCHPAD) {
                continue;
            }
            let transitioned = dev.set_disable_reason(DisableReason::ExternalMouse, enable_reason);
            if transitioned && dev.disabled {
                event_queue.extend(release_tracked_device_state(dev, time_us));
            }
        }
    }

    /// Run delayed filter deadlines (middle-button emulation, drag lock) for
    /// all enabled devices and queue any resulting events.
    ///
    /// The compositor should call this from its event-loop timer at
    /// [`Context::next_timeout_us`]. Timeout handling does not require
    /// unrelated input to arrive.
    pub fn dispatch_timeouts(&mut self, now_us: u64) {
        let Context {
            devices,
            seats,
            event_queue,
            ..
        } = self;
        let mut out = Vec::new();

        for (_, dev) in devices.iter_mut() {
            if dev.disabled {
                continue;
            }
            let seat = seats.get(dev.seat);
            let events = dev.filters.dispatch_timeouts(now_us, seat, Some(&dev.info));
            for event in events {
                record_filtered_event(dev, &event);
                out.push(event);
            }
        }
        event_queue.extend(out);
    }

    /// Earliest absolute microsecond deadline among all enabled devices.
    pub fn next_timeout_us(&self) -> Option<u64> {
        self.devices
            .values()
            .filter(|dev| !dev.disabled)
            .filter_map(|dev| dev.filters.next_timeout_us())
            .min()
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

        let profile = self
            .devices
            .get(id)
            .and_then(|dev| dev.profile.as_deref())
            .and_then(|name| self.profiles.iter().find(|profile| profile.name == name))
            .cloned();

        if caps.contains(DeviceCaps::POINTER) && !caps.contains(DeviceCaps::TOUCHPAD) {
            // Plain mouse — profile defaults, flat acceleration unless the
            // profile selects adaptive.
            if let Some(dev) = self.devices.get_mut(id) {
                let pointer = profile
                    .as_ref()
                    .map(|profile| profile.pointer.clone())
                    .unwrap_or_default();
                let config = MouseConfig {
                    dpi: pointer.dpi.unwrap_or(1000),
                    speed_adjustment: pointer.speed_adjustment.unwrap_or(0.0),
                    profile: if pointer.adaptive == Some(true) {
                        MouseAccelProfile::Adaptive
                    } else {
                        MouseAccelProfile::Flat
                    },
                };
                dev.filters
                    .add(FilterKind::Mouse(Box::new(MouseFilter::with_config(
                        config,
                    ))));

                if let Some(angle) = pointer.rotation_degrees {
                    let _ = dev.filters.set_rotation(angle);
                }
                if let Some(natural) = pointer.natural_scrolling {
                    dev.filters.set_natural_scroll_mouse(natural);
                }
                if let Some(x) = pointer.scroll_scale_x {
                    let y = pointer.scroll_scale_y.unwrap_or(1.0);
                    dev.filters.set_scroll_scale(x, y);
                } else if let Some(y) = pointer.scroll_scale_y {
                    dev.filters.set_scroll_scale(1.0, y);
                }
                if let Some(map) = pointer.button_map.clone() {
                    dev.filters.set_button_map(map);
                }
            }
        }

        if caps.contains(DeviceCaps::TOUCHPAD) {
            if let Some(dev) = self.devices.get_mut(id) {
                let mut config = TouchpadConfig::default();
                if let Some(profile) = &profile {
                    profile.touchpad.apply(&mut config);
                }
                dev.filters.add(FilterKind::TouchpadMotion(Box::new(
                    TouchpadMotionFilter::new(config),
                )));
                dev.filters
                    .add(FilterKind::Touchpad(Box::new(TouchpadFilter::new(
                        130.0, 0.0,
                    ))));
                if let Some(profile) = &profile {
                    let pointer = &profile.pointer;
                    if let Some(x) = pointer.scroll_scale_x {
                        let y = pointer.scroll_scale_y.unwrap_or(1.0);
                        dev.filters.set_scroll_scale(x, y);
                    } else if let Some(y) = pointer.scroll_scale_y {
                        dev.filters.set_scroll_scale(1.0, y);
                    }
                }
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

/// Maintain per-device tracked key/button/touch state for any event that
/// leaves a filter chain, including timeout-generated releases.
fn record_filtered_event(dev: &mut Device, event: &InputEvent) {
    match event {
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
            backend::{Backend, DeviceCaps, DeviceInfo},
            events::{InputEvent, SwitchKind},
        },
    };

    #[derive(Default)]
    struct MockBackend {
        events: Vec<(DeviceId, InputEvent)>,
        infos: HashMap<String, DeviceInfo>,
        fail_add: bool,
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
            name: &str,
        ) -> std::io::Result<DeviceInfo> {
            if self.fail_add {
                return Err(std::io::Error::other("mock add failure"));
            }
            Ok(self.infos.get(name).cloned().unwrap_or_default())
        }
        fn get_fd(&self) -> std::os::unix::io::RawFd {
            -1
        }
    }

    #[test]
    fn test_architecture() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
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
            ..Default::default()
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

    fn add_mouse_device(ctx: &mut Context) -> DeviceId {
        let seat = ctx.create_seat("seat0");
        let mut backend = MockBackend::default();
        backend.infos.insert(
            "mouse".into(),
            DeviceInfo {
                capabilities: DeviceCaps::POINTER,
                ..Default::default()
            },
        );
        ctx.backend = Box::new(backend);
        let id = ctx.add_device_fd(
            "mouse",
            seat,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        ctx.configure_device_filters(id);
        while ctx.next_event().is_some() {}
        id
    }

    #[test]
    fn left_handed_toggle_remaps_a_held_button() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let id = add_mouse_device(&mut ctx);

        send(
            &mut ctx,
            id,
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 1,
                button: 0x110,
                pressed: true,
            },
        );
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x110,
                pressed: true,
                ..
            })
        ));

        ctx.set_left_handed(id, true);
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x110,
                pressed: false,
                ..
            })
        ));
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x111,
                pressed: true,
                ..
            })
        ));

        // Physical release is mapped by the newly installed button swap.
        send(
            &mut ctx,
            id,
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 2,
                button: 0x110,
                pressed: false,
            },
        );
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x111,
                pressed: false,
                ..
            })
        ));

        let dev = ctx.devices.get(id).unwrap();
        assert!(!dev.buttons_pressed[0x110]);
        assert!(!dev.buttons_pressed[0x111]);
    }

    #[test]
    fn enabling_debounce_while_button_held_preserves_the_release() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let id = add_mouse_device(&mut ctx);

        send(
            &mut ctx,
            id,
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 1,
                button: 0x110,
                pressed: true,
            },
        );
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton { pressed: true, .. })
        ));

        ctx.set_debounce(
            id,
            DebounceConfig {
                enabled: true,
                bounce_timeout_us: 25_000,
                spurious_timeout_us: 12_000,
            },
        );

        send(
            &mut ctx,
            id,
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 2,
                button: 0x110,
                pressed: false,
            },
        );
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x110,
                pressed: false,
                ..
            })
        ));
    }

    #[test]
    fn disabling_middle_emulation_flushes_pending_press() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let id = add_mouse_device(&mut ctx);
        ctx.set_middle_button_emulation(id, true);

        send(
            &mut ctx,
            id,
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 1,
                button: 0x110,
                pressed: true,
            },
        );
        assert!(
            ctx.next_event().is_none(),
            "press is held for chord detection"
        );

        ctx.set_middle_button_emulation(id, false);
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x110,
                pressed: true,
                ..
            })
        ));

        send(
            &mut ctx,
            id,
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 2,
                button: 0x110,
                pressed: false,
            },
        );
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x110,
                pressed: false,
                ..
            })
        ));
    }

    #[test]
    fn disabling_middle_emulation_releases_synthesized_middle() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let id = add_mouse_device(&mut ctx);
        ctx.set_middle_button_emulation(id, true);

        for (time_us, button) in [(1, 0x110), (2, 0x111)] {
            send(
                &mut ctx,
                id,
                InputEvent::PointerButton {
                    device_index: DevIndex(0),
                    time_us,
                    button,
                    pressed: true,
                },
            );
        }
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x112,
                pressed: true,
                ..
            })
        ));

        ctx.set_middle_button_emulation(id, false);
        assert!(matches!(
            ctx.next_event(),
            Some(InputEvent::PointerButton {
                button: 0x112,
                pressed: false,
                ..
            })
        ));

        // The physical chord releases were never exposed as left/right
        // presses, so they must be swallowed rather than emitted unmatched.
        for (time_us, button) in [(3, 0x110), (4, 0x111)] {
            send(
                &mut ctx,
                id,
                InputEvent::PointerButton {
                    device_index: DevIndex(0),
                    time_us,
                    button,
                    pressed: false,
                },
            );
            assert!(ctx.next_event().is_none());
        }
    }

    #[test]
    fn failed_device_add_is_transactional() {
        let mut ctx = Context::new(Box::new(MockBackend {
            fail_add: true,
            ..Default::default()
        }));
        let seat = ctx.create_seat("seat0");
        let error = ctx
            .try_add_device_fd(
                "broken",
                seat,
                std::fs::File::open("/dev/null").unwrap().into(),
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "mock add failure");
        assert_eq!(ctx.devices.len(), 0);
        assert!(ctx.next_event().is_none());
        assert!(ctx.seats.get(seat).unwrap().devices.is_empty());
    }

    fn internal_touchpad() -> DeviceInfo {
        DeviceInfo {
            capabilities: DeviceCaps::TOUCHPAD,
            internal: true,
            ..Default::default()
        }
    }

    fn switch_info(tablet_mode: Option<bool>) -> DeviceInfo {
        DeviceInfo {
            capabilities: DeviceCaps::SWITCH,
            supported_switches: vec![SwitchKind::TabletMode],
            switch_states: tablet_mode
                .map(|active| vec![(SwitchKind::TabletMode, active)])
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn add_switch_and_touchpad(
        ctx: &mut Context,
        tablet_mode: Option<bool>,
    ) -> (DeviceId, DeviceId) {
        let seat = ctx.create_seat("seat0");
        let mut backend = MockBackend::default();
        backend
            .infos
            .insert("switch".into(), switch_info(tablet_mode));
        backend.infos.insert("touchpad".into(), internal_touchpad());
        ctx.backend = Box::new(backend);
        let switch = ctx.add_device_fd(
            "switch",
            seat,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        let touchpad = ctx.add_device_fd(
            "touchpad",
            seat,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        while ctx.next_event().is_some() {}
        (switch, touchpad)
    }

    fn send(ctx: &mut Context, id: DeviceId, event: InputEvent) {
        let mut backend = MockBackend::default();
        backend.events.push((id, event));
        ctx.backend = Box::new(backend);
        ctx.dispatch().unwrap();
    }

    #[test]
    fn tablet_mode_disables_and_restores_internal_touchpad() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let (switch, touchpad) = add_switch_and_touchpad(&mut ctx, None);

        send(
            &mut ctx,
            switch,
            InputEvent::SwitchToggle {
                device_index: DevIndex(0),
                time_us: 10,
                switch: SwitchKind::TabletMode,
                enabled: true,
            },
        );
        assert!(ctx.devices.get(touchpad).unwrap().disabled);
        assert!(ctx.devices.get(touchpad).unwrap().disable_reasons.switch);

        send(
            &mut ctx,
            switch,
            InputEvent::SwitchToggle {
                device_index: DevIndex(0),
                time_us: 20,
                switch: SwitchKind::TabletMode,
                enabled: false,
            },
        );
        assert!(!ctx.devices.get(touchpad).unwrap().disabled);
        assert!(!ctx.devices.get(touchpad).unwrap().disable_reasons.switch);
    }

    #[test]
    fn user_disabled_device_stays_disabled_across_switch_changes() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let (switch, touchpad) = add_switch_and_touchpad(&mut ctx, None);

        ctx.set_device_enabled(touchpad, false);
        while ctx.next_event().is_some() {}

        for (time_us, enabled) in [(10, true), (20, false)] {
            send(
                &mut ctx,
                switch,
                InputEvent::SwitchToggle {
                    device_index: DevIndex(0),
                    time_us,
                    switch: SwitchKind::TabletMode,
                    enabled,
                },
            );
            let dev = ctx.devices.get(touchpad).unwrap();
            assert!(dev.disabled);
            assert!(dev.disable_reasons.user);
        }
    }

    #[test]
    fn switch_removal_clears_only_its_contribution() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let (switch, touchpad) = add_switch_and_touchpad(&mut ctx, None);

        send(
            &mut ctx,
            switch,
            InputEvent::SwitchToggle {
                device_index: DevIndex(0),
                time_us: 10,
                switch: SwitchKind::TabletMode,
                enabled: true,
            },
        );
        assert!(ctx.devices.get(touchpad).unwrap().disabled);

        send(
            &mut ctx,
            switch,
            InputEvent::DeviceRemoved {
                device_index: DevIndex(0),
            },
        );
        assert!(!ctx.devices.contains_key(switch));
        assert!(!ctx.devices.get(touchpad).unwrap().disabled);
    }

    #[test]
    fn external_mouse_policy_composes_with_other_reasons() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let seat = ctx.create_seat("seat0");
        let mut backend = MockBackend::default();
        backend.infos.insert(
            "touchpad".into(),
            DeviceInfo {
                capabilities: DeviceCaps::TOUCHPAD,
                internal: true,
                ..Default::default()
            },
        );
        backend.infos.insert(
            "mouse".into(),
            DeviceInfo {
                capabilities: DeviceCaps::POINTER,
                internal: false,
                ..Default::default()
            },
        );
        ctx.backend = Box::new(backend);
        let touchpad = ctx.add_device_fd(
            "touchpad",
            seat,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        let mouse = ctx.add_device_fd(
            "mouse",
            seat,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        while ctx.next_event().is_some() {}

        ctx.set_disable_internal_on_external_mouse(true);
        assert!(ctx.devices.get(touchpad).unwrap().disabled);
        assert!(
            ctx.devices
                .get(touchpad)
                .unwrap()
                .disable_reasons
                .external_mouse
        );
        assert!(!ctx.devices.get(mouse).unwrap().disabled);

        // User-disabled stays disabled when the external-mouse reason clears.
        ctx.set_device_enabled(touchpad, false);
        ctx.set_disable_internal_on_external_mouse(false);
        let dev = ctx.devices.get(touchpad).unwrap();
        assert!(dev.disabled);
        assert!(dev.disable_reasons.user);
        assert!(!dev.disable_reasons.external_mouse);
    }

    #[test]
    fn led_state_is_recorded_through_context() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let (_, touchpad) = add_switch_and_touchpad(&mut ctx, None);
        let state = crate::policy::LedState {
            caps_lock: true,
            num_lock: false,
            scroll_lock: true,
        };
        ctx.set_led_state(touchpad, state).unwrap();
        assert_eq!(ctx.led_state(touchpad), Some(state));
    }

    #[test]
    fn device_profile_applies_defaults_on_add() {
        use crate::profile::DeviceProfile;

        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let seat = ctx.create_seat("seat0");

        let profile = DeviceProfile {
            name: "internal-touchpad".into(),
            match_vendor_id: Some(7),
            internal: Some(true),
            touchpad: crate::profile::TouchpadProfile {
                tap_timeout_us: Some(777),
                palm_pressure_threshold: Some(55.0),
                ..Default::default()
            },
            ..Default::default()
        };
        ctx.add_device_profile(profile);

        let mut backend = MockBackend::default();
        backend.infos.insert(
            "touchpad".into(),
            DeviceInfo {
                vendor_id: 7,
                capabilities: DeviceCaps::TOUCHPAD,
                ..Default::default()
            },
        );
        ctx.backend = Box::new(backend);
        let id = ctx.add_device_fd(
            "touchpad",
            seat,
            std::fs::File::open("/dev/null").unwrap().into(),
        );
        ctx.configure_device_filters(id);

        let dev = ctx.devices.get(id).unwrap();
        assert_eq!(dev.profile.as_deref(), Some("internal-touchpad"));
        assert!(dev.info.internal);
        let config = dev.filters.touchpad_config().unwrap();
        assert_eq!(config.tap_timeout_us, 777);
        assert_eq!(config.palm_pressure_threshold, Some(55.0));
    }

    #[test]
    fn explicit_profile_selection_ignores_match_keys() {
        use crate::profile::DeviceProfile;

        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let (_, touchpad) = add_switch_and_touchpad(&mut ctx, None);
        ctx.configure_device_filters(touchpad);

        ctx.add_device_profile(DeviceProfile {
            name: "forced".into(),
            match_vendor_id: Some(999),
            touchpad: crate::profile::TouchpadProfile {
                tap_timeout_us: Some(123),
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(ctx.select_device_profile(touchpad, "forced"));
        assert_eq!(
            ctx.devices
                .get(touchpad)
                .unwrap()
                .filters
                .touchpad_config()
                .unwrap()
                .tap_timeout_us,
            123
        );
    }

    #[test]
    fn initial_tablet_mode_switch_state_seeds_policy() {
        let mut ctx = Context::new(Box::new(MockBackend::default()));
        let (_, touchpad) = add_switch_and_touchpad(&mut ctx, Some(true));

        assert!(ctx.devices.get(touchpad).unwrap().disabled);
        assert!(ctx.devices.get(touchpad).unwrap().disable_reasons.switch);
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

    /// Change the acceleration profile, returning validation errors instead of
    /// logging and keeping the old filter.
    pub fn try_set_mouse_profile(
        &mut self,
        id: DeviceId,
        profile: MouseAccelProfile,
    ) -> Result<(), crate::processing::mouse::MouseConfigError> {
        match self.devices.get_mut(id) {
            Some(dev) => dev.filters.try_set_mouse_profile(profile),
            None => Ok(()),
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

    /// Set or clear arbitrary pointer-button remapping for a device.
    pub fn set_button_map(&mut self, id: DeviceId, map: crate::processing::button_map::ButtonMap) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_button_map(map);
        }
    }

    /// Set or clear tablet pressure-range and active-area transforms.
    pub fn set_tablet_config(
        &mut self,
        id: DeviceId,
        config: crate::processing::tablet::TabletConfig,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_tablet_config(config);
        }
    }

    /// Current tablet transform configuration for a device.
    pub fn tablet_config(&self, id: DeviceId) -> Option<crate::processing::tablet::TabletConfig> {
        self.devices
            .get(id)
            .and_then(|dev| dev.filters.tablet_config())
    }

    /// Owned snapshot of the device's effective filter configuration.
    pub fn config_snapshot(&self, id: DeviceId) -> Option<crate::processing::DeviceConfigSnapshot> {
        self.devices
            .get(id)
            .map(|dev| dev.filters.config_snapshot())
    }

    /// Current button map for a device, if installed.
    pub fn button_map(&self, id: DeviceId) -> Option<&crate::processing::button_map::ButtonMap> {
        self.devices
            .get(id)
            .and_then(|dev| dev.filters.button_map())
    }

    /// Set independent scroll scaling/inversion factors for a device.
    ///
    /// Positive values scale, negative values invert that axis, and zero
    /// suppresses it. Natural scrolling is a separate boolean toggle.
    pub fn set_scroll_scale(&mut self, id: DeviceId, x: f64, y: f64) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_scroll_scale(x, y);
        }
    }

    /// Current scroll scale factors for a device, if installed.
    pub fn scroll_scale(&self, id: DeviceId) -> Option<(f64, f64)> {
        self.devices
            .get(id)
            .and_then(|dev| dev.filters.scroll_scale())
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

    /// Select the two/three-finger tap button map.
    pub fn set_tap_button_map(
        &mut self,
        id: DeviceId,
        map: crate::processing::config::TapButtonMap,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_tap_button_map(map);
        }
    }

    /// Select the clickfinger button map.
    pub fn set_clickfinger_button_map(
        &mut self,
        id: DeviceId,
        map: crate::processing::config::ClickfingerButtonMap,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_clickfinger_button_map(map);
        }
    }

    /// Replace touchpad tap timing and movement thresholds.
    pub fn set_tap_timing(
        &mut self,
        id: DeviceId,
        tap_timeout_us: u64,
        tap_move_threshold_mm: f64,
        tap_drag_timeout_us: u64,
        drag_lock_timeout_us: u64,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_tap_timing(
                tap_timeout_us,
                tap_move_threshold_mm,
                tap_drag_timeout_us,
                drag_lock_timeout_us,
            );
        }
    }

    /// Set the maximum finger count that may produce a tap.
    pub fn set_max_tap_fingers(&mut self, id: DeviceId, fingers: u8) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_max_tap_fingers(fingers);
        }
    }

    /// Override clickpad detection for a touchpad device.
    pub fn set_clickpad_override(&mut self, id: DeviceId, clickpad: Option<bool>) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_clickpad_override(clickpad);
        }
    }

    /// Set raw pressure/size palm thresholds and the thumb pressure threshold.
    pub fn set_touchpad_palm_thresholds(
        &mut self,
        id: DeviceId,
        pressure: Option<f64>,
        size: Option<f64>,
        thumb_pressure: Option<f64>,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters
                .set_touchpad_palm_thresholds(pressure, size, thumb_pressure);
        }
    }

    /// Replace touchpad gesture timing and thresholds.
    pub fn set_touchpad_gesture_config(
        &mut self,
        id: DeviceId,
        hold_timeout_us: u64,
        motion_threshold_mm: f64,
        pinch_scale_threshold: f64,
        pinch_rotation_threshold_rad: f64,
    ) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_touchpad_gesture_config(
                hold_timeout_us,
                motion_threshold_mm,
                pinch_scale_threshold,
                pinch_rotation_threshold_rad,
            );
        }
    }

    /// Set the disable-while-typing suppression timeout.
    pub fn set_touchpad_dwt_timeout(&mut self, id: DeviceId, timeout_us: u64) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_touchpad_dwt_timeout(timeout_us);
        }
    }

    /// Enable or disable disable-while-trackpointing.
    pub fn set_touchpad_dwtp(&mut self, id: DeviceId, enabled: bool, timeout_us: u64) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_touchpad_dwtp(enabled, timeout_us);
        }
    }

    /// Enable or disable stylus-vs-touchpad arbitration.
    pub fn set_touchpad_stylus_arbitration(&mut self, id: DeviceId, enabled: bool) {
        if let Some(dev) = self.devices.get_mut(id) {
            dev.filters.set_touchpad_stylus_arbitration(enabled);
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

    /// Remove an installed absolute-device calibration matrix.
    pub fn reset_calibration_matrix(&mut self, id: DeviceId) -> bool {
        self.devices
            .get_mut(id)
            .map(|dev| dev.filters.clear_calibration_matrix())
            .unwrap_or(false)
    }

    /// Return the installed calibration matrix for a device.
    pub fn calibration_matrix(
        &self,
        id: DeviceId,
    ) -> Option<crate::processing::absolute_transform::CalibrationMatrix> {
        self.devices
            .get(id)
            .and_then(|dev| dev.filters.calibration_matrix())
    }

    /// Set or replace the relative-device rotation angle for a device.
    ///
    /// Invalid angles are rejected without changing the installed filter.
    pub fn set_rotation(
        &mut self,
        id: DeviceId,
        angle_degrees: i32,
    ) -> Result<(), crate::processing::rotation::RotationError> {
        match self.devices.get_mut(id) {
            Some(dev) => dev.filters.set_rotation(angle_degrees),
            None => Ok(()),
        }
    }

    /// Remove relative-device rotation from a device.
    pub fn reset_rotation(&mut self, id: DeviceId) -> bool {
        self.devices
            .get_mut(id)
            .map(|dev| dev.filters.clear_rotation())
            .unwrap_or(false)
    }

    /// Return the configured relative rotation angle for a device.
    pub fn rotation_angle(&self, id: DeviceId) -> Option<i32> {
        self.devices
            .get(id)
            .and_then(|dev| dev.filters.rotation_angle())
    }

    /// Set left-handed mode for a device.
    ///
    /// If a left/right button is currently held, its logical identity is
    /// remapped with a release/press transition. Without this, enabling
    /// left-handed mode mid-hold would leave the old logical button stuck
    /// because the physical release is mapped under the new configuration.
    pub fn set_left_handed(&mut self, id: DeviceId, enabled: bool) {
        const BTN_LEFT: u32 = 0x110;
        const BTN_RIGHT: u32 = 0x111;

        let mut transition = Vec::new();
        if let Some(dev) = self.devices.get_mut(id) {
            let was_left_handed = dev.filters.left_handed();
            if was_left_handed != enabled {
                let time_us = now_us();
                let left = BTN_LEFT as usize;
                let right = BTN_RIGHT as usize;
                let held = [
                    (BTN_LEFT, dev.buttons_pressed[left]),
                    (BTN_RIGHT, dev.buttons_pressed[right]),
                ];

                for (button, pressed) in held {
                    if pressed {
                        transition.push(InputEvent::PointerButton {
                            device_index: dev.index,
                            time_us,
                            button,
                            pressed: false,
                        });
                        dev.buttons_pressed.set(button as usize, false);
                    }
                }
                for (button, pressed) in held {
                    if pressed {
                        let mapped = match button {
                            BTN_LEFT => BTN_RIGHT,
                            BTN_RIGHT => BTN_LEFT,
                            other => other,
                        };
                        transition.push(InputEvent::PointerButton {
                            device_index: dev.index,
                            time_us,
                            button: mapped,
                            pressed: true,
                        });
                        dev.buttons_pressed.set(mapped as usize, true);
                    }
                }
            }

            dev.filters.set_left_handed(enabled);
        }
        self.event_queue.extend(transition);
    }

    /// Enable or disable middle-button emulation for a pointer device.
    ///
    /// Disabling flushes a pending delayed press through the remaining filter
    /// chain and releases an active synthesized middle button, so removing the
    /// filter cannot leave the client with a stuck or unmatched button.
    pub fn set_middle_button_emulation(&mut self, id: DeviceId, enabled: bool) {
        const BTN_MIDDLE: u32 = 0x112;
        let mut out = Vec::new();

        let Context {
            devices,
            seats,
            event_queue,
            ..
        } = self;

        if let Some(dev) = devices.get_mut(id) {
            let was_enabled = dev.filters.middle_button_emulation_enabled();
            if was_enabled == enabled {
                return;
            }

            if was_enabled && !enabled {
                let seat = seats.get(dev.seat);
                let flushed = dev.filters.flush_middle_button(seat, Some(&dev.info));
                for event in flushed {
                    record_filtered_event(dev, &event);
                    out.push(event);
                }

                if dev.buttons_pressed[BTN_MIDDLE as usize] {
                    let time_us = now_us();
                    out.push(InputEvent::PointerButton {
                        device_index: dev.index,
                        time_us,
                        button: BTN_MIDDLE,
                        pressed: false,
                    });
                    dev.buttons_pressed.set(BTN_MIDDLE as usize, false);
                }
            }

            dev.filters.set_middle_button_emulation(enabled);
        }
        event_queue.extend(out);
    }

    /// Set debounce configuration.
    ///
    /// Pending deferred transitions are flushed first, so disabling or
    /// replacing debounce cannot drop a suppressed release.
    pub fn set_debounce(&mut self, id: DeviceId, config: DebounceConfig) {
        let mut out = Vec::new();
        let Context {
            devices,
            seats,
            event_queue,
            ..
        } = self;

        if let Some(dev) = devices.get_mut(id) {
            let seat = seats.get(dev.seat);
            let flushed = dev.filters.flush_debounce(seat, Some(&dev.info));
            for event in flushed {
                record_filtered_event(dev, &event);
                out.push(event);
            }
            dev.filters.set_debounce(config);

            if config.enabled {
                // Seed already-held codes so their later releases are not
                // treated as unknown transitions.
                let pressed_buttons: Vec<u32> = (0..768)
                    .filter(|index| dev.buttons_pressed[*index])
                    .map(|index| index as u32)
                    .collect();
                let pressed_keys: Vec<u32> = (0..768)
                    .filter(|index| dev.keys_pressed[*index])
                    .map(|index| index as u32)
                    .collect();
                for code in pressed_buttons {
                    dev.filters.seed_debounce_state(code, false);
                }
                for code in pressed_keys {
                    dev.filters.seed_debounce_state(code, true);
                }
            }
        }
        event_queue.extend(out);
    }

    /// Enable or disable a device. Disabling first releases tracked keys/buttons
    /// and cancels tracked touches, then future events are silently dropped.
    /// This implements libinput's "send-events" disabled mode.
    pub fn set_device_enabled(&mut self, id: DeviceId, enabled: bool) {
        let mut cleanup = Vec::new();
        if let Some(dev) = self.devices.get_mut(id) {
            let transitioned = dev.set_disable_reason(DisableReason::User, !enabled);
            if transitioned && dev.disabled {
                cleanup = release_tracked_device_state(dev, now_us());
            }
        }
        self.event_queue.extend(cleanup);
    }

    /// Apply or clear a non-user disable reason (for example a policy layer
    /// disabling an internal device because an external mouse is present).
    /// Returns whether the device became disabled.
    pub fn set_device_disable_reason(
        &mut self,
        id: DeviceId,
        reason: DisableReason,
        disabled: bool,
    ) -> bool {
        let mut cleanup = Vec::new();
        let mut now_disabled = false;
        if let Some(dev) = self.devices.get_mut(id) {
            let transitioned = dev.set_disable_reason(reason, disabled);
            now_disabled = dev.disabled;
            if transitioned && dev.disabled {
                cleanup = release_tracked_device_state(dev, now_us());
            }
        }
        self.event_queue.extend(cleanup);
        now_disabled
    }

    /// Update the kernel LED state for a device.
    ///
    /// input-rs owns the LED write so the compositor does not need to issue
    /// evdev ioctls itself. Backends that cannot write LEDs return `Ok(())`
    /// and still record the logical state.
    pub fn set_led_state(
        &mut self,
        id: DeviceId,
        state: crate::policy::LedState,
    ) -> std::io::Result<()> {
        let exists = if let Some(dev) = self.devices.get_mut(id) {
            dev.info.led_state = state;
            true
        } else {
            false
        };
        if !exists {
            return Ok(());
        }
        self.backend.set_led_state(id, state)
    }

    /// Last known LED state for a device.
    pub fn led_state(&self, id: DeviceId) -> Option<crate::policy::LedState> {
        self.devices.get(id).map(|dev| dev.info.led_state)
    }

    /// Returns whether a device is enabled (not disabled).
    pub fn is_device_enabled(&self, id: DeviceId) -> bool {
        self.devices
            .get(id)
            .map(|dev| !dev.disabled)
            .unwrap_or(true)
    }
}
