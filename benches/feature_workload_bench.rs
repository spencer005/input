//! Human-like workload benches for missing libinput parity features.
//!
//! This benchmark target is organized to match
//! `docs/missing-features/*.md`. Some groups exercise current input-rs
//! filters directly. Others exercise current event-model or policy scaffolding
//! for features whose backend and compositor integration are still incomplete.

use {
    criterion::{Criterion, black_box, criterion_group, criterion_main},
    input_rs::{
        TouchData, TouchEndData,
        backend::{AxisInfo, Backend, BusType, DeviceCaps, DeviceInfo},
        events::{
            DevIndex, InputEvent, ScrollSource, SwitchKind, TabletPadButtonData, TabletPadDialData,
            TabletPadValueData, TabletToolAxisData, TabletToolButtonData, TabletToolProximityData,
            TabletToolTipData, TouchToolType,
        },
        policy::{DeviceMetadataProbe, DevicePolicyEvent, DevicePolicyState},
        processing::{
            Filter, FilterChain, FilterKind,
            absolute_transform::{AbsoluteTransformFilter, CalibrationMatrix},
            button_scroll::ButtonScrollFilter,
            config::{TouchpadClickMethod, TouchpadConfig, TouchpadScrollMethod},
            debounce::{ButtonDebounceFilter, DebounceConfig},
            middle_button::MiddleButtonEmulationFilter,
            mouse::{MouseAccelProfile, MouseConfig, MouseFilter, parse_mouse_dpi_property},
            natural_scroll::NaturalScrollFilter,
            touchpad_motion::TouchpadMotionFilter,
        },
        state::{Context, DeviceId, Seat},
    },
    smallvec::smallvec,
    std::{io, os::unix::io::OwnedFd},
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const KEY_A: u32 = 30;
const VARIANT_SUITE_SEEDS: [u64; 3] = [21, 34, 55];

struct StaticInfoBackend {
    info: DeviceInfo,
    events: Vec<InputEvent>,
    device_id: Option<DeviceId>,
}

impl Backend for StaticInfoBackend {
    fn initialize(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
        if let Some(device_id) = self.device_id {
            for event in self.events.drain(..) {
                callback(device_id, event);
            }
        }
        Ok(())
    }

    fn add_device_fd(
        &mut self,
        _id: DeviceId,
        _fd: OwnedFd,
        _name: &str,
    ) -> io::Result<DeviceInfo> {
        self.device_id = Some(_id);
        Ok(self.info.clone())
    }

    fn get_fd(&self) -> std::os::unix::io::RawFd {
        -1
    }
}

fn bit(value: bool) -> u64 {
    if value { 1 } else { 0 }
}

fn bench_bus_score(bus: BusType) -> u64 {
    match bus {
        BusType::Unknown => 0,
        BusType::Usb => 1,
        BusType::Bluetooth => 2,
        BusType::I2c => 3,
        BusType::Ps2 => 4,
    }
}

fn axis_score(axis: &AxisInfo, rotation: u32) -> u64 {
    axis.min.to_bits()
        ^ axis.max.to_bits().rotate_left(rotation)
        ^ axis.resolution.to_bits().rotate_left((rotation + 11) % 64)
}

fn score_optional_f64(value: Option<f64>, rotation: u32) -> u64 {
    value.map_or(0, |value| value.to_bits().rotate_left(rotation))
}

fn score_optional_i32(value: Option<i32>, rotation: u32) -> u64 {
    value.map_or(0, |value| (value as i64 as u64).rotate_left(rotation))
}

fn score_optional_touch_tool(value: Option<TouchToolType>, rotation: u32) -> u64 {
    value.map_or(0, |value| {
        (value.linux_value() as i64 as u64).rotate_left(rotation)
    })
}

fn touchpad_info() -> DeviceInfo {
    let mut caps = DeviceCaps::default();
    caps.insert(DeviceCaps::TOUCHPAD);
    caps.insert(DeviceCaps::POINTER);

    DeviceInfo {
        name: "bench laptop touchpad".to_string(),
        x: AxisInfo {
            min: 0.0,
            max: 4200.0,
            resolution: 42.0,
        },
        y: AxisInfo {
            min: 0.0,
            max: 2600.0,
            resolution: 42.0,
        },
        vendor_id: 0x06cb,
        product_id: 0x0001,
        capabilities: caps,
        ..Default::default()
    }
}

fn touchscreen_info() -> DeviceInfo {
    let mut caps = DeviceCaps::default();
    caps.insert(DeviceCaps::TOUCHSCREEN);

    DeviceInfo {
        name: "bench calibrated touchscreen".to_string(),
        x: AxisInfo {
            min: 0.0,
            max: 1920.0,
            resolution: 12.0,
        },
        y: AxisInfo {
            min: 0.0,
            max: 1080.0,
            resolution: 12.0,
        },
        vendor_id: 0x1234,
        product_id: 0x5678,
        capabilities: caps,
        ..Default::default()
    }
}

fn pointer_info() -> DeviceInfo {
    let mut caps = DeviceCaps::default();
    caps.insert(DeviceCaps::POINTER);

    DeviceInfo {
        name: "bench relative pointer".to_string(),
        vendor_id: 0x046d,
        product_id: 0xc539,
        bus: BusType::Usb,
        capabilities: caps,
        ..Default::default()
    }
}

fn keyboard_info_with_name(name: &str) -> DeviceInfo {
    let mut caps = DeviceCaps::default();
    caps.insert(DeviceCaps::KEYBOARD);

    DeviceInfo {
        name: name.to_string(),
        vendor_id: 0x046d,
        product_id: 0x0007,
        bus: BusType::Usb,
        capabilities: caps,
        ..Default::default()
    }
}

fn bench_seat(last_key_time: u64) -> Seat {
    Seat {
        name: "seat0".to_string(),
        devices: Vec::new(),
        last_key_time,
    }
}

fn touch_data(time_us: u64, slot: i32, x: f64, y: f64) -> TouchData {
    TouchData {
        device_index: DevIndex(0),
        time_us,
        slot,
        x,
        y,
        pressure: None,
        touch_major: None,
        touch_minor: None,
        tool_type: None,
        tracking_id: None,
    }
}

fn td(time_us: u64, slot: i32, x: f64, y: f64) -> InputEvent {
    InputEvent::TouchDown(Box::new(touch_data(time_us, slot, x, y)))
}

fn tm(time_us: u64, slot: i32, x: f64, y: f64) -> InputEvent {
    InputEvent::TouchMotion(Box::new(touch_data(time_us, slot, x, y)))
}

fn td_meta(
    time_us: u64,
    slot: i32,
    x: f64,
    y: f64,
    pressure: Option<f64>,
    touch_major: Option<f64>,
    touch_minor: Option<f64>,
    tool_type: Option<TouchToolType>,
    tracking_id: Option<i32>,
) -> InputEvent {
    let mut data = touch_data(time_us, slot, x, y);
    data.pressure = pressure;
    data.touch_major = touch_major;
    data.touch_minor = touch_minor;
    data.tool_type = tool_type;
    data.tracking_id = tracking_id;
    InputEvent::TouchDown(Box::new(data))
}

fn tm_meta(
    time_us: u64,
    slot: i32,
    x: f64,
    y: f64,
    pressure: Option<f64>,
    touch_major: Option<f64>,
    touch_minor: Option<f64>,
    tool_type: Option<TouchToolType>,
    tracking_id: Option<i32>,
) -> InputEvent {
    let mut data = touch_data(time_us, slot, x, y);
    data.pressure = pressure;
    data.touch_major = touch_major;
    data.touch_minor = touch_minor;
    data.tool_type = tool_type;
    data.tracking_id = tracking_id;
    InputEvent::TouchMotion(Box::new(data))
}

fn tu(time_us: u64, slot: i32) -> InputEvent {
    InputEvent::TouchUp(Box::new(TouchEndData {
        device_index: DevIndex(0),
        time_us,
        slot,
    }))
}

fn tc(time_us: u64, slot: i32) -> InputEvent {
    InputEvent::TouchCancel(Box::new(TouchEndData {
        device_index: DevIndex(0),
        time_us,
        slot,
    }))
}

fn frame(time_us: u64) -> InputEvent {
    InputEvent::Frame {
        device_index: DevIndex(0),
        time_us,
    }
}

fn pointer_button(time_us: u64, button: u32, pressed: bool) -> InputEvent {
    InputEvent::PointerButton {
        device_index: DevIndex(0),
        time_us,
        button,
        pressed,
    }
}

fn pointer_motion(time_us: u64, dx: f64, dy: f64) -> InputEvent {
    InputEvent::PointerMotion {
        device_index: DevIndex(0),
        time_us,
        dx,
        dy,
        dx_unaccelerated: dx,
        dy_unaccelerated: dy,
    }
}

fn key(time_us: u64, key: u32, pressed: bool) -> InputEvent {
    InputEvent::Key {
        device_index: DevIndex(0),
        time_us,
        key,
        pressed,
    }
}

fn tablet_proximity(time_us: u64, x: f64, y: f64, in_range: bool) -> InputEvent {
    InputEvent::TabletToolProximity(Box::new(TabletToolProximityData {
        device_index: DevIndex(0),
        time_us,
        x,
        y,
        in_range,
    }))
}

fn tablet_axis(
    time_us: u64,
    x: f64,
    y: f64,
    pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
) -> InputEvent {
    InputEvent::TabletToolAxis(Box::new(TabletToolAxisData {
        device_index: DevIndex(0),
        time_us,
        x,
        y,
        pressure,
        tilt_x,
        tilt_y,
    }))
}

fn tablet_tip(time_us: u64, down: bool) -> InputEvent {
    InputEvent::TabletToolTip(Box::new(TabletToolTipData {
        device_index: DevIndex(0),
        time_us,
        down,
    }))
}

fn tablet_button(time_us: u64, button: u32, pressed: bool) -> InputEvent {
    InputEvent::TabletToolButton(Box::new(TabletToolButtonData {
        device_index: DevIndex(0),
        time_us,
        button,
        pressed,
    }))
}

fn tablet_pad_button(time_us: u64, index: u32, pressed: bool) -> InputEvent {
    InputEvent::TabletPadButton(Box::new(TabletPadButtonData {
        device_index: DevIndex(0),
        time_us,
        index,
        pressed,
    }))
}

fn tablet_pad_ring(time_us: u64, index: u32, position: f64) -> InputEvent {
    InputEvent::TabletPadRing(Box::new(TabletPadValueData {
        device_index: DevIndex(0),
        time_us,
        index,
        position,
    }))
}

fn tablet_pad_strip(time_us: u64, index: u32, position: f64) -> InputEvent {
    InputEvent::TabletPadStrip(Box::new(TabletPadValueData {
        device_index: DevIndex(0),
        time_us,
        index,
        position,
    }))
}

fn tablet_pad_dial(time_us: u64, index: u32, delta: f64) -> InputEvent {
    InputEvent::TabletPadDial(Box::new(TabletPadDialData {
        device_index: DevIndex(0),
        time_us,
        index,
        delta,
    }))
}

fn event_score(event: &InputEvent) -> u64 {
    let base = event.time_us().wrapping_add(event.device_index().0 as u64);
    match event {
        InputEvent::DeviceAdded { .. } => base ^ 0x0100,
        InputEvent::DeviceRemoved { .. } => base ^ 0x0200,
        InputEvent::PointerMotion { dx, dy, .. } => {
            base ^ dx.to_bits().rotate_left(7) ^ dy.to_bits().rotate_right(3)
        }
        InputEvent::PointerButton {
            button, pressed, ..
        } => base ^ ((*button as u64) << 1) ^ bit(*pressed),
        InputEvent::Key { key, pressed, .. } => base ^ ((*key as u64) << 2) ^ (bit(*pressed) << 1),
        InputEvent::PointerScroll { dx, dy, .. } => {
            base ^ dx.to_bits().rotate_left(11) ^ dy.to_bits().rotate_right(9)
        }
        InputEvent::SwitchToggle {
            switch, enabled, ..
        } => base ^ ((switch.linux_code() as u64) << 4) ^ bit(*enabled),
        InputEvent::Frame { .. } => base ^ 0x0300,
        InputEvent::TouchDown(data) => {
            base ^ ((data.slot as u64) << 8)
                ^ data.x.to_bits()
                ^ data.y.to_bits()
                ^ touch_metadata_score(data)
        }
        InputEvent::TouchUp(data) => base ^ ((data.slot as u64) << 8) ^ 0x0400,
        InputEvent::TouchCancel(data) => base ^ ((data.slot as u64) << 8) ^ 0x0500,
        InputEvent::TouchMotion(data) => {
            base ^ ((data.slot as u64) << 8)
                ^ data.x.to_bits().rotate_left(5)
                ^ data.y.to_bits()
                ^ touch_metadata_score(data).rotate_left(1)
        }
        InputEvent::GestureSwipeBegin(data) => base ^ ((data.fingers as u64) << 8),
        InputEvent::GestureSwipeUpdate(data) => {
            base ^ data.dx.to_bits().rotate_left(13) ^ data.dy.to_bits()
        }
        InputEvent::GestureSwipeEnd(data) => base ^ (bit(data.cancelled) << 9),
        InputEvent::GesturePinchBegin(data) => base ^ ((data.fingers as u64) << 10),
        InputEvent::GesturePinchUpdate(data) => {
            base ^ data.dx.to_bits()
                ^ data.dy.to_bits()
                ^ data.scale.to_bits()
                ^ data.angle.to_bits()
        }
        InputEvent::GesturePinchEnd(data) => base ^ (bit(data.cancelled) << 11),
        InputEvent::GestureHoldBegin(data) => base ^ ((data.fingers as u64) << 12),
        InputEvent::GestureHoldEnd(data) => {
            base ^ ((data.fingers as u64) << 12) ^ (bit(data.cancelled) << 13)
        }
        InputEvent::TabletToolProximity(data) => {
            base ^ data.x.to_bits() ^ data.y.to_bits() ^ (bit(data.in_range) << 12)
        }
        InputEvent::TabletToolAxis(data) => {
            base ^ data.x.to_bits()
                ^ data.y.to_bits().rotate_left(3)
                ^ data.pressure.to_bits().rotate_left(5)
                ^ data.tilt_x.to_bits()
                ^ data.tilt_y.to_bits().rotate_right(7)
        }
        InputEvent::TabletToolTip(data) => base ^ (bit(data.down) << 13),
        InputEvent::TabletToolButton(data) => {
            base ^ ((data.button as u64) << 8) ^ (bit(data.pressed) << 14)
        }
        InputEvent::TabletPadButton(data) => {
            base ^ ((data.index as u64) << 9) ^ (bit(data.pressed) << 15)
        }
        InputEvent::TabletPadRing(data) | InputEvent::TabletPadStrip(data) => {
            base ^ ((data.index as u64) << 10) ^ data.position.to_bits()
        }
        InputEvent::TabletPadDial(data) => {
            base ^ ((data.index as u64) << 10) ^ data.delta.to_bits()
        }
    }
}

fn touch_metadata_score(data: &TouchData) -> u64 {
    score_optional_f64(data.pressure, 3)
        ^ score_optional_f64(data.touch_major, 7)
        ^ score_optional_f64(data.touch_minor, 11)
        ^ score_optional_touch_tool(data.tool_type, 17)
        ^ score_optional_i32(data.tracking_id, 23)
}

fn varied_time_us(time_us: u64, seed: u64) -> u64 {
    time_us
        .wrapping_add(11_000 + seed * 379)
        .wrapping_add(((time_us / 8_000 + seed) % 5) * 137)
}

fn varied_slot(slot: i32, seed: u64) -> i32 {
    if slot < 0 {
        slot
    } else {
        slot + 1 + (seed % 3) as i32
    }
}

fn vary_touch_data(data: &mut TouchData, seed: u64) {
    data.time_us = varied_time_us(data.time_us, seed);
    data.slot = varied_slot(data.slot, seed);
    let phase = ((data.time_us / 8_000 + seed) % 7) as f64;
    let slot = data.slot as f64;
    data.x += (phase - 3.0) * 2.25 + slot * 0.65;
    data.y += (3.0 - phase) * 1.75 - slot * 0.45;
    if let Some(pressure) = &mut data.pressure {
        *pressure += (seed % 4) as f64;
    }
    if let Some(touch_major) = &mut data.touch_major {
        *touch_major += ((seed + data.slot as u64) % 3) as f64;
    }
    if let Some(touch_minor) = &mut data.touch_minor {
        *touch_minor += (seed % 2) as f64;
    }
}

fn vary_touch_end_data(data: &mut TouchEndData, seed: u64) {
    data.time_us = varied_time_us(data.time_us, seed);
    data.slot = varied_slot(data.slot, seed);
}

fn varied_touch_workload(events: &[InputEvent], seed: u64) -> Vec<InputEvent> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            match &mut event {
                InputEvent::TouchDown(data) | InputEvent::TouchMotion(data) => {
                    vary_touch_data(data, seed)
                }
                InputEvent::TouchUp(data) | InputEvent::TouchCancel(data) => {
                    vary_touch_end_data(data, seed)
                }
                InputEvent::Frame { time_us, .. } => {
                    *time_us = varied_time_us(*time_us, seed);
                }
                InputEvent::PointerButton { time_us, .. }
                | InputEvent::PointerMotion { time_us, .. }
                | InputEvent::PointerScroll { time_us, .. }
                | InputEvent::Key { time_us, .. }
                | InputEvent::SwitchToggle { time_us, .. } => {
                    *time_us = varied_time_us(*time_us, seed);
                }
                _ => {}
            }
            event
        })
        .collect()
}

fn varied_pointer_workload(events: &[InputEvent], seed: u64) -> Vec<InputEvent> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            match &mut event {
                InputEvent::PointerMotion {
                    time_us,
                    dx,
                    dy,
                    dx_unaccelerated,
                    dy_unaccelerated,
                    ..
                } => {
                    *time_us = varied_time_us(*time_us, seed);
                    let x_scale = 1.0 + seed as f64 * 0.011;
                    let y_scale = 1.0 - seed as f64 * 0.006;
                    *dx *= x_scale;
                    *dy *= y_scale;
                    *dx_unaccelerated *= x_scale;
                    *dy_unaccelerated *= y_scale;
                }
                InputEvent::PointerButton { time_us, .. }
                | InputEvent::PointerScroll { time_us, .. }
                | InputEvent::Frame { time_us, .. } => {
                    *time_us = varied_time_us(*time_us, seed);
                }
                _ => {}
            }
            event
        })
        .collect()
}

fn varied_tablet_workload(events: &[InputEvent], seed: u64) -> Vec<InputEvent> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            match &mut event {
                InputEvent::TabletToolProximity(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                    data.x += seed as f64 * 2.0;
                    data.y -= seed as f64 * 1.5;
                }
                InputEvent::TabletToolAxis(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                    data.x += seed as f64 * 2.0;
                    data.y -= seed as f64 * 1.5;
                    data.pressure = (data.pressure + seed as f64 * 0.004).min(1.0);
                    data.tilt_x += seed as f64 * 0.15;
                    data.tilt_y -= seed as f64 * 0.10;
                }
                InputEvent::TabletToolTip(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                }
                InputEvent::TabletToolButton(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                }
                InputEvent::TabletPadButton(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                }
                InputEvent::TabletPadRing(data) | InputEvent::TabletPadStrip(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                    data.position = (data.position + seed as f64 * 0.002).min(1.0);
                }
                InputEvent::TabletPadDial(data) => {
                    data.time_us = varied_time_us(data.time_us, seed);
                    data.delta *= 1.0 + seed as f64 * 0.01;
                }
                InputEvent::Frame { time_us, .. } => {
                    *time_us = varied_time_us(*time_us, seed);
                }
                _ => {}
            }
            event
        })
        .collect()
}

fn varied_switch_workload(events: &[InputEvent], seed: u64) -> Vec<InputEvent> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            match &mut event {
                InputEvent::SwitchToggle { time_us, .. } | InputEvent::Frame { time_us, .. } => {
                    *time_us = varied_time_us(*time_us, seed);
                }
                _ => {}
            }
            event
        })
        .collect()
}

fn varied_device_policy_workload(
    events: &[DevicePolicyEvent],
    seed: u64,
) -> Vec<DevicePolicyEvent> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            match &mut event {
                DevicePolicyEvent::Metadata(metadata) => {
                    metadata.time_us = varied_time_us(metadata.time_us, seed);
                    metadata.vendor_id ^= (seed as u16) << 4;
                    metadata.product_id ^= seed as u16;
                }
                DevicePolicyEvent::SendEvents { time_us, .. }
                | DevicePolicyEvent::Led { time_us, .. } => {
                    *time_us = varied_time_us(*time_us, seed);
                }
            }
            event
        })
        .collect()
}

fn touch_variant_suite(events: &[InputEvent]) -> Vec<Vec<InputEvent>> {
    VARIANT_SUITE_SEEDS
        .iter()
        .map(|seed| varied_touch_workload(events, *seed))
        .collect()
}

fn pointer_variant_suite(events: &[InputEvent]) -> Vec<Vec<InputEvent>> {
    VARIANT_SUITE_SEEDS
        .iter()
        .map(|seed| varied_pointer_workload(events, *seed))
        .collect()
}

fn tablet_variant_suite(events: &[InputEvent]) -> Vec<Vec<InputEvent>> {
    VARIANT_SUITE_SEEDS
        .iter()
        .map(|seed| varied_tablet_workload(events, *seed))
        .collect()
}

fn switch_variant_suite(events: &[InputEvent]) -> Vec<Vec<InputEvent>> {
    VARIANT_SUITE_SEEDS
        .iter()
        .map(|seed| varied_switch_workload(events, *seed))
        .collect()
}

fn policy_variant_suite(events: &[DevicePolicyEvent]) -> Vec<Vec<DevicePolicyEvent>> {
    VARIANT_SUITE_SEEDS
        .iter()
        .map(|seed| varied_device_policy_workload(events, *seed))
        .collect()
}

fn run_touchpad_motion(events: &[InputEvent], config: TouchpadConfig, seat: Option<&Seat>) -> u64 {
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for event in events {
        let out = filter.process(black_box(event.clone()), seat, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_touchpad_motion_variant_suite(
    workloads: &[Vec<InputEvent>],
    config: TouchpadConfig,
    seat: Option<&Seat>,
) -> u64 {
    let mut score = 0u64;
    for (idx, workload) in workloads.iter().enumerate() {
        score = score
            .wrapping_mul(131)
            .wrapping_add(idx as u64)
            .wrapping_add(run_touchpad_motion(black_box(workload), config, seat));
    }
    score
}

fn run_touchpad_motion_with_runtime_reset(
    events: &[InputEvent],
    config: TouchpadConfig,
    reset_before_event: usize,
) -> u64 {
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == reset_before_event {
            filter.reset_runtime_state();
            score = score.wrapping_add(0x0d15_ab1e);
        }
        let out = filter.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_touchpad_motion_with_scroll_method_change(
    events: &[InputEvent],
    initial: TouchpadScrollMethod,
    changed: TouchpadScrollMethod,
    change_before_event: usize,
) -> u64 {
    let mut config = TouchpadConfig::default();
    config.scroll_method = initial;
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            filter.set_scroll_method(changed);
            score = score.wrapping_add(0x5c20_11ed);
        }
        let out = filter.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_touchpad_motion_with_natural_scroll_change(
    events: &[InputEvent],
    initial: bool,
    changed: bool,
    change_before_event: usize,
) -> u64 {
    let mut config = TouchpadConfig::default();
    config.natural_scrolling = initial;
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            filter.set_natural_scrolling(changed);
            score = score.wrapping_add(0x5c20_11e0);
        }
        let out = filter.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_touchpad_motion_with_tap_to_click_change(
    events: &[InputEvent],
    initial: bool,
    changed: bool,
    change_before_event: usize,
) -> u64 {
    let mut config = TouchpadConfig::default();
    config.tap_to_click = initial;
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            filter.set_tap_to_click(changed);
            score = score.wrapping_add(0x7a9_70c1);
        }
        let out = filter.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_touchpad_motion_with_two_finger_tap_right_click_change(
    events: &[InputEvent],
    initial: bool,
    changed: bool,
    change_before_event: usize,
) -> u64 {
    let mut config = TouchpadConfig::default();
    config.two_finger_tap_right_click = initial;
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            filter.set_two_finger_tap_right_click(changed);
            score = score.wrapping_add(0x2f1_972);
        }
        let out = filter.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_touchpad_motion_with_click_method_change(
    events: &[InputEvent],
    initial: TouchpadClickMethod,
    changed: TouchpadClickMethod,
    change_before_event: usize,
) -> u64 {
    let mut config = TouchpadConfig::default();
    config.click_method = initial;
    let mut filter = TouchpadMotionFilter::new(config);
    let info = touchpad_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            filter.set_click_method(changed);
            score = score.wrapping_add(0xc11c_0000);
        }
        let out = filter.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_filter_chain(events: &[InputEvent], chain: FilterChain) -> u64 {
    let info = touchpad_info();
    run_filter_chain_with_info(events, chain, info)
}

fn run_touchpad_left_handed_chain(events: &[InputEvent], config: TouchpadConfig) -> u64 {
    let mut chain = FilterChain::new();
    chain.add(FilterKind::TouchpadMotion(Box::new(
        TouchpadMotionFilter::new(config),
    )));
    chain.set_left_handed(true);
    run_filter_chain(events, chain)
}

fn run_filter_chain_with_info(
    events: &[InputEvent],
    mut chain: FilterChain,
    info: DeviceInfo,
) -> u64 {
    let mut score = 0u64;

    for event in events {
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_filter_chain_variant_suite<F>(
    workloads: &[Vec<InputEvent>],
    mut chain_factory: F,
    info: DeviceInfo,
) -> u64
where
    F: FnMut() -> FilterChain,
{
    let mut score = 0u64;
    for (idx, workload) in workloads.iter().enumerate() {
        score = score
            .wrapping_mul(131)
            .wrapping_add(idx as u64)
            .wrapping_add(run_filter_chain_with_info(
                black_box(workload),
                chain_factory(),
                info.clone(),
            ));
    }
    score
}

fn run_filter_chain_with_calibration_change(
    events: &[InputEvent],
    info: DeviceInfo,
    initial: CalibrationMatrix,
    changed: CalibrationMatrix,
    change_before_event: usize,
) -> u64 {
    let mut chain = FilterChain::new();
    chain.set_calibration_matrix(initial);
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            chain.set_calibration_matrix(changed);
            score = score.wrapping_add(0xca11_b8a7);
        }
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_filter_chain_with_button_scroll_change(
    events: &[InputEvent],
    initial_enabled: bool,
    initial_button: u32,
    initial_lock: bool,
    changed_enabled: bool,
    changed_button: u32,
    changed_lock: bool,
    change_before_event: usize,
) -> u64 {
    let mut chain = FilterChain::new();
    chain.set_button_scroll(initial_enabled, initial_button, initial_lock);
    let info = pointer_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            chain.set_button_scroll(changed_enabled, changed_button, changed_lock);
            score = score.wrapping_add(0xb077_5c20);
        }
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_filter_chain_with_middle_button_change(
    events: &[InputEvent],
    initial_enabled: bool,
    changed_enabled: bool,
    change_before_event: usize,
) -> u64 {
    let mut chain = FilterChain::new();
    chain.set_middle_button_emulation(initial_enabled);
    let info = pointer_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            chain.set_middle_button_emulation(changed_enabled);
            score = score.wrapping_add(0x0b1d_0000);
        }
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_filter_chain_with_debounce_change(
    events: &[InputEvent],
    initial: DebounceConfig,
    changed: DebounceConfig,
    change_before_event: usize,
) -> u64 {
    let mut chain = FilterChain::new();
    chain.set_debounce(initial);
    let info = pointer_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            chain.set_debounce(changed);
            score = score.wrapping_add(0xd3b0_0000);
        }
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_calibration_matrix_config_sequence(configs: &[[f64; 6]]) -> u64 {
    let mut score = 0u64;
    for values in configs {
        let accepted = CalibrationMatrix::new(black_box(*values)).is_some();
        let value_score = values
            .iter()
            .fold(0u64, |score, value| score.rotate_left(7) ^ value.to_bits());
        score = score
            .wrapping_mul(131)
            .wrapping_add(value_score)
            .wrapping_add(bit(accepted));
    }
    score
}

fn run_mouse_dpi(events: &[InputEvent], dpi: i32) -> u64 {
    let mut chain = FilterChain::new();
    chain.add(FilterKind::Mouse(Box::new(MouseFilter::flat(dpi))));
    run_filter_chain_with_info(events, chain, pointer_info())
}

fn run_mouse_dpi_variant_suite(workloads: &[Vec<InputEvent>], dpi: i32) -> u64 {
    let mut score = 0u64;
    for (idx, workload) in workloads.iter().enumerate() {
        score = score
            .wrapping_mul(131)
            .wrapping_add(idx as u64)
            .wrapping_add(run_mouse_dpi(black_box(workload), dpi));
    }
    score
}

fn run_mouse_config(events: &[InputEvent], config: MouseConfig) -> u64 {
    let mut chain = FilterChain::new();
    chain.add(FilterKind::Mouse(Box::new(MouseFilter::with_config(
        config,
    ))));
    run_filter_chain_with_info(events, chain, pointer_info())
}

fn run_mouse_config_variant_suite(workloads: &[Vec<InputEvent>], config: MouseConfig) -> u64 {
    let mut score = 0u64;
    for (idx, workload) in workloads.iter().enumerate() {
        score = score
            .wrapping_mul(131)
            .wrapping_add(idx as u64)
            .wrapping_add(run_mouse_config(black_box(workload), config.clone()));
    }
    score
}

fn run_mouse_profile_change(
    events: &[InputEvent],
    initial: MouseAccelProfile,
    changed: MouseAccelProfile,
    change_before_event: usize,
) -> u64 {
    let mut chain = FilterChain::new();
    chain.add(FilterKind::Mouse(Box::new(MouseFilter::with_config(
        MouseConfig {
            dpi: 1000,
            speed_adjustment: 0.0,
            profile: initial,
        },
    ))));
    let info = pointer_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            chain.set_mouse_profile(changed.clone());
            score = score.wrapping_add(0xa11c_0000);
        }
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_mouse_speed_change(
    events: &[InputEvent],
    initial_speed: f64,
    changed_speed: f64,
    change_before_event: usize,
) -> u64 {
    let mut chain = FilterChain::new();
    chain.add(FilterKind::Mouse(Box::new(MouseFilter::with_config(
        MouseConfig {
            dpi: 1000,
            speed_adjustment: initial_speed,
            profile: MouseAccelProfile::Adaptive,
        },
    ))));
    let info = pointer_info();
    let mut score = 0u64;

    for (idx, event) in events.iter().enumerate() {
        if idx == change_before_event {
            chain.set_mouse_speed_adjustment(changed_speed);
            score = score.wrapping_add(0x5eed_0000);
        }
        let out = chain.process(black_box(event.clone()), None, Some(info.clone()));
        score = score.wrapping_add(out.len() as u64);
        for event in &out {
            score = score.wrapping_add(event_score(event));
        }
    }

    score
}

fn run_mouse_dpi_property_parser(properties: &[&str]) -> u64 {
    let mut score = 0u64;
    for property in properties {
        let dpi = parse_mouse_dpi_property(black_box(*property)).unwrap_or_default();
        score = score.wrapping_mul(131).wrapping_add(dpi as u64);
    }
    score
}

fn bench_name_matches(requested: &str, actual: &str) -> bool {
    actual == requested || actual.starts_with(requested)
}

fn run_device_policy(events: &[DevicePolicyEvent]) -> u64 {
    let mut state = DevicePolicyState::default();
    let mut score = 0u64;
    for event in events {
        state.apply(black_box(*event));
        score = score.wrapping_add(state.diagnostic_score());
    }
    score
}

fn run_device_policy_variant_suite(workloads: &[Vec<DevicePolicyEvent>]) -> u64 {
    let mut score = 0u64;
    for (idx, workload) in workloads.iter().enumerate() {
        score = score
            .wrapping_mul(131)
            .wrapping_add(idx as u64)
            .wrapping_add(run_device_policy(black_box(workload)));
    }
    score
}

fn run_device_capability_inventory(infos: &[DeviceInfo]) -> u64 {
    let mut score = 0u64;

    for info in infos {
        let mut switch_score = 0u64;
        for switch in &info.supported_switches {
            switch_score = switch_score
                .rotate_left(3)
                .wrapping_add(switch.linux_code() as u64);
        }
        for (switch, enabled) in &info.switch_states {
            switch_score = switch_score.rotate_left(5)
                ^ (switch.linux_code() as u64)
                ^ bit(*enabled).rotate_left(17);
        }

        score = score
            .rotate_left(7)
            .wrapping_add(info.name.len() as u64)
            .wrapping_add((info.vendor_id as u64) << 32)
            .wrapping_add((info.product_id as u64) << 16)
            ^ bench_bus_score(info.bus)
            ^ ((info.capabilities.0 as u64) << 8)
            ^ axis_score(&info.x, 13)
            ^ axis_score(&info.y, 29)
            ^ switch_score;
    }

    score
}

fn single_finger_tap() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1800.0, 1200.0),
        frame(0),
        tu(82_000, 0),
        frame(82_000),
    ]
}

fn two_finger_tap() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1700.0, 1160.0),
        td(0, 1, 2080.0, 1190.0),
        frame(0),
        tu(94_000, 0),
        tu(94_000, 1),
        frame(94_000),
    ]
}

fn two_finger_tap_staggered_down_reverse_up() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1700.0, 1160.0),
        frame(0),
        td(32_000, 1, 2080.0, 1190.0),
        frame(32_000),
        tu(92_000, 1),
        frame(92_000),
        tu(104_000, 0),
        frame(104_000),
    ]
}

fn two_finger_tap_same_frame_reverse_up() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1700.0, 1160.0),
        td(0, 1, 2080.0, 1190.0),
        frame(0),
        tu(88_000, 1),
        frame(88_000),
        tu(104_000, 0),
        frame(104_000),
    ]
}

fn three_finger_tap() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1480.0, 1180.0),
        td(0, 1, 1840.0, 1200.0),
        td(0, 2, 2200.0, 1190.0),
        frame(0),
        tu(104_000, 0),
        tu(104_000, 1),
        tu(104_000, 2),
        frame(104_000),
    ]
}

fn alternate_tap_map_candidate() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1800.0, 1200.0),
        frame(0),
        tu(78_000, 0),
        frame(78_000),
        td(220_000, 0, 1700.0, 1160.0),
        td(220_000, 1, 2080.0, 1190.0),
        frame(220_000),
        tu(312_000, 0),
        tu(312_000, 1),
        frame(312_000),
        td(470_000, 0, 1480.0, 1180.0),
        td(470_000, 1, 1840.0, 1200.0),
        td(470_000, 2, 2200.0, 1190.0),
        frame(470_000),
        tu(575_000, 0),
        tu(575_000, 1),
        tu(575_000, 2),
        frame(575_000),
    ]
}

fn tap_then_drag_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1600.0, 1250.0),
        frame(0),
        tu(72_000, 0),
        frame(72_000),
        td(118_000, 0, 1602.0, 1250.0),
        frame(118_000),
    ];

    for i in 0..24 {
        let t = 126_000 + i * 8_000;
        events.push(tm(t, 0, 1602.0 + i as f64 * 18.0, 1250.0 + i as f64 * 3.0));
        events.push(frame(t));
    }

    events.push(tu(326_000, 0));
    events.push(frame(326_000));
    events
}

fn tap_drag_active_before_cleanup() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1600.0, 1250.0),
        frame(0),
        tu(72_000, 0),
        frame(72_000),
        td(118_000, 0, 1602.0, 1250.0),
        frame(118_000),
    ];

    for i in 0..12 {
        let t = 126_000 + i * 8_000;
        events.push(tm(t, 0, 1602.0 + i as f64 * 18.0, 1250.0 + i as f64 * 3.0));
        events.push(frame(t));
    }

    events
}

fn tap_drag_active_device_removed() -> Vec<InputEvent> {
    let mut events = tap_drag_active_before_cleanup();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn tap_drag_lock_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1600.0, 1250.0),
        frame(0),
        tu(70_000, 0),
        frame(70_000),
        td(112_000, 0, 1604.0, 1252.0),
        frame(112_000),
    ];

    for i in 0..10 {
        let t = 120_000 + i * 8_000;
        events.push(tm(t, 0, 1604.0 + i as f64 * 22.0, 1252.0 + i as f64 * 4.0));
        events.push(frame(t));
    }

    events.push(tu(212_000, 0));
    events.push(frame(212_000));
    events.push(td(252_000, 0, 1840.0, 1290.0));
    events.push(frame(252_000));

    for i in 0..12 {
        let t = 260_000 + i * 8_000;
        events.push(tm(t, 0, 1840.0 + i as f64 * 24.0, 1290.0 + i as f64 * 3.0));
        events.push(frame(t));
    }

    events.push(tu(380_000, 0));
    events.push(frame(380_000));
    events
}

fn tap_drag_lock_active_before_disable() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1600.0, 1250.0),
        frame(0),
        tu(70_000, 0),
        frame(70_000),
        td(112_000, 0, 1604.0, 1252.0),
        frame(112_000),
    ];

    for i in 0..10 {
        let t = 120_000 + i * 8_000;
        events.push(tm(t, 0, 1604.0 + i as f64 * 22.0, 1252.0 + i as f64 * 4.0));
        events.push(frame(t));
    }

    events.push(tu(212_000, 0));
    events.push(frame(212_000));
    events.push(td(252_000, 0, 1840.0, 1290.0));
    events.push(frame(252_000));

    for i in 0..8 {
        let t = 260_000 + i * 8_000;
        events.push(tm(t, 0, 1840.0 + i as f64 * 24.0, 1290.0 + i as f64 * 3.0));
        events.push(frame(t));
    }

    events
}

fn tap_drag_lock_active_device_removed() -> Vec<InputEvent> {
    let mut events = tap_drag_lock_active_before_disable();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn tap_cancelled_by_movement() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1600.0, 1200.0),
        frame(0),
        tm(32_000, 0, 1710.0, 1218.0),
        frame(32_000),
        tm(64_000, 0, 1840.0, 1240.0),
        frame(64_000),
        tu(96_000, 0),
        frame(96_000),
    ]
}

fn tap_cancelled_by_timeout() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1700.0, 1180.0),
        frame(0),
        frame(210_000),
        tu(235_000, 0),
        frame(235_000),
    ]
}

fn tap_near_timeout_candidate() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1700.0, 1180.0),
        frame(0),
        frame(150_000),
        tu(172_000, 0),
        frame(172_000),
    ]
}

fn tap_near_movement_threshold_candidate() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1600.0, 1200.0),
        frame(0),
        tm(34_000, 0, 1628.0, 1210.0),
        frame(34_000),
        tu(82_000, 0),
        frame(82_000),
    ]
}

fn tap_drag_second_touch_after_timeout_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1600.0, 1250.0),
        frame(0),
        tu(70_000, 0),
        frame(70_000),
        td(430_000, 0, 1604.0, 1252.0),
        frame(430_000),
    ];

    for i in 0..12 {
        let t = 438_000 + i * 8_000;
        events.push(tm(t, 0, 1604.0 + i as f64 * 18.0, 1252.0 + i as f64 * 3.0));
        events.push(frame(t));
    }

    events.push(tu(550_000, 0));
    events.push(frame(550_000));
    events
}

fn clickpad_click_at(x: f64, y: f64) -> Vec<InputEvent> {
    vec![
        td(0, 0, x, y),
        frame(0),
        pointer_button(24_000, BTN_LEFT, true),
        frame(24_000),
        pointer_button(86_000, BTN_LEFT, false),
        frame(86_000),
        tu(92_000, 0),
        frame(92_000),
    ]
}

fn clickpad_main_area_click() -> Vec<InputEvent> {
    clickpad_click_at(2100.0, 1320.0)
}

fn clickpad_bottom_left_click() -> Vec<InputEvent> {
    clickpad_click_at(520.0, 2440.0)
}

fn clickpad_bottom_middle_click() -> Vec<InputEvent> {
    clickpad_click_at(2100.0, 2440.0)
}

fn clickpad_bottom_right_click() -> Vec<InputEvent> {
    clickpad_click_at(3920.0, 2440.0)
}

fn physical_click_during_tap_candidate() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1800.0, 1200.0),
        frame(0),
        pointer_button(34_000, BTN_LEFT, true),
        frame(34_000),
        pointer_button(96_000, BTN_LEFT, false),
        frame(96_000),
        tu(112_000, 0),
        frame(112_000),
    ]
}

fn clickpad_latched_button_after_contact_move() -> Vec<InputEvent> {
    vec![
        td(0, 0, 3920.0, 2440.0),
        frame(0),
        pointer_button(24_000, BTN_LEFT, true),
        frame(24_000),
        tm(58_000, 0, 520.0, 2440.0),
        frame(58_000),
        pointer_button(92_000, BTN_LEFT, false),
        frame(92_000),
        tu(108_000, 0),
        frame(108_000),
    ]
}

fn clickfinger_click(fingers: usize) -> Vec<InputEvent> {
    let mut events = Vec::new();
    let contacts = [(1680.0, 1200.0), (2140.0, 1210.0), (1900.0, 1460.0)];

    for (slot, (x, y)) in contacts.iter().copied().enumerate().take(fingers) {
        events.push(td(0, slot as i32, x, y));
    }
    events.extend([
        frame(0),
        pointer_button(30_000, BTN_LEFT, true),
        frame(30_000),
        pointer_button(92_000, BTN_LEFT, false),
        frame(92_000),
    ]);
    for slot in 0..fingers {
        events.push(tu(105_000, slot as i32));
    }
    events.push(frame(105_000));
    events
}

fn one_finger_clickfinger_click() -> Vec<InputEvent> {
    clickfinger_click(1)
}

fn two_finger_clickfinger_click() -> Vec<InputEvent> {
    clickfinger_click(2)
}

fn three_finger_clickfinger_click() -> Vec<InputEvent> {
    clickfinger_click(3)
}

fn two_finger_clickfinger_staggered_arrival_click() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1680.0, 1200.0),
        frame(0),
        td(28_000, 1, 2140.0, 1210.0),
        frame(28_000),
        pointer_button(54_000, BTN_LEFT, true),
        frame(54_000),
        pointer_button(116_000, BTN_LEFT, false),
        frame(116_000),
        tu(132_000, 0),
        tu(132_000, 1),
        frame(132_000),
    ]
}

fn clickfinger_latched_button_after_finger_added() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1680.0, 1200.0),
        frame(0),
        pointer_button(30_000, BTN_LEFT, true),
        frame(30_000),
        td(58_000, 1, 2140.0, 1210.0),
        frame(58_000),
        pointer_button(112_000, BTN_LEFT, false),
        frame(112_000),
        tu(128_000, 1),
        tu(136_000, 0),
        frame(136_000),
    ]
}

fn alternate_clickfinger_map_candidate() -> Vec<InputEvent> {
    vec![
        td(0, 0, 1680.0, 1200.0),
        frame(0),
        pointer_button(30_000, BTN_LEFT, true),
        frame(30_000),
        pointer_button(92_000, BTN_LEFT, false),
        frame(92_000),
        tu(105_000, 0),
        frame(105_000),
        td(220_000, 0, 1680.0, 1200.0),
        td(220_000, 1, 2140.0, 1210.0),
        frame(220_000),
        pointer_button(255_000, BTN_LEFT, true),
        frame(255_000),
        pointer_button(318_000, BTN_LEFT, false),
        frame(318_000),
        tu(334_000, 0),
        tu(334_000, 1),
        frame(334_000),
        td(470_000, 0, 1680.0, 1200.0),
        td(470_000, 1, 2140.0, 1210.0),
        td(470_000, 2, 1900.0, 1460.0),
        frame(470_000),
        pointer_button(505_000, BTN_LEFT, true),
        frame(505_000),
        pointer_button(568_000, BTN_LEFT, false),
        frame(568_000),
        tu(584_000, 0),
        tu(584_000, 1),
        tu(584_000, 2),
        frame(584_000),
    ]
}

fn two_finger_scroll_with_step(step_x: f64, step_y: f64, frames: usize) -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1700.0, 1100.0), td(0, 1, 2100.0, 1120.0), frame(0)];

    for i in 0..frames {
        let t = 8_000 + i as u64 * 8_000;
        let x0 = 1700.0 + i as f64 * step_x;
        let x1 = 2100.0 + i as f64 * step_x;
        let y0 = 1100.0 + i as f64 * step_y;
        let y1 = 1120.0 + i as f64 * step_y;
        events.push(tm(t, 0, x0, y0));
        events.push(tm(t, 1, x1, y1));
        events.push(frame(t));
    }

    events.push(tu(280_000, 0));
    events.push(tu(280_000, 1));
    events.push(frame(280_000));
    events
}

fn two_finger_scroll_vertical() -> Vec<InputEvent> {
    two_finger_scroll_with_step(0.7, 14.0, 32)
}

fn two_finger_scroll_horizontal() -> Vec<InputEvent> {
    two_finger_scroll_with_step(15.0, 0.6, 32)
}

fn two_finger_scroll_diagonal() -> Vec<InputEvent> {
    two_finger_scroll_with_step(11.0, 13.0, 32)
}

fn two_finger_scroll_below_threshold() -> Vec<InputEvent> {
    two_finger_scroll_with_step(0.08, 0.08, 12)
}

fn scroll_speed_multiplier_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1700.0, 1100.0), td(0, 1, 2100.0, 1120.0), frame(0)];

    let mut y_offset = 0.0;
    for i in 0..36 {
        let t = 8_000 + i * 8_000;
        let speed_step = 4.0 + (i % 9) as f64 * 3.0;
        let x_wobble = ((i % 4) as f64 - 1.5) * 0.9;
        y_offset += speed_step;
        events.push(tm(t, 0, 1700.0 + x_wobble, 1100.0 + y_offset));
        events.push(tm(t, 1, 2100.0 + x_wobble, 1120.0 + y_offset));
        events.push(frame(t));
    }

    events.push(tu(320_000, 0));
    events.push(tu(320_000, 1));
    events.push(frame(320_000));
    events
}

fn coasting_scroll_flick_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1700.0, 1100.0), td(0, 1, 2100.0, 1120.0), frame(0)];

    for i in 0..14 {
        let t = 8_000 + i * 8_000;
        let dy = i as f64 * (12.0 + i as f64 * 2.4);
        events.push(tm(t, 0, 1700.0, 1100.0 + dy));
        events.push(tm(t, 1, 2100.0, 1120.0 + dy));
        events.push(frame(t));
    }

    events.push(tu(128_000, 0));
    events.push(tu(128_000, 1));
    events.push(frame(128_000));

    for i in 1..12 {
        events.push(frame(128_000 + i * 8_000));
    }

    events
}

fn circular_scroll_candidate() -> Vec<InputEvent> {
    let center_x = 2100.0;
    let center_y = 1300.0;
    let radius_x = 1980.0;
    let radius_y = 980.0;
    let mut events = vec![td(0, 0, center_x + radius_x, center_y), frame(0)];

    for i in 1..=42 {
        let t = i * 8_000;
        let angle = i as f64 * 0.15;
        let x = center_x + radius_x * angle.cos();
        let y = center_y + radius_y * angle.sin();
        events.push(tm(t, 0, x, y));
        events.push(frame(t));
    }

    events.push(tu(360_000, 0));
    events.push(frame(360_000));
    events
}

fn pressure_dependent_motion_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td_meta(
            0,
            0,
            1700.0,
            1200.0,
            Some(18.0),
            Some(12.0),
            Some(9.0),
            Some(TouchToolType::Finger),
            Some(4100),
        ),
        frame(0),
    ];

    for i in 0..30 {
        let t = 8_000 + i * 8_000;
        let pressure = 18.0 + (i % 10) as f64 * 5.0;
        events.push(tm_meta(
            t,
            0,
            1700.0 + i as f64 * 15.0,
            1200.0 + i as f64 * 2.0,
            Some(pressure),
            Some(12.0 + (i % 4) as f64),
            Some(9.0),
            Some(TouchToolType::Finger),
            Some(4100),
        ));
        events.push(frame(t));
    }

    events.push(tu(280_000, 0));
    events.push(frame(280_000));
    events
}

fn edge_motion_continuation_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1500.0, 1200.0), frame(0)];

    for i in 0..18 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 1500.0 + i as f64 * 145.0, 1200.0 + i as f64 * 3.0));
        events.push(frame(t));
    }

    for i in 0..18 {
        let t = 160_000 + i * 8_000;
        events.push(tm(t, 0, 4140.0, 1254.0 + (i % 3) as f64));
        events.push(frame(t));
    }

    events.push(tu(330_000, 0));
    events.push(frame(330_000));
    events
}

fn touchpad_active_rectangle_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 520.0, 520.0), frame(0)];

    for i in 0..34 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 520.0 + i as f64 * 92.0, 520.0 + i as f64 * 35.0));
        events.push(frame(t));
    }

    events.push(tu(300_000, 0));
    events.push(frame(300_000));
    events
}

fn left_handed_touchpad_rotation_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 3550.0, 2140.0), frame(0)];

    for i in 0..28 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 3550.0 - i as f64 * 42.0, 2140.0 - i as f64 * 22.0));
        events.push(frame(t));
    }

    events.push(tu(250_000, 0));
    events.push(frame(250_000));
    events
}

fn touchpad_stream_without_final_lift(mut events: Vec<InputEvent>) -> Vec<InputEvent> {
    if matches!(events.last(), Some(InputEvent::Frame { .. })) {
        events.pop();
    }
    while matches!(events.last(), Some(InputEvent::TouchUp(_))) {
        events.pop();
    }
    events
}

fn touchpad_extension_device_removed(events: Vec<InputEvent>) -> Vec<InputEvent> {
    let mut events = touchpad_stream_without_final_lift(events);
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn touch_event_device_removed(events: Vec<InputEvent>) -> Vec<InputEvent> {
    let mut events = touchpad_stream_without_final_lift(events);
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn touch_event_device_disable_context(events: Vec<InputEvent>) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchscreen_info(),
        events: touchpad_stream_without_final_lift(events),
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench touchscreen", seat, fd);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn touchpad_extension_device_disable_context(events: Vec<InputEvent>) -> u64 {
    gesture_device_disable_context(touchpad_stream_without_final_lift(events))
}

fn touchpad_extension_device_removal_context(events: Vec<InputEvent>) -> u64 {
    gesture_device_removal_context(touchpad_stream_without_final_lift(events))
}

fn right_edge_scroll_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 4095.0, 700.0), frame(0)];

    for i in 0..28 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 4100.0, 700.0 + i as f64 * 22.0));
        events.push(frame(t));
    }

    events.push(tu(250_000, 0));
    events.push(frame(250_000));
    events
}

fn bottom_edge_scroll_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1450.0, 2550.0), frame(0)];

    for i in 0..28 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 1450.0 + i as f64 * 24.0, 2550.0));
        events.push(frame(t));
    }

    events.push(tu(250_000, 0));
    events.push(frame(250_000));
    events
}

fn main_area_crosses_into_right_edge_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 3000.0, 760.0), frame(0)];

    for i in 0..28 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 3000.0 + i as f64 * 44.0, 760.0 + i as f64 * 16.0));
        events.push(frame(t));
    }

    events.push(tu(250_000, 0));
    events.push(frame(250_000));
    events
}

fn right_edge_crosses_into_main_area_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 4100.0, 760.0), frame(0)];

    for i in 0..28 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 4100.0 - i as f64 * 42.0, 760.0 + i as f64 * 20.0));
        events.push(frame(t));
    }

    events.push(tu(250_000, 0));
    events.push(frame(250_000));
    events
}

fn bottom_edge_crosses_into_main_area_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1450.0, 2550.0), frame(0)];

    for i in 0..28 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 1450.0 + i as f64 * 24.0, 2550.0 - i as f64 * 28.0));
        events.push(frame(t));
    }

    events.push(tu(250_000, 0));
    events.push(frame(250_000));
    events
}

fn right_edge_scroll_active_device_removed() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 4100.0, 760.0), frame(0)];

    for i in 0..14 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 4100.0, 760.0 + i as f64 * 34.0));
        events.push(frame(t));
    }

    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn three_finger_swipe_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1450.0, 1200.0),
        td(0, 1, 1850.0, 1205.0),
        td(0, 2, 2250.0, 1195.0),
        frame(0),
    ];

    for i in 0..20 {
        let t = 8_000 + i * 8_000;
        let dx = i as f64 * 26.0;
        let dy = i as f64 * -4.0;
        events.push(tm(t, 0, 1450.0 + dx, 1200.0 + dy));
        events.push(tm(t, 1, 1850.0 + dx, 1205.0 + dy));
        events.push(tm(t, 2, 2250.0 + dx, 1195.0 + dy));
        events.push(frame(t));
    }

    events.push(tu(184_000, 0));
    events.push(tu(184_000, 1));
    events.push(tu(184_000, 2));
    events.push(frame(184_000));
    events
}

fn four_finger_swipe_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1280.0, 1120.0),
        td(0, 1, 1680.0, 1130.0),
        td(0, 2, 2080.0, 1125.0),
        td(0, 3, 2480.0, 1115.0),
        frame(0),
    ];

    for i in 0..20 {
        let t = 8_000 + i * 8_000;
        let dx = i as f64 * -24.0;
        let dy = i as f64 * 7.0;
        events.push(tm(t, 0, 1280.0 + dx, 1120.0 + dy));
        events.push(tm(t, 1, 1680.0 + dx, 1130.0 + dy));
        events.push(tm(t, 2, 2080.0 + dx, 1125.0 + dy));
        events.push(tm(t, 3, 2480.0 + dx, 1115.0 + dy));
        events.push(frame(t));
    }

    for slot in 0..4 {
        events.push(tu(184_000, slot as i32));
    }
    events.push(frame(184_000));
    events
}

fn pinch_zoom_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1800.0, 1300.0), td(0, 1, 2200.0, 1300.0), frame(0)];

    for i in 0..24 {
        let t = 8_000 + i * 8_000;
        let spread = i as f64 * 16.0;
        let center_shift = i as f64 * 2.0;
        events.push(tm(
            t,
            0,
            1800.0 - spread + center_shift,
            1300.0 - spread * 0.20,
        ));
        events.push(tm(
            t,
            1,
            2200.0 + spread + center_shift,
            1300.0 + spread * 0.25,
        ));
        events.push(frame(t));
    }

    events.push(tu(216_000, 0));
    events.push(tu(216_000, 1));
    events.push(frame(216_000));
    events
}

fn pinch_zoom_in_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1600.0, 1300.0), td(0, 1, 2400.0, 1300.0), frame(0)];

    for i in 0..24 {
        let t = 8_000 + i * 8_000;
        let inset = i as f64 * 24.0;
        let center_shift = i as f64 * 1.5;
        events.push(tm(
            t,
            0,
            1600.0 + inset + center_shift,
            1300.0 + inset * 0.12,
        ));
        events.push(tm(
            t,
            1,
            2400.0 - inset + center_shift,
            1300.0 - inset * 0.15,
        ));
        events.push(frame(t));
    }

    events.push(tu(216_000, 0));
    events.push(tu(216_000, 1));
    events.push(frame(216_000));
    events
}

fn pinch_rotate_candidate(direction: f64) -> Vec<InputEvent> {
    let center_x = 2000.0;
    let center_y = 1300.0;
    let radius = 260.0;
    let mut events = vec![
        td(0, 0, center_x - radius, center_y),
        td(0, 1, center_x + radius, center_y),
        frame(0),
    ];

    for i in 1..=22 {
        let t = i * 8_000;
        let angle = direction * i as f64 * 0.14;
        let x = radius * angle.cos();
        let y = radius * angle.sin();
        events.push(tm(t, 0, center_x - x, center_y - y));
        events.push(tm(t, 1, center_x + x, center_y + y));
        events.push(frame(t));
    }

    events.push(tu(200_000, 0));
    events.push(tu(200_000, 1));
    events.push(frame(200_000));
    events
}

fn pinch_rotate_clockwise_candidate() -> Vec<InputEvent> {
    pinch_rotate_candidate(-1.0)
}

fn pinch_rotate_counterclockwise_candidate() -> Vec<InputEvent> {
    pinch_rotate_candidate(1.0)
}

fn hold_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1900.0, 1200.0), frame(0)];

    for i in 1..80 {
        let t = i * 8_000;
        events.push(frame(t));
    }

    events.push(tu(660_000, 0));
    events.push(frame(660_000));
    events
}

fn hold_cancelled_by_motion_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1900.0, 1200.0), frame(0)];

    for i in 1..66 {
        let t = i * 8_000;
        events.push(frame(t));
    }

    events.push(tm(540_000, 0, 2025.0, 1230.0));
    events.push(frame(540_000));
    events.push(tu(572_000, 0));
    events.push(frame(572_000));
    events
}

fn hold_cancelled_by_touch_cancel_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1900.0, 1200.0), frame(0)];

    for i in 1..70 {
        let t = i * 8_000;
        events.push(frame(t));
    }

    events.push(tc(580_000, 0));
    events
}

fn gesture_finger_count_change_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1450.0, 1200.0),
        td(0, 1, 1850.0, 1205.0),
        td(0, 2, 2250.0, 1195.0),
        frame(0),
    ];

    for i in 0..8 {
        let t = 8_000 + i * 8_000;
        let dx = i as f64 * 28.0;
        events.push(tm(t, 0, 1450.0 + dx, 1200.0));
        events.push(tm(t, 1, 1850.0 + dx, 1205.0));
        events.push(tm(t, 2, 2250.0 + dx, 1195.0));
        events.push(frame(t));
    }

    events.push(td(92_000, 3, 2650.0, 1210.0));
    events.push(frame(92_000));
    for slot in 0..4 {
        events.push(tu(124_000, slot as i32));
    }
    events.push(frame(124_000));
    events
}

fn pinch_finger_count_change_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1800.0, 1300.0), td(0, 1, 2200.0, 1300.0), frame(0)];

    for i in 0..8 {
        let t = 8_000 + i * 8_000;
        let spread = i as f64 * 18.0;
        events.push(tm(t, 0, 1800.0 - spread, 1300.0 - spread * 0.15));
        events.push(tm(t, 1, 2200.0 + spread, 1300.0 + spread * 0.20));
        events.push(frame(t));
    }

    events.push(td(88_000, 2, 2000.0, 1620.0));
    events.push(frame(88_000));
    events.push(tu(124_000, 0));
    events.push(tu(124_000, 1));
    events.push(tu(124_000, 2));
    events.push(frame(124_000));
    events
}

fn active_two_finger_pinch_before_disable() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1800.0, 1300.0), td(0, 1, 2200.0, 1300.0), frame(0)];

    for i in 0..14 {
        let t = 8_000 + i * 8_000;
        let spread = i as f64 * 18.0;
        let center_shift = i as f64 * 2.0;
        events.push(tm(
            t,
            0,
            1800.0 - spread + center_shift,
            1300.0 - spread * 0.20,
        ));
        events.push(tm(
            t,
            1,
            2200.0 + spread + center_shift,
            1300.0 + spread * 0.25,
        ));
        events.push(frame(t));
    }

    events
}

fn active_two_finger_pinch_device_removed() -> Vec<InputEvent> {
    let mut events = active_two_finger_pinch_before_disable();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn active_hold_before_disable() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1900.0, 1200.0), frame(0)];

    for i in 1..70 {
        let t = i * 8_000;
        events.push(frame(t));
    }

    events
}

fn active_hold_device_removed() -> Vec<InputEvent> {
    let mut events = active_hold_before_disable();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn active_three_finger_swipe_before_disable() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1450.0, 1200.0),
        td(0, 1, 1850.0, 1205.0),
        td(0, 2, 2250.0, 1195.0),
        frame(0),
    ];

    for i in 0..10 {
        let t = 8_000 + i * 8_000;
        let dx = i as f64 * 30.0;
        let dy = i as f64 * -5.0;
        events.push(tm(t, 0, 1450.0 + dx, 1200.0 + dy));
        events.push(tm(t, 1, 1850.0 + dx, 1205.0 + dy));
        events.push(tm(t, 2, 2250.0 + dx, 1195.0 + dy));
        events.push(frame(t));
    }

    events
}

fn active_three_finger_swipe_device_removed() -> Vec<InputEvent> {
    let mut events = active_three_finger_swipe_before_disable();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn palm_contact_during_active_gesture_candidate() -> Vec<InputEvent> {
    let mut events = active_three_finger_swipe_before_disable();
    events.push(td_meta(
        92_000,
        3,
        4020.0,
        1180.0,
        Some(62.0),
        Some(22.0),
        Some(16.0),
        Some(TouchToolType::Palm),
        Some(77_003),
    ));
    events.push(frame(92_000));

    for i in 0..4 {
        let t = 100_000 + i * 8_000;
        let dx = i as f64 * 22.0;
        events.push(tm(t, 0, 1750.0 + dx, 1150.0));
        events.push(tm(t, 1, 2150.0 + dx, 1155.0));
        events.push(tm(t, 2, 2550.0 + dx, 1145.0));
        events.push(tm_meta(
            t,
            3,
            4020.0,
            1180.0 + i as f64,
            Some(64.0),
            Some(22.0),
            Some(16.0),
            Some(TouchToolType::Palm),
            Some(77_003),
        ));
        events.push(frame(t));
    }

    for slot in 0..4 {
        events.push(tu(150_000, slot));
    }
    events.push(frame(150_000));
    events
}

fn single_finger_pointer_motion_candidate() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 1800.0, 1220.0), frame(0)];

    for i in 0..24 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 1800.0 + i as f64 * 18.0, 1220.0 + i as f64 * 2.0));
        events.push(frame(t));
    }

    events.push(tu(220_000, 0));
    events.push(frame(220_000));
    events
}

fn gesture_device_disable_context(events: Vec<InputEvent>) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchpad_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench laptop touchpad", seat, fd);
    ctx.configure_device_filters(device);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn gesture_device_removal_context(events: Vec<InputEvent>) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchpad_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench laptop touchpad", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    ctx.configure_device_filters(device);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn active_gesture_device_disable_context() -> u64 {
    gesture_device_disable_context(active_three_finger_swipe_before_disable())
}

fn palm_edge_contact() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 60.0, 1100.0), frame(0)];

    for i in 0..16 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 60.0 + i as f64 * 2.0, 1100.0 + i as f64 * 8.0));
        events.push(frame(t));
    }

    events.push(tu(150_000, 0));
    events.push(frame(150_000));
    events
}

fn thumb_rest_contact() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1780.0, 1180.0),
        frame(0),
        td(40_000, 1, 2600.0, 2380.0),
        frame(40_000),
    ];

    for i in 0..24 {
        let t = 48_000 + i * 8_000;
        events.push(tm(t, 0, 1780.0 + i as f64 * 15.0, 1180.0 + i as f64 * 2.0));
        events.push(tm(t, 1, 2600.0 + i as f64 * 1.0, 2380.0 + i as f64 * 0.5));
        events.push(frame(t));
    }

    events.push(tu(260_000, 1));
    events.push(frame(260_000));
    events.push(tu(268_000, 0));
    events.push(frame(268_000));
    events
}

fn thumb_only_lower_zone_contact() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 2560.0, 2380.0), frame(0)];

    for i in 0..18 {
        let t = 8_000 + i * 8_000;
        events.push(tm(t, 0, 2560.0 + i as f64 * 4.0, 2380.0 + i as f64 * 1.5));
        events.push(frame(t));
    }

    events.push(tu(180_000, 0));
    events.push(frame(180_000));
    events
}

fn dwt_suppressed_motion() -> Vec<InputEvent> {
    let mut events = vec![td(130_000, 0, 1900.0, 1250.0), frame(130_000)];

    for i in 0..18 {
        let t = 138_000 + i * 8_000;
        events.push(tm(t, 0, 1900.0 + i as f64 * 13.0, 1250.0));
        events.push(frame(t));
    }

    events.push(tu(300_000, 0));
    events.push(frame(300_000));
    events
}

fn middle_chord_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(18_000, BTN_RIGHT, true),
        pointer_motion(26_000, 2.0, 0.0),
        pointer_button(74_000, BTN_LEFT, false),
        pointer_button(82_000, BTN_RIGHT, false),
    ]
}

fn middle_right_left_chord_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_RIGHT, true),
        pointer_button(18_000, BTN_LEFT, true),
        pointer_motion(26_000, -2.0, 0.0),
        pointer_button(74_000, BTN_RIGHT, false),
        pointer_button(82_000, BTN_LEFT, false),
    ]
}

fn middle_timeout_left_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        frame(60_000),
        pointer_motion(68_000, 4.0, 0.0),
        pointer_button(96_000, BTN_LEFT, false),
    ]
}

fn middle_timeout_right_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_RIGHT, true),
        frame(60_000),
        pointer_motion(68_000, -4.0, 0.0),
        pointer_button(96_000, BTN_RIGHT, false),
    ]
}

fn middle_chord_release_second_button_first_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(18_000, BTN_RIGHT, true),
        pointer_motion(34_000, 0.0, 1.5),
        pointer_button(74_000, BTN_RIGHT, false),
        pointer_button(82_000, BTN_LEFT, false),
    ]
}

fn middle_chord_bouncy_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(6_000, BTN_LEFT, false),
        pointer_button(9_000, BTN_LEFT, true),
        pointer_button(18_000, BTN_RIGHT, true),
        pointer_motion(42_000, 0.0, -2.0),
        pointer_button(80_000, BTN_RIGHT, false),
        pointer_button(88_000, BTN_LEFT, false),
    ]
}

fn middle_chord_active_buttons_only() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(18_000, BTN_RIGHT, true),
    ]
}

fn middle_chord_pending_device_removed() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ]
}

fn middle_chord_active_device_removed() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(18_000, BTN_RIGHT, true),
        pointer_motion(34_000, 1.0, -1.0),
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ]
}

fn middle_chord_pending_button_only() -> Vec<InputEvent> {
    vec![pointer_button(0, BTN_LEFT, true)]
}

fn debounce_bouncy_button_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(4_000, BTN_LEFT, false),
        pointer_button(8_000, BTN_LEFT, true),
        pointer_button(84_000, BTN_LEFT, false),
    ]
}

fn debounce_spurious_double_click_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(78_000, BTN_LEFT, false),
        pointer_button(84_000, BTN_LEFT, true),
        pointer_button(90_000, BTN_LEFT, false),
        pointer_button(140_000, BTN_LEFT, true),
        pointer_button(214_000, BTN_LEFT, false),
    ]
}

fn debounce_independent_buttons_candidate() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(5_000, BTN_LEFT, false),
        pointer_button(8_000, BTN_RIGHT, true),
        pointer_button(12_000, BTN_LEFT, true),
        pointer_button(72_000, BTN_RIGHT, false),
        pointer_button(96_000, BTN_LEFT, false),
    ]
}

fn debounce_keyboard_key_candidate() -> Vec<InputEvent> {
    vec![
        key(0, KEY_A, true),
        key(3_000, KEY_A, false),
        key(7_000, KEY_A, true),
        key(88_000, KEY_A, false),
        key(130_000, KEY_A, true),
        key(190_000, KEY_A, false),
    ]
}

fn debounce_bouncy_pending_before_cleanup() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(4_000, BTN_LEFT, false),
    ]
}

fn debounce_bouncy_pending_device_removed() -> Vec<InputEvent> {
    let mut events = debounce_bouncy_pending_before_cleanup();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn debounce_spurious_pending_before_cleanup() -> Vec<InputEvent> {
    vec![
        pointer_button(0, BTN_LEFT, true),
        pointer_button(78_000, BTN_LEFT, false),
        pointer_button(84_000, BTN_LEFT, true),
    ]
}

fn debounce_spurious_pending_device_removed() -> Vec<InputEvent> {
    let mut events = debounce_spurious_pending_before_cleanup();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn debounce_device_disable_context(events: Vec<InputEvent>, config: DebounceConfig) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    ctx.configure_device_filters(device);
    ctx.set_debounce(device, config);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn debounce_device_removal_context(events: Vec<InputEvent>, config: DebounceConfig) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    ctx.configure_device_filters(device);
    ctx.set_debounce(device, config);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn middle_button_device_disable_context(events: Vec<InputEvent>) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    ctx.configure_device_filters(device);
    ctx.set_middle_button_emulation(device, true);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn middle_button_device_removal_context(events: Vec<InputEvent>) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    ctx.configure_device_filters(device);
    ctx.set_middle_button_emulation(device, true);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn button_scroll_candidate() -> Vec<InputEvent> {
    button_scroll_candidate_for_button(BTN_MIDDLE)
}

fn right_button_scroll_candidate() -> Vec<InputEvent> {
    button_scroll_candidate_for_button(BTN_RIGHT)
}

fn button_scroll_candidate_for_button(button: u32) -> Vec<InputEvent> {
    let mut events = vec![pointer_button(0, button, true)];
    for i in 0..40 {
        let t = 8_000 + i * 8_000;
        events.push(pointer_motion(t, 0.0, -3.0 - (i % 4) as f64));
    }
    events.push(pointer_button(340_000, button, false));
    events
}

fn button_scroll_lock_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        pointer_button(0, BTN_MIDDLE, true),
        pointer_button(18_000, BTN_MIDDLE, false),
    ];

    for i in 0..32 {
        let t = 34_000 + i * 8_000;
        events.push(pointer_motion(t, 0.4 + (i % 3) as f64 * 0.2, -2.5));
    }

    events.push(pointer_button(310_000, BTN_MIDDLE, true));
    events.push(pointer_button(326_000, BTN_MIDDLE, false));
    events.push(pointer_motion(350_000, 4.0, 0.0));
    events
}

fn button_scroll_hold_active_before_disable() -> Vec<InputEvent> {
    let mut events = vec![pointer_button(0, BTN_MIDDLE, true)];
    for i in 0..18 {
        let t = 8_000 + i * 8_000;
        events.push(pointer_motion(t, 0.0, -3.0 - (i % 4) as f64));
    }
    events
}

fn button_scroll_lock_active_before_disable() -> Vec<InputEvent> {
    let mut events = vec![
        pointer_button(0, BTN_MIDDLE, true),
        pointer_button(18_000, BTN_MIDDLE, false),
    ];

    for i in 0..18 {
        let t = 34_000 + i * 8_000;
        events.push(pointer_motion(t, 0.4 + (i % 3) as f64 * 0.2, -2.5));
    }

    events
}

fn button_scroll_lock_active_device_removed() -> Vec<InputEvent> {
    let mut events = button_scroll_lock_active_before_disable();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn button_scroll_hold_active_device_removed() -> Vec<InputEvent> {
    let mut events = button_scroll_hold_active_before_disable();
    events.push(InputEvent::DeviceRemoved {
        device_index: DevIndex(0),
    });
    events
}

fn button_scroll_device_disable_context(events: Vec<InputEvent>, lock_enabled: bool) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    ctx.configure_device_filters(device);
    ctx.set_button_scroll(device, true, BTN_MIDDLE, lock_enabled);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn button_scroll_device_removal_context(events: Vec<InputEvent>, lock_enabled: bool) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    ctx.configure_device_filters(device);
    ctx.set_button_scroll(device, true, BTN_MIDDLE, lock_enabled);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn touchpad_scroll_device_disable_context(
    events: Vec<InputEvent>,
    method: TouchpadScrollMethod,
) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchpad_info(),
        events: touchpad_stream_without_final_lift(events),
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench laptop touchpad", seat, fd);
    ctx.configure_device_filters(device);
    ctx.set_scroll_method(device, method);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn touchpad_scroll_device_removal_context(
    events: Vec<InputEvent>,
    method: TouchpadScrollMethod,
) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchpad_info(),
        events: touchpad_stream_without_final_lift(events),
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench laptop touchpad", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    ctx.configure_device_filters(device);
    ctx.set_scroll_method(device, method);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn single_touch_fallback_events() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 960.0, 540.0), frame(0)];

    for i in 0..36 {
        let time_us = 8_000 + i * 8_000;
        events.push(tm(
            time_us,
            0,
            960.0 + i as f64 * 11.0,
            540.0 + (i as f64 * 0.35).sin() * 20.0,
        ));
        events.push(frame(time_us));
    }

    events.push(tu(310_000, 0));
    events.push(frame(310_000));
    events
}

fn touch_cancel_sequence() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 720.0, 420.0), frame(0)];

    for i in 0..18 {
        let time_us = 8_000 + i * 8_000;
        events.push(tm(
            time_us,
            0,
            720.0 + i as f64 * 9.0,
            420.0 + (i as f64 * 0.45).sin() * 18.0,
        ));
        events.push(frame(time_us));
    }

    events.push(tc(180_000, 0));
    events
}

fn mt_b_down_motion_up_sequence() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 760.0, 420.0), td(0, 1, 1180.0, 430.0), frame(0)];

    for i in 0..24 {
        let time_us = 8_000 + i * 8_000;
        events.push(tm(
            time_us,
            0,
            760.0 + i as f64 * 8.0,
            420.0 + i as f64 * 3.0,
        ));
        events.push(tm(
            time_us,
            1,
            1180.0 + i as f64 * 7.0,
            430.0 + i as f64 * 3.5,
        ));
        events.push(frame(time_us));
    }

    events.push(tu(220_000, 0));
    events.push(tu(228_000, 1));
    events.push(frame(228_000));
    events
}

fn multi_slot_same_frame_sequence() -> Vec<InputEvent> {
    vec![
        td(0, 0, 900.0, 500.0),
        td(0, 1, 1260.0, 510.0),
        td(0, 2, 1620.0, 505.0),
        frame(0),
        tm(12_000, 0, 920.0, 518.0),
        tm(12_000, 1, 1280.0, 528.0),
        tm(12_000, 2, 1640.0, 523.0),
        frame(12_000),
        tm(24_000, 0, 944.0, 540.0),
        tm(24_000, 1, 1304.0, 548.0),
        tm(24_000, 2, 1664.0, 544.0),
        frame(24_000),
        tu(52_000, 0),
        tu(52_000, 1),
        tu(52_000, 2),
        frame(52_000),
    ]
}

fn slot_reuse_after_up_sequence() -> Vec<InputEvent> {
    vec![
        td(0, 0, 720.0, 420.0),
        frame(0),
        tm(16_000, 0, 760.0, 450.0),
        frame(16_000),
        tu(48_000, 0),
        frame(48_000),
        td(110_000, 0, 1320.0, 740.0),
        frame(110_000),
        tm(126_000, 0, 1375.0, 780.0),
        frame(126_000),
        tu(170_000, 0),
        frame(170_000),
    ]
}

fn invalid_calibration_matrix_config_sequence() -> Vec<[f64; 6]> {
    vec![
        [f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0],
        [1.0, 0.0, f64::INFINITY, 0.0, 1.0, 0.0],
        [1.0, 0.0, 0.0, 0.0, f64::NEG_INFINITY, 1.0],
        [0.0, 1.0, 0.0, f64::NAN, 0.0, 1.0],
        [1.0, 0.0, -0.15, 0.0, 1.0, f64::INFINITY],
        [0.75, 0.0, 0.10, 0.0, 0.80, 0.05],
    ]
}

fn touch_pressure_size_tool_metadata_sequence() -> Vec<InputEvent> {
    let mut events = vec![
        td_meta(
            0,
            0,
            840.0,
            520.0,
            Some(24.0),
            Some(9.0),
            Some(6.0),
            Some(TouchToolType::Finger),
            Some(41_000),
        ),
        frame(0),
    ];

    for i in 1..36 {
        let time_us = i * 8_000;
        let f = i as f64 / 35.0;
        let pressure = 24.0 + (f * std::f64::consts::PI).sin() * 18.0;
        let major = 9.0 + f * 4.0;
        let minor = 6.0 + (1.0 - f) * 1.5;
        events.push(tm_meta(
            time_us,
            0,
            840.0 + f * 620.0,
            520.0 + (f * std::f64::consts::TAU).sin() * 45.0,
            Some(pressure),
            Some(major),
            Some(minor),
            Some(TouchToolType::Finger),
            Some(41_000),
        ));
        events.push(frame(time_us));
    }

    events.push(tu(304_000, 0));
    events.push(frame(304_000));
    events
}

fn tracking_id_change_candidate_sequence() -> Vec<InputEvent> {
    vec![
        td_meta(
            0,
            0,
            920.0,
            540.0,
            Some(18.0),
            Some(8.0),
            Some(5.0),
            Some(TouchToolType::Finger),
            Some(71_000),
        ),
        frame(0),
        tm_meta(
            18_000,
            0,
            960.0,
            570.0,
            Some(22.0),
            Some(9.0),
            Some(5.0),
            Some(TouchToolType::Finger),
            Some(71_001),
        ),
        frame(18_000),
        tu(48_000, 0),
        frame(48_000),
    ]
}

fn firmware_palm_tool_metadata_sequence() -> Vec<InputEvent> {
    let mut events = vec![
        td(0, 0, 1550.0, 1180.0),
        frame(0),
        tm(16_000, 0, 1585.0, 1190.0),
        frame(16_000),
        td_meta(
            32_000,
            1,
            2140.0,
            2460.0,
            Some(95.0),
            Some(34.0),
            Some(22.0),
            Some(TouchToolType::Palm),
            Some(51_200),
        ),
        frame(32_000),
    ];

    for i in 1..16 {
        let time_us = 32_000 + i * 10_000;
        let f = i as f64 / 15.0;
        events.push(tm(time_us, 0, 1585.0 + f * 240.0, 1190.0 + f * 40.0));
        events.push(tm_meta(
            time_us,
            1,
            2140.0 + f * 18.0,
            2460.0 + f * 8.0,
            Some(95.0 + f * 8.0),
            Some(34.0 + f * 2.0),
            Some(22.0),
            Some(TouchToolType::Palm),
            Some(51_200),
        ));
        events.push(frame(time_us));
    }

    events.push(tu(210_000, 1));
    events.push(frame(210_000));
    events.push(tu(232_000, 0));
    events.push(frame(232_000));
    events
}

fn pressure_size_palm_candidate_sequence() -> Vec<InputEvent> {
    let mut events = vec![
        td_meta(
            0,
            0,
            1880.0,
            1220.0,
            Some(118.0),
            Some(42.0),
            Some(28.0),
            Some(TouchToolType::Finger),
            Some(61_000),
        ),
        frame(0),
    ];

    for i in 1..18 {
        let time_us = i * 10_000;
        let f = i as f64 / 17.0;
        events.push(tm_meta(
            time_us,
            0,
            1880.0 + f * 180.0,
            1220.0 + f * 50.0,
            Some(118.0 + f * 12.0),
            Some(42.0 + f * 3.0),
            Some(28.0),
            Some(TouchToolType::Finger),
            Some(61_000),
        ));
        events.push(frame(time_us));
    }

    events.push(tu(210_000, 0));
    events.push(frame(210_000));
    events
}

fn late_firmware_palm_reclassification_sequence() -> Vec<InputEvent> {
    let mut events = vec![
        td_meta(
            0,
            0,
            1720.0,
            1160.0,
            Some(24.0),
            Some(10.0),
            Some(7.0),
            Some(TouchToolType::Finger),
            Some(62_000),
        ),
        frame(0),
        tm_meta(
            28_000,
            0,
            1750.0,
            1170.0,
            Some(96.0),
            Some(38.0),
            Some(25.0),
            Some(TouchToolType::Palm),
            Some(62_000),
        ),
        frame(28_000),
    ];

    for i in 1..14 {
        let time_us = 28_000 + i * 10_000;
        let f = i as f64 / 13.0;
        events.push(tm_meta(
            time_us,
            0,
            1750.0 + f * 210.0,
            1170.0 + f * 45.0,
            Some(96.0 + f * 8.0),
            Some(38.0 + f * 2.0),
            Some(25.0),
            Some(TouchToolType::Palm),
            Some(62_000),
        ));
        events.push(frame(time_us));
    }

    events.push(tu(190_000, 0));
    events.push(frame(190_000));
    events
}

fn stylus_touch_arbitration_candidate() -> Vec<InputEvent> {
    vec![
        tablet_proximity(0, 1180.0, 900.0, true),
        tablet_axis(12_000, 1195.0, 912.0, 0.0, -4.0, 7.0),
        InputEvent::TouchDown(Box::new(TouchData {
            device_index: DevIndex(1),
            time_us: 24_000,
            slot: 0,
            x: 640.0,
            y: 420.0,
            pressure: Some(12.0),
            touch_major: Some(9.0),
            touch_minor: Some(6.0),
            tool_type: Some(TouchToolType::Finger),
            tracking_id: Some(72_100),
        })),
        frame(24_000),
        tablet_tip(44_000, true),
        tablet_axis(58_000, 1240.0, 930.0, 0.30, -3.0, 7.0),
        InputEvent::TouchMotion(Box::new(TouchData {
            device_index: DevIndex(1),
            time_us: 70_000,
            slot: 0,
            x: 660.0,
            y: 432.0,
            pressure: Some(18.0),
            touch_major: Some(10.0),
            touch_minor: Some(6.5),
            tool_type: Some(TouchToolType::Finger),
            tracking_id: Some(72_100),
        })),
        frame(70_000),
        tablet_tip(96_000, false),
        InputEvent::TouchUp(Box::new(TouchEndData {
            device_index: DevIndex(1),
            time_us: 112_000,
            slot: 0,
        })),
        frame(112_000),
        tablet_proximity(132_000, 1240.0, 930.0, false),
    ]
}

fn tablet_stroke_and_pad() -> Vec<InputEvent> {
    let mut events = vec![
        tablet_proximity(0, 1200.0, 900.0, true),
        tablet_tip(18_000, true),
    ];

    for i in 0..96 {
        let t = 26_000 + i * 5_000;
        let f = i as f64 / 95.0;
        events.push(tablet_axis(
            t,
            1200.0 + f * 2600.0,
            900.0 + (f * std::f64::consts::TAU).sin() * 260.0,
            (0.08 + f * 0.72).min(1.0),
            -12.0 + f * 24.0,
            18.0 - f * 9.0,
        ));
    }

    events.push(tablet_button(420_000, 1, true));
    events.push(tablet_button(455_000, 1, false));
    events.push(tablet_tip(520_000, false));
    events.push(tablet_proximity(545_000, 3800.0, 900.0, false));
    events.push(tablet_pad_button(570_000, 0, true));
    events.push(tablet_pad_ring(590_000, 0, 0.25));
    events.push(tablet_pad_strip(610_000, 0, 0.75));
    events.push(tablet_pad_dial(630_000, 0, 1.0));
    events.push(tablet_pad_button(650_000, 0, false));
    events
}

fn tablet_pressure_range_candidate() -> Vec<InputEvent> {
    let mut events = vec![
        tablet_proximity(0, 1400.0, 850.0, true),
        tablet_tip(16_000, true),
    ];

    for i in 0..64 {
        let t = 24_000 + i * 6_000;
        let f = i as f64 / 63.0;
        let pressure = if i < 32 {
            f * 1.35
        } else {
            (1.35 - f * 0.70).max(0.0)
        };
        events.push(tablet_axis(
            t,
            1400.0 + f * 1800.0,
            850.0 + (f * std::f64::consts::TAU).sin() * 160.0,
            pressure,
            -8.0 + f * 16.0,
            10.0 - f * 6.0,
        ));
    }

    events.push(tablet_tip(430_000, false));
    events.push(tablet_proximity(456_000, 3200.0, 850.0, false));
    events
}

fn tablet_tool_identity_cycles_candidate() -> Vec<InputEvent> {
    vec![
        tablet_proximity(0, 1200.0, 900.0, true),
        tablet_axis(12_000, 1210.0, 906.0, 0.10, -4.0, 8.0),
        tablet_tip(30_000, true),
        tablet_axis(54_000, 1320.0, 960.0, 0.45, -3.0, 7.0),
        tablet_tip(86_000, false),
        tablet_proximity(110_000, 1320.0, 960.0, false),
        tablet_proximity(420_000, 1280.0, 920.0, true),
        tablet_axis(438_000, 1300.0, 928.0, 0.12, -4.0, 8.0),
        tablet_tip(466_000, true),
        tablet_axis(492_000, 1425.0, 1010.0, 0.55, -2.0, 6.0),
        tablet_button(520_000, 1, true),
        tablet_button(548_000, 1, false),
        tablet_tip(590_000, false),
        tablet_proximity(620_000, 1425.0, 1010.0, false),
    ]
}

fn tablet_tool_button_held_until_proximity_out_candidate() -> Vec<InputEvent> {
    vec![
        tablet_proximity(0, 1500.0, 920.0, true),
        tablet_axis(14_000, 1518.0, 930.0, 0.0, -5.0, 8.0),
        tablet_button(32_000, 1, true),
        tablet_tip(54_000, true),
        tablet_axis(82_000, 1660.0, 990.0, 0.42, -4.0, 7.0),
        tablet_tip(124_000, false),
        tablet_axis(150_000, 1680.0, 1000.0, 0.0, -4.0, 7.0),
        tablet_proximity(176_000, 1680.0, 1000.0, false),
        tablet_button(196_000, 1, false),
    ]
}

fn tablet_tool_active_device_removed_candidate() -> Vec<InputEvent> {
    vec![
        tablet_proximity(0, 1510.0, 940.0, true),
        tablet_axis(16_000, 1530.0, 952.0, 0.08, -6.0, 7.0),
        tablet_tip(34_000, true),
        tablet_button(52_000, 1, true),
        tablet_axis(82_000, 1595.0, 990.0, 0.64, -4.0, 6.0),
        tablet_axis(112_000, 1645.0, 1025.0, 0.72, -3.0, 6.0),
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ]
}

fn tablet_pad_without_tool_proximity_candidate() -> Vec<InputEvent> {
    vec![
        tablet_pad_button(0, 0, true),
        tablet_pad_ring(18_000, 0, 0.10),
        tablet_pad_ring(42_000, 0, 0.35),
        tablet_pad_strip(66_000, 0, 0.80),
        tablet_pad_dial(90_000, 0, 1.0),
        tablet_pad_dial(114_000, 0, -1.0),
        tablet_pad_button(138_000, 0, false),
    ]
}

fn tablet_pad_active_device_removed_candidate() -> Vec<InputEvent> {
    vec![
        tablet_pad_button(0, 0, true),
        tablet_pad_button(20_000, 3, true),
        tablet_pad_ring(44_000, 0, 0.18),
        tablet_pad_strip(68_000, 1, 0.72),
        tablet_pad_dial(92_000, 0, 1.0),
        tablet_pad_dial(116_000, 0, 1.0),
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ]
}

fn tablet_separate_tool_pad_touch_nodes_candidate() -> Vec<InputEvent> {
    vec![
        InputEvent::TabletToolProximity(Box::new(TabletToolProximityData {
            device_index: DevIndex(0),
            time_us: 0,
            x: 1100.0,
            y: 880.0,
            in_range: true,
        })),
        InputEvent::TabletToolAxis(Box::new(TabletToolAxisData {
            device_index: DevIndex(0),
            time_us: 18_000,
            x: 1160.0,
            y: 910.0,
            pressure: 0.20,
            tilt_x: -6.0,
            tilt_y: 9.0,
        })),
        InputEvent::TabletToolTip(Box::new(TabletToolTipData {
            device_index: DevIndex(0),
            time_us: 30_000,
            down: true,
        })),
        InputEvent::TabletPadButton(Box::new(TabletPadButtonData {
            device_index: DevIndex(1),
            time_us: 46_000,
            index: 0,
            pressed: true,
        })),
        InputEvent::TabletPadRing(Box::new(TabletPadValueData {
            device_index: DevIndex(1),
            time_us: 62_000,
            index: 0,
            position: 0.33,
        })),
        InputEvent::TouchDown(Box::new(TouchData {
            device_index: DevIndex(2),
            time_us: 74_000,
            slot: 0,
            x: 640.0,
            y: 420.0,
            pressure: Some(18.0),
            touch_major: Some(7.0),
            touch_minor: Some(5.0),
            tool_type: Some(TouchToolType::Finger),
            tracking_id: Some(62_001),
        })),
        InputEvent::TouchMotion(Box::new(TouchData {
            device_index: DevIndex(2),
            time_us: 92_000,
            slot: 0,
            x: 700.0,
            y: 455.0,
            pressure: Some(22.0),
            touch_major: Some(7.5),
            touch_minor: Some(5.2),
            tool_type: Some(TouchToolType::Finger),
            tracking_id: Some(62_001),
        })),
        InputEvent::TouchUp(Box::new(TouchEndData {
            device_index: DevIndex(2),
            time_us: 124_000,
            slot: 0,
        })),
        InputEvent::TabletPadButton(Box::new(TabletPadButtonData {
            device_index: DevIndex(1),
            time_us: 140_000,
            index: 0,
            pressed: false,
        })),
        InputEvent::TabletToolTip(Box::new(TabletToolTipData {
            device_index: DevIndex(0),
            time_us: 166_000,
            down: false,
        })),
        InputEvent::TabletToolProximity(Box::new(TabletToolProximityData {
            device_index: DevIndex(0),
            time_us: 190_000,
            x: 1160.0,
            y: 910.0,
            in_range: false,
        })),
    ]
}

fn tablet_touch_node_removed_while_group_active_candidate() -> Vec<InputEvent> {
    vec![
        InputEvent::TabletToolProximity(Box::new(TabletToolProximityData {
            device_index: DevIndex(0),
            time_us: 0,
            x: 1180.0,
            y: 860.0,
            in_range: true,
        })),
        InputEvent::TabletToolAxis(Box::new(TabletToolAxisData {
            device_index: DevIndex(0),
            time_us: 18_000,
            x: 1205.0,
            y: 882.0,
            pressure: 0.0,
            tilt_x: -7.0,
            tilt_y: 10.0,
        })),
        InputEvent::TabletPadButton(Box::new(TabletPadButtonData {
            device_index: DevIndex(1),
            time_us: 30_000,
            index: 1,
            pressed: true,
        })),
        InputEvent::TouchDown(Box::new(TouchData {
            device_index: DevIndex(2),
            time_us: 44_000,
            slot: 0,
            x: 590.0,
            y: 390.0,
            pressure: Some(15.0),
            touch_major: Some(6.5),
            touch_minor: Some(4.4),
            tool_type: Some(TouchToolType::Finger),
            tracking_id: Some(72_001),
        })),
        InputEvent::TouchMotion(Box::new(TouchData {
            device_index: DevIndex(2),
            time_us: 70_000,
            slot: 0,
            x: 642.0,
            y: 420.0,
            pressure: Some(19.0),
            touch_major: Some(7.1),
            touch_minor: Some(4.9),
            tool_type: Some(TouchToolType::Finger),
            tracking_id: Some(72_001),
        })),
        InputEvent::DeviceRemoved {
            device_index: DevIndex(2),
        },
        InputEvent::TabletPadButton(Box::new(TabletPadButtonData {
            device_index: DevIndex(1),
            time_us: 112_000,
            index: 1,
            pressed: false,
        })),
        InputEvent::TabletToolProximity(Box::new(TabletToolProximityData {
            device_index: DevIndex(0),
            time_us: 140_000,
            x: 1205.0,
            y: 882.0,
            in_range: false,
        })),
    ]
}

fn wheel_scroll_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::PointerScroll {
            device_index: DevIndex(0),
            time_us: 0,
            source: ScrollSource::Wheel,
            dx: 0.0,
            dy: -1.0,
        },
        frame(0),
        InputEvent::PointerScroll {
            device_index: DevIndex(0),
            time_us: 24_000,
            source: ScrollSource::Wheel,
            dx: 0.5,
            dy: -1.0,
        },
        frame(24_000),
        InputEvent::PointerScroll {
            device_index: DevIndex(0),
            time_us: 48_000,
            source: ScrollSource::Wheel,
            dx: -0.5,
            dy: 1.0,
        },
        frame(48_000),
    ]
}

fn switch_laptop_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::Lid,
            enabled: false,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 1_200_000,
            switch: SwitchKind::TabletMode,
            enabled: true,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 3_400_000,
            switch: SwitchKind::TabletMode,
            enabled: false,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 6_000_000,
            switch: SwitchKind::Lid,
            enabled: true,
        },
    ]
}

fn unknown_switch_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::Unknown(12),
            enabled: false,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 750_000,
            switch: SwitchKind::Unknown(12),
            enabled: true,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 1_500_000,
            switch: SwitchKind::Unknown(12),
            enabled: false,
        },
    ]
}

fn initial_switch_state_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::Lid,
            enabled: true,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::TabletMode,
            enabled: false,
        },
    ]
}

fn active_switch_device_removal_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::TabletMode,
            enabled: true,
        },
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ]
}

fn rapid_switch_toggle_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::TabletMode,
            enabled: true,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 110_000,
            switch: SwitchKind::TabletMode,
            enabled: false,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 230_000,
            switch: SwitchKind::TabletMode,
            enabled: true,
        },
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 1_800_000,
            switch: SwitchKind::TabletMode,
            enabled: false,
        },
    ]
}

fn unknown_active_switch_device_removal_sequence() -> Vec<InputEvent> {
    vec![
        InputEvent::SwitchToggle {
            device_index: DevIndex(0),
            time_us: 0,
            switch: SwitchKind::Unknown(12),
            enabled: true,
        },
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ]
}

fn absolute_transform_sequence() -> Vec<InputEvent> {
    let mut events = vec![td(0, 0, 40.0, 40.0), frame(0), tu(24_000, 0), frame(24_000)];

    for i in 0..64 {
        let time_us = 80_000 + i * 8_000;
        let x = 300.0 + i as f64 * 22.0;
        let y = 180.0 + i as f64 * 13.0;
        if i == 0 {
            events.push(td(time_us, 1, x, y));
        } else {
            events.push(tm(time_us, 1, x, y));
        }
        events.push(frame(time_us));
    }

    events.push(tu(610_000, 1));
    events.push(frame(610_000));
    events
}

fn tablet_active_area_candidate_sequence() -> Vec<InputEvent> {
    let mut events = vec![
        tablet_proximity(0, 520.0, 340.0, true),
        tablet_tip(18_000, true),
    ];

    for i in 0..72 {
        let time_us = 26_000 + i * 6_000;
        let f = i as f64 / 71.0;
        events.push(tablet_axis(
            time_us,
            520.0 + f * 3600.0,
            340.0 + f * 1760.0,
            (0.12 + f * 0.62).min(1.0),
            -9.0 + f * 18.0,
            11.0 - f * 8.0,
        ));
    }

    events.push(tablet_tip(476_000, false));
    events.push(tablet_proximity(504_000, 4120.0, 2100.0, false));
    events
}

fn relative_rotation_candidate_sequence() -> Vec<InputEvent> {
    let mut events = Vec::new();

    for i in 0..72 {
        let time_us = i * 8_000;
        let phase = i % 36;
        let (dx, dy) = if phase < 18 { (1.6, 0.2) } else { (0.2, 1.6) };
        events.push(pointer_motion(time_us, dx, dy));
        events.push(frame(time_us));
    }

    events
}

fn device_policy_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 0,
            vendor_id: 0x0001,
            product_id: 0x0001,
            bus: BusType::Ps2,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 8_000,
            vendor_id: 0x06cb,
            product_id: 0x0001,
            bus: BusType::I2c,
            internal: true,
        }),
        DevicePolicyEvent::SendEvents {
            time_us: 30_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::Led {
            time_us: 48_000,
            caps_lock: true,
            num_lock: false,
            scroll_lock: false,
        },
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 1_000_000,
            vendor_id: 0x046d,
            product_id: 0xc539,
            bus: BusType::Usb,
            internal: false,
        }),
        DevicePolicyEvent::SendEvents {
            time_us: 1_006_000,
            internal: true,
            external_mouse_present: true,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 4_000_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn representative_metadata_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 0,
            vendor_id: 0x046d,
            product_id: 0xc539,
            bus: BusType::Usb,
            internal: false,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 16_000,
            vendor_id: 0x05ac,
            product_id: 0x0267,
            bus: BusType::Bluetooth,
            internal: false,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 32_000,
            vendor_id: 0x06cb,
            product_id: 0x0001,
            bus: BusType::I2c,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 48_000,
            vendor_id: 0x0001,
            product_id: 0x0001,
            bus: BusType::Ps2,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 64_000,
            vendor_id: 0x0000,
            product_id: 0x0000,
            bus: BusType::Unknown,
            internal: false,
        }),
    ]
}

fn internal_external_policy_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 0,
            vendor_id: 0x06cb,
            product_id: 0x0001,
            bus: BusType::I2c,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 8_000,
            vendor_id: 0x046d,
            product_id: 0xc539,
            bus: BusType::Usb,
            internal: false,
        }),
        DevicePolicyEvent::SendEvents {
            time_us: 20_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 44_000,
            internal: true,
            external_mouse_present: true,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 76_000,
            internal: false,
            external_mouse_present: true,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 104_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn keyboard_touchpad_trackpoint_metadata_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 0,
            vendor_id: 0x0001,
            product_id: 0x0001,
            bus: BusType::Ps2,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 10_000,
            vendor_id: 0x06cb,
            product_id: 0x0001,
            bus: BusType::I2c,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 20_000,
            vendor_id: 0x0002,
            product_id: 0x000a,
            bus: BusType::Ps2,
            internal: true,
        }),
        DevicePolicyEvent::SendEvents {
            time_us: 32_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn tablet_tool_pad_touch_metadata_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 0,
            vendor_id: 0x056a,
            product_id: 0x0357,
            bus: BusType::Usb,
            internal: false,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 8_000,
            vendor_id: 0x056a,
            product_id: 0x0357,
            bus: BusType::Usb,
            internal: false,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 16_000,
            vendor_id: 0x056a,
            product_id: 0x0357,
            bus: BusType::Usb,
            internal: false,
        }),
        DevicePolicyEvent::Led {
            time_us: 24_000,
            caps_lock: false,
            num_lock: false,
            scroll_lock: false,
        },
    ]
}

fn device_profile_override_candidate_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 0,
            vendor_id: 0x06cb,
            product_id: 0x0001,
            bus: BusType::I2c,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 12_000,
            vendor_id: 0x0001,
            product_id: 0x0001,
            bus: BusType::Ps2,
            internal: true,
        }),
        DevicePolicyEvent::Metadata(DeviceMetadataProbe {
            time_us: 24_000,
            vendor_id: 0x0000,
            product_id: 0x0000,
            bus: BusType::Unknown,
            internal: true,
        }),
        DevicePolicyEvent::SendEvents {
            time_us: 40_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn capability_inventory_sequence() -> Vec<DeviceInfo> {
    let mut keyboard_caps = DeviceCaps::default();
    keyboard_caps.insert(DeviceCaps::KEYBOARD);

    let mut pointer_caps = DeviceCaps::default();
    pointer_caps.insert(DeviceCaps::POINTER);

    let mut clickpad_caps = DeviceCaps::default();
    clickpad_caps.insert(DeviceCaps::KEYBOARD);
    clickpad_caps.insert(DeviceCaps::TOUCHPAD);
    clickpad_caps.insert(DeviceCaps::POINTER);

    let mut touchscreen_caps = DeviceCaps::default();
    touchscreen_caps.insert(DeviceCaps::TOUCHSCREEN);

    let mut switch_caps = DeviceCaps::default();
    switch_caps.insert(DeviceCaps::SWITCH);

    let mut tablet_caps = DeviceCaps::default();
    tablet_caps.insert(DeviceCaps::TABLET);

    let mut tablet_pad_caps = DeviceCaps::default();
    tablet_pad_caps.insert(DeviceCaps::TABLET_PAD);
    tablet_pad_caps.insert(DeviceCaps::KEYBOARD);

    vec![
        DeviceInfo {
            name: "bench internal laptop keyboard".to_string(),
            vendor_id: 0x0001,
            product_id: 0x0001,
            bus: BusType::Ps2,
            capabilities: keyboard_caps,
            ..Default::default()
        },
        DeviceInfo {
            name: "bench usb gaming mouse".to_string(),
            vendor_id: 0x046d,
            product_id: 0xc539,
            bus: BusType::Usb,
            capabilities: pointer_caps,
            ..Default::default()
        },
        DeviceInfo {
            name: "bench internal trackpoint".to_string(),
            vendor_id: 0x0002,
            product_id: 0x000a,
            bus: BusType::Ps2,
            capabilities: pointer_caps,
            ..Default::default()
        },
        DeviceInfo {
            name: "bench clickpad touch node".to_string(),
            x: AxisInfo {
                min: 0.0,
                max: 4200.0,
                resolution: 42.0,
            },
            y: AxisInfo {
                min: 0.0,
                max: 2600.0,
                resolution: 42.0,
            },
            vendor_id: 0x06cb,
            product_id: 0x0001,
            bus: BusType::I2c,
            capabilities: clickpad_caps,
            ..Default::default()
        },
        DeviceInfo {
            name: "bench direct touchscreen".to_string(),
            x: AxisInfo {
                min: 0.0,
                max: 1920.0,
                resolution: 12.0,
            },
            y: AxisInfo {
                min: 0.0,
                max: 1080.0,
                resolution: 12.0,
            },
            vendor_id: 0x1234,
            product_id: 0x5678,
            bus: BusType::Usb,
            capabilities: touchscreen_caps,
            ..Default::default()
        },
        DeviceInfo {
            name: "bench lid tablet-mode switch".to_string(),
            vendor_id: 0x17aa,
            product_id: 0x5054,
            bus: BusType::I2c,
            capabilities: switch_caps,
            supported_switches: vec![SwitchKind::Lid, SwitchKind::TabletMode],
            switch_states: vec![(SwitchKind::Lid, false), (SwitchKind::TabletMode, false)],
            ..Default::default()
        },
        DeviceInfo {
            name: "bench tablet pen tool".to_string(),
            x: AxisInfo {
                min: 0.0,
                max: 62200.0,
                resolution: 200.0,
            },
            y: AxisInfo {
                min: 0.0,
                max: 43200.0,
                resolution: 200.0,
            },
            vendor_id: 0x056a,
            product_id: 0x0357,
            bus: BusType::Usb,
            capabilities: tablet_caps,
            ..Default::default()
        },
        DeviceInfo {
            name: "bench tablet pad buttons".to_string(),
            vendor_id: 0x056a,
            product_id: 0x0357,
            bus: BusType::Usb,
            capabilities: tablet_pad_caps,
            ..Default::default()
        },
    ]
}

fn switch_internal_disable_restore_policy_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::SendEvents {
            time_us: 0,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 750_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: false,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 2_500_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn switch_user_disabled_policy_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::SendEvents {
            time_us: 0,
            internal: true,
            external_mouse_present: false,
            user_enabled: false,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 800_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: false,
            switch_enabled: false,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 1_600_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: false,
            switch_enabled: true,
        },
    ]
}

fn switch_external_device_policy_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::SendEvents {
            time_us: 0,
            internal: false,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 900_000,
            internal: false,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: false,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 1_800_000,
            internal: false,
            external_mouse_present: true,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn switch_external_mouse_composed_policy_sequence() -> Vec<DevicePolicyEvent> {
    vec![
        DevicePolicyEvent::SendEvents {
            time_us: 0,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 600_000,
            internal: true,
            external_mouse_present: true,
            user_enabled: true,
            switch_enabled: true,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 1_200_000,
            internal: true,
            external_mouse_present: true,
            user_enabled: true,
            switch_enabled: false,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 2_400_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: false,
        },
        DevicePolicyEvent::SendEvents {
            time_us: 3_800_000,
            internal: true,
            external_mouse_present: false,
            user_enabled: true,
            switch_enabled: true,
        },
    ]
}

fn low_dpi_pointer_drag() -> Vec<InputEvent> {
    let mut events = Vec::new();
    for i in 0..96 {
        let time_us = i * 8_000;
        let dy = if i % 12 < 6 { 1.0 } else { 0.0 };
        events.push(pointer_motion(time_us, 1.0, dy));
        events.push(frame(time_us));
    }
    events
}

fn high_dpi_micro_aim() -> Vec<InputEvent> {
    let mut events = Vec::new();
    for i in 0..128 {
        let time_us = i * 4_000;
        let direction = if i % 16 < 8 { 1.0 } else { -1.0 };
        let vertical = if i % 20 < 10 { 0.5 } else { -0.5 };
        events.push(pointer_motion(time_us, direction, vertical));
        events.push(frame(time_us));
    }
    events
}

fn adaptive_pointer_speed_ramp() -> Vec<InputEvent> {
    let mut events = Vec::new();
    for i in 0..96 {
        let time_us = i * 6_000;
        let step = 0.4 + (i % 24) as f64 * 0.18;
        let direction = if i / 24 % 2 == 0 { 1.0 } else { -1.0 };
        events.push(pointer_motion(time_us, direction * step, step * 0.25));
        events.push(frame(time_us));
    }
    events
}

fn mouse_dpi_property_sequence() -> Vec<&'static str> {
    vec![
        "400@125",
        "450 *1800 3200",
        "450@37 1800@12 *3200@6",
        "800, 1600, *2400",
        "450 1800 3200",
        "*12, *450, 800",
        "0 450 1800 *3200",
        "112@125, 450@125, 800@125, 900@-125",
        "12@34 *45@",
        "                                      *12, 450, 800",
    ]
}

fn mouse_dpi_runtime_change_context() -> u64 {
    let events = high_dpi_micro_aim();
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    ctx.configure_device_filters(device);
    let _ = ctx.next_event();

    ctx.set_mouse_dpi(device, 35_000);
    let _ = ctx.dispatch();

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn apply_pending_mouse_dpi_rule(
    ctx: &mut Context,
    device: DeviceId,
    configured_name: &str,
    add_name: &str,
    dpi: i32,
) -> u64 {
    let Some(info) = ctx.device_info(device) else {
        return 0;
    };
    let actual_name = info.name.to_string();
    let is_pointer = info.capabilities.contains(DeviceCaps::POINTER);

    if is_pointer
        && (bench_name_matches(configured_name, add_name)
            || bench_name_matches(configured_name, &actual_name))
    {
        ctx.set_mouse_dpi(device, dpi);
        0xd11_0000 ^ (dpi as u64)
    } else {
        0
    }
}

fn mouse_dpi_hotplug_replay_context() -> u64 {
    let pending_name = "bench relative";
    let pending_dpi = 35_000;
    let events = high_dpi_micro_aim();
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench relative pointer", seat, fd);
    ctx.configure_device_filters(device);

    let mut score = apply_pending_mouse_dpi_rule(
        &mut ctx,
        device,
        pending_name,
        "bench relative pointer",
        pending_dpi,
    );

    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn mouse_dpi_hotplug_generic_add_name_context() -> u64 {
    let pending_name = "bench relative pointer";
    let pending_dpi = 35_000;
    let events = high_dpi_micro_aim();
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("event7", seat, fd);
    ctx.configure_device_filters(device);

    let mut score =
        apply_pending_mouse_dpi_rule(&mut ctx, device, pending_name, "event7", pending_dpi);

    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn mouse_dpi_hotplug_duplicate_name_context() -> u64 {
    let pending_name = "bench relative pointer";
    let pending_dpi = 35_000;
    let events = high_dpi_micro_aim();
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let first_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let first = ctx.add_device_fd("event7", seat, first_fd);
    ctx.configure_device_filters(first);
    let mut score =
        apply_pending_mouse_dpi_rule(&mut ctx, first, pending_name, "event7", pending_dpi);

    let second_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let second = ctx.add_device_fd("event8", seat, second_fd);
    ctx.configure_device_filters(second);
    score ^= apply_pending_mouse_dpi_rule(&mut ctx, second, pending_name, "event8", pending_dpi);

    let _ = ctx.next_event();
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn mouse_dpi_hotplug_readd_same_name_context() -> u64 {
    let pending_name = "bench relative pointer";
    let pending_dpi = 35_000;
    let events = high_dpi_micro_aim();
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: pointer_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let first_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let first = ctx.add_device_fd("event7", seat, first_fd);
    ctx.configure_device_filters(first);
    let mut score =
        apply_pending_mouse_dpi_rule(&mut ctx, first, pending_name, "event7", pending_dpi);

    let readd_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let readded = ctx.add_device_fd("event7", seat, readd_fd);
    ctx.configure_device_filters(readded);
    score ^= apply_pending_mouse_dpi_rule(&mut ctx, readded, pending_name, "event7", pending_dpi);

    let _ = ctx.next_event();
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn mouse_dpi_hotplug_matching_non_pointer_context() -> u64 {
    let pending_name = "bench relative pointer";
    let pending_dpi = 35_000;
    let events = high_dpi_micro_aim();
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: keyboard_info_with_name("bench relative pointer"),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("event7", seat, fd);
    ctx.configure_device_filters(device);

    let mut score =
        apply_pending_mouse_dpi_rule(&mut ctx, device, pending_name, "event7", pending_dpi);

    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn device_disable_cleanup_context() -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchscreen_info(),
        events: Vec::new(),
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench touchscreen", seat, fd);
    let _ = ctx.next_event();

    if let Some(dev) = ctx.devices.get_mut(device) {
        dev.keys_pressed.set(KEY_A as usize, true);
        dev.buttons_pressed.set(BTN_LEFT as usize, true);
        dev.touches_active.set(0, true);
    }

    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn device_removal_cleanup_context() -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchscreen_info(),
        events: Vec::new(),
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench touchscreen", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    let _ = ctx.next_event();

    if let Some(dev) = ctx.devices.get_mut(device) {
        dev.keys_pressed.set(KEY_A as usize, true);
        dev.buttons_pressed.set(BTN_LEFT as usize, true);
        dev.touches_active.set(0, true);
    }

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn backend_device_removed_cleanup_context() -> u64 {
    let events = vec![
        td(0, 0, 920.0, 540.0),
        frame(0),
        tm(20_000, 0, 955.0, 560.0),
        frame(20_000),
        InputEvent::DeviceRemoved {
            device_index: DevIndex(0),
        },
    ];
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchscreen_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench touchscreen", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.device_id_by_index(device_index).is_none());
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn touchpad_tap_click_device_disable_context(events: Vec<InputEvent>, tap_drag_lock: bool) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchpad_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench laptop touchpad", seat, fd);
    ctx.configure_device_filters(device);
    ctx.set_tap_drag_lock(device, tap_drag_lock);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();
    ctx.set_device_enabled(device, false);

    let mut score = 0;
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn touchpad_tap_click_device_removal_context(events: Vec<InputEvent>, tap_drag_lock: bool) -> u64 {
    let mut ctx = Context::new(Box::new(StaticInfoBackend {
        info: touchpad_info(),
        events,
        device_id: None,
    }));
    let seat = ctx.create_seat("bench-seat");
    let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let device = ctx.add_device_fd("bench laptop touchpad", seat, fd);
    let device_index = ctx.device_index_by_id(device).unwrap();
    ctx.configure_device_filters(device);
    ctx.set_tap_drag_lock(device, tap_drag_lock);
    let _ = ctx.next_event();
    let _ = ctx.dispatch();

    let mut score = bit(ctx.remove_device_by_index(device_index));
    while let Some(event) = ctx.next_event() {
        score ^= event_score(&event);
    }
    score
}

fn bench_touchpad_tap_and_click(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_touchpad_tap_and_click");
    let single = single_finger_tap();
    let two = two_finger_tap();
    let two_staggered = two_finger_tap_staggered_down_reverse_up();
    let two_reverse = two_finger_tap_same_frame_reverse_up();
    let three = three_finger_tap();
    let alternate_tap_map = alternate_tap_map_candidate();
    let drag = tap_then_drag_candidate();
    let drag_remove_context = tap_drag_active_before_cleanup();
    let drag_removed = tap_drag_active_device_removed();
    let drag_lock = tap_drag_lock_candidate();
    let drag_lock_disable = tap_drag_lock_active_before_disable();
    let drag_lock_remove_context = tap_drag_lock_active_before_disable();
    let drag_lock_removed = tap_drag_lock_active_device_removed();
    let cancel_move = tap_cancelled_by_movement();
    let cancel_timeout = tap_cancelled_by_timeout();
    let near_timeout = tap_near_timeout_candidate();
    let near_move_threshold = tap_near_movement_threshold_candidate();
    let delayed_drag = tap_drag_second_touch_after_timeout_candidate();
    let clickpad_main = clickpad_main_area_click();
    let clickpad_left = clickpad_bottom_left_click();
    let clickpad_middle = clickpad_bottom_middle_click();
    let clickpad_right = clickpad_bottom_right_click();
    let physical_click_during_tap = physical_click_during_tap_candidate();
    let clickpad_latched = clickpad_latched_button_after_contact_move();
    let clickfinger_one = one_finger_clickfinger_click();
    let clickfinger_two = two_finger_clickfinger_click();
    let clickfinger_three = three_finger_clickfinger_click();
    let clickfinger_staggered = two_finger_clickfinger_staggered_arrival_click();
    let clickfinger_latched = clickfinger_latched_button_after_finger_added();
    let alternate_clickfinger_map = alternate_clickfinger_map_candidate();
    let palm = palm_edge_contact();
    let single_variant = varied_touch_workload(&single, 1);
    let two_variant = varied_touch_workload(&two, 2);
    let two_variant_suite = touch_variant_suite(&two);

    group.bench_function("single_finger_tap_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&single), TouchpadConfig::default(), None))
    });
    group.bench_function(
        "single_finger_tap_slot_timing_variant_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(black_box(&single_variant), TouchpadConfig::default(), None)
            })
        },
    );
    group.bench_function("two_finger_tap_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&two), TouchpadConfig::default(), None))
    });
    group.bench_function("two_finger_tap_slot_timing_variant_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&two_variant), TouchpadConfig::default(), None))
    });
    group.bench_function("two_finger_tap_variant_suite_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion_variant_suite(
                black_box(&two_variant_suite),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_tap_right_click_disabled_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.two_finger_tap_right_click = false;
            run_touchpad_motion(black_box(&two), config, None)
        })
    });
    group.bench_function(
        "two_finger_tap_right_click_change_while_neutral_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_two_finger_tap_right_click_change(
                    black_box(&two),
                    false,
                    true,
                    0,
                )
            })
        },
    );
    group.bench_function(
        "two_finger_tap_right_click_change_while_active_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_two_finger_tap_right_click_change(
                    black_box(&two),
                    true,
                    false,
                    3,
                )
            })
        },
    );
    group.bench_function(
        "two_finger_tap_staggered_down_reverse_up_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(black_box(&two_staggered), TouchpadConfig::default(), None)
            })
        },
    );
    group.bench_function("two_finger_tap_same_frame_reverse_up_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&two_reverse), TouchpadConfig::default(), None))
    });
    group.bench_function("three_finger_tap_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&three), TouchpadConfig::default(), None))
    });
    group.bench_function("alternate_tap_map_candidate_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&alternate_tap_map),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("tap_then_drag_candidate_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&drag), TouchpadConfig::default(), None))
    });
    group.bench_function("tap_drag_device_removed_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&drag_removed), TouchpadConfig::default(), None))
    });
    group.bench_function("tap_drag_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_tap_click_device_removal_context(
                drag_remove_context.clone(),
                false,
            ))
        })
    });
    group.bench_function("tap_drag_lock_candidate_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.tap_drag_lock = true;
            run_touchpad_motion(black_box(&drag_lock), config, None)
        })
    });
    group.bench_function("tap_drag_lock_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.tap_drag_lock = true;
            run_touchpad_motion(black_box(&drag_lock_removed), config, None)
        })
    });
    group.bench_function("tap_drag_lock_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_tap_click_device_disable_context(
                drag_lock_disable.clone(),
                true,
            ))
        })
    });
    group.bench_function("tap_drag_lock_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_tap_click_device_removal_context(
                drag_lock_remove_context.clone(),
                true,
            ))
        })
    });
    group.bench_function("tap_cancelled_by_movement_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&cancel_move), TouchpadConfig::default(), None))
    });
    group.bench_function("tap_cancelled_by_timeout_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&cancel_timeout), TouchpadConfig::default(), None))
    });
    group.bench_function("tap_near_timeout_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&near_timeout), TouchpadConfig::default(), None))
    });
    group.bench_function("tap_near_movement_threshold_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&near_move_threshold),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("tap_drag_second_touch_after_timeout_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&delayed_drag), TouchpadConfig::default(), None))
    });
    group.bench_function("physical_click_cancels_pending_tap_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&physical_click_during_tap),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("clickpad_main_area_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_motion(black_box(&clickpad_main), config, None)
        })
    });
    group.bench_function("clickpad_bottom_left_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_motion(black_box(&clickpad_left), config, None)
        })
    });
    group.bench_function("clickpad_bottom_middle_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_motion(black_box(&clickpad_middle), config, None)
        })
    });
    group.bench_function("clickpad_bottom_right_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_motion(black_box(&clickpad_right), config, None)
        })
    });
    group.bench_function(
        "clickpad_latched_button_after_contact_move_current_filter",
        |b| {
            b.iter(|| {
                let mut config = TouchpadConfig::default();
                config.click_method = TouchpadClickMethod::ButtonAreas;
                run_touchpad_motion(black_box(&clickpad_latched), config, None)
            })
        },
    );
    group.bench_function("one_finger_clickfinger_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_motion(black_box(&clickfinger_one), config, None)
        })
    });
    group.bench_function("two_finger_clickfinger_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_motion(black_box(&clickfinger_two), config, None)
        })
    });
    group.bench_function("three_finger_clickfinger_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_motion(black_box(&clickfinger_three), config, None)
        })
    });
    group.bench_function(
        "two_finger_clickfinger_staggered_arrival_current_filter",
        |b| {
            b.iter(|| {
                let mut config = TouchpadConfig::default();
                config.click_method = TouchpadClickMethod::Clickfinger;
                run_touchpad_motion(black_box(&clickfinger_staggered), config, None)
            })
        },
    );
    group.bench_function(
        "clickfinger_latched_button_after_finger_added_current_filter",
        |b| {
            b.iter(|| {
                let mut config = TouchpadConfig::default();
                config.click_method = TouchpadClickMethod::Clickfinger;
                run_touchpad_motion(black_box(&clickfinger_latched), config, None)
            })
        },
    );
    group.bench_function("alternate_clickfinger_map_candidate_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_motion(black_box(&alternate_clickfinger_map), config, None)
        })
    });
    group.bench_function("tap_to_click_change_while_neutral_current_filter", |b| {
        b.iter(|| run_touchpad_motion_with_tap_to_click_change(black_box(&single), false, true, 0))
    });
    group.bench_function("tap_to_click_change_while_active_current_filter", |b| {
        b.iter(|| run_touchpad_motion_with_tap_to_click_change(black_box(&single), true, false, 2))
    });
    group.bench_function(
        "click_method_change_neutral_to_button_areas_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_click_method_change(
                    black_box(&clickpad_right),
                    TouchpadClickMethod::None,
                    TouchpadClickMethod::ButtonAreas,
                    0,
                )
            })
        },
    );
    group.bench_function(
        "click_method_change_neutral_to_clickfinger_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_click_method_change(
                    black_box(&clickfinger_two),
                    TouchpadClickMethod::None,
                    TouchpadClickMethod::Clickfinger,
                    0,
                )
            })
        },
    );
    group.bench_function(
        "click_method_change_while_touch_active_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_click_method_change(
                    black_box(&clickpad_right),
                    TouchpadClickMethod::None,
                    TouchpadClickMethod::ButtonAreas,
                    2,
                )
            })
        },
    );
    group.bench_function(
        "click_method_change_while_button_active_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_click_method_change(
                    black_box(&clickpad_latched),
                    TouchpadClickMethod::ButtonAreas,
                    TouchpadClickMethod::Clickfinger,
                    4,
                )
            })
        },
    );
    group.bench_function("tap_suppressed_by_dwt_current_filter", |b| {
        b.iter(|| {
            let seat = bench_seat(40_000);
            run_touchpad_motion(black_box(&single), TouchpadConfig::default(), Some(&seat))
        })
    });
    group.bench_function("tap_suppressed_by_palm_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&palm), TouchpadConfig::default(), None))
    });
    group.bench_function("tap_with_natural_scroll_enabled_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.natural_scrolling = true;
            run_touchpad_motion(black_box(&single), config, None)
        })
    });
    group.bench_function("left_handed_single_finger_tap_current_chain", |b| {
        b.iter(|| run_touchpad_left_handed_chain(black_box(&single), TouchpadConfig::default()))
    });
    group.bench_function("left_handed_two_finger_tap_current_chain", |b| {
        b.iter(|| run_touchpad_left_handed_chain(black_box(&two), TouchpadConfig::default()))
    });
    group.bench_function("left_handed_three_finger_tap_current_chain", |b| {
        b.iter(|| run_touchpad_left_handed_chain(black_box(&three), TouchpadConfig::default()))
    });
    group.bench_function("left_handed_clickpad_main_area_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_left_handed_chain(black_box(&clickpad_main), config)
        })
    });
    group.bench_function("left_handed_clickpad_bottom_left_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_left_handed_chain(black_box(&clickpad_left), config)
        })
    });
    group.bench_function("left_handed_clickpad_bottom_middle_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_left_handed_chain(black_box(&clickpad_middle), config)
        })
    });
    group.bench_function("left_handed_clickpad_bottom_right_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            run_touchpad_left_handed_chain(black_box(&clickpad_right), config)
        })
    });
    group.bench_function("left_handed_one_finger_clickfinger_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_left_handed_chain(black_box(&clickfinger_one), config)
        })
    });
    group.bench_function("left_handed_two_finger_clickfinger_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_left_handed_chain(black_box(&clickfinger_two), config)
        })
    });
    group.bench_function("left_handed_three_finger_clickfinger_current_chain", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::Clickfinger;
            run_touchpad_left_handed_chain(black_box(&clickfinger_three), config)
        })
    });
    group.bench_function(
        "middle_emulation_after_three_finger_tap_current_chain",
        |b| {
            b.iter(|| {
                let mut chain = FilterChain::new();
                chain.add(FilterKind::TouchpadMotion(Box::new(
                    TouchpadMotionFilter::new(TouchpadConfig::default()),
                )));
                chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
                run_filter_chain(black_box(&three), chain)
            })
        },
    );

    group.finish();
}

fn bench_touchpad_scroll_methods(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_touchpad_scroll_methods");
    let two_finger_vertical = two_finger_scroll_vertical();
    let two_finger_horizontal = two_finger_scroll_horizontal();
    let two_finger_diagonal = two_finger_scroll_diagonal();
    let two_finger_below = two_finger_scroll_below_threshold();
    let scroll_speed = scroll_speed_multiplier_candidate();
    let coasting_flick = coasting_scroll_flick_candidate();
    let circular = circular_scroll_candidate();
    let right_edge = right_edge_scroll_candidate();
    let bottom_edge = bottom_edge_scroll_candidate();
    let main_to_right_edge = main_area_crosses_into_right_edge_candidate();
    let right_edge_to_main = right_edge_crosses_into_main_area_candidate();
    let bottom_edge_to_main = bottom_edge_crosses_into_main_area_candidate();
    let two_finger_removed = touch_event_device_removed(two_finger_scroll_vertical());
    let edge_removed = right_edge_scroll_active_device_removed();
    let button = button_scroll_candidate();
    let right_button = right_button_scroll_candidate();
    let button_lock = button_scroll_lock_candidate();
    let button_hold_disable = button_scroll_hold_active_before_disable();
    let button_lock_disable = button_scroll_lock_active_before_disable();
    let button_hold_removed = button_scroll_hold_active_device_removed();
    let button_lock_removed = button_scroll_lock_active_device_removed();
    let wheel = wheel_scroll_sequence();
    let two_finger_vertical_variant = varied_touch_workload(&two_finger_vertical, 4);
    let two_finger_vertical_variant_suite = touch_variant_suite(&two_finger_vertical);
    let button_variant = varied_pointer_workload(&button, 5);
    let button_variant_suite = pointer_variant_suite(&button);

    group.bench_function("two_finger_scroll_vertical_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&two_finger_vertical),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function(
        "two_finger_scroll_slot_timing_variant_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(
                    black_box(&two_finger_vertical_variant),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function("two_finger_scroll_variant_suite_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion_variant_suite(
                black_box(&two_finger_vertical_variant_suite),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_scroll_horizontal_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&two_finger_horizontal),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_scroll_diagonal_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&two_finger_diagonal),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_scroll_below_threshold_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&two_finger_below),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_scroll_device_removed_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&two_finger_removed),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_scroll_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_scroll_device_disable_context(
                two_finger_scroll_vertical(),
                TouchpadScrollMethod::TwoFinger,
            ))
        })
    });
    group.bench_function("two_finger_scroll_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_scroll_device_removal_context(
                two_finger_scroll_vertical(),
                TouchpadScrollMethod::TwoFinger,
            ))
        })
    });
    group.bench_function("scroll_speed_multiplier_candidate_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&scroll_speed), TouchpadConfig::default(), None))
    });
    group.bench_function("coasting_scroll_flick_candidate_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&coasting_flick), TouchpadConfig::default(), None))
    });
    group.bench_function("circular_scroll_candidate_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&circular), config, None)
        })
    });
    group.bench_function("right_edge_scroll_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&right_edge), config, None)
        })
    });
    group.bench_function("bottom_edge_scroll_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&bottom_edge), config, None)
        })
    });
    group.bench_function("right_edge_natural_scroll_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            config.natural_scrolling = true;
            run_touchpad_motion(black_box(&right_edge), config, None)
        })
    });
    group.bench_function("bottom_edge_natural_scroll_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            config.natural_scrolling = true;
            run_touchpad_motion(black_box(&bottom_edge), config, None)
        })
    });
    group.bench_function("main_area_crosses_into_right_edge_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&main_to_right_edge), config, None)
        })
    });
    group.bench_function("right_edge_crosses_into_main_area_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&right_edge_to_main), config, None)
        })
    });
    group.bench_function("bottom_edge_crosses_into_main_area_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&bottom_edge_to_main), config, None)
        })
    });
    group.bench_function("right_edge_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.scroll_method = TouchpadScrollMethod::Edge;
            run_touchpad_motion(black_box(&edge_removed), config, None)
        })
    });
    group.bench_function("right_edge_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_scroll_device_disable_context(
                right_edge_scroll_candidate(),
                TouchpadScrollMethod::Edge,
            ))
        })
    });
    group.bench_function("right_edge_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_scroll_device_removal_context(
                right_edge_scroll_candidate(),
                TouchpadScrollMethod::Edge,
            ))
        })
    });
    group.bench_function("bottom_edge_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_scroll_device_removal_context(
                bottom_edge_scroll_candidate(),
                TouchpadScrollMethod::Edge,
            ))
        })
    });
    group.bench_function("two_finger_natural_scroll_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.natural_scrolling = true;
            run_touchpad_motion(black_box(&two_finger_vertical), config, None)
        })
    });
    group.bench_function("button_scroll_candidate_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::ButtonScroll(ButtonScrollFilter::middle_button()));
            run_filter_chain(black_box(&button), chain)
        })
    });
    group.bench_function("button_scroll_timing_delta_variant_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::ButtonScroll(ButtonScrollFilter::middle_button()));
            run_filter_chain(black_box(&button_variant), chain)
        })
    });
    group.bench_function("button_scroll_variant_suite_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&button_variant_suite),
                || {
                    let mut chain = FilterChain::new();
                    chain.add(FilterKind::ButtonScroll(ButtonScrollFilter::middle_button()));
                    chain
                },
                pointer_info(),
            )
        })
    });
    group.bench_function("button_scroll_lock_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::ButtonScroll(
                ButtonScrollFilter::middle_button().with_lock(true),
            ));
            run_filter_chain(black_box(&button_lock), chain)
        })
    });
    group.bench_function("button_scroll_enable_while_neutral_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_with_button_scroll_change(
                black_box(&button),
                false,
                BTN_MIDDLE,
                false,
                true,
                BTN_MIDDLE,
                false,
                0,
            )
        })
    });
    group.bench_function("button_scroll_disable_while_active_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_with_button_scroll_change(
                black_box(&button_hold_disable),
                true,
                BTN_MIDDLE,
                false,
                false,
                BTN_MIDDLE,
                false,
                10,
            )
        })
    });
    group.bench_function(
        "button_scroll_replace_button_while_neutral_current_chain",
        |b| {
            b.iter(|| {
                run_filter_chain_with_button_scroll_change(
                    black_box(&right_button),
                    true,
                    BTN_MIDDLE,
                    false,
                    true,
                    BTN_RIGHT,
                    false,
                    0,
                )
            })
        },
    );
    group.bench_function("button_scroll_hold_disable_current_context", |b| {
        b.iter(|| {
            black_box(button_scroll_device_disable_context(
                button_hold_disable.clone(),
                false,
            ))
        })
    });
    group.bench_function("button_scroll_lock_disable_current_context", |b| {
        b.iter(|| {
            black_box(button_scroll_device_disable_context(
                button_lock_disable.clone(),
                true,
            ))
        })
    });
    group.bench_function("button_scroll_hold_removal_current_context", |b| {
        b.iter(|| {
            black_box(button_scroll_device_removal_context(
                button_hold_disable.clone(),
                false,
            ))
        })
    });
    group.bench_function("button_scroll_lock_removal_current_context", |b| {
        b.iter(|| {
            black_box(button_scroll_device_removal_context(
                button_lock_disable.clone(),
                true,
            ))
        })
    });
    group.bench_function("button_scroll_lock_device_removed_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::ButtonScroll(
                ButtonScrollFilter::middle_button().with_lock(true),
            ));
            run_filter_chain(black_box(&button_lock_removed), chain)
        })
    });
    group.bench_function("button_scroll_hold_device_removed_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::ButtonScroll(ButtonScrollFilter::middle_button()));
            run_filter_chain(black_box(&button_hold_removed), chain)
        })
    });
    group.bench_function("button_natural_scroll_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::ButtonScroll(ButtonScrollFilter::middle_button()));
            chain.add(FilterKind::NaturalScroll(NaturalScrollFilter::new(true)));
            run_filter_chain(black_box(&button), chain)
        })
    });
    group.bench_function("wheel_natural_scroll_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::NaturalScroll(NaturalScrollFilter::new(true)));
            run_filter_chain(black_box(&wheel), chain)
        })
    });
    group.bench_function("scroll_method_change_while_neutral_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion_with_scroll_method_change(
                black_box(&right_edge),
                TouchpadScrollMethod::TwoFinger,
                TouchpadScrollMethod::Edge,
                0,
            )
        })
    });
    group.bench_function("scroll_method_change_while_scrolling_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion_with_scroll_method_change(
                black_box(&two_finger_vertical),
                TouchpadScrollMethod::TwoFinger,
                TouchpadScrollMethod::Edge,
                18,
            )
        })
    });
    group.bench_function(
        "natural_scroll_change_while_scrolling_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_with_natural_scroll_change(
                    black_box(&two_finger_vertical),
                    false,
                    true,
                    18,
                )
            })
        },
    );

    group.finish();
}

fn bench_touchpad_motion_extensions(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_touchpad_motion_extensions");
    let pressure_motion = pressure_dependent_motion_candidate();
    let edge_motion = edge_motion_continuation_candidate();
    let active_rectangle = touchpad_active_rectangle_candidate();
    let left_handed_rotation = left_handed_touchpad_rotation_candidate();
    let pressure_removed = touchpad_extension_device_removed(pressure_dependent_motion_candidate());
    let edge_removed = touchpad_extension_device_removed(edge_motion_continuation_candidate());
    let active_rectangle_removed =
        touchpad_extension_device_removed(touchpad_active_rectangle_candidate());
    let left_handed_rotation_removed =
        touchpad_extension_device_removed(left_handed_touchpad_rotation_candidate());
    let pressure_motion_variant = varied_touch_workload(&pressure_motion, 18);
    let pressure_motion_variant_suite = touch_variant_suite(&pressure_motion);

    group.bench_function("pressure_dependent_motion_candidate_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&pressure_motion), TouchpadConfig::default(), None))
    });
    group.bench_function(
        "pressure_dependent_motion_slot_timing_metadata_variant_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(
                    black_box(&pressure_motion_variant),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function(
        "pressure_dependent_motion_variant_suite_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion_variant_suite(
                    black_box(&pressure_motion_variant_suite),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function(
        "pressure_dependent_motion_device_removed_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(
                    black_box(&pressure_removed),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function(
        "pressure_dependent_motion_device_disable_current_context",
        |b| {
            b.iter(|| {
                black_box(touchpad_extension_device_disable_context(
                    pressure_dependent_motion_candidate(),
                ))
            })
        },
    );
    group.bench_function(
        "pressure_dependent_motion_device_removal_current_context",
        |b| {
            b.iter(|| {
                black_box(touchpad_extension_device_removal_context(
                    pressure_dependent_motion_candidate(),
                ))
            })
        },
    );
    group.bench_function("edge_motion_continuation_candidate_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&edge_motion), TouchpadConfig::default(), None))
    });
    group.bench_function(
        "edge_motion_continuation_device_removed_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(black_box(&edge_removed), TouchpadConfig::default(), None)
            })
        },
    );
    group.bench_function(
        "edge_motion_continuation_device_disable_current_context",
        |b| {
            b.iter(|| {
                black_box(touchpad_extension_device_disable_context(
                    edge_motion_continuation_candidate(),
                ))
            })
        },
    );
    group.bench_function(
        "edge_motion_continuation_device_removal_current_context",
        |b| {
            b.iter(|| {
                black_box(touchpad_extension_device_removal_context(
                    edge_motion_continuation_candidate(),
                ))
            })
        },
    );
    group.bench_function("active_rectangle_candidate_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.palm_detection = false;
            run_touchpad_motion(black_box(&active_rectangle), config, None)
        })
    });
    group.bench_function("active_rectangle_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.palm_detection = false;
            run_touchpad_motion(black_box(&active_rectangle_removed), config, None)
        })
    });
    group.bench_function("active_rectangle_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_extension_device_disable_context(
                touchpad_active_rectangle_candidate(),
            ))
        })
    });
    group.bench_function("active_rectangle_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_extension_device_removal_context(
                touchpad_active_rectangle_candidate(),
            ))
        })
    });
    group.bench_function("left_handed_rotation_candidate_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&left_handed_rotation),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("left_handed_rotation_device_removed_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&left_handed_rotation_removed),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("left_handed_rotation_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_extension_device_disable_context(
                left_handed_touchpad_rotation_candidate(),
            ))
        })
    });
    group.bench_function("left_handed_rotation_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(touchpad_extension_device_removal_context(
                left_handed_touchpad_rotation_candidate(),
            ))
        })
    });

    group.finish();
}

fn bench_touchpad_gestures(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_touchpad_gestures");
    let swipe_three = three_finger_swipe_candidate();
    let swipe_four = four_finger_swipe_candidate();
    let pinch_out = pinch_zoom_candidate();
    let pinch_in = pinch_zoom_in_candidate();
    let pinch_rotate_cw = pinch_rotate_clockwise_candidate();
    let pinch_rotate_ccw = pinch_rotate_counterclockwise_candidate();
    let hold = hold_candidate();
    let hold_cancel = hold_cancelled_by_motion_candidate();
    let hold_touch_cancel = hold_cancelled_by_touch_cancel_candidate();
    let finger_count_change = gesture_finger_count_change_candidate();
    let pinch_count_change = pinch_finger_count_change_candidate();
    let scroll_competition = two_finger_scroll_vertical();
    let tap_competition = single_finger_tap();
    let pointer_competition = single_finger_pointer_motion_candidate();
    let active_pinch_disable = active_two_finger_pinch_before_disable();
    let active_hold_disable = active_hold_before_disable();
    let active_swipe_remove_context = active_three_finger_swipe_before_disable();
    let active_pinch_remove_context = active_two_finger_pinch_before_disable();
    let active_hold_remove_context = active_hold_before_disable();
    let active_removed = active_three_finger_swipe_device_removed();
    let active_pinch_removed = active_two_finger_pinch_device_removed();
    let active_hold_removed = active_hold_device_removed();
    let swipe_three_variant = varied_touch_workload(&swipe_three, 6);
    let pinch_out_variant = varied_touch_workload(&pinch_out, 7);
    let swipe_three_variant_suite = touch_variant_suite(&swipe_three);
    let pinch_out_variant_suite = touch_variant_suite(&pinch_out);

    group.bench_function("three_finger_swipe_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&swipe_three), TouchpadConfig::default(), None))
    });
    group.bench_function(
        "three_finger_swipe_slot_timing_variant_current_recognizer",
        |b| {
            b.iter(|| {
                run_touchpad_motion(
                    black_box(&swipe_three_variant),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function("three_finger_swipe_variant_suite_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion_variant_suite(
                black_box(&swipe_three_variant_suite),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("four_finger_swipe_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&swipe_four), TouchpadConfig::default(), None))
    });
    group.bench_function("pinch_zoom_out_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&pinch_out), TouchpadConfig::default(), None))
    });
    group.bench_function(
        "pinch_zoom_out_slot_timing_variant_current_recognizer",
        |b| {
            b.iter(|| {
                run_touchpad_motion(
                    black_box(&pinch_out_variant),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function("pinch_zoom_out_variant_suite_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion_variant_suite(
                black_box(&pinch_out_variant_suite),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("pinch_zoom_in_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&pinch_in), TouchpadConfig::default(), None))
    });
    group.bench_function("pinch_rotate_clockwise_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&pinch_rotate_cw), TouchpadConfig::default(), None))
    });
    group.bench_function("pinch_rotate_counterclockwise_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&pinch_rotate_ccw),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("stationary_hold_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&hold), TouchpadConfig::default(), None))
    });
    group.bench_function("hold_cancelled_by_motion_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&hold_cancel), TouchpadConfig::default(), None))
    });
    group.bench_function("hold_cancelled_by_touch_cancel_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&hold_touch_cancel),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("finger_count_change_cancel_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&finger_count_change),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("pinch_finger_count_change_cancel_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&pinch_count_change),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("two_finger_scroll_competition_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&scroll_competition),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("tap_competition_current_recognizer", |b| {
        b.iter(|| run_touchpad_motion(black_box(&tap_competition), TouchpadConfig::default(), None))
    });
    group.bench_function("pointer_motion_competition_current_recognizer", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&pointer_competition),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("active_gesture_runtime_reset_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion_with_runtime_reset(
                black_box(&swipe_three),
                TouchpadConfig::default(),
                18,
            )
        })
    });
    group.bench_function("active_gesture_device_disable_current_context", |b| {
        b.iter(|| black_box(active_gesture_device_disable_context()))
    });
    group.bench_function("active_pinch_device_disable_current_context", |b| {
        b.iter(|| black_box(gesture_device_disable_context(active_pinch_disable.clone())))
    });
    group.bench_function("active_hold_device_disable_current_context", |b| {
        b.iter(|| black_box(gesture_device_disable_context(active_hold_disable.clone())))
    });
    group.bench_function("active_gesture_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(gesture_device_removal_context(
                active_swipe_remove_context.clone(),
            ))
        })
    });
    group.bench_function("active_pinch_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(gesture_device_removal_context(
                active_pinch_remove_context.clone(),
            ))
        })
    });
    group.bench_function("active_hold_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(gesture_device_removal_context(
                active_hold_remove_context.clone(),
            ))
        })
    });
    group.bench_function("active_gesture_device_removed_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&active_removed), TouchpadConfig::default(), None))
    });
    group.bench_function("active_pinch_device_removed_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&active_pinch_removed),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("active_hold_device_removed_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&active_hold_removed),
                TouchpadConfig::default(),
                None,
            )
        })
    });

    group.finish();
}

fn bench_button_debounce(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_button_debounce");
    let bouncy = debounce_bouncy_button_candidate();
    let spurious = debounce_spurious_double_click_candidate();
    let independent = debounce_independent_buttons_candidate();
    let keyboard = debounce_keyboard_key_candidate();
    let bouncy_removed = debounce_bouncy_pending_device_removed();
    let spurious_removed = debounce_spurious_pending_device_removed();
    let bouncy_variant = varied_pointer_workload(&bouncy, 8);
    let bouncy_variant_suite = pointer_variant_suite(&bouncy);
    let enabled_debounce = DebounceConfig {
        enabled: true,
        ..DebounceConfig::default()
    };

    group.bench_function("disabled_bouncy_button_passthrough_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig::default(),
            )));
            run_filter_chain(black_box(&bouncy), chain)
        })
    });
    group.bench_function("bouncy_button_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
            )));
            run_filter_chain(black_box(&bouncy), chain)
        })
    });
    group.bench_function("bouncy_button_timing_variant_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
            )));
            run_filter_chain(black_box(&bouncy_variant), chain)
        })
    });
    group.bench_function("bouncy_button_variant_suite_current_filter", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&bouncy_variant_suite),
                || {
                    let mut chain = FilterChain::new();
                    chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                        DebounceConfig {
                            enabled: true,
                            ..DebounceConfig::default()
                        },
                    )));
                    chain
                },
                pointer_info(),
            )
        })
    });
    group.bench_function("spurious_double_click_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
            )));
            run_filter_chain(black_box(&spurious), chain)
        })
    });
    group.bench_function("independent_buttons_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
            )));
            run_filter_chain(black_box(&independent), chain)
        })
    });
    group.bench_function("keyboard_key_bounce_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
            )));
            run_filter_chain(black_box(&keyboard), chain)
        })
    });
    group.bench_function("bouncy_pending_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                enabled_debounce,
            )));
            run_filter_chain(black_box(&bouncy_removed), chain)
        })
    });
    group.bench_function("spurious_pending_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                enabled_debounce,
            )));
            run_filter_chain(black_box(&spurious_removed), chain)
        })
    });
    group.bench_function("bouncy_pending_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(debounce_device_disable_context(
                debounce_bouncy_pending_before_cleanup(),
                enabled_debounce,
            ))
        })
    });
    group.bench_function("spurious_pending_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(debounce_device_disable_context(
                debounce_spurious_pending_before_cleanup(),
                enabled_debounce,
            ))
        })
    });
    group.bench_function("bouncy_pending_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(debounce_device_removal_context(
                debounce_bouncy_pending_before_cleanup(),
                enabled_debounce,
            ))
        })
    });
    group.bench_function("spurious_pending_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(debounce_device_removal_context(
                debounce_spurious_pending_before_cleanup(),
                enabled_debounce,
            ))
        })
    });
    group.bench_function("zero_spurious_timeout_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    spurious_timeout_us: 0,
                    ..DebounceConfig::default()
                },
            )));
            run_filter_chain(black_box(&spurious), chain)
        })
    });
    group.bench_function("debounce_enable_while_neutral_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_with_debounce_change(
                black_box(&bouncy),
                DebounceConfig::default(),
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
                0,
            )
        })
    });
    group.bench_function("debounce_disable_while_active_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_with_debounce_change(
                black_box(&bouncy),
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
                DebounceConfig::default(),
                2,
            )
        })
    });
    group.bench_function(
        "debounce_threshold_change_while_neutral_current_chain",
        |b| {
            b.iter(|| {
                run_filter_chain_with_debounce_change(
                    black_box(&spurious),
                    DebounceConfig {
                        enabled: true,
                        ..DebounceConfig::default()
                    },
                    DebounceConfig {
                        enabled: true,
                        bounce_timeout_us: 40_000,
                        spurious_timeout_us: 0,
                    },
                    0,
                )
            })
        },
    );

    group.finish();
}

fn bench_middle_button_emulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_middle_button_emulation");
    let left_right = middle_chord_candidate();
    let right_left = middle_right_left_chord_candidate();
    let timeout_left = middle_timeout_left_candidate();
    let timeout_right = middle_timeout_right_candidate();
    let release_second_first = middle_chord_release_second_button_first_candidate();
    let bouncy = middle_chord_bouncy_candidate();
    let pending_disable = middle_chord_pending_button_only();
    let active_disable = middle_chord_active_buttons_only();
    let pending_removal_context = middle_chord_pending_button_only();
    let active_removal_context = middle_chord_active_buttons_only();
    let pending_removed = middle_chord_pending_device_removed();
    let active_removed = middle_chord_active_device_removed();
    let left_right_variant = varied_pointer_workload(&left_right, 9);
    let left_right_variant_suite = pointer_variant_suite(&left_right);

    group.bench_function("left_right_chord_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&left_right), chain)
        })
    });
    group.bench_function("left_right_chord_timing_variant_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&left_right_variant), chain)
        })
    });
    group.bench_function("left_right_chord_variant_suite_current_filter", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&left_right_variant_suite),
                || {
                    let mut chain = FilterChain::new();
                    chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
                    chain
                },
                pointer_info(),
            )
        })
    });
    group.bench_function("right_left_chord_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&right_left), chain)
        })
    });
    group.bench_function("left_timeout_fallback_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&timeout_left), chain)
        })
    });
    group.bench_function("right_timeout_fallback_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&timeout_right), chain)
        })
    });
    group.bench_function("release_second_button_first_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&release_second_first), chain)
        })
    });
    group.bench_function("left_handed_chord_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.set_left_handed(true);
            chain.set_middle_button_emulation(true);
            run_filter_chain(black_box(&left_right), chain)
        })
    });
    group.bench_function("middle_chord_button_scroll_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            chain.add(FilterKind::ButtonScroll(ButtonScrollFilter::middle_button()));
            run_filter_chain(black_box(&left_right), chain)
        })
    });
    group.bench_function("middle_emulation_enable_while_neutral_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_with_middle_button_change(black_box(&left_right), false, true, 0)
        })
    });
    group.bench_function(
        "middle_emulation_disable_while_pending_current_chain",
        |b| {
            b.iter(|| {
                run_filter_chain_with_middle_button_change(black_box(&left_right), true, false, 1)
            })
        },
    );
    group.bench_function("middle_emulation_disable_while_active_current_chain", |b| {
        b.iter(|| {
            run_filter_chain_with_middle_button_change(black_box(&left_right), true, false, 2)
        })
    });
    group.bench_function("debounced_bouncy_chord_current_chain", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::Debounce(ButtonDebounceFilter::new(
                DebounceConfig {
                    enabled: true,
                    ..DebounceConfig::default()
                },
            )));
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&bouncy), chain)
        })
    });
    group.bench_function("pending_chord_disable_current_context", |b| {
        b.iter(|| {
            black_box(middle_button_device_disable_context(
                pending_disable.clone(),
            ))
        })
    });
    group.bench_function("active_chord_disable_current_context", |b| {
        b.iter(|| black_box(middle_button_device_disable_context(active_disable.clone())))
    });
    group.bench_function("pending_chord_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(middle_button_device_removal_context(
                pending_removal_context.clone(),
            ))
        })
    });
    group.bench_function("active_chord_device_removal_current_context", |b| {
        b.iter(|| {
            black_box(middle_button_device_removal_context(
                active_removal_context.clone(),
            ))
        })
    });
    group.bench_function("pending_chord_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&pending_removed), chain)
        })
    });
    group.bench_function("active_chord_device_removed_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            run_filter_chain(black_box(&active_removed), chain)
        })
    });

    group.finish();
}

fn bench_palm_thumb_and_device_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_palm_thumb_and_device_policy");
    let palm = palm_edge_contact();
    let thumb = thumb_rest_contact();
    let thumb_only = thumb_only_lower_zone_contact();
    let thumb_clickpad = clickpad_bottom_left_click();
    let dwt = dwt_suppressed_motion();
    let firmware_palm = firmware_palm_tool_metadata_sequence();
    let pressure_size_palm = pressure_size_palm_candidate_sequence();
    let late_firmware_palm = late_firmware_palm_reclassification_sequence();
    let palm_during_gesture = palm_contact_during_active_gesture_candidate();
    let firmware_palm_variant = varied_touch_workload(&firmware_palm, 10);
    let firmware_palm_variant_suite = touch_variant_suite(&firmware_palm);
    let grouped_trackpoint = keyboard_touchpad_trackpoint_metadata_sequence();
    let device_profile_override = device_profile_override_candidate_sequence();
    let stylus_touch = stylus_touch_arbitration_candidate();
    let seat = bench_seat(100_000);

    group.bench_function("edge_palm_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&palm), TouchpadConfig::default(), None))
    });
    group.bench_function("thumb_rest_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.thumb_detection = true;
            run_touchpad_motion(black_box(&thumb), config, None)
        })
    });
    group.bench_function("thumb_only_lower_zone_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.thumb_detection = true;
            run_touchpad_motion(black_box(&thumb_only), config, None)
        })
    });
    group.bench_function("thumb_clickpad_button_area_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.click_method = TouchpadClickMethod::ButtonAreas;
            config.thumb_detection = true;
            run_touchpad_motion(black_box(&thumb_clickpad), config, None)
        })
    });
    group.bench_function("disable_while_typing_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&dwt), TouchpadConfig::default(), Some(&seat)))
    });
    group.bench_function("dwt_internal_keyboard_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&dwt), TouchpadConfig::default(), Some(&seat)))
    });
    group.bench_function("dwt_external_keyboard_ignored_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&dwt), TouchpadConfig::default(), None))
    });
    group.bench_function("firmware_palm_tool_metadata_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&firmware_palm), TouchpadConfig::default(), None))
    });
    group.bench_function(
        "firmware_palm_slot_timing_metadata_variant_current_filter",
        |b| {
            b.iter(|| {
                run_touchpad_motion(
                    black_box(&firmware_palm_variant),
                    TouchpadConfig::default(),
                    None,
                )
            })
        },
    );
    group.bench_function("firmware_palm_variant_suite_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion_variant_suite(
                black_box(&firmware_palm_variant_suite),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("pressure_size_palm_candidate_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&pressure_size_palm),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("late_firmware_palm_reclassification_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&late_firmware_palm),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("firmware_palm_detection_disabled_current_filter", |b| {
        b.iter(|| {
            let mut config = TouchpadConfig::default();
            config.palm_detection = false;
            run_touchpad_motion(black_box(&firmware_palm), config, None)
        })
    });
    group.bench_function("palm_during_active_gesture_current_filter", |b| {
        b.iter(|| {
            run_touchpad_motion(
                black_box(&palm_during_gesture),
                TouchpadConfig::default(),
                None,
            )
        })
    });
    group.bench_function("dwtp_grouped_trackpoint_metadata_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&grouped_trackpoint)))
    });
    group.bench_function("device_profile_override_candidate_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&device_profile_override)))
    });
    group.bench_function("stylus_touch_arbitration_candidate_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&stylus_touch), chain)
        })
    });

    group.finish();
}

fn bench_touch_event_completeness(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_touch_event_completeness");
    let mt_b = two_finger_scroll_vertical();
    let single_touch = single_touch_fallback_events();
    let touch_cancel = touch_cancel_sequence();
    let mt_b_direct = mt_b_down_motion_up_sequence();
    let same_frame = multi_slot_same_frame_sequence();
    let mt_b_removed = touch_event_device_removed(mt_b_down_motion_up_sequence());
    let same_frame_removed = touch_event_device_removed(multi_slot_same_frame_sequence());
    let single_touch_removed = touch_event_device_removed(single_touch_fallback_events());
    let single_touch_disable = single_touch_fallback_events();
    let slot_reuse = slot_reuse_after_up_sequence();
    let metadata = touch_pressure_size_tool_metadata_sequence();
    let tracking_id_change = tracking_id_change_candidate_sequence();
    let mt_b_direct_variant = varied_touch_workload(&mt_b_direct, 11);
    let mt_b_direct_variant_suite = touch_variant_suite(&mt_b_direct);

    group.bench_function("mt_b_multi_slot_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&mt_b), TouchpadConfig::default(), None))
    });
    group.bench_function("mt_b_down_motion_up_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&mt_b_direct), chain)
        })
    });
    group.bench_function("mt_b_slot_timing_metadata_variant_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&mt_b_direct_variant), chain)
        })
    });
    group.bench_function("mt_b_variant_suite_current_events", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&mt_b_direct_variant_suite),
                FilterChain::new,
                touchpad_info(),
            )
        })
    });
    group.bench_function("mt_b_active_device_removed_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&mt_b_removed), chain)
        })
    });
    group.bench_function("multi_slot_same_frame_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&same_frame), chain)
        })
    });
    group.bench_function("multi_slot_active_device_removed_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&same_frame_removed), chain)
        })
    });
    group.bench_function("slot_reuse_after_up_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&slot_reuse), chain)
        })
    });
    group.bench_function("pressure_size_tool_metadata_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&metadata), chain)
        })
    });
    group.bench_function("tracking_id_change_candidate_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&tracking_id_change), chain)
        })
    });
    group.bench_function("single_touch_fallback_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&single_touch), chain)
        })
    });
    group.bench_function("single_touch_active_device_removed_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&single_touch_removed), chain)
        })
    });
    group.bench_function("active_touch_device_disable_current_context", |b| {
        b.iter(|| {
            black_box(touch_event_device_disable_context(
                single_touch_disable.clone(),
            ))
        })
    });
    group.bench_function("touch_cancel_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&touch_cancel), chain)
        })
    });
    group.bench_function("touchpad_cancel_cleanup_current_filter", |b| {
        b.iter(|| run_touchpad_motion(black_box(&touch_cancel), TouchpadConfig::default(), None))
    });

    group.finish();
}

fn bench_tablet_tool_and_pad(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_tablet_tool_and_pad");
    let stroke = tablet_stroke_and_pad();
    let pressure = tablet_pressure_range_candidate();
    let identity_cycles = tablet_tool_identity_cycles_candidate();
    let button_proximity_out = tablet_tool_button_held_until_proximity_out_candidate();
    let tool_removed = tablet_tool_active_device_removed_candidate();
    let pad_without_tool = tablet_pad_without_tool_proximity_candidate();
    let pad_removed = tablet_pad_active_device_removed_candidate();
    let separate_nodes = tablet_separate_tool_pad_touch_nodes_candidate();
    let touch_node_removed = tablet_touch_node_removed_while_group_active_candidate();
    let stroke_variant = varied_tablet_workload(&stroke, 12);
    let stroke_variant_suite = tablet_variant_suite(&stroke);

    group.bench_function("stylus_stroke_and_pad_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&stroke), chain)
        })
    });
    group.bench_function("stylus_stroke_timing_axis_variant_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&stroke_variant), chain)
        })
    });
    group.bench_function("stylus_stroke_variant_suite_current_events", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&stroke_variant_suite),
                FilterChain::new,
                touchpad_info(),
            )
        })
    });
    group.bench_function("pressure_range_candidate_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&pressure), chain)
        })
    });
    group.bench_function("tool_identity_proximity_cycles_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&identity_cycles), chain)
        })
    });
    group.bench_function("tool_button_held_until_proximity_out_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&button_proximity_out), chain)
        })
    });
    group.bench_function("tool_active_device_removed_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&tool_removed), chain)
        })
    });
    group.bench_function("pad_controls_without_tool_proximity_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&pad_without_tool), chain)
        })
    });
    group.bench_function("pad_controls_device_removed_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&pad_removed), chain)
        })
    });
    group.bench_function("separate_tool_pad_touch_nodes_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&separate_nodes), chain)
        })
    });
    group.bench_function(
        "touch_node_removed_while_group_active_current_events",
        |b| {
            b.iter(|| {
                let chain = FilterChain::new();
                run_filter_chain(black_box(&touch_node_removed), chain)
            })
        },
    );

    group.finish();
}

fn bench_switch_devices(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_switch_devices");
    let switches = switch_laptop_sequence();
    let unknown_switches = unknown_switch_sequence();
    let initial_switches = initial_switch_state_sequence();
    let active_removal = active_switch_device_removal_sequence();
    let rapid_toggles = rapid_switch_toggle_sequence();
    let unknown_active_removal = unknown_active_switch_device_removal_sequence();
    let rapid_toggles_variant = varied_switch_workload(&rapid_toggles, 13);
    let rapid_toggles_variant_suite = switch_variant_suite(&rapid_toggles);
    let internal_policy = switch_internal_disable_restore_policy_sequence();
    let user_disabled_policy = switch_user_disabled_policy_sequence();
    let external_policy = switch_external_device_policy_sequence();
    let external_mouse_composed_policy = switch_external_mouse_composed_policy_sequence();

    group.bench_function("lid_and_tablet_mode_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&switches), chain)
        })
    });

    group.bench_function("unknown_switch_passthrough_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&unknown_switches), chain)
        })
    });
    group.bench_function("initial_switch_state_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&initial_switches), chain)
        })
    });
    group.bench_function("active_switch_device_removal_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&active_removal), chain)
        })
    });
    group.bench_function("rapid_tablet_mode_toggle_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&rapid_toggles), chain)
        })
    });
    group.bench_function("rapid_tablet_mode_timing_variant_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&rapid_toggles_variant), chain)
        })
    });
    group.bench_function("rapid_tablet_mode_variant_suite_current_events", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&rapid_toggles_variant_suite),
                FilterChain::new,
                touchpad_info(),
            )
        })
    });
    group.bench_function("unknown_switch_active_removal_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&unknown_active_removal), chain)
        })
    });
    group.bench_function("switch_internal_disable_restore_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&internal_policy)))
    });
    group.bench_function("switch_user_disabled_stays_disabled_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&user_disabled_policy)))
    });
    group.bench_function(
        "switch_external_devices_remain_enabled_current_state",
        |b| b.iter(|| run_device_policy(black_box(&external_policy))),
    );
    group.bench_function("switch_external_mouse_composition_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&external_mouse_composed_policy)))
    });

    group.finish();
}

fn bench_absolute_device_transforms(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_absolute_device_transforms");
    let absolute = absolute_transform_sequence();
    let info = touchscreen_info();
    let tablet_active_area = tablet_active_area_candidate_sequence();
    let relative_rotation = relative_rotation_candidate_sequence();
    let invalid_configs = invalid_calibration_matrix_config_sequence();
    let absolute_variant = varied_touch_workload(&absolute, 14);
    let absolute_variant_suite = touch_variant_suite(&absolute);

    group.bench_function("identity_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                CalibrationMatrix::identity(),
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function(
        "identity_calibration_slot_timing_variant_current_filter",
        |b| {
            b.iter(|| {
                let mut chain = FilterChain::new();
                chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                    CalibrationMatrix::identity(),
                )));
                run_filter_chain_with_info(black_box(&absolute_variant), chain, info.clone())
            })
        },
    );
    group.bench_function("identity_calibration_variant_suite_current_filter", |b| {
        b.iter(|| {
            run_filter_chain_variant_suite(
                black_box(&absolute_variant_suite),
                || {
                    let mut chain = FilterChain::new();
                    chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                        CalibrationMatrix::identity(),
                    )));
                    chain
                },
                info.clone(),
            )
        })
    });
    group.bench_function("touchscreen_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            let matrix = CalibrationMatrix::new([-1.0, 0.0, 1.0, 0.0, 1.0, 0.0])
                .unwrap_or_else(CalibrationMatrix::identity);
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                matrix,
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function("y_inversion_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            let matrix = CalibrationMatrix::new([1.0, 0.0, 0.0, 0.0, -1.0, 1.0])
                .unwrap_or_else(CalibrationMatrix::identity);
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                matrix,
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function("rotate_clockwise_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            let matrix = CalibrationMatrix::new([0.0, 1.0, 0.0, -1.0, 0.0, 1.0])
                .unwrap_or_else(CalibrationMatrix::identity);
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                matrix,
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function("rotate_counterclockwise_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            let matrix = CalibrationMatrix::new([0.0, -1.0, 1.0, 1.0, 0.0, 0.0])
                .unwrap_or_else(CalibrationMatrix::identity);
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                matrix,
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function("translate_scale_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            let matrix = CalibrationMatrix::new([0.75, 0.0, 0.10, 0.0, 0.80, 0.05])
                .unwrap_or_else(CalibrationMatrix::identity);
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                matrix,
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function("clamping_calibration_current_filter", |b| {
        b.iter(|| {
            let mut chain = FilterChain::new();
            let matrix = CalibrationMatrix::new([1.60, 0.0, -0.20, 0.0, 1.40, -0.15])
                .unwrap_or_else(CalibrationMatrix::identity);
            chain.add(FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(
                matrix,
            )));
            run_filter_chain_with_info(black_box(&absolute), chain, info.clone())
        })
    });
    group.bench_function("invalid_calibration_matrix_config_current", |b| {
        b.iter(|| run_calibration_matrix_config_sequence(black_box(&invalid_configs)))
    });
    group.bench_function("calibration_change_while_neutral_current_chain", |b| {
        b.iter(|| {
            let initial = CalibrationMatrix::identity();
            let changed = CalibrationMatrix::new([0.0, 1.0, 0.0, -1.0, 0.0, 1.0])
                .unwrap_or_else(CalibrationMatrix::identity);
            run_filter_chain_with_calibration_change(
                black_box(&absolute),
                info.clone(),
                initial,
                changed,
                0,
            )
        })
    });
    group.bench_function("calibration_change_while_active_current_chain", |b| {
        b.iter(|| {
            let initial = CalibrationMatrix::identity();
            let changed = CalibrationMatrix::new([-1.0, 0.0, 1.0, 0.0, 1.0, 0.0])
                .unwrap_or_else(CalibrationMatrix::identity);
            run_filter_chain_with_calibration_change(
                black_box(&absolute),
                info.clone(),
                initial,
                changed,
                30,
            )
        })
    });
    group.bench_function("tablet_active_area_candidate_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&tablet_active_area), chain)
        })
    });
    group.bench_function("relative_rotation_candidate_current_events", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&relative_rotation), chain)
        })
    });

    group.finish();
}

fn bench_pointer_dpi_and_motion_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_pointer_dpi_and_motion_config");
    let low_dpi = low_dpi_pointer_drag();
    let high_dpi = high_dpi_micro_aim();
    let speed_ramp = adaptive_pointer_speed_ramp();
    let dpi_properties = mouse_dpi_property_sequence();
    let high_dpi_variant = varied_pointer_workload(&high_dpi, 15);
    let speed_ramp_variant = varied_pointer_workload(&speed_ramp, 16);
    let high_dpi_variant_suite = pointer_variant_suite(&high_dpi);
    let speed_ramp_variant_suite = pointer_variant_suite(&speed_ramp);

    group.bench_function("low_dpi_pointer_drag_current_filter", |b| {
        b.iter(|| run_mouse_dpi(black_box(&low_dpi), 400))
    });
    group.bench_function("high_dpi_micro_aim_current_filter", |b| {
        b.iter(|| run_mouse_dpi(black_box(&high_dpi), 35_000))
    });
    group.bench_function(
        "high_dpi_micro_aim_timing_delta_variant_current_filter",
        |b| b.iter(|| run_mouse_dpi(black_box(&high_dpi_variant), 35_000)),
    );
    group.bench_function("high_dpi_micro_aim_variant_suite_current_filter", |b| {
        b.iter(|| run_mouse_dpi_variant_suite(black_box(&high_dpi_variant_suite), 35_000))
    });
    group.bench_function("adaptive_profile_speed_ramp_current_filter", |b| {
        b.iter(|| {
            run_mouse_config(
                black_box(&speed_ramp),
                MouseConfig {
                    dpi: 1000,
                    speed_adjustment: 0.0,
                    profile: MouseAccelProfile::Adaptive,
                },
            )
        })
    });
    group.bench_function(
        "adaptive_profile_speed_ramp_timing_delta_variant_current_filter",
        |b| {
            b.iter(|| {
                run_mouse_config(
                    black_box(&speed_ramp_variant),
                    MouseConfig {
                        dpi: 1000,
                        speed_adjustment: 0.0,
                        profile: MouseAccelProfile::Adaptive,
                    },
                )
            })
        },
    );
    group.bench_function(
        "adaptive_profile_speed_ramp_variant_suite_current_filter",
        |b| {
            b.iter(|| {
                run_mouse_config_variant_suite(
                    black_box(&speed_ramp_variant_suite),
                    MouseConfig {
                        dpi: 1000,
                        speed_adjustment: 0.0,
                        profile: MouseAccelProfile::Adaptive,
                    },
                )
            })
        },
    );
    group.bench_function("custom_profile_speed_ramp_current_filter", |b| {
        b.iter(|| {
            run_mouse_config(
                black_box(&speed_ramp),
                MouseConfig {
                    dpi: 1000,
                    speed_adjustment: 0.0,
                    profile: MouseAccelProfile::Custom(smallvec![
                        (0.0, 0.60),
                        (10.0, 1.00),
                        (50.0, 1.80),
                        (120.0, 2.20),
                    ]),
                },
            )
        })
    });
    group.bench_function("runtime_profile_change_current_chain", |b| {
        b.iter(|| {
            run_mouse_profile_change(
                black_box(&speed_ramp),
                MouseAccelProfile::Flat,
                MouseAccelProfile::Adaptive,
                40,
            )
        })
    });
    group.bench_function("runtime_custom_profile_change_current_chain", |b| {
        b.iter(|| {
            run_mouse_profile_change(
                black_box(&speed_ramp),
                MouseAccelProfile::Adaptive,
                MouseAccelProfile::Custom(smallvec![(0.0, 0.70), (15.0, 1.10), (75.0, 2.00),]),
                40,
            )
        })
    });
    group.bench_function("runtime_speed_adjustment_change_current_chain", |b| {
        b.iter(|| run_mouse_speed_change(black_box(&speed_ramp), -0.5, 0.75, 40))
    });
    group.bench_function("runtime_dpi_change_current_context", |b| {
        b.iter(|| black_box(mouse_dpi_runtime_change_context()))
    });
    group.bench_function("hotplug_dpi_replay_current_context", |b| {
        b.iter(|| black_box(mouse_dpi_hotplug_replay_context()))
    });
    group.bench_function("hotplug_dpi_generic_add_name_current_context", |b| {
        b.iter(|| black_box(mouse_dpi_hotplug_generic_add_name_context()))
    });
    group.bench_function("hotplug_dpi_duplicate_name_current_context", |b| {
        b.iter(|| black_box(mouse_dpi_hotplug_duplicate_name_context()))
    });
    group.bench_function("hotplug_dpi_readd_same_name_current_context", |b| {
        b.iter(|| black_box(mouse_dpi_hotplug_readd_same_name_context()))
    });
    group.bench_function("hotplug_dpi_matching_non_pointer_current_context", |b| {
        b.iter(|| black_box(mouse_dpi_hotplug_matching_non_pointer_context()))
    });
    group.bench_function("mouse_dpi_property_parser_current", |b| {
        b.iter(|| run_mouse_dpi_property_parser(black_box(dpi_properties.as_slice())))
    });

    group.finish();
}

fn bench_device_policy_and_metadata(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_device_policy_and_metadata");
    let policy = device_policy_sequence();
    let metadata = representative_metadata_sequence();
    let internal_external = internal_external_policy_sequence();
    let laptop_group = keyboard_touchpad_trackpoint_metadata_sequence();
    let tablet_group = tablet_tool_pad_touch_metadata_sequence();
    let profile_override = device_profile_override_candidate_sequence();
    let capability_inventory = capability_inventory_sequence();
    let keyboard = vec![key(0, KEY_A, true), key(22_000, KEY_A, false)];
    let policy_variant = varied_device_policy_workload(&policy, 17);
    let policy_variants = policy_variant_suite(&policy);

    group.bench_function("metadata_policy_led_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&policy)))
    });
    group.bench_function(
        "metadata_policy_led_timing_metadata_variant_current_state",
        |b| b.iter(|| run_device_policy(black_box(&policy_variant))),
    );
    group.bench_function("metadata_policy_led_variant_suite_current_state", |b| {
        b.iter(|| run_device_policy_variant_suite(black_box(&policy_variants)))
    });
    group.bench_function("representative_metadata_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&metadata)))
    });
    group.bench_function("internal_external_policy_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&internal_external)))
    });
    group.bench_function("keyboard_touchpad_trackpoint_metadata_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&laptop_group)))
    });
    group.bench_function("tablet_tool_pad_touch_metadata_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&tablet_group)))
    });
    group.bench_function("device_profile_override_candidate_current_state", |b| {
        b.iter(|| run_device_policy(black_box(&profile_override)))
    });
    group.bench_function("capability_inventory_candidate_current_info", |b| {
        b.iter(|| run_device_capability_inventory(black_box(&capability_inventory)))
    });
    group.bench_function("keyboard_led_related_event_passthrough", |b| {
        b.iter(|| {
            let chain = FilterChain::new();
            run_filter_chain(black_box(&keyboard), chain)
        })
    });
    group.bench_function("device_disable_cleanup_current_context", |b| {
        b.iter(|| black_box(device_disable_cleanup_context()))
    });
    group.bench_function("device_removal_cleanup_current_context", |b| {
        b.iter(|| black_box(device_removal_cleanup_context()))
    });
    group.bench_function("backend_device_removed_cleanup_current_context", |b| {
        b.iter(|| black_box(backend_device_removed_cleanup_context()))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_touchpad_tap_and_click,
    bench_touchpad_scroll_methods,
    bench_touchpad_motion_extensions,
    bench_touchpad_gestures,
    bench_button_debounce,
    bench_middle_button_emulation,
    bench_palm_thumb_and_device_policy,
    bench_touch_event_completeness,
    bench_tablet_tool_and_pad,
    bench_switch_devices,
    bench_absolute_device_transforms,
    bench_pointer_dpi_and_motion_config,
    bench_device_policy_and_metadata
);
criterion_main!(benches);
