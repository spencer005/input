# Feature: Middle-Button Emulation

## Gap

Libinput supports middle-button emulation where a near-simultaneous left and
right button chord produces a logical middle click. This is usually enabled for
devices without a physical middle button.

`input-rs` has a `MiddleButtonEmulationFilter` plus runtime plumbing from jay
config into `Context::set_middle_button_emulation()`. Timeout fallback is now
timer-addressable through `Filter::next_timeout_us()` and
`Context::dispatch_timeouts()`; deterministic tests cover chord synthesis,
release ordering, and deadline flush without unrelated input. Remaining work is
jay-side default/timeout documentation, an availability query, and broader
removal/reconfiguration assertions.

## Current Partial Implementation

- `MiddleButtonEmulationFilter` delays an isolated left or right press for the
  current `DEFAULT_CHORD_TIMEOUT_US` window.
- If the opposite button press arrives within that window, the pending press is
  replaced by a synthesized `BTN_MIDDLE` press.
- The synthesized middle button is released when both physical left and right
  buttons are released.
- If the timeout expires on a later frame, or if another non-frame event forces a
  flush, the pending physical button press is emitted as its original left or
  right button.
- `FilterChain::set_middle_button_emulation()` can install or remove the filter
  for an active input-rs device.
- `Context::set_middle_button_emulation()` is wired to the jay backend
  configuration path.
- Explicit input-rs device disable resets the filter through
  `FilterChain::reset_runtime_state()` and releases any tracked logical button
  state through `Context::set_device_enabled(false)`.
- Timeout-generated events traverse the remaining filter chain, so a delayed
  left/right press is still seen by downstream swaps, debounce, and button
  scrolling.
- Disabling middle emulation flushes a pending delayed press, releases an active
  synthesized middle button, and leaves a disengaged filter that swallows the
  physical chord releases until the buttons are neutral.
- `feature_middle_button_emulation` has human-like Criterion workloads for:
  - left-then-right and right-then-left chords;
  - left-button and right-button timeout fallback;
  - releasing the second physical button before the first while a synthesized
    middle button is active;
  - the current left-handed filter ordering;
  - a bouncy left-button transition passing through debounce before chord
    recognition;
  - the current filter-chain interaction where a synthesized middle chord flows
    into middle-button button scrolling;
  - runtime emulation enable while neutral, disable while one chord button is
    pending, and disable while a synthesized middle button is active through the
    current filter-chain API;
  - explicit device disable while a chord is pending and while a synthesized
    middle button is active;
  - direct `DeviceRemoved` input while a chord is pending and while a synthesized
    middle button is active.

## Scope

This feature covers physical or logical pointer button events from mice,
touchpads, trackpoints, and similar pointer devices:

- Left+right chord detection.
- Middle-button press and release synthesis.
- Configurable enable/disable.
- Interaction with left-handed mode.
- Interaction with button scrolling.
- Interaction with debounce.

It does not cover three-finger tap or clickfinger middle click. Those are
tracked in `touchpad-tap-and-click.md`.

## Requirements To Mark Complete

### Public Configuration

- [x] Add a runtime `Context` method to enable or disable middle-button
  emulation per device.
- [ ] Add a way to report whether emulation is available or useful for a
  device.
- [ ] Define defaults by device class:
  - [ ] devices with a physical middle button.
  - [ ] two-button mice.
  - [ ] touchpads with physical buttons.
  - [ ] trackpoints.
- [x] Propagate jay config into the `input-rs` backend.
- [ ] Runtime enable/disable must define what happens to in-flight state.
  - [x] Add human-like Criterion workloads for enabling while neutral, disabling
    while one chord button is pending, and disabling while an emulated middle
    button is active through the current filter-chain API.
  - [ ] Add deterministic assertions for whether disabling flushes pending
    physical buttons, releases an active synthesized middle button, drops only
    future chord recognition, or waits until neutral.
- [ ] Document whether the setting is applied before or after left-handed
  button swapping.

### State Machine

- [x] Pressing left and then right within the chord timeout must emit a logical
  middle press instead of separate left/right clicks.
- [x] Pressing right and then left within the chord timeout must behave the
  same way.
- [x] If the second button does not arrive before timeout, the first button must
  be emitted as its original button.
- [ ] Releasing either physical button while the emulated middle button is held
  must release the logical middle button according to the chosen policy.
- [x] The state machine must avoid duplicate release events for left, right, or
  middle.
- [ ] The state machine must handle both buttons being released in either order.
- [ ] The state machine must handle device removal, disablement, and compositor
  seat reset without leaving a logical button pressed.
  - [x] Explicit input-rs device disable resets pending middle-button emulation
    state through `FilterChain::reset_runtime_state()`.
  - [x] Explicit input-rs device disable releases any tracked synthesized middle
    button through `Context::set_device_enabled(false)`.
  - [x] Human-like Criterion workloads now cover context-level device removal
    through `Context::remove_device_by_index()` while a chord is pending and
    while a synthesized middle press is active.
  - [ ] Cover compositor seat reset and deterministic removal assertions.

### Interaction With Other Filters

- [ ] Define filter ordering relative to button debounce. Chord detection must
  operate on debounced button transitions if debounce is enabled.
- [ ] Define filter ordering relative to left-handed mode. The user-visible
  behavior must be documented.
- [ ] Define interaction with button scrolling. If middle button is configured
  as a scroll button, the implementation must state whether left+right chord
  enters scroll mode, emits middle click, or is disabled.
  - [x] Add a human-like Criterion workload for the current filter chain where
    `MiddleButtonEmulationFilter` synthesizes `BTN_MIDDLE` before
    `ButtonScrollFilter` consumes the configured middle scroll button.
- [ ] Define interaction with clickpad/clickfinger generated middle clicks.
  Synthesized middle events must not be chorded again.
  - [x] The tap/click benchmark group has a human-like Criterion workload where a
    synthesized three-finger middle tap passes through middle-button emulation.

### Timing

- [x] Use a named configurable or documented chord timeout
  (`MiddleButtonEmulationFilter::with_timeout`, default 50_000us).
- [x] The timeout is short enough to avoid delaying ordinary clicks: 50ms,
  with flush-on-motion behavior for drags.
- [x] Timeout handling does not depend on unrelated input. Filters expose
  `next_timeout_us()` and `Context::dispatch_timeouts(now_us)` flushes pending
  presses from the compositor timer.

### Coverage

- [ ] Cover left-then-right chord.
  - [x] Add a human-like Criterion workload for a left press followed quickly by
    right press, small pointer motion, and release.
  - [x] Add deterministic assertions that the physical left/right press pair
    produces exactly one logical middle press and no leaked left/right press
    (`chord_within_timeout_emits_only_middle`).
- [ ] Cover right-then-left chord.
  - [x] Add a human-like Criterion workload for a right press followed quickly by
    left press, small pointer motion, and release.
  - [x] Add deterministic assertions that this direction behaves identically to
    left-then-right; covered by `chord_within_timeout_emits_only_middle`.
- [ ] Cover timeout fallback to ordinary left click.
  - [x] Add a human-like Criterion workload where the left button is held past the
    chord timeout, pointer motion occurs, and the left button is released.
  - [x] Add deterministic assertions that the delayed left press is emitted once
    and the final release matches it
    (`timer_deadline_flushes_pending_press_without_other_input`).
- [ ] Cover timeout fallback to ordinary right click.
  - [x] Add a human-like Criterion workload where the right button is held past the
    chord timeout, pointer motion occurs, and the right button is released.
  - [x] Add deterministic assertions for the right-button fallback through the
    shared pending-flush state machine (same code path as the left test).
- [ ] Cover release ordering while emulated middle is active.
  - [x] Add a human-like Criterion workload where the second physical button is
    released before the first while the synthesized middle button is active.
  - [ ] Add deterministic assertions for both release orders and prove no duplicate
    middle release is emitted.
- [ ] Cover interaction with left-handed mode.
  - [x] Add a human-like Criterion workload through the current
    left-handed-plus-middle-emulation filter chain.
  - [ ] Document and assert the intended filter ordering: whether chord detection
    operates on physical buttons before swap or logical buttons after swap.
- [ ] Cover interaction with debounce enabled.
  - [x] Add a human-like Criterion workload where a bouncy left-button transition
    is debounced before the right-button press completes the chord.
  - [ ] Add deterministic assertions for debounce-before-middle ordering and for
    bounces on both physical buttons.
- [ ] Cover device disable/removal while a chord is pending or active.
  - [x] Add human-like Criterion workloads for explicit input-rs device disable
    while one chord button is pending and while a synthesized middle press is
    active.
  - [x] Add human-like Criterion workloads for context-level input-rs device
    removal while one chord button is pending and while a synthesized middle
    press is active.
  - [x] Add human-like Criterion workloads for direct `DeviceRemoved` input while
    one chord button is pending and while a synthesized middle press is active.
  - [ ] Add deterministic coverage for explicit disable, evdev removal,
    monitor-driven removal, and compositor seat reset. Monitor-driven removal
    now reaches input-rs context cleanup, but final chord release assertions are
    still missing.
- [ ] Cover runtime emulation config changes while neutral, pending, and active.
  - [x] Add human-like Criterion workloads for enabling while neutral, disabling
    while one chord button is pending, and disabling while a synthesized middle
    button is active through the current filter-chain API.
  - [x] Add deterministic coverage through `Context::set_middle_button_emulation()`
    for pending and active chords, including flush and chord-tail behavior.

### Documentation

- [x] Update `FEATURE_PARITY.md`.
- [ ] Document config defaults and any device classes where emulation is
  intentionally unavailable.

## Local References

- `input-rs/src/processing/button_swap.rs`
- `input-rs/src/processing/debounce.rs`
- `input-rs/src/processing/mod.rs`
- `input-rs/src/state.rs`
- `libinput/doc/user/middle-button-emulation.rst`
- `libinput/src/libinput.h`
