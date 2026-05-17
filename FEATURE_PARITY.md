# input-rs Feature Parity with libinput

## Status: Core mouse/keyboard functional, touchpad basic, many libinput features not yet ported

### Legend
- ✅ = Implemented and working
- ⚠️ = Partially implemented or has known gaps
- ❌ = Not implemented
- 🚫 = Not planned (explicit design decision)

---

## Event Processing Pipeline

| Feature | Status | Notes |
|---------|--------|-------|
| evdev device reading via epoll | ✅ | `EvdevBackend` with epoll, SYN_REPORT aggregation |
| Device hotplug (inotify) | ✅ | `LinuxDiscovery` watches `/dev/input` |
| Device capability detection | ✅ | KEY→KEYBOARD, REL→POINTER, ABS_MT→TOUCHPAD/TOUCHSCREEN |
| Compact DevIndex remapping | ✅ | SlotMap DeviceId → u32 DevIndex |
| Event queue with filter chain | ✅ | `FilterChain` with single-filter fast path |
| Unaccelerated motion deltas | ✅ | `dx_unaccelerated`/`dy_unaccelerated` in PointerMotion |
| Sub-pixel error diffusion (f64→Fixed) | ✅ | `DualSubpixelAccumulator` for accel/unaccel streams |

## Public API Re-exports

| Symbol | Where |
|--------|-------|
| `MouseAccelProfile` | `input_rs::MouseAccelProfile` (from `processing::mouse`) |
| `DebounceConfig` | `input_rs::DebounceConfig` (from `processing::debounce`) |
| `Context` | `input_rs::Context` |
| `DeviceId` | `input_rs::DeviceId` |
| `EvdevBackend` | `input_rs::backend::evdev::EvdevBackend` |

## Mouse / Pointer

| Feature | Status | Notes |
|---------|--------|-------|
| Flat (no acceleration) profile | ✅ | `MouseFilter::flat(dpi)`, fast path skips velocity tracking |
| Adaptive acceleration profile | ✅ |
| Custom acceleration profile | ✅ | `MouseAccelProfile::Custom(SmallVec<...>)` supports piecewise-linear speed→factor curves. libinput parity with `LIBINPUT_CONFIG_ACCEL_PROFILE_CUSTOM`. |
| Adaptive acceleration profile | ✅ | `MouseFilter::new(dpi, speed)`, velocity-based with Simpson's rule |
| DPI normalization | ✅ | `dpi_factor = 1000.0 / dpi`, configurable |
| Left-handed button swap | ✅ | `ButtonSwapFilter` implemented, wired through `FilterChain::set_left_handed()` → `InputBackend` → jay `InputDeviceState` |
| Middle-button emulation | ❌ | Not implemented (2-button→middle click emulation) |
| Scroll wheel (hi-res v120) | ✅ | `PointerScroll` with 120 normalization |
| Natural scrolling (touchpad) | ✅ | `TouchpadMotionFilter.config.natural_scrolling` + `Context::set_natural_scroll()` |
| Natural scrolling (mouse) | ✅ | `NaturalScrollFilter` inverts scroll deltas, wired through `FilterChain::set_natural_scroll_mouse()` |

## Touchpad

| Feature | Status | Notes |
|---------|--------|-------|
| Two-finger scroll | ✅ | `TouchpadMotionFilter` with natural scroll config |
| Tap-to-click | ✅ | Single tap, two-finger right-click tap |
| Palm detection (edge zone) | ✅ | Configurable `palm_edge_ratio` |
| DWT (disable-while-typing) | ✅ | `dwt` config, keyboard activity suppresses touchpad |
| Touchpad acceleration | ✅ | `TouchpadFilter` with resolution-aware adaptive profile |
| Clickpad button handling | ❌ | No click method areas (button areas vs clickfinger) |
| Three-finger swipe gestures | ❌ | Gesture events defined but no filter processing |
| Pinch gestures | ❌ | Gesture events defined but no filter processing |
| Drag lock | ❌ | Not implemented |
| Thumb detection | ❌ | No thumb pressure threshold (libinput has per-model quirks) |

## Keyboard / Button

| Feature | Status | Notes |
|---------|--------|-------|
| Key event passthrough | ✅ | Direct evdev→jay mapping |
| Button debounce (configurable) | ✅ | `ButtonDebounceFilter` with full state machine, disabled by default |
| Key repeat | 🚫 | Not input-rs responsibility; jay compositor handles this |

## Device Configuration Runtime API

| Feature | Status | Notes |
|---------|--------|-------|
| Runtime DPI change | ✅ | `Context::set_mouse_dpi()` → `FilterChain::set_mouse_dpi()`, wired through `InputBackend` trait to jay `InputDeviceState` |
| Runtime accel profile change | ✅ | `Context::set_mouse_profile()` replaces filter in-place, wired through `InputBackend` trait. Supports Flat, Adaptive, and Custom profiles. |
| Natural scroll toggle (mouse) | ✅ | `NaturalScrollFilter` + `FilterChain::set_natural_scroll_mouse()` + `InputBackend::set_natural_scrolling_mouse()` |
| Natural scroll toggle (touchpad) | ✅ | `Context::set_natural_scroll()` → `InputBackend` → jay `InputDeviceState` propagation wired |
| Tap-to-click toggle | ✅ | `Context::set_tap_to_click()` + `FilterChain::set_tap_to_click()` + `InputBackend::set_tap_to_click()` |
| Click method toggle | ❌ | Not implemented |
| Middle emulation toggle | ❌ | Not implemented |
| Device enable/disable (send-events) | ✅ | `Device.disabled` flag. `Context::set_device_enabled()`. When disabled, `dispatch()` silently drops all events. `InputBackend::set_device_enabled()`. |
| LED state (numlock/capslock) | ❌ | Not forwarded from compositor to device |

## Device Discovery & Metadata

| Feature | Status | Notes |
|---------|--------|-------|
| Device name from sysfs / evdev | ✅ | Reads name, vendor/product ID, bus type, and capabilities from `evdev::Device`. DeviceCaps used for scroll source tagging (Wheel vs Finger). `
| Device vendor/product ID | ✅ | Populated from `evdev::Device::input_id()` (vendor, product, bus). Bus type maps USB2fBluetooth2fPS22fI2C. |
| Bus type (USB/Bluetooth/PS2) | ❌ | Not populated |
| Touchpad resolution info | ⚠️ | `AxisInfo` parsed from evdev absinfo but not forwarded to touchpad filter config |
| Device speed adjustment | ✅ | Context::set_mouse_speed_adjustment() → FilterChain::set_mouse_speed_adjustment(). InputRsBackend::set_accel_speed() implemented. Wired through `InputBackend` trait → `InputRsBackend` → `Context` → `FilterChain`. Jay `InputDeviceState` `#[cfg(not(feature = "libinput"))]` methods propagate via `State.backend.configure_input_device()`. jay sends both touchpad + mouse natural scroll config when `set_natural_scrolling_enabled()` is called. |

## Gesture Events

| Feature | Status | Notes |
|---------|--------|-------|
| Swipe begin/update/end | ✅ | `InputEvent` variants defined, evdev produces them, jay bridge maps to `InputEvent::SwipeBegin/Update/End` |
| Pinch begin/update/end | ✅ | `InputEvent` variants defined, jay bridge maps to `InputEvent::PinchBegin/Update/End` with scale/rotation |
| Hold begin/end | ❌ | Not defined in `InputEvent` at all |
| Gesture→jay bridge mapping | ✅ | Swipe begin/update/end and pinch begin/update/end mapped in `push_input_rs_event()`. Hold gestures not yet defined in input-rs. |

## Tablet Support

| Feature | Status | Notes |
|---------|--------|-------|
| Tablet tool proximity/tip/axis | ❌ | Not implemented |
| Tablet pad buttons/rings/strips | ❌ | Not implemented |

## Switch Support

| Feature | Status | Notes |
|---------|--------|-------|
| Switch toggle (lid switch) | ❌ | Not implemented |

## Performance Considerations

| Concern | Current State | Impact |
|---------|--------------|--------|
| PointerMotion event size | 48 bytes (was 32) | One cache line on modern CPUs. The +16 bytes is for `dx/dy_unaccelerated` which FPS games need via `zwp_relative_pointer_v1`. |
| Flat profile fast path | 74ns through FilterChain (~80ns direct) | Down from 168ns pre-optimization |
| Adaptive profile | ~132ns | Acceptable for touchpad use cases |
| FilterChain single-filter bypass | ✅ Implemented | Skips SmallVec fan-out for mouse (most common case) |
| RefCell removed from jay bridge | ✅ | Direct field access in `dispatch(&mut self)` |
| Dual SubpixelAccumulator | ✅ | Separate error diffusion streams for accel/unaccel |
| ButtonDebounceFilter disabled by default | ✅ | Zero-latency passthrough, configurable per device |
| Device name from sysfs / evdev | ✅ | Reads name, vendor/product ID, bus type, and capabilities from `evdev::Device`. DeviceCaps used for scroll source tagging (Wheel vs Finger). `

## Design Decisions (not bugs)

| Decision | Rationale |
|----------|-----------|
| Flat profile is default for mice | Lower latency, predictable pointer movement, FPS-friendly |
| Debounce disabled by default | Most devices debounce in hardware; adding latency by default is wrong |
| No quirks database (yet) | libinput has ~50 vendor-specific quirk files; input-rs relies on configurable defaults instead. A future quirks system could be added. |
| input-rs is a Rust crate (no C linking) | Direct integration with jay, no FFI overhead |
| Gesture events are cold-path variants | Boxed in InputEvent enum to keep hot path compact |
| VT/seat switching | 🚫 Not planned — user explicitly deprecated this use case |

## Touch Events

| Feature | Status | Notes |
|---------|--------|-------|
| Multi-touch slot tracking (MT-B) | ✅ | Up to 16 slots, tracks active/dirty/changed per slot |
| TouchDown event | ✅ | Emitted when ABS_MT_TRACKING_ID goes from -1 to ≥0 |
| TouchUp event | ✅ | Emitted when ABS_MT_TRACKING_ID goes to -1 |
| TouchMotion event | ✅ | Emitted when active slot has dirty x/y |
| Single-touch fallback (ABS_X/Y + BTN_TOUCH) | ❌ | Only MT-B protocol is handled. Devices using st protocol (single-touch ABS_X/Y + BTN_TOUCH without MT) will not produce touch events. Most modern touchpads use MT-B. |
| Touch cancel | ❌ | Not defined in InputEvent, not in wayland protocol either (wayland only has down/up/motion) |
| Touch frame grouping | ✅ | SYN_REPORT delimits, Frame event emitted |

## Scroll handling

| Feature | Status | Notes |
|---------|--------|-------|
| REL_WHEEL (legacy) → v120 | ✅ | Multiplied by 120.0 to normalize to v120 units |
| REL_WHEEL_HI_RES (v120) | ✅ | Preferred over legacy; passed through as-is |
| REL_HWHEEL / REL_HWHEEL_HI_RES | ✅ | Horizontal scroll, same v120 normalization |
| v120 → jay Axis120 | ✅ | Bridge converts to `InputEvent::Axis120` with dist in 120ths of a click |
| Scroll source tagging (wheel/finger/continuous) | ✅ | Touchpads emit `AxisSource::Finger`, mice emit `AxisSource::Wheel`. Based on `DeviceCaps`. |
| Hi-res scroll acceleration | ❌ | No filter applied to scroll deltas. libinput also does not accelerate scroll, but it does apply scroll method configuration (edge/two-finger/button) which we handle in TouchpadMotionFilter |

## Sub-pixel Accuracy vs libinput

**Our approach:**
- f64 → Fixed(24.8) conversion uses `round()` (round-to-nearest)
- Error diffusion via `SubpixelAccumulator` / `DualSubpixelAccumulator` carries remainders forward
- Maximum accumulated position error: ±1/256 pixel regardless of event count
- This eliminates the drift bug that would occur at non-power-of-2 DPI factors (e.g. 1200 DPI → factor 5/6)

**libinput's approach:**
- libinput uses `double` (f64) internally for all pointer motion, including accumulated deltas in the tracker ring
- It converts to `wl_fixed_t` (s23.8, same 1/256 resolution) only at the wayland protocol boundary
- The conversion at the boundary is a simple truncation (cast to i32 after multiplying by 256), NOT round-to-nearest
- libinput does NOT do error diffusion — the fractional sub-pixel error is simply discarded at the protocol boundary each event
- This means libinput has a systematic negative bias of up to ~1/256 pixel per event for motion that doesn't land on exact 1/256 boundaries

**Our advantage:** We have both round-to-nearest conversion AND error diffusion. This means:
- No systematic positive or negative bias (round-to-nearest is symmetric)
- No accumulated drift over time (error diffusion bounds total error to ±1/256 pixel)
- At 400 DPI (worst case for rounding), the per-event error is at most 1/512 pixel — imperceptible

**For scroll v120:** libinput explicitly truncates the fractional part when converting scroll acceleration to v120 integers, noting "this is acceptable because the v120 unit maps one logical click to 120 units." Our approach is consistent — we also cast to i32 for Axis120, which is fine at that resolution.

## Touchpad Feature Parity (Detailed)

What libinput provides for touchpads that input-rs does NOT yet handle:

| Feature | libinput | input-rs | Notes |
|---------|----------|----------|-------|
| Two-finger scroll | ✅ linear acceleration on scroll deltas | ✅ basic two-finger scroll in TouchpadMotionFilter | Our scroll deltas are raw counts scaled by factor, not accelerated |
| Edge scroll | ❌ | Needs edge detection zones + state machine |
| Button scroll (clickpad middle button area) | ❌ | Needs button-area mapping on clickpads |
| Click method: none | ✅ | ✅ | Tap-to-click still works |

## User-Configurable Knobs libinput Does NOT Expose (That People Want)

Based on your research, these are the touchpad-facing features synaptics exposed that libinput deliberately hides:

1. **User-tunable pressure/width/palm thresholds** — libinput hard-codes these in quirks. We should make them configurable but with sensible defaults.
2. **Fine-grained tap timing (MaxTapTime, MaxDoubleTapTime, MaxTapMove)** — libinput offers only enable/disable. Our `TouchpadConfig` could expose these.
3. **Arbitrary tap/click remapping** — synaptics per-corner actions. We only have two-finger right-click.
4. **Circular scrolling** — repeatedly requested, libinput refuses. Implementation opportunity.
5. **Kinetic/coasting scroll** — synaptics exposes CoastingSpeed. libinput says "not implemented by design." Another opportunity.
6. **Adjustable scroll speed** — probably the #1 complaint about libinput. No scroll speed setting exists. We should add a scroll speed multiplier.
7. **Pressure-dependent pointer motion** — synaptics can make cursor speed depend on finger pressure. We don't read ABS_MT_PRESSURE yet.
8. **Edge-motion continuation** — keep cursor moving when finger hits pad edge. Not in libinput.
9. **Active rectangle / area configuration** — ignore regions of the touchpad. Not in libinput for touchpads.
10. **Left-handed touchpad (rotate coordinates 180°)** — needs ABS_X/Y transformation. Not yet configurable.

## Acceleration "Feel" Notes

Your research identifies 5 factors that affect perceived feel:

1. **Physical-unit normalization before accel** — We do this: `dpi_factor = 1000.0 / dpi`. If DPI metadata is wrong (and it often is — libinput's FAQ explicitly calls this out), the same curve feels wrong because input enters at the wrong physical scale. We currently hardcode DPI=1000 as default.
   
2. **Velocity estimation window** — We use a 16-entry tracker ring with the last 16 timestamps, matching libinput's approach. Both discard trackers that diverge too far from initial velocity.

3. **Base plateau at low speed** — Our adaptive profile follows libinput's three-region model: low-speed boost (0.3→1.0), constant plateau below threshold, linear incline above. The low-speed region (`speed_ms < 0.07 → factor = 10*speed+0.3`) mimics libinput's touch-friendly deceleration.

4. **Transition band slope** — Controlled by `incline_factor` and `accel` (max) parameters. Currently hardcoded. Should be runtime-configurable.

5. **Maximum gain** — Our `accel` parameter caps at 2.0+speed*1.5. Matches libinput's default range.

**Custom acceleration profile:** libinput now exposes `LIBINPUT_CONFIG_ACCEL_PROFILE_CUSTOM` where you define f(x) as a set of (speed, factor) points. We should support this — it's a direct response to "feel" complaints and would be a superior approach to the fixed adaptive profile.

**Key insight from Hutterer's analysis:** For properly configured mice, the "feel" problems mostly come from bad MOUSE_DPI metadata, not the curve itself. Our DPI hardcoding at 1000 is a risk — we need runtime DPI configuration from sysfs/evdev, and eventually from the compositor.
