# Feature: Touchpad Scroll Methods

## Gap

Libinput supports multiple scroll methods: two-finger scrolling, edge scrolling,
and button scrolling. It also exposes scroll method selection, scroll button
selection, scroll button lock, natural scrolling, and scroll source
information.

`input-rs` currently has two-finger touchpad scrolling, initial right/bottom edge
scrolling, and wheel handling. It also has an initial `ButtonScrollFilter` that
converts pointer motion into continuous `PointerScroll` events while a configured
scroll button is active. Explicit scroll scaling and per-axis inversion are
also available through `ScrollScaleFilter` / `Context::set_scroll_scale()`.
It does not yet implement full public scroll-method selection, automatic
trackpoint defaults, complete source classification, circular scrolling, or
coasting scroll.

## Current Partial Implementation

- Two-finger touchpad scrolling exists in `TouchpadMotionFilter`.
- `TouchpadScrollMethod::Edge` enables vertical scrolling from the right edge
  and horizontal scrolling from the bottom edge for contacts that start in those
  zones. Edge-scroll contacts are not treated as palms.
- `InputBackend::set_touchpad_scroll_method()` can forward touchpad scroll method
  selection from jay's backend configuration path into input-rs.
- `TouchCancel` clears the affected touchpad slot, including any edge-scroll
  latch and pending scroll movement for that contact.
- Wheel and high-resolution wheel events are normalized in the evdev backend.
- `ButtonScrollFilter` supports hold-to-scroll with a configured button; the
  middle button helper is used by the human-like benchmark workload.
- `ButtonScrollFilter::with_lock()` supports local toggle behavior.
- `Context::set_button_scroll()` and `InputBackend::set_button_scroll()` can
  enable, disable, or replace button scrolling on a live input-rs device.
- `NaturalScrollFilter` can invert wheel/button scroll deltas, and
  `TouchpadMotionFilter` can invert finger scroll deltas through
  `TouchpadConfig::natural_scrolling`.
- Explicit input-rs device disable calls `FilterChain::reset_runtime_state()`,
  which clears button-scroll hold/lock state and touchpad scroll recognizer
  state.
- `feature_touchpad_scroll_methods` has human-like two-finger scroll workloads for
  vertical, horizontal, diagonal, and below-threshold movement through
  `TouchpadMotionFilter`.
- `feature_touchpad_scroll_methods` has candidate human-like workloads for:
  - scroll-speed-multiplier-shaped two-finger scroll with mixed slow and fast
    movement;
  - coasting/flick-shaped two-finger scroll with high velocity before finger
    lift and idle frames after lift;
  - circular-scroll-shaped one-finger motion starting near the right edge and
    moving around the touchpad center.
  These are current-behavior baselines only. `input-rs` does not yet implement
  scroll speed scaling, coasting, or circular scrolling.
- `feature_touchpad_scroll_methods/button_scroll_candidate_current_chain` runs a
  human-like middle-button hold plus pointer-motion workload through the current
  button-scroll filter.
- `feature_touchpad_scroll_methods/button_scroll_lock_current_chain` runs a
  human-like middle-button toggle-lock workload through the current button-scroll
  filter.
- `feature_touchpad_scroll_methods` has human-like workloads for enabling
  button scrolling while neutral, disabling button scrolling while hold-to-scroll
  is active, and replacing the configured scroll button before a right-button
  scroll gesture.
- `feature_touchpad_scroll_methods` has human-like context workloads for
  explicit input-rs device disable while button scrolling is active in hold mode
  and while button-scroll lock mode is active.
- `feature_touchpad_scroll_methods` also has cleanup-shaped workloads for active
  two-finger scrolling, right-edge scrolling, button-scroll hold, and
  button-scroll lock through the current filter/context paths.
- `feature_touchpad_scroll_methods` has human-like right-edge and bottom-edge
  drag workloads through `TouchpadMotionFilter` with edge scrolling selected.
- `feature_touchpad_scroll_methods` has human-like edge-scroll workloads for
  right-edge and bottom-edge natural scrolling, for a contact that starts in the
  main area and crosses into the right edge, and for contacts that start in the
  right or bottom edge and then cross into the main area. These workloads cover
  the current latch-at-touch-down policy.
- `feature_touchpad_scroll_methods` also exercises current runtime reconfiguration
  hooks with human-like workloads for:
  - changing from two-finger to edge scrolling before a neutral right-edge drag;
  - changing from two-finger to edge scrolling while a two-finger scroll gesture is
    active;
  - enabling natural scrolling while a two-finger scroll gesture is active.

## Scope

This feature covers how pointer and touchpad motion becomes scroll events:

- Touchpad two-finger scrolling.
- Touchpad edge scrolling.
- Button scrolling for pointing sticks, mice, and compatible touchpad devices.
- Scroll method configuration.
- Scroll button configuration.
- Scroll button lock.
- Scroll source classification.
- Optional input-rs-owned scroll extensions that users commonly expect from
  older configurable touchpad stacks:
  - scroll speed multiplier.
  - circular scrolling.
  - kinetic/coasting scroll.

Kinetic/coasting scroll is not required for libinput parity, and many clients can
implement kinetic behavior themselves. Because input-rs is intended to own input
behavior and expose explicit configuration, this document still tracks the
requirements that must be satisfied if jay/input-rs chooses to provide compositor
level coasting.

## Requirements To Mark Complete

### Public Configuration

- [ ] Define a scroll method enum with at least:
  - [ ] none.
  - [x] two-finger.
  - [x] edge.
  - [ ] button.
- [ ] Expose per-device supported scroll methods. Unsupported methods must be
  rejected or ignored with a traceable status.
- [ ] Add runtime `Context` methods for:
  - [x] setting scroll method in input-rs through the jay backend configuration
    path.
  - [ ] querying current scroll method.
  - [x] setting scroll button for button scrolling.
  - [x] setting scroll button lock.
  - [ ] setting natural scrolling for every scroll source that supports it.
- [ ] Propagate user-facing jay config into these runtime methods. The lower-level
  backend path exists, but no user-facing scroll-method command is wired yet.
- [ ] Document the default method per device class.

### Two-Finger Scrolling

- [ ] Preserve current two-finger scrolling behavior while moving it behind the
  scroll method state machine.
- [ ] Add a movement threshold before two-finger scrolling begins. Small
  two-finger noise must not emit scroll immediately.
- [ ] Once scrolling is engaged, smaller deltas must be emitted so scrolling
  does not feel sticky after threshold crossing.
- [ ] Horizontal and vertical scroll must be tracked independently enough to
  support straight-line locking or intentional diagonal scroll.
- [ ] Ending a two-finger scroll must reset the scroll state without leaking
  into subsequent pointer motion or tap gestures.

### Edge Scrolling

- [x] Define edge zones from device geometry and resolution.
- [x] Support vertical edge scrolling on the right edge.
- [x] Support horizontal edge scrolling on the bottom edge.
- [x] Do not start edge scrolling from the main touchpad area.
- [x] Define what happens when a finger crosses into or out of the edge zone
  after touch-down. Current behavior latches edge-scroll status at touch-down.
  - [x] Add human-like Criterion workloads for a contact that starts in the main
    area and crosses into the right edge, and for contacts that start in the
    right or bottom edge and cross back into the main area.
- [ ] Define precedence between edge scrolling, clickpad software button areas,
  palm edge rejection, and tap recognition.
- [x] Edge scrolling must obey natural scroll.
  - [x] Add human-like Criterion workloads for natural scrolling on right-edge
    vertical scrolling and bottom-edge horizontal scrolling.

### Button Scrolling

- [x] Convert relative pointer motion into scroll events while the configured
  scroll button is logically held.
- [x] Preserve the button event semantics required by the compositor. The
  current filter deliberately swallows the configured scroll button so it does
  not also reach clients as a click.
- [ ] Support pointing-stick style button scrolling as a default where the
  device is classified as a trackpoint/trackstick.
- [ ] Support button scrolling on ordinary pointer devices only when explicitly
  configured or when device metadata says it is expected.
- [x] Add human-like Criterion workloads for runtime button-scroll enable,
  disable, and configured-button replacement through the current filter-chain
  API.
- [x] Scroll button lock must allow one press/release to enter scroll mode and
  another press/release to exit scroll mode. Local toggle behavior exists in the
  filter and is reachable through runtime input-rs/backend configuration.
- [ ] Button lock must be cleared on device disable, device removal, or
  compositor seat reset.
  - [x] Explicit input-rs device disable clears `ButtonScrollFilter` lock state.
  - [x] Add human-like Criterion workloads for explicit input-rs device disable
    while hold-to-scroll is active and while lock-to-scroll is active.
  - [x] Add human-like Criterion workloads for context-level input-rs device
    removal while hold-to-scroll is active and while lock-to-scroll is active.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    while button-scroll hold mode is active through the current filter-chain
    path.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    while button-scroll lock mode is active through the current filter-chain
    path.
  - [ ] Add deterministic coverage for evdev removal, monitor-driven removal,
    and compositor seat reset. Monitor-driven removal now reaches input-rs
    context cleanup, but scroll-specific lock/hold assertions still need to
    prove the final policy.
- [x] Button scrolling must emit continuous `PointerScroll` events, not wheel
  click events.

### Scroll Source And Units

- [ ] Preserve wheel v120 handling for `REL_WHEEL_HI_RES`,
  `REL_WHEEL`, `REL_HWHEEL_HI_RES`, and `REL_HWHEEL`.
- [ ] Distinguish wheel, finger, and continuous/button scroll sources in the
  event model or jay bridge.
- [ ] Ensure touchpad finger scrolling and button scrolling do not get reported
  as wheel events.
- [ ] Keep scroll deltas in units that jay can convert consistently to its
  protocol-level axis events.
- [ ] Avoid double-applying natural scroll. Natural scroll must be applied once
  in exactly one layer.

### Advanced Scroll Extensions

- [x] Add an explicit scroll speed multiplier if `input-rs` owns scroll-speed
  configuration. Implemented as `ScrollScaleFilter` with independent x/y
  factors; it applies to every `PointerScroll` source after button-scroll
  generation and before/after natural inversion (multiplication commutes).
  - [x] Add a human-like Criterion workload carrying mixed slow and fast
    two-finger scroll motion through the current filter as a baseline for future
    multiplier behavior.
  - [x] Define whether the multiplier applies to finger, edge, button, wheel, or
    all scroll sources: every `PointerScroll` event.
  - [x] Define the order relative to natural scrolling: canonical order is
    button scroll → natural inversion → scroll scale; magnitude and direction
    each apply once.
  - [x] Validate multiplier values: non-finite factors fall back to 1.0;
    positive scales, negative inverts, zero suppresses that axis; default is
    (1.0, 1.0).
  - [x] Add runtime input-rs configuration (`Context::set_scroll_scale`,
    `FilterChain::set_scroll_scale`, profile defaults) and deterministic tests
    for scaling, inversion, identity fallback, and source pass-through.
- [ ] Add circular scrolling only if it is an intentional input-rs feature.
  - [x] Add a human-like Criterion workload carrying one-finger circular motion
    that starts near the right edge and moves around the touchpad center through
    the current touchpad filter.
  - [ ] Define the activation gesture: edge start, corner start, explicit mode,
    or another user-visible setting.
  - [ ] Define clockwise and counter-clockwise sign, dead zone around the center,
    wrap handling across the angle discontinuity, and how vertical/horizontal
    axes are selected.
  - [ ] Define interactions with edge scrolling, two-finger scrolling, taps,
    palm/thumb rejection, clickpad button areas, and natural scrolling.
  - [ ] Add runtime input-rs configuration, jay forwarding, capability reporting,
    and deterministic assertions for activation, update, cancellation, and
    disabled-device cleanup.
- [ ] Add kinetic/coasting scroll only if jay/input-rs deliberately owns it
  rather than leaving it to clients.
  - [x] Add a human-like Criterion workload carrying a fast two-finger flick,
    finger lift, and idle frames through the current touchpad filter.
  - [ ] Define the velocity estimator, deceleration curve, stop threshold,
    maximum duration, source tag, and timestamp policy for synthesized post-lift
    scroll events.
  - [ ] Coasting must stop on new touch contact, pointer button press, keyboard
    shortcut if configured, device disable, device removal, seat reset, or
    compositor policy cancellation.
  - [ ] Timer-backed post-lift event generation must be integrated into
    input-rs dispatch without busy loops or client-visible event bursts.
  - [ ] Add runtime input-rs configuration, jay forwarding, diagnostics, and
    deterministic assertions for coasting start, decay, cancellation, and
    disabled-device cleanup.

### Neutral State And Reconfiguration

- [ ] Changing scroll method mid-gesture must not emit both pointer motion and
  scroll for the same physical movement.
- [ ] If a method change waits for neutral state, the pending method must be
  stored and applied predictably.
- [ ] Device disable/removal must clear all active scroll gesture state.
  - [x] Touch cancel clears touchpad scroll state for the affected contact.
  - [x] Explicit input-rs device disable clears touchpad scroll recognizer state
    through `FilterChain::reset_runtime_state()`.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    while two-finger scrolling is active through the current touchpad filter.
  - [x] Add a human-like Criterion workload for explicit
    `Context::set_device_enabled(false)` while two-finger scrolling is active.
  - [x] Add a human-like Criterion workload for context-level
    `Context::remove_device_by_index()` while two-finger scrolling is active.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    while right-edge scrolling is active through the current touchpad filter.
  - [x] Add a human-like Criterion workload for explicit
    `Context::set_device_enabled(false)` while right-edge scrolling is active.
  - [x] Add human-like Criterion workloads for context-level
    `Context::remove_device_by_index()` while right-edge and bottom-edge
    scrolling are active.
  - [ ] Add deterministic coverage for every evdev removal, monitor-driven
    removal, and compositor seat-reset path.

### Coverage

- [ ] Cover two-finger vertical, horizontal, diagonal, and below-threshold
  movement.
  - [x] Add human-like Criterion workloads for all four movement patterns.
  - [x] Add a human-like Criterion workload for mixed-speed two-finger scroll
    motion as the current baseline for a future scroll speed multiplier.
  - [ ] Add deterministic tests or replay fixtures for thresholds, direction, and
    frame grouping.
- [ ] Cover edge scrolling on right and bottom edges.
  - [x] Add human-like Criterion workloads for right-edge and bottom-edge
    scrolling.
  - [x] Add human-like Criterion workloads for crossing into an edge after
    touch-down and crossing out of an edge after touch-down, matching the
    current latch-at-touch-down behavior.
  - [ ] Add deterministic tests or replay fixtures for edge thresholds and axis
    selection.
- [x] Add human-like Criterion coverage for touch cancel cleanup in
  `feature_touch_event_completeness/touchpad_cancel_cleanup_current_filter`.
- [ ] Cover edge scroll cancellation and transition to neutral in deterministic
  tests or replay fixtures.
- [x] Add a human-like Criterion workload for direct `DeviceRemoved` input while
  two-finger scrolling is active through the current touchpad filter.
- [x] Add a human-like Criterion workload for explicit
  `Context::set_device_enabled(false)` while two-finger scrolling is active.
- [x] Add a human-like Criterion workload for context-level
  `Context::remove_device_by_index()` while two-finger scrolling is active.
- [x] Add a human-like Criterion workload for direct `DeviceRemoved` input while
  right-edge scrolling is active through the current touchpad filter.
- [x] Add a human-like Criterion workload for explicit
  `Context::set_device_enabled(false)` while right-edge scrolling is active.
- [x] Add human-like Criterion workloads for context-level
  `Context::remove_device_by_index()` while right-edge and bottom-edge scrolling
  are active.
- [x] Add human-like Criterion workloads for right-edge and bottom-edge scrolling
  through the current touchpad filter.
- [x] Add a human-like Criterion workload for button scrolling hold mode.
- [x] Add a human-like Criterion workload for button scrolling lock mode.
- [x] Add human-like Criterion workloads for button-scroll runtime enable,
  disable while active, and scroll-button replacement.
- [x] Add human-like Criterion workloads for device disable while button
  scrolling is active in hold mode and lock mode.
- [x] Add human-like Criterion workloads for context-level device removal while
  button scrolling is active in hold mode and lock mode.
- [x] Add a human-like Criterion workload for direct `DeviceRemoved` input while
  button-scroll hold mode is active.
- [x] Add a human-like Criterion workload for direct `DeviceRemoved` input while
  button-scroll lock mode is active.
- [ ] Cover button scrolling hold mode and lock mode in deterministic tests or
  replay fixtures.
- [ ] Cover natural scroll for wheel, finger, and button scroll.
  - [x] Add human-like Criterion workloads for two-finger natural scroll, button
    natural scroll, wheel natural scroll, and right/bottom edge natural scroll
    through the current filters.
  - [ ] Add deterministic tests or replay fixtures for all sources and avoid
    double-application.
- [ ] Cover config changes while scrolling and while neutral.
  - [x] Add human-like Criterion workloads for scroll-method changes before a
    neutral edge-scroll sequence and during an active two-finger scroll sequence.
  - [x] Add a human-like Criterion workload for changing natural-scroll direction
    during an active two-finger scroll sequence.
  - [x] Add human-like Criterion workloads for button-scroll enable while
    neutral, button-scroll disable while active, and configured-button
    replacement while neutral.
  - [ ] Add deterministic assertions for whether each config change applies
    immediately or waits for neutral state, and prove that a single physical
    movement frame cannot produce both pointer motion and scroll because of the
    change.
- [ ] Cover devices that support only one method and devices that support
  multiple methods.
- [ ] Cover advanced scroll extensions if implemented.
  - [x] Add human-like Criterion workloads for scroll-speed-multiplier-shaped,
    circular-scroll-shaped, and coasting/flick-shaped input through current
    filters.
  - [ ] Add implementation coverage and deterministic assertions for every
    advanced extension that is actually enabled.

### Documentation

- [x] Update `FEATURE_PARITY.md` with separate statuses for two-finger, edge,
  and button scroll.
- [ ] Document default methods and user configuration names.
- [ ] Document any difference from libinput's method defaults.

## Local References

- `input-rs/src/backend/evdev.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/processing/touchpad_motion.rs`
- `input-rs/src/processing/natural_scroll.rs`
- `input-rs/src/state.rs`
- `libinput/doc/user/scrolling.rst`
- `libinput/doc/user/configuration.rst`
- `libinput/src/libinput.h`
