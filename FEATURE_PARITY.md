# input-rs Feature Parity with libinput

## Status: Core mouse/keyboard functional, touchpad basic, many libinput features not yet ported

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

---

## Event Processing Pipeline

| Feature | Status | Notes |
|---------|--------|-------|
| evdev device reading via epoll | ✅ | `EvdevBackend` with epoll, SYN_REPORT aggregation |
| Device hotplug (inotify) | ✅ | `LinuxDiscovery` watches `/dev/input` |
| Device capability detection | ⚠️ | KEY/REL/ABS/EV_SW detection exists for keyboard, pointer, touchpad, touchscreen, and switch. `feature_device_policy_and_metadata/capability_inventory_candidate_current_info` now carries human-like `DeviceInfo` shapes for keyboard, mouse, trackpoint, touchpad/clickpad, touchscreen, switch, tablet tool, and tablet pad. Still lacks real evdev/sysfs classification for tablets, tablet pads, trackpoints, clickpads, internal/external policy, and explicit device-profile/config overrides. See [`device-policy-and-metadata.md`](docs/missing-features/device-policy-and-metadata.md). |
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
| Middle-button emulation | ⚠️ | Initial `MiddleButtonEmulationFilter` and input-rs config plumbing added. Criterion workloads now cover both chord directions, timeout fallback for left/right, release ordering, left-handed interaction, debounce interaction, middle-button button-scroll interaction, runtime emulation enable/disable while neutral/pending/active, direct removal-shaped input while pending/active, explicit device disable while pending/active, and context-level device removal while pending/active. Still needs timer-backed timeout policy, deterministic assertions, complete removal/reset coverage, and documented defaults before complete. |
| Scroll wheel (hi-res v120) | ✅ | `PointerScroll` with 120 normalization |
| Natural scrolling (touchpad) | ✅ | `TouchpadMotionFilter.config.natural_scrolling` + `Context::set_natural_scroll()` |
| Natural scrolling (mouse) | ✅ | `NaturalScrollFilter` inverts scroll deltas, wired through `FilterChain::set_natural_scroll_mouse()` |

## Touchpad

| Feature | Status | Notes |
|---------|--------|-------|
| Two-finger scroll | ✅ | `TouchpadMotionFilter` with natural scroll config; now represented alongside `TouchpadScrollMethod::Edge` in touchpad config. |
| Tap-to-click | ⚠️ | Single tap, optional two-finger right-click tap, three-finger middle tap, and initial tap-and-drag are implemented in `TouchpadMotionFilter`. Criterion workloads cover taps, the current two-finger right-click toggle, alternate tap-map-shaped input through the current hardcoded map, staggered/reverse two-finger ordering, cancellation, near-threshold timing/movement, tap-and-drag, delayed second-touch tap-drag timing, drag-lock-shaped input and cleanup, direct removal-shaped input while tap-drag is active, context-level device removal while tap-drag is active, physical-click cancellation of a pending tap, tap and click-method config changes, DWT/palm/natural-scroll interactions, current left-handed filter-chain interaction, and synthesized middle tap passing through middle emulation. Still missing configurable tap timing, full configurable tap button maps, real drag-lock behavior, deterministic coverage for all cancellation/order cases, a documented left-handed/tap-map policy, and exact libinput-style delayed tap release for tap-and-drag. |
| Palm detection (edge zone) | ✅ | Configurable `palm_edge_ratio` |
| Palm detection (firmware tool flag) | ⚠️ | Raw `ABS_MT_TOOL_TYPE` is preserved in `TouchData`, and `TouchpadMotionFilter` ignores contacts marked `TouchToolType::Palm` on touch down when `palm_detection` is enabled. Criterion workloads now cover touch-down firmware palm metadata, late finger-to-palm metadata on motion, firmware-palm-shaped input with palm detection disabled, and a firmware-palm contact joining an active gesture stream. Still missing late reclassification handling, a final active-gesture arbitration policy, deterministic coverage, pressure/size fallback policy, and device-profile/config override defaults. |
| DWT (disable-while-typing) | ⚠️ | `dwt` config and seat keyboard timestamp suppress touchpad motion, and Criterion workloads cover internal-keyboard-shaped suppression plus an external-keyboard-shaped non-suppression case. Still missing production internal/external keyboard grouping, configurable timeout, and deterministic grouping coverage. |
| DWTP / stylus-touch arbitration | ⚠️ | Criterion workloads carry grouped keyboard/touchpad/trackpoint metadata and stylus-proximity plus touch-contact event streams. Actual trackpoint-driven touchpad suppression, stylus-touch suppression/restoration, grouping policy, and deterministic coverage are still missing. |
| Touchpad acceleration | ✅ | `TouchpadFilter` with resolution-aware adaptive profile |
| Clickpad button handling | ⚠️ | `TouchpadMotionFilter` has an initial physical `BTN_LEFT` remap for `ButtonAreas` and `Clickfinger`. Button areas use bottom-zone geometry, latch the logical button at press time, and clickfinger maps active finger count to left/right/middle. Criterion workloads now cover current left-handed filter-chain interaction for main/bottom-left/bottom-middle/bottom-right button areas, press-position latching after contact movement, neutral and active click-method changes, one-/two-/three-finger clickfinger clicks, alternate clickfinger-map-shaped input through the current hardcoded map, staggered clickfinger finger arrival, and adding a finger while a clickfinger button is held. Still missing clickpad detection, configurable areas/maps, stable finger-count policy, a final left-handed policy, correct filter ordering for clickpad raw `BTN_LEFT`, and deterministic coverage. |
| Three-finger swipe gestures | ⚠️ | `TouchpadMotionFilter` has an initial three-or-more-finger center-motion recognizer and jay bridge forwarding. Touch cancel emits a cancelled gesture end, and explicit device disable resets filter state. Criterion workloads now include three-finger swipe, four-finger swipe, swipe and pinch finger-count cancellation, hold touch-cancel cleanup, competition cases, active filter reset, context-level active device disable while swipe/pinch/hold streams are active, context-level removal while swipe/pinch/hold streams are active, and direct removal-shaped input while swipe/pinch/hold streams are active. Monitor-driven removal reaches input-rs context cleanup. Still needs deterministic removal/cancellation coverage and complete disambiguation before it can be marked complete. |
| Pinch gestures | ⚠️ | `TouchpadMotionFilter` has an initial two-finger pinch recognizer for spacing/angle changes and jay bridge forwarding. Touch cancel emits a cancelled gesture end, and explicit device disable resets filter state. Criterion workloads now cover scale in/out, rotation in both directions, finger-count cancellation, context-level active device disable, context-level active device removal, and direct removal-shaped input while a pinch stream is active. Monitor-driven removal reaches input-rs context cleanup. Still needs multi-finger parity, deterministic removal/cancellation assertions, and documented disambiguation. |
| Drag lock | ❌ | Runtime config is plumbed into `TouchpadConfig`, and Criterion workloads cover drag-lock-shaped lift/re-touch, direct removal-shaped input, explicit device disable, and context-level device removal through current paths. Actual drag-lock hold/release behavior is not implemented. |
| Thumb detection | ⚠️ | Optional lower-zone thumb-rest heuristic exists in `TouchpadMotionFilter`, raw pressure/width/tool metadata is now represented, and the human-like benchmark covers a resting thumb during one-finger motion, a lower-zone single contact, a clickpad bottom-area click while thumb detection is enabled, and high-pressure/large-touch palm-shaped metadata without a firmware palm flag. Still lacks pressure/width threshold policy, device-profile/config overrides, final clickpad-area/lone-contact arbitration, and deterministic coverage. |

## Keyboard / Button

| Feature | Status | Notes |
|---------|--------|-------|
| Key event passthrough | ✅ | Direct evdev→jay mapping |
| Button debounce (configurable) | ⚠️ | `ButtonDebounceFilter` has an initial state machine and is disabled by default. Low-level `Context::set_debounce()`, `InputBackend::set_debounce()`, and `InputDeviceConfig::SetDebounce` plumbing exists, and Criterion workloads cover disabled passthrough, bouncy pointer button suppression, spurious double-click suppression, independent left/right button state, keyboard key bounce, zero-spurious-timeout configuration, runtime debounce enable/disable/threshold changes, pending bouncy/spurious cleanup on direct removal, context-level removal, and explicit device disable, plus debounce-before-middle ordering. Still missing a normal jay input-device command/protocol path, hotplug replay, diagnostics/inspection, final policy for key debounce and multi-button frame flushing, collision-free per-button state, and deterministic coverage for the delayed/spurious and runtime-change edge cases. See [`button-debounce.md`](docs/missing-features/button-debounce.md). |
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
| LED state (numlock/capslock) | ⚠️ | Standalone `DevicePolicyState` stores caps-lock, num-lock, and scroll-lock state, and Criterion workloads cover policy-level LED state updates plus keyboard-event passthrough. Still missing jay-to-input-rs LED configuration, evdev LED writes, hotplug reapply/reconcile behavior, diagnostics, and deterministic hardware success/failure coverage. |

## Device Discovery & Metadata

| Feature | Status | Notes |
|---------|--------|-------|
| Device name from sysfs / evdev | ✅ | Uses the evdev-reported name when available and falls back to the name supplied during device add. |
| Device vendor/product ID | ✅ | Populated from `evdev::Device::input_id()`. |
| Bus type (USB/Bluetooth/I2C/PS2/unknown) | ✅ | Populated from `evdev::Device::input_id().bus_type()` and mapped into `BusType`. |
| Touchpad axis min/max/resolution info | ✅ | `AxisInfo` is parsed from evdev absinfo and passed through `DeviceInfo` to touchpad filters. |
| Disable reason composition | ⚠️ | Standalone `DevicePolicyState` models user-disabled, external-mouse-disabled, switch-disabled, temporary disabled, LED state, representative bus metadata, internal/external policy changes, grouping-shaped metadata, and profile/fallback-shaped metadata with human-like Criterion workloads. Not wired into `Context::dispatch()`, device hotplug, jay config, stable grouping, device-profile matching, or evdev LED updates yet. |
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
| Tablet tool proximity/tip/axis | ⚠️ | Event model variants and human-like benchmark events exist for proximity, axis, tip, tool button input, tool-button/proximity-out ordering, pressure-range-shaped axis data, repeated proximity cycles, active tool-node removal, separate tool/pad/touch node-shaped streams, and tablet-touch-node removal while related tool/pad nodes remain active. Evdev parsing, capability detection, tool identity, pressure normalization policy, grouped-node removal policy, and jay tablet protocol bridging are still missing. |
| Tablet pad buttons/rings/strips | ⚠️ | Event model variants and benchmark events exist for pad buttons, rings, strips, dials, pad controls without tool proximity, active pad-node removal, and separate pad-node-shaped streams. Evdev parsing, pad mode groups, capability detection, pad-button cleanup policy, grouping, and jay tablet protocol bridging are still missing. |

## Absolute Device Transforms

| Feature | Status | Notes |
|---------|--------|-------|
| Touchscreen calibration matrix | ⚠️ | `AbsoluteTransformFilter` applies a validated affine matrix to touch down/motion coordinates using device axis bounds and clamps transformed normalized coordinates. jay calibration config now forwards into input-rs and installs/replaces the per-device transform filter. Criterion workloads cover identity, X/Y inversion, 90-degree clockwise/counter-clockwise rotation matrices, translate/scale, clamping, invalid non-finite config rejection, runtime replacement while neutral/active, and candidate input shapes for tablet active-area mapping and relative-device rotation. Still missing a final exposure policy, reset/query API, active-contact reconfiguration policy, tablet absolute-axis support, actual active-area/relative-rotation configuration, and deterministic coverage. |
| Relative-device rotation | ❌ | Not implemented. A Criterion workload now carries relative-pointer deltas shaped like motion on a rotated display through the current event model so the future transform path has a human-like baseline. |
| Tablet active area / pressure range | ⚠️ | Actual tablet active-area and pressure-range configuration are not implemented. Tablet event variants and Criterion workloads now carry pressure-range-shaped tablet axis data, tablet active-area-shaped strokes, and separate tool/pad/touch node streams through the current event model, but evdev tablet parsing, per-tool/per-device config, normalization, and deterministic coverage are still missing. |

## Switch Support

| Feature | Status | Notes |
|---------|--------|-------|
| Switch toggle (lid/tablet-mode switch) | ⚠️ | Initial `EV_SW` detection, typed `SwitchKind` event model, supported-code inventory, initial switch-state read, lid/tablet-mode evdev parsing, SYN_REPORT coalescing, duplicate same-state suppression, unknown switch passthrough, jay bridge mapping for known switch kinds, and benchmark workloads added. Criterion workloads also cover startup-shaped switch state, rapid tablet-mode transitions, active known/unknown switch removal, and standalone switch-policy composition for internal, user-disabled, external-device, and external-mouse-disabled cases. Still missing switch-removal policy handling and dispatch-integrated switch-driven internal-device policy. See [`switch-devices.md`](docs/missing-features/switch-devices.md). |

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
| Device metadata from evdev | ✅ | Reads name, vendor/product ID, bus type, axis info, and capabilities from `evdev::Device`. |

## Design Decisions (not bugs)

| Decision | Rationale |
|----------|-----------|
| Flat profile is default for mice | Lower latency, predictable pointer movement, FPS-friendly |
| Debounce disabled by default | Most devices debounce in hardware; adding latency by default is wrong |
| No libinput-style quirks database | Intentional: input-rs should not copy libinput's hidden device-quirks model. Device-specific behavior should use configurable defaults, explicit device profiles, or another user-visible override mechanism with diagnostics. A candidate Criterion workload now carries representative profile/fallback metadata through the standalone policy model; actual profile schema, matching, conflict handling, diagnostics, and deterministic coverage are still missing. |
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
| Single-touch fallback (ABS_X/Y + BTN_TOUCH) | ⚠️ | Conservative evdev fallback added for non-MT, non-relative, non-tablet-tool devices: `BTN_TOUCH` down/up with `ABS_X/Y` emits slot-0 touch down/motion/up, and active synthetic contacts are cancelled on `SYN_DROPPED`/device removal. Still needs deterministic tests and documented pointer-emulation behavior. |
| Touch cancel | ⚠️ | `InputEvent::TouchCancel` exists, jay bridge forwarding is wired, evdev cancels active touches on `SYN_DROPPED` or device removal, `Context::set_device_enabled(false)` and context removal cancel tracked active touches, and touchpad motion cleanup releases active gesture/tap state. Human-like Criterion workloads cover direct touch cancel, touchpad cancel cleanup, MT-B down/motion/up/frame streams, same-frame multi-slot updates, active MT-B/multi-slot/single-touch direct removal, slot reuse, tracking-id-change-shaped metadata, metadata propagation, active-touch manual device-disable cleanup through `Context::set_device_enabled(false)`, and backend-originated removal cleanup. Still missing deterministic replay coverage. |
| Per-contact touch metadata | ⚠️ | `TouchData` carries optional raw pressure, touch major/minor, tool type, and tracking id. Evdev fills MT-B pressure/size/tool/tracking metadata and single-touch pressure, and clears slot metadata on contact end. Still missing normalized units, deterministic replay coverage, single-touch size/tool metadata, and pressure/size policy consumers. |
| Touch frame grouping | ✅ | SYN_REPORT delimits, Frame event emitted |

## Scroll handling

| Feature | Status | Notes |
|---------|--------|-------|
| REL_WHEEL (legacy) → v120 | ✅ | Multiplied by 120.0 to normalize to v120 units |
| REL_WHEEL_HI_RES (v120) | ✅ | Preferred over legacy; passed through as-is |
| REL_HWHEEL / REL_HWHEEL_HI_RES | ✅ | Horizontal scroll, same v120 normalization |
| v120 → jay Axis120 | ✅ | Bridge converts to `InputEvent::Axis120` with dist in 120ths of a click |
| Scroll source tagging (wheel/finger/continuous) | ✅ | Touchpads emit `AxisSource::Finger`, mice emit `AxisSource::Wheel`. Based on `DeviceCaps`. |
| Edge scrolling | ⚠️ | `TouchpadScrollMethod::Edge` supports right-edge vertical and bottom-edge horizontal scrolling for contacts that start in the edge zone, with natural-scroll handling. Runtime backend configuration can select the method, and Criterion workloads now cover right/bottom edge drags, natural scroll on both edge axes, crossing into/out of edge zones under the current latch-at-touch-down policy, active direct-removal, explicit-disable, and context-removal shapes, and method changes while neutral and while scrolling. Still missing user-facing jay config, method capability reporting, and deterministic coverage. |
| Button scrolling | ⚠️ | `ButtonScrollFilter` converts pointer motion to `PointerScroll` while a configured button is held or lock-toggled, and deliberately swallows that button. Runtime input-rs/backend configuration can enable/replace it with a selected button and lock mode. Criterion workloads cover hold mode, lock mode, natural scroll, runtime enable/disable/replacement through the current filter-chain API, explicit device disable while hold/lock scroll is active, context-level device removal while hold/lock scroll is active, and direct removal-shaped input while hold/lock scroll is active. Still missing user-facing jay config, device classification defaults, removal/seat-reset coverage, and continuous scroll source tagging in jay. |
| Hi-res scroll acceleration | 🚫 | Not a libinput feature to copy: libinput does not accelerate scroll deltas. If input-rs adds user-visible scroll speed scaling, that is tracked as an explicit advanced scroll extension in [`touchpad-scroll-methods.md`](docs/missing-features/touchpad-scroll-methods.md). |

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

Based on your research, these are the touchpad-facing features synaptics exposed that libinput deliberately hides:

1. **User-tunable pressure/width/palm thresholds** — libinput often handles these through hidden device data. `input-rs` should expose them through explicit configuration, sensible defaults, and user-visible device-profile overrides with diagnostics.
2. **Fine-grained tap timing (MaxTapTime, MaxDoubleTapTime, MaxTapMove)** — libinput offers only enable/disable. Our `TouchpadConfig` could expose these; current human-like workloads for near-timeout, near-movement-threshold, and delayed second-touch tap-drag sequences live in `touchpad-tap-and-click.md`.
3. **Arbitrary tap/click remapping** — synaptics per-corner actions. We only have two-finger right-click. Current-behavior workloads for alternate tap and clickfinger map input shapes live in `touchpad-tap-and-click.md`.
4. **Circular scrolling** — repeatedly requested, libinput refuses. Implementation opportunity. A human-like current-behavior workload is tracked in `touchpad-scroll-methods.md`.
5. **Kinetic/coasting scroll** — synaptics exposes CoastingSpeed. libinput says "not implemented by design." Another opportunity. A flick-and-idle current-behavior workload is tracked in `touchpad-scroll-methods.md`.
6. **Adjustable scroll speed** — probably the #1 complaint about libinput. No scroll speed setting exists. We should add a scroll speed multiplier. A mixed-speed two-finger scroll baseline workload is tracked in `touchpad-scroll-methods.md`.
7. **Pressure-dependent pointer motion** — synaptics can make cursor speed depend on finger pressure. input-rs now preserves raw `ABS_MT_PRESSURE`, and current-behavior workloads including direct active device-removal, context-level active device-removal, and device-disable shapes are tracked in `touchpad-motion-extensions.md`, but it does not yet apply pressure-dependent motion or expose a configuration policy for it.
8. **Edge-motion continuation** — keep cursor moving when finger hits pad edge. Not in libinput. Current-behavior workloads including direct active removal, context-level active removal, and disable shapes are tracked in `touchpad-motion-extensions.md`.
9. **Active rectangle / area configuration** — ignore regions of the touchpad. Not in libinput for touchpads. Current-behavior workloads including direct active removal, context-level active removal, and disable shapes are tracked in `touchpad-motion-extensions.md`; tablet active area remains tracked separately in `absolute-device-transforms.md`.
10. **Left-handed touchpad (rotate coordinates 180 degrees)** — needs touchpad coordinate transformation. Current-behavior workloads including direct active removal, context-level active removal, and disable shapes are tracked in `touchpad-motion-extensions.md`, but it is not yet configurable.

## Acceleration "Feel" Notes

Your research identifies 5 factors that affect perceived feel:

1. **Physical-unit normalization before accel** — We do this: `dpi_factor = 1000.0 / dpi`. If DPI metadata is wrong (and it often is — libinput's FAQ explicitly calls this out), the same curve feels wrong because input enters at the wrong physical scale. We still default to DPI=1000 when no metadata or override is available.
   
2. **Velocity estimation window** — We use a 16-entry tracker ring with the last 16 timestamps, matching libinput's approach. Both discard trackers that diverge too far from initial velocity.

3. **Base plateau at low speed** — Our adaptive profile follows libinput's three-region model: low-speed boost (0.3→1.0), constant plateau below threshold, linear incline above. The low-speed region (`speed_ms < 0.07 → factor = 10*speed+0.3`) mimics libinput's touch-friendly deceleration.

4. **Transition band slope** — Controlled by `incline_factor` and `accel` (max) parameters. Currently hardcoded. Should be runtime-configurable.

5. **Maximum gain** — Our `accel` parameter caps at 2.0+speed*1.5. Matches libinput's default range.

**Custom acceleration profile:** libinput now exposes `LIBINPUT_CONFIG_ACCEL_PROFILE_CUSTOM` where you define f(x) as a set of (speed, factor) points. input-rs has the core `MouseAccelProfile::Custom` filter path and a human-like Criterion speed-ramp workload. The missing piece is configurability: jay still needs a user-facing custom-curve API, validation rules, hotplug replay semantics, inspection, and deterministic coverage before this can be called complete.

**Key insight from Hutterer's analysis:** For properly configured mice, the "feel" problems mostly come from bad MOUSE_DPI metadata, not the curve itself. `input-rs` now has a backend-level runtime DPI hook and pending hotplug application, but it still needs metadata parsing, a user-facing jay config path, and effective-DPI diagnostics before this area is complete.
