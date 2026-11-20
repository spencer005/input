# input-rs Feature Parity with libinput

## Status: Core mouse/keyboard and touchpad tap/click functional; tablet, policy, and several libinput feature areas still partial

### Legend
- ✅ = Implemented and working
- ⚠️ = Partially implemented or has known gaps
- ❌ = Not implemented
- 🚫 = Not planned (explicit design decision)

### Completion Requirements

Detailed requirements for marking missing libinput-parity areas complete live in
[`docs/missing-features/`](docs/missing-features/README.md). A feature should
not move to ✅ here until its linked completion document is satisfied or the
remaining requirements are explicitly moved to another tracked document.

Criterion workloads are performance baselines, not correctness proof. The
missing-feature benchmark suite now includes deterministic anti-overfit variants
for every benchmark group, plus multi-seed variant suites for the highest-risk
fixed-trace paths, but completion still requires deterministic assertions or
replay fixtures for observable event semantics, cleanup behavior, and runtime
configuration effects.

### Latest input-rs implementation pass

The following behavior is now implemented in `input-rs` with deterministic unit
tests (102 library tests passing):

- Configurable tap button maps, clickfinger maps, tap timeout/movement
  threshold, tap-drag timeout, drag-lock timeout, and maximum tap finger count.
- Real drag lock (hold across lift, unlock on next contact or timeout) with
  cancel/removal cleanup.
- Clickpad detection from `INPUT_PROP_BUTTONPAD`, configurable software button
  geometry, clickfinger mapping, and a documented left-handed clickpad policy
  applied after click-method selection.
- Raw pressure/size palm thresholds, thumb pressure threshold, and late
  firmware palm reclassification.
- Configurable DWT timeout with internal-only keyboard gating, DWTP via
  `POINTING_STICK` capability, and stylus-proximity touchpad suppression.
- Timer-addressable delayed output (`Filter::next_timeout_us`,
  `Context::dispatch_timeouts`) for middle-button fallback and drag lock.
- Relative-device rotation plus calibration reset/query.
- Tablet tool proximity/axis/tip/button and tablet pad button/ring/strip/dial
  evdev parsing.
- Switch-driven tablet-mode policy in `Context`, initial switch seeding,
  removal-source cleanup, and composed user/switch/external-mouse disable
  reasons.
- LED state read plus evdev `EV_LED` writes through `Context::set_led_state`.
- Explicit `DeviceProfile` matching with ambiguity rejection, internal
  classification overrides, pointer defaults, and touchpad defaults.
- Explicit scroll scaling/inversion (`ScrollScaleFilter`) and arbitrary
  pointer-button remapping (`ButtonMapFilter`).
- Effective-configuration inspection through `Context::config_snapshot()` /
  `FilterChain::config_snapshot()`.
- Tablet tool identity, pressure-range normalization, and normalized active-area transforms.
- Hysteresis validation is folded into one finite check after accumulation, and hot filter/chain methods are inlined.
- Hot-path `DeviceInfo` is borrowed through the filter chain instead of cloned
  per event/per filter; the external-mouse policy scan no longer runs on every
  dispatch; and filters now transform the owned event in place, returning a
  16-byte `Step` so the common path moves no event and allocates no per-filter
  buffer. The Context benchmark was corrected to exclude per-iteration backend
  allocation.
- Button-lifecycle and processing-invariant fixes: per-code debounce state with
  real timeouts, timer-generated events traversing the remaining filter chain,
  canonical setter ordering, held-button remapping on left-handed changes,
  disengaged filter cleanup for middle emulation/button scrolling, transactional
  device add, physical-unit custom acceleration curves with validation, and
  time-windowed velocity estimation that includes the newest sample.

The remaining jay-side work is user-facing configuration/command plumbing for
the new knobs (tap timing/maps, clickpad geometry, profile selection, rotation,
LED updates, switch policy), plus sysfs-based device grouping. Those gaps are
still marked ⚠️ below rather than complete.

---

## Event Processing Pipeline

| Feature | Status | Notes |
|---------|--------|-------|
| evdev device reading via epoll | ✅ | `EvdevBackend` with epoll, SYN_REPORT aggregation |
| Device hotplug (inotify) | ✅ | `LinuxDiscovery` watches `/dev/input` |
| SYN_DROPPED resynchronization | ✅ | Cancels active touches, clears partial relative/scroll/switch state, diffs `EVIOCGKEY` against tracked keys/buttons (deterministic order), and resynchronizes tablet tool proximity/axis/tip plus pad ring/strip from `EVIOCGABS`. MT-B slot resync still relies on subsequent frames because the evdev crate does not expose `EVIOCGMTSLOTS`. |
| Device capability detection | ⚠️ | KEY/REL/ABS/EV_SW detection exists for keyboard, pointer, touchpad, touchscreen, and switch. `DeviceInfo` now also reports `INPUT_PROP_BUTTONPAD`/`TOPBUTTONPAD` (clickpads), `INPUT_PROP_POINTING_STICK`, bus-derived internal/external classification, initial LED state, and dedicated `TABLET`/`TABLET_PAD` capabilities from real evdev tool/pad nodes. Explicit device profiles can override internal classification and supply touchpad/pointer defaults. Still lacks real sysfs grouping, top-button clickpad policy, and tablet tool identity. See [`device-policy-and-metadata.md`](docs/missing-features/device-policy-and-metadata.md). |
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
| Adaptive acceleration profile | ✅ | `MouseFilter::new(dpi, speed)`, velocity-based with Simpson's rule |
| Custom acceleration profile | ⚠️ | `MouseAccelProfile::Custom(SmallVec<...>)` supports piecewise-linear speed→factor curves in input-rs, and `feature_pointer_dpi_and_motion_config/custom_profile_speed_ramp_current_filter` covers a human-like speed ramp through the current filter. Not complete because jay's user-facing profile config still exposes only flat/adaptive, there is no custom curve syntax or validation contract at the API boundary, and hotplug replay/inspection for custom curves is not documented. |
| DPI normalization | ✅ | `dpi_factor = 1000.0 / dpi`, configurable |
| Left-handed button swap | ✅ | `ButtonSwapFilter` implemented, wired through `FilterChain::set_left_handed()` → `InputBackend` → jay `InputDeviceState` |
| Middle-button emulation | ⚠️ | `MiddleButtonEmulationFilter` and input-rs config plumbing exist. Timeout fallback is now timer-addressable: `Filter::next_timeout_us()` and `FilterChain`/`Context::dispatch_timeouts()` flush a pending press without unrelated input, and deterministic tests cover chord synthesis plus deadline flush. Criterion workloads cover both chord directions, release ordering, filter ordering, runtime changes, and removal/disable cleanup. Still missing jay-side timeout/default documentation, a public availability query, and deterministic coverage for every removal and reconfiguration path. |
| Scroll wheel (hi-res v120) | ✅ | `PointerScroll` with 120 normalization |
| Natural scrolling (touchpad) | ✅ | `TouchpadMotionFilter.config.natural_scrolling` + `Context::set_natural_scroll()` |
| Natural scrolling (mouse) | ✅ | `NaturalScrollFilter` inverts scroll deltas, wired through `FilterChain::set_natural_scroll_mouse()` |
| Scroll scale / per-axis inversion | ✅ | `ScrollScaleFilter` with independent x/y factors; `Context::set_scroll_scale()`, `FilterChain::set_scroll_scale()`, and `PointerProfile::scroll_scale_x/y`. Positive scales, negative inverts, zero suppresses. Explicit extension; not libinput parity. |
| Arbitrary button remapping | ✅ | `ButtonMap` + `ButtonMapFilter`; `Context::set_button_map()`, `FilterChain::set_button_map()`, and `PointerProfile::button_map`. Runs after left-handed swap and after middle emulation, so delayed chord fallback output is mapped too. |

## Touchpad

| Feature | Status | Notes |
|---------|--------|-------|
| Two-finger scroll | ✅ | `TouchpadMotionFilter` with natural scroll config; now represented alongside `TouchpadScrollMethod::Edge` in touchpad config. |
| Tap-to-click | ⚠️ | Single tap, optional two-finger right-click tap, three-finger middle tap, configurable tap button maps (left/right/middle and left/middle/right), configurable tap timing/movement/drag timeouts, maximum tap finger count, and tap-and-drag are implemented in `TouchpadMotionFilter`. Tap maps are independent of left-handed mode, matching libinput. Deterministic tests now cover tap maps, finger-count limits, configurable timeout/movement cancellation, drag lock, and physical-click/tap interaction; Criterion workloads cover the remaining interaction matrix. Still missing jay config forwarding for the new timing/map knobs and exact libinput-style delayed tap release for tap-and-drag. |
| Palm detection (edge zone) | ✅ | Configurable `palm_edge_ratio` |
| Palm detection (firmware tool flag) | ⚠️ | A palm contact joining an active gesture is ignored rather than cancelling the gesture; configurable gesture thresholds are covered by deterministic tests. Raw `ABS_MT_TOOL_TYPE` is preserved in `TouchData`. Contacts marked `TouchToolType::Palm` are ignored on touch down, and a late finger-to-palm reclassification on `TouchMotion` now latches the palm classification, decrements the tap finger count, drops pending motion, and cancels gestures when no finger remains. Raw pressure/size palm thresholds are configurable per device/profile, and deterministic tests cover touch-down and late-reclassification suppression. The palm-vs-gesture policy is now deterministic (palm is ignored, gesture continues) and covered by a test. Still missing normalized pressure/size units and device-profile threshold polish. |
| DWT (disable-while-typing) | ⚠️ | `dwt` config, a configurable per-device timeout, and the seat keyboard timestamp suppress touchpad motion. With `Context::dwt_internal_only` (default true), only bus-classified internal keyboards update the DWT timestamp, so unrelated external keyboards do not suppress the touchpad. Deterministic tests cover expiry; full sysfs/group-based keyboard grouping is still missing. |
| DWTP / stylus-touch arbitration | ⚠️ | `Seat.last_trackpoint_time` is updated from devices with `DeviceCaps::POINTING_STICK`, and `TouchpadMotionFilter` suppresses motion/taps for the configurable DWTP timeout. `Seat.stylus_in_proximity` is updated from `TabletToolProximity` events, and touchpad input is suppressed while a stylus is in range (configurable). Removal clears stylus proximity. Still relies on capability/bus classification rather than real device groups, and deterministic multi-device grouping coverage is missing. |
| Touchpad acceleration | ✅ | `TouchpadFilter` with resolution-aware adaptive profile |
| Clickpad button handling | ⚠️ | Clickpad detection now comes from `INPUT_PROP_BUTTONPAD` (`DeviceInfo::is_clickpad`, `DeviceCaps::CLICKPAD`) with an explicit config override, and physical `BTN_LEFT` is only remapped on detected/overridden clickpads. `ButtonAreas` supports configurable bottom/left/right geometry, latches the logical button at press time, and applies the documented left-handed policy (bottom-left/bottom-right swap, main area stays primary) inside `TouchpadMotionFilter` rather than pre-swapping raw input. `Clickfinger` supports configurable left/right/middle and left/middle/right maps. Deterministic tests cover detection gating, left-handed areas, and clickfinger maps. Still missing jay config for the new geometry/map knobs, a stable finger-count decision window, and top-button clickpad policy. |
| Three-finger swipe gestures | ⚠️ | Gesture thresholds are now configurable (`hold_timeout_us`, `gesture_motion_threshold_mm`, `pinch_scale_threshold`, `pinch_rotation_threshold_rad`), and a palm joining an active gesture no longer cancels it. `TouchpadMotionFilter` has an initial three-or-more-finger center-motion recognizer and jay bridge forwarding. Touch cancel emits a cancelled gesture end, and explicit device disable resets filter state. Criterion workloads now include three-finger swipe, four-finger swipe, swipe and pinch finger-count cancellation, hold touch-cancel cleanup, competition cases, active filter reset, context-level active device disable while swipe/pinch/hold streams are active, context-level removal while swipe/pinch/hold streams are active, and direct removal-shaped input while swipe/pinch/hold streams are active. Monitor-driven removal reaches input-rs context cleanup. Still needs deterministic removal/cancellation coverage and complete disambiguation before it can be marked complete. |
| Pinch gestures | ⚠️ | Pinch thresholds are configurable through `TouchpadConfig`/`Context::set_touchpad_gesture_config()`. `TouchpadMotionFilter` has an initial two-finger pinch recognizer for spacing/angle changes and jay bridge forwarding. Touch cancel emits a cancelled gesture end, and explicit device disable resets filter state. Criterion workloads now cover scale in/out, rotation in both directions, finger-count cancellation, context-level active device disable, context-level active device removal, and direct removal-shaped input while a pinch stream is active. Monitor-driven removal reaches input-rs context cleanup. Still needs multi-finger parity, deterministic removal/cancellation assertions, and documented disambiguation. |
| Drag lock | ✅ | `TouchpadMotionFilter` keeps the logical drag button pressed across a lift when `tap_drag_lock` is enabled, releases it on the next contact or the configurable `drag_lock_timeout_us`, and clears state on touch cancel/removal. Timeouts are dispatchable without unrelated input through `Context::next_timeout_us()` / `dispatch_timeouts()`. Deterministic tests cover lock, unlock, timeout, and cleanup. jay's existing `SetTapDragLock` path already forwards the enable flag; timeout exposure is still only in input-rs. |
| Thumb detection | ⚠️ | The lower-zone thumb-rest heuristic now supports an optional raw `thumb_pressure_threshold`, is configurable per device/profile, and deterministic tests cover palm thresholding. A lone lower-zone contact is still treated as an ordinary finger (documented current policy), and an intentional clickpad bottom-area click is not reclassified as a thumb because thumb detection only applies while another contact is active. Still missing width/speed heuristics, a final clickpad-area arbitration policy, and deterministic lone-contact/clickpad coverage. |

## Keyboard / Button

| Feature | Status | Notes |
|---------|--------|-------|
| Key event passthrough | ✅ | Direct evdev→jay mapping |
| Button debounce (configurable) | ⚠️ | `ButtonDebounceFilter` is disabled by default and now has collision-free state per Linux input code (0..0x2ff), deadline-driven deferred releases/represses, in-place configuration updates, and held-code seeding when enabled at runtime. `Context::set_debounce()` flushes pending transitions through the remaining chain, and canonical filter ordering places debounce first. Deterministic tests cover bounce suppression, aliasing, deferred key/button releases, spurious re-press, boundary equality, and flush traversal. Still missing a normal compositor command/protocol path, hotplug replay, diagnostics/inspection, multi-button frame-flush policy, and zero-timeout edge coverage. See [`button-debounce.md`](docs/missing-features/button-debounce.md). |
| Key repeat | 🚫 | Not input-rs responsibility; jay compositor handles this |

## Device Configuration Runtime API

| Feature | Status | Notes |
|---------|--------|-------|
| Runtime DPI change | ⚠️ | `Context::set_mouse_dpi()` and `InputBackend::set_mouse_dpi()` now reach input-rs, pending backend-level DPI requests apply to hotplugged matching pointers, `MOUSE_DPI` string parsing exists, and Criterion workloads cover low DPI, high DPI, live context changes, parser formats, generic-name hotplug replay, duplicate-name replay, re-add-shaped replay, and matching non-pointer rejection through bench-local input-rs context hooks. Still missing device-add metadata discovery, user-facing jay config, effective-DPI inspection, and jay-side deterministic coverage for all production alias replay cases. |
| Runtime accel profile change | ⚠️ | `Context::set_mouse_profile()` replaces the mouse filter in-place while preserving current DPI and speed adjustment, and Criterion workloads cover flat→adaptive plus adaptive→custom runtime changes during representative pointer movement. jay's current backend/config path only exposes flat/adaptive via `InputBackend::set_accel_profile(flat: bool)`, so custom profile configuration, clear/reset behavior, inspection, hotplug replay semantics, and deterministic DPI-preservation coverage are still missing. |
| Natural scroll toggle (mouse) | ✅ | `NaturalScrollFilter` + `FilterChain::set_natural_scroll_mouse()` + `InputBackend::set_natural_scrolling_mouse()` |
| Natural scroll toggle (touchpad) | ✅ | `Context::set_natural_scroll()` → `InputBackend` → jay `InputDeviceState` propagation wired |
| Tap-to-click toggle | ✅ | `Context::set_tap_to_click()` + `FilterChain::set_tap_to_click()` + `InputBackend::set_tap_to_click()` |
| Click method toggle | ⚠️ | Runtime config is plumbed from jay into `TouchpadConfig`, and initial button-area/clickfinger physical-click behavior exists. Still missing complete policy, detection, configuration, and deterministic coverage. |
| Middle emulation toggle | ⚠️ | jay config now routes to input-rs `Context::set_middle_button_emulation()`, and Criterion workloads cover runtime enable/disable while neutral, pending, and active through the current filter-chain API. Still missing deterministic assertions, documented defaults, and jay-side config replay coverage before complete. |
| Device enable/disable (send-events) | ⚠️ | `Device.disabled` flag, `Context::set_device_enabled()`, and explicit context removal through `Context::remove_device_by_index()` exist, and jay's `SetDeviceEnabled` config now forwards into input-rs. Disabling or device removal synthesizes releases for tracked keys/buttons, emits `TouchCancel` for tracked touches, resets stateful filters, and removal drops the backend fd/state where supported. Jay monitor-driven removal now stores the input-rs `DevIndex` and calls back into input-rs removal cleanup; backend-originated `DeviceRemoved` also clears the `Context` device record. Criterion workloads cover manual disable, monitor-shaped context removal, and backend-originated removal while touch is active. Still missing composed policy reasons and deterministic replay coverage for every removal path. |
| LED state (numlock/capslock) | ⚠️ | `DeviceInfo` reads the initial kernel LED state, `Context::set_led_state()`/`led_state()` expose and record it, and `EvdevBackend` writes `EV_LED` events (`LED_NUML`, `LED_CAPSL`, `LED_SCROLLL`) plus `SYN_REPORT` to the device fd. Standalone `DevicePolicyState` still models policy-level LED state. Still missing jay-side config/command plumbing, hotplug reapply policy, and deterministic hardware write coverage (the raw write path is exercised only by code inspection). |

## Device Discovery & Metadata

| Feature | Status | Notes |
|---------|--------|-------|
| Device name from sysfs / evdev | ✅ | Uses the evdev-reported name when available and falls back to the name supplied during device add. |
| Device vendor/product ID | ✅ | Populated from `evdev::Device::input_id()`. |
| Bus type (USB/Bluetooth/I2C/PS2/unknown) | ✅ | Populated from `evdev::Device::input_id().bus_type()` and mapped into `BusType`. |
| Touchpad axis min/max/resolution info | ✅ | `AxisInfo` is parsed from evdev absinfo and passed through `DeviceInfo` to touchpad filters. |
| Disable reason composition | ⚠️ | `Device.disable_reasons` now composes user, switch, external-mouse, and temporary reasons in real dispatch without one policy re-enabling another. Tablet-mode policy is applied in `Context`, external-mouse policy is opt-in via `Context::set_disable_internal_on_external_mouse()`, and `DeviceProfile` supplies explicit internal/pointer/touchpad defaults with documented match/ambiguity rules. Standalone `DevicePolicyState` remains available for policy tests. Still missing jay config for the new policy switches, real sysfs grouping, and profile diagnostics surfaced to users. |
| Device speed adjustment | ⚠️ | `Context::set_mouse_speed_adjustment()` → `FilterChain::set_mouse_speed_adjustment()` preserves current DPI/profile, `InputRsBackend::set_accel_speed()` stores hotplug-replay state, and `feature_pointer_dpi_and_motion_config/runtime_speed_adjustment_change_current_chain` covers a runtime change during representative pointer movement. Wired through `InputBackend` trait → `InputRsBackend` → `Context` → `FilterChain`, with jay `InputDeviceState` propagation via `State.backend.configure_input_device()`. Still needs deterministic assertions for DPI/profile preservation, clear/reset semantics, and user-visible inspection of the effective speed before it can be complete. See [`pointer-dpi-and-motion-config.md`](docs/missing-features/pointer-dpi-and-motion-config.md). |

## Gesture Events

| Feature | Status | Notes |
|---------|--------|-------|
| Swipe begin/update/end | ⚠️ | Boxed `InputEvent` variants exist, `TouchpadMotionFilter` generates an initial touchpad swipe stream, the jay bridge maps it to `InputEvent::SwipeBegin/Update/End`, and `TouchCancel` produces a cancelled end. Device disable and context removal reset stateful filters, and active tracked touches are cancelled on disable/removal. Still missing deterministic coverage and full libinput gesture policy parity. |
| Pinch begin/update/end | ⚠️ | Boxed `InputEvent` variants exist, `TouchpadMotionFilter` generates an initial two-finger pinch stream with scale/rotation, the jay bridge maps it to `InputEvent::PinchBegin/Update/End`, and `TouchCancel` produces a cancelled end. Device disable and context removal reset stateful filters, and active tracked touches are cancelled on disable/removal. Still missing multi-finger parity, documented disambiguation, and deterministic coverage. |
| Hold begin/end | ⚠️ | Boxed `InputEvent` variants exist, `TouchpadMotionFilter` generates stationary hold begin/end, the jay bridge maps to jay `HoldBegin/End`, and `TouchCancel` produces a cancelled end. Device disable and context removal reset stateful filters, and active tracked touches are cancelled on disable/removal. Criterion workloads cover stationary hold, movement cancellation, touch-cancel cancellation after hold begin, and context removal while hold is active. Still needs default policy documentation and deterministic cancellation coverage. |
| Gesture→jay bridge mapping | ⚠️ | Swipe, pinch, and hold variants are mapped in `push_input_rs_event()`. Mapping exists, but gesture recognition is still partial. |

## Tablet Support

| Feature | Status | Notes |
|---------|--------|-------|
| Tablet tool proximity/tip/axis | ⚠️ | Evdev tool nodes are detected and emit proximity/axis/tip/tool-button events with `TabletToolKind` identity (pen/eraser/brush/pencil/airbrush/mouse/lens), coalesced at `SYN_REPORT` in proximity→axis→tip order. `SYN_DROPPED` resyncs key/abs/tablet state. `TabletTransformFilter`/`Context::set_tablet_config()` provide pressure-range normalization and normalized active-area mapping. Still missing serial tracking, per-tool pressure policy, grouped-node removal policy, and compositor tablet protocol bridging. |
| Tablet pad buttons/rings/strips | ⚠️ | Evdev pad nodes are detected from `BTN_0..BTN_9`/`ABS_WHEEL`, with `DeviceCaps::TABLET_PAD`. Pad buttons emit `TabletPadButton`; `ABS_WHEEL` emits a ring, `ABS_MISC` a strip, and `REL_DIAL` a dial delta at `SYN_REPORT`. Deterministic tests cover all three value events. Still missing pad mode groups, top-button/ring-strip capabilities beyond the single index, cleanup policy, grouping, and jay tablet protocol bridging. |

## Absolute Device Transforms

| Feature | Status | Notes |
|---------|--------|-------|
| Touchscreen calibration matrix | ✅ | `AbsoluteTransformFilter` applies a validated affine matrix to touch down/motion coordinates using device axis bounds and clamps transformed normalized coordinates. `Context::reset_calibration_matrix()` and `Context::calibration_matrix()` now provide reset/query, and deterministic tests cover identity, inversion, 90-degree rotation, clamping, and non-finite rejection. jay's existing calibration config forwards through the same path. Remaining policy work is active-contact reconfiguration (currently immediate) and tablet absolute-axis composition. |
| Relative-device rotation | ✅ | `RelativeRotationFilter` rotates accelerated and unaccelerated pointer deltas by a validated `0..=359` degree angle, leaving scroll untouched. `Context::set_rotation()`, `reset_rotation()`, and `rotation_angle()` provide per-device runtime config; `DeviceProfile::pointer.rotation_degrees` supplies a default. Deterministic tests cover 0/90/180/arbitrary angles, both delta streams, scroll pass-through, and invalid-angle rejection without changing the installed filter. jay-side user config still needs to forward the angle. |
| Tablet active area / pressure range | ✅ | `TabletTransformFilter` applies a per-device `TabletConfig` with `pressure_range: Option<(min, max)>` (clamped rescale to `[0,1]`) and `active_area: Option<ActiveArea>` (normalized rectangle, outside coordinates clamped and mapped to the full logical range). Runtime setters and deterministic tests cover normalization, clamping, mapping, and invalid-config rejection. Per-tool configuration and compositor forwarding remain open. |

## Switch Support

| Feature | Status | Notes |
|---------|--------|-------|
| Switch toggle (lid/tablet-mode switch) | ⚠️ | `EV_SW` detection, typed `SwitchKind`, initial switch-state read, SYN_REPORT coalescing, duplicate suppression, unknown passthrough, and jay mapping exist. Tablet-mode policy is now dispatch-integrated: initial switch state seeds policy, internal touchpads/pointers are disabled while tablet mode is active, removal clears only the removed switch's contribution, and switch/user/external-mouse disable reasons compose independently. `SwitchPolicy` exposes per-class defaults. Deterministic tests cover disable/restore, user-disable composition, removal, and initial state. Still relies on bus-derived internal classification and lacks sysfs device grouping. See [`switch-devices.md`](docs/missing-features/switch-devices.md). |

## Performance Considerations

| Concern | Current State | Impact |
|---------|--------------|--------|
| PointerMotion event size | 48 bytes (was 32) | One cache line on modern CPUs. The +16 bytes is for `dx/dy_unaccelerated` which FPS games need via `zwp_relative_pointer_v1`. |
| Flat profile fast path | 74ns through FilterChain (~80ns direct) | Down from 168ns pre-optimization |
| Adaptive profile | ~132ns | Acceptable for touchpad use cases |
| FilterChain single-filter bypass | ✅ Implemented | Skips SmallVec fan-out for mouse (most common case) |
| FilterChain one-event fast path | ✅ Implemented | `Filter::process(&mut InputEvent, ...)` mutates the event in place and returns a 16-byte `Step::{Keep, Suppress, FanOut}`. The chain owns the event once and borrows it through each filter; only multi-event filters allocate a fan-out result. `Context` reuses one output buffer via `FilterChain::process_into`. 9-event 8 kHz-style batch: ~0.31–0.38 µs full `Context::dispatch` (from ~1.62 µs before the pass); two-filter chain ~19 ns/event. Touchpad motion/frame/buttons are in-place; touch down/up/cancel still use a local result container for their multi-event outputs. |
| RefCell removed from jay bridge | ✅ | Direct field access in `dispatch(&mut self)` |
| Dual SubpixelAccumulator | ✅ | Separate error diffusion streams for accel/unaccel |
| ButtonDebounceFilter disabled by default | ✅ | Zero-latency passthrough, configurable per device |
| Device metadata from evdev | ✅ | Reads name, vendor/product ID, bus type, axis info, and capabilities from `evdev::Device`. |
| Velocity window | ✅ | 128-entry tracker ring, 16 ms sample window, newest displacement included. Stable physical window across 125 Hz–8 kHz instead of a fixed 16 reports. |
| Custom acceleration units | ✅ | Points are `(mm/s, factor)`; normalized counts/µs are converted with the 1000-counts-per-inch scale. Validation happens at construction/configuration, not per event. |
| Canonical filter order | ✅ | Configuration setters normalize to a fixed pipeline; equivalent configurations no longer depend on setter call order. |
| Debounce state | ✅ | Per-code state (0..0x2ff), no modulo aliasing; deferred transitions are deadline-driven and flushed through the remaining chain. |
| Device add | ✅ | `try_add_device_fd()` is transactional and returns the backend error; no phantom `DeviceAdded`. |
| Per-event metadata | ✅ | `Filter::process` takes `Option<&DeviceInfo>`; Context passes `&dev.info`. No per-event `String`/`Vec` clones in the common path. |
| Dispatch policy scan | ✅ | External-mouse presence is refreshed on add/remove, not on every dispatch. |

## Design Decisions (not bugs)

| Decision | Rationale |
|----------|-----------|
| Flat profile is default for mice | Lower latency, predictable pointer movement, FPS-friendly |
| Debounce disabled by default | Most devices debounce in hardware; adding latency by default is wrong |
| No libinput-style quirks database | Intentional: input-rs should not copy libinput's hidden device-quirks model. `DeviceProfile` now provides the explicit, user-visible alternative: bus/vendor/product/name match keys, internal-classification overrides, pointer DPI/speed/rotation defaults, broken-axis declarations, and a full touchpad defaults block. Specificity wins; equally specific matches are rejected as ambiguous with a diagnostic; explicit selection by name ignores match keys; missing profiles fall back to safe defaults. Deterministic tests cover matching, ambiguity, and override application. |
| input-rs is a Rust crate (no C linking) | Direct integration with jay, no FFI overhead |
| Gesture events are cold-path variants | Boxed in InputEvent enum to keep hot path compact |
| Circular scrolling | 🚫 Not planned for this pass. It is a synaptics extension, not libinput parity; adding it later should be an explicit opt-in extension. |
| Kinetic/coasting scroll | 🚫 Not planned for this pass. It needs timer-driven synthesis and is not libinput parity. |
| Scroll speed multiplier | ❌ Tracked extension. libinput deliberately does not accelerate scroll; input-rs may add an explicit user-visible multiplier later. |
| Canonical filter order | Configuration setters normalize the chain to calibration → debounce → motion precision → touchpad recognition → middle emulation → button swap → button mapping → pointer/touchpad acceleration → button scroll → natural scroll → rotation. Equivalent configurations no longer depend on setter call order. `FilterChain::add` remains explicit for advanced callers. |
| Transactional device add | `Context::try_add_device_fd()` returns the backend registration error and leaves no placeholder device or `DeviceAdded` event. `add_device_fd()` remains as a compatibility wrapper that logs and returns a null id. |
| VT/seat switching | 🚫 Not planned — user explicitly deprecated this use case |

## Touch Events

| Feature | Status | Notes |
|---------|--------|-------|
| Multi-touch slot tracking (MT-B) | ✅ | Up to 16 slots, tracks active/dirty/changed per slot |
| TouchDown event | ✅ | Emitted when ABS_MT_TRACKING_ID goes from -1 to ≥0 |
| TouchUp event | ✅ | Emitted when ABS_MT_TRACKING_ID goes to -1 |
| TouchMotion event | ✅ | Emitted when active slot has dirty x/y |
| Single-touch fallback (ABS_X/Y + BTN_TOUCH) | ⚠️ | Conservative evdev fallback added for non-MT, non-relative, non-tablet-tool devices: `BTN_TOUCH` down/up with `ABS_X/Y` emits slot-0 touch down/motion/up, and active synthetic contacts are cancelled on `SYN_DROPPED`/device removal. Still needs deterministic tests and documented pointer-emulation behavior. |
| Touch cancel | ⚠️ | `InputEvent::TouchCancel` exists, jay bridge forwarding is wired, evdev cancels active touches on `SYN_DROPPED` or device removal, `Context::set_device_enabled(false)` and context removal cancel tracked active touches, and touchpad motion cleanup releases active gesture/tap state. Human-like Criterion workloads cover direct touch cancel, touchpad cancel cleanup, MT-B down/motion/up/frame streams, same-frame multi-slot updates, active MT-B/multi-slot/single-touch direct removal, slot reuse, tracking-id-change-shaped metadata, metadata propagation, active-touch manual device-disable cleanup through `Context::set_device_enabled(false)`, and backend-originated removal cleanup. Still missing deterministic replay coverage. |
| Per-contact touch metadata | ⚠️ | `TouchData` carries optional raw pressure, touch major/minor, tool type, and tracking id. Evdev fills MT-B pressure/size/tool/tracking metadata and single-touch pressure, and clears slot metadata on contact end. Still missing normalized units, deterministic replay coverage, single-touch size/tool metadata, and pressure/size policy consumers. |
| Touch frame grouping | ✅ | SYN_REPORT delimits, Frame event emitted |

## Scroll handling

| Feature | Status | Notes |
|---------|--------|-------|
| REL_WHEEL (legacy) → v120 | ✅ | Multiplied by 120.0 to normalize to v120 units |
| REL_WHEEL_HI_RES (v120) | ✅ | Preferred over legacy based on explicit presence in the frame, even when the accumulated high-resolution value is zero; passed through as-is. `SYN_DROPPED` clears partial scroll state. |
| REL_HWHEEL / REL_HWHEEL_HI_RES | ✅ | Horizontal scroll, same v120 normalization |
| v120 → jay Axis120 | ✅ | Bridge converts to `InputEvent::Axis120` with dist in 120ths of a click |
| Scroll source tagging (wheel/finger/continuous) | ✅ | Touchpads emit `AxisSource::Finger`, mice emit `AxisSource::Wheel`. Based on `DeviceCaps`. |
| Edge scrolling | ⚠️ | `TouchpadScrollMethod::Edge` supports right-edge vertical and bottom-edge horizontal scrolling for contacts that start in the edge zone, with natural-scroll handling. Runtime backend configuration can select the method, and Criterion workloads now cover right/bottom edge drags, natural scroll on both edge axes, crossing into/out of edge zones under the current latch-at-touch-down policy, active direct-removal, explicit-disable, and context-removal shapes, and method changes while neutral and while scrolling. Still missing user-facing jay config, method capability reporting, and deterministic coverage. |
| Button scrolling | ⚠️ | `ButtonScrollFilter` converts pointer motion to `PointerScroll` while a configured button is held or lock-toggled, and deliberately swallows that button. Runtime input-rs/backend configuration can enable/replace it with a selected button and lock mode. Criterion workloads cover hold mode, lock mode, natural scroll, runtime enable/disable/replacement through the current filter-chain API, explicit device disable while hold/lock scroll is active, context-level device removal while hold/lock scroll is active, and direct removal-shaped input while hold/lock scroll is active. Still missing user-facing jay config, device classification defaults, removal/seat-reset coverage, and continuous scroll source tagging in jay. |
| Hi-res scroll acceleration | 🚫 | Not a libinput feature to copy: libinput does not accelerate scroll deltas. input-rs now offers explicit per-axis scroll scaling/inversion as an extension (`ScrollScaleFilter`, `Context::set_scroll_scale`, profile `scroll_scale_x/y`) rather than acceleration inside the v120 path. |

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

Touchpad behavior that is not complete yet or still differs from libinput:

| Feature | libinput | input-rs | Notes |
|---------|----------|----------|-------|
| Two-finger scroll | ✅ linear acceleration on scroll deltas | ✅ basic two-finger scroll in TouchpadMotionFilter | Our scroll deltas are raw counts scaled by factor, not accelerated |
| Edge scroll | ✅ edge method configuration | ⚠️ initial `TouchpadScrollMethod::Edge` | Right-edge vertical and bottom-edge horizontal scrolling exist with latch-at-touch-down behavior, natural-scroll handling, runtime method changes, and human-like direct-removal, context-removal, and disable cleanup workloads. Still needs user-facing jay config, capability reporting, and deterministic coverage before the broader feature area can be complete. |
| Button scroll | ✅ button scrolling configuration | ⚠️ initial `ButtonScrollFilter` | Hold and lock modes exist through the current filter-chain/backend configuration path, including natural scroll, runtime replacement, explicit device-disable cleanup, context-removal cleanup, and direct removal-shaped workloads for hold and lock mode. Still needs user-facing jay config, device-class defaults such as trackpoint/button-area policy, final clickpad middle-area mapping if desired, and deterministic coverage. |
| Click method: none | ✅ | ✅ | Tap-to-click still works |

## User-Configurable Knobs libinput Does NOT Expose (That People Want)

Based on your research, these are the touchpad-facing features synaptics exposed that libinput deliberately hides. Items 7–10 (pressure-dependent motion, edge-motion continuation, active rectangle, and 180-degree touchpad rotation) are **not planned for this pass**: they are documented as optional extensions, not libinput parity. Relative-device rotation and left-handed clickpad policy are implemented separately.

1. **User-tunable pressure/width/palm thresholds** — libinput often handles these through hidden device data. `input-rs` should expose them through explicit configuration, sensible defaults, and user-visible device-profile overrides with diagnostics.
2. **Fine-grained tap timing (MaxTapTime, MaxDoubleTapTime, MaxTapMove)** — libinput offers only enable/disable. `TouchpadConfig` now exposes `tap_timeout_us`, `tap_move_threshold_mm`, `tap_drag_timeout_us`, and `drag_lock_timeout_us`, with sanitization and deterministic boundary tests. jay still needs user-facing config for these.
3. **Arbitrary tap/click remapping** — synaptics per-corner actions. We support the two libinput tap/clickfinger maps (`LeftRightMiddle`, `LeftMiddleRight`) plus three-finger tap mapping; arbitrary per-corner actions remain an extension.
4. **Circular scrolling** — repeatedly requested, libinput refuses. Implementation opportunity. A human-like current-behavior workload is tracked in `touchpad-scroll-methods.md`.
5. **Kinetic/coasting scroll** — synaptics exposes CoastingSpeed. libinput says "not implemented by design." Another opportunity. A flick-and-idle current-behavior workload is tracked in `touchpad-scroll-methods.md`.
6. **Adjustable scroll speed / independent axis inversion** — implemented as an explicit extension: `ScrollScaleFilter` with independent x/y factors, `Context::set_scroll_scale()`, and `PointerProfile::scroll_scale_x/y`. Positive values scale, negative values invert, zero suppresses. It composes with the separate natural-scrolling toggle.
7. **Pressure-dependent pointer motion** — synaptics can make cursor speed depend on finger pressure. input-rs now preserves raw `ABS_MT_PRESSURE`, and current-behavior workloads including direct active device-removal, context-level active device-removal, and device-disable shapes are tracked in `touchpad-motion-extensions.md`, but it does not yet apply pressure-dependent motion or expose a configuration policy for it.
8. **Edge-motion continuation** — keep cursor moving when finger hits pad edge. Not in libinput. Current-behavior workloads including direct active removal, context-level active removal, and disable shapes are tracked in `touchpad-motion-extensions.md`.
9. **Active rectangle / area configuration** — ignore regions of the touchpad. Not in libinput for touchpads. Current-behavior workloads including direct active removal, context-level active removal, and disable shapes are tracked in `touchpad-motion-extensions.md`; tablet active area remains tracked separately in `absolute-device-transforms.md`.
10. **Left-handed touchpad (rotate coordinates 180 degrees)** — needs touchpad coordinate transformation. Current-behavior workloads including direct active removal, context-level active removal, and disable shapes are tracked in `touchpad-motion-extensions.md`, but it is not yet configurable.

## Acceleration "Feel" Notes

Your research identifies 5 factors that affect perceived feel:

1. **Physical-unit normalization before accel** — We do this: `dpi_factor = 1000.0 / dpi`. If DPI metadata is wrong (and it often is — libinput's FAQ explicitly calls this out), the same curve feels wrong because input enters at the wrong physical scale. We still default to DPI=1000 when no metadata or override is available.
   
2. **Velocity estimation window** — We use a 128-entry tracker ring and select samples within a 16 ms window, so the estimate covers roughly the same physical time at 125 Hz through 8 kHz rather than varying with report count. The newest displacement is included in the estimate, and samples that diverge too far from the initial velocity are discarded. The earlier 16-entry/16-sample approach was report-count-dependent on high-polling-rate mice.

3. **Base plateau at low speed** — Our adaptive profile follows libinput's three-region model: low-speed boost (0.3→1.0), constant plateau below threshold, linear incline above. The low-speed region (`speed_ms < 0.07 → factor = 10*speed+0.3`) mimics libinput's touch-friendly deceleration.

4. **Transition band slope** — Controlled by `incline_factor` and `accel` (max) parameters. Currently hardcoded. Should be runtime-configurable.

5. **Maximum gain** — Our `accel` parameter caps at 2.0+speed*1.5. Matches libinput's default range.

**Custom acceleration profile:** libinput now exposes `LIBINPUT_CONFIG_ACCEL_PROFILE_CUSTOM` where you define f(x) as a set of (speed, factor) points. input-rs has the core `MouseAccelProfile::Custom` filter path and a human-like Criterion speed-ramp workload. The missing piece is configurability: jay still needs a user-facing custom-curve API, validation rules, hotplug replay semantics, inspection, and deterministic coverage before this can be called complete.

**Key insight from Hutterer's analysis:** For properly configured mice, the "feel" problems mostly come from bad MOUSE_DPI metadata, not the curve itself. `input-rs` now has a backend-level runtime DPI hook and pending hotplug application, but it still needs metadata parsing, a user-facing jay config path, and effective-DPI diagnostics before this area is complete.
