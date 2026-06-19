use crate::backend::BusType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisableReasons {
    pub user: bool,
    pub external_mouse: bool,
    pub switch: bool,
    pub temporary: bool,
}

impl DisableReasons {
    pub fn any(self) -> bool {
        self.user || self.external_mouse || self.switch || self.temporary
    }

    pub fn effective_enabled(self) -> bool {
        !self.any()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LedState {
    pub caps_lock: bool,
    pub num_lock: bool,
    pub scroll_lock: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceMetadataProbe {
    pub time_us: u64,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus: BusType,
    pub internal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DevicePolicyEvent {
    Metadata(DeviceMetadataProbe),
    SendEvents {
        time_us: u64,
        internal: bool,
        external_mouse_present: bool,
        user_enabled: bool,
        switch_enabled: bool,
    },
    Led {
        time_us: u64,
        caps_lock: bool,
        num_lock: bool,
        scroll_lock: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DevicePolicyState {
    pub last_time_us: u64,
    pub metadata_seen: u32,
    pub internal_devices_seen: u32,
    pub external_devices_seen: u32,
    pub metadata_hash: u64,
    pub disable_reasons: DisableReasons,
    pub led_state: LedState,
}

impl DevicePolicyState {
    pub fn apply(&mut self, event: DevicePolicyEvent) {
        match event {
            DevicePolicyEvent::Metadata(metadata) => {
                self.last_time_us = metadata.time_us;
                self.metadata_seen += 1;
                if metadata.internal {
                    self.internal_devices_seen += 1;
                } else {
                    self.external_devices_seen += 1;
                }
                self.metadata_hash ^= ((metadata.vendor_id as u64) << 32)
                    ^ ((metadata.product_id as u64) << 16)
                    ^ bus_score(metadata.bus)
                    ^ (metadata.internal as u64);
            }
            DevicePolicyEvent::SendEvents {
                time_us,
                internal,
                external_mouse_present,
                user_enabled,
                switch_enabled,
            } => {
                self.last_time_us = time_us;
                self.disable_reasons.user = !user_enabled;
                self.disable_reasons.external_mouse = internal && external_mouse_present;
                self.disable_reasons.switch = internal && !switch_enabled;
            }
            DevicePolicyEvent::Led {
                time_us,
                caps_lock,
                num_lock,
                scroll_lock,
            } => {
                self.last_time_us = time_us;
                self.led_state = LedState {
                    caps_lock,
                    num_lock,
                    scroll_lock,
                };
            }
        }
    }

    pub fn effective_enabled(&self) -> bool {
        self.disable_reasons.effective_enabled()
    }

    pub fn diagnostic_score(&self) -> u64 {
        self.last_time_us
            ^ ((self.metadata_seen as u64) << 8)
            ^ ((self.internal_devices_seen as u64) << 16)
            ^ ((self.external_devices_seen as u64) << 24)
            ^ self.metadata_hash
            ^ ((self.disable_reasons.user as u64) << 1)
            ^ ((self.disable_reasons.external_mouse as u64) << 2)
            ^ ((self.disable_reasons.switch as u64) << 3)
            ^ ((self.disable_reasons.temporary as u64) << 4)
            ^ ((self.led_state.caps_lock as u64) << 5)
            ^ ((self.led_state.num_lock as u64) << 6)
            ^ ((self.led_state.scroll_lock as u64) << 7)
            ^ (self.effective_enabled() as u64)
    }
}

fn bus_score(bus: BusType) -> u64 {
    match bus {
        BusType::Unknown => 0,
        BusType::Usb => 1,
        BusType::Bluetooth => 2,
        BusType::I2c => 3,
        BusType::Ps2 => 4,
    }
}
