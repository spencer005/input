# Feature: Button Debounce Configuration

## Gap

Libinput has a button-debounce plugin for worn or faulty hardware buttons. Its
user documentation describes two debounce modes:

- bounce filtering for quick press-release-press or release-press-release chatter
  around a real state change.
- spurious-release filtering for devices that briefly report release/press while
  the button is physically held.

`input-rs` has a `ButtonDebounceFilter` and runtime plumbing down to
`Context::set_debounce()`, but this feature is not complete as a configurable
input-rs-owned option yet. The current jay input-device command path does not
expose debounce, debounce settings are not replayed for devices that appear
later, the default/auto-enable policy is different from libinput and is not fully
documented, and diagnostics do not show whether debounce is enabled for a device.

Libinput-style hidden quirks are intentionally out of scope. If input-rs needs
device-specific debounce defaults, they should come from explicit configuration,
device profiles, or another user-visible override mechanism with diagnostics.

## Current Partial Implementation

- `DebounceConfig` stores:
  - `enabled`, which disables the filter as a zero-latency passthrough when
    false.
  - `bounce_timeout_us`, a microsecond timeout for suppressing short bounce
    transitions.
  - `spurious_timeout_us`, a microsecond timeout for suppressing spurious
    release/press sequences. A value of `0` disables that phase.
- `DebounceConfig::default()` is disabled by default with current timeout values
  of 25 ms for bounce and 12 ms for spurious detection.
- `ButtonDebounceFilter` tracks one independent state entry per Linux input
  code (0..0x2ff), so key/button codes can never alias through a modulo table.
  Codes above that range pass through without debounce.
- Deferred transitions are deadline-driven: `next_timeout_us()` and
  `dispatch_timeout()` emit a suppressed release or deferred spurious-repress
  without requiring another input event. `FilterKind` and `Context` expose the
  timer through the normal timeout dispatch path, and timer-generated events
  traverse the remaining filter chain.
- `ButtonDebounceFilter::reset_runtime_state()` clears pending debounce state.
- `FilterChain::set_debounce()` updates an existing filter in place so pending
  deadlines survive configuration changes, and normalizes chain order.
- `Context::set_debounce()` flushes pending debounce transitions through the
  remaining chain before applying new configuration, so disabling debounce
  cannot drop a suppressed release.
- jay's low-level `InputDeviceConfig::SetDebounce` and `InputBackend::set_debounce()`
  exist, and `InputRsBackend::set_debounce()` forwards to `Context`.
- There is no `InputDeviceCommand::SetDebounce`, CLI command, jay input protocol
  message, or config handler path that lets normal users configure debounce per
  device today.
- `InputRsBackend::set_debounce()` only applies to currently resolved device IDs.
  Unlike DPI/profile/speed settings, it does not store pending debounce
  configuration for later hotplug replay.
- `feature_button_debounce` has human-like Criterion workloads for:
  - disabled passthrough of a bouncy pointer button.
  - enabled bounce suppression for a noisy pointer button.
  - spurious double-click suppression.
  - independent left/right pointer button state.
  - keyboard key bounce through the current filter.
  - zero-spurious-timeout configuration.
  - enabling debounce while neutral.
  - disabling debounce while a bouncy sequence is active.
  - changing debounce thresholds while neutral.
  - direct `DeviceRemoved` input while a bouncy debounce sequence is pending.
  - direct `DeviceRemoved` input while a spurious-release debounce sequence is
    pending.
  - explicit `Context::set_device_enabled(false)` while a bouncy debounce
    sequence is pending.
  - explicit `Context::set_device_enabled(false)` while a spurious-release
    debounce sequence is pending.
- `feature_middle_button_emulation/debounced_bouncy_chord_current_chain` also
  covers debounce-before-middle-button ordering through the current filter chain.

## Local Libinput References

- `libinput/doc/user/button-debouncing.rst` describes bounce and spurious button
  debouncing, the latency tradeoff for delayed releases, and the fact that
  libinput's debounce is hardware debouncing rather than an accessibility
  feature.
- `libinput/src/libinput-plugin-button-debounce.c` implements the current
  libinput `button-debounce` plugin, including state names for bounce,
  spurious-release detection, timeout events, and "other button" flushing.
- `libinput/src/libinput-plugin.c` registers the debounce plugin.

## Scope

This feature covers input-rs-owned debounce behavior for physical button/key
events:

- Pointer button bounce suppression.
- Pointer button spurious-release suppression.
- Optional keyboard key debounce if input-rs keeps that behavior.
- Runtime debounce enable/disable.
- Runtime timeout configuration.
- Hotplug replay of explicit user/device-profile settings.
- Diagnostics for the effective debounce policy and source.
- Filter ordering with button swap, middle-button emulation, button scrolling,
  clickpad button mapping, and device disable/removal cleanup.

It does not cover accessibility "bounce keys" semantics unless that is
explicitly chosen as a separate feature. Accessibility filtering has different
requirements, user expectations, and timing policy than hardware debounce.

## Requirements To Mark Complete

### Policy And Defaults

- [ ] Decide whether debounce is a user option, a device-profile option, an
  automatic hardware-detection behavior, or some combination of those.
- [x] Keep debounce disabled by default for the current input-rs filter.
- [ ] If input-rs keeps debounce disabled by default, document why this differs
  from libinput's always-enabled bounce method and what users should configure
  for faulty hardware.
- [ ] If input-rs adds automatic spurious-release detection, define the exact
  sequence that enables it, how long it remains enabled, and how users can
  override it.
- [ ] Do not add hidden libinput-style quirks. Any per-device default must be
  visible through an explicit device profile, user config rule, or diagnostics.
- [ ] Decide whether key debounce is in scope. Current code filters
  `InputEvent::Key`; completion requires documenting whether this is hardware
  keyboard debounce, accessibility bounce keys, or an implementation detail that
  should be removed for keys.

### Data Model

- [x] Represent debounce enablement and timeout values in `DebounceConfig`.
- [x] Store current debounce state per physical button/key in the filter.
- [ ] Store the effective debounce configuration on the input-rs device or
  filter state in a queryable form.
- [ ] Store the source of the effective debounce configuration:
  - [ ] built-in default.
  - [ ] user runtime config.
  - [ ] explicit device-profile default.
  - [ ] automatic spurious-detection escalation, if implemented.
- [ ] Define valid timeout ranges. Completion must state whether zero is accepted
  for each timeout, whether very large timeouts are clamped, and whether
  non-finite or negative values are impossible at the public API boundary.

### Runtime Configuration

- [x] Provide `Context::set_debounce()` for live input-rs devices.
- [x] Provide a low-level `InputBackend::set_debounce()` hook and
  `InputDeviceConfig::SetDebounce` variant.
- [ ] Expose a normal jay input-device command/protocol/config path for debounce
  enablement and timeout values.
- [ ] Persist backend-level debounce requests by device alias so they apply to
  matching devices that are plugged in later.
- [ ] Match pending debounce configuration against the same aliases used for
  other pointer configuration: jay human-readable name, sysfs name, evdev backend
  name, and raw kernel sysname such as `event7`.
- [ ] Expose a clear/reset operation that returns a device to the default or
  device-profile debounce policy.
- [x] Define whether changing debounce configuration while a button is held is
  immediate, deferred until all tracked buttons are neutral, or cancels/releases
  pending debounce state. It is immediate: pending transitions are flushed, and
  enabling seeds held codes as `Down`.
- [x] Ensure disabling debounce while a release is delayed cannot leave clients
  believing a button is still held.
- [x] Ensure enabling debounce while buttons are already held does not synthesize
  duplicate presses or suppress the eventual real release
  (`enabling_debounce_while_button_held_preserves_the_release`).

### State Machine Behavior

- [x] Pass pointer button/key events through unchanged when debounce is disabled.
- [x] Suppress quick bounce transitions in the current enabled filter.
- [x] Support a zero spurious timeout that disables spurious-release suppression.
- [x] Define exact timeout semantics for equality at the timeout boundary:
  deadlines fire when `now >= last_event_time + timeout`; an event at exactly
  the boundary is emitted, not suppressed.
- [x] Define behavior for timestamp rollback, duplicate timestamps, and very
  large timestamp gaps: elapsed time is saturating, so rollback behaves like
  zero elapsed and the timeout fires on the next deadline check.
- [x] Track button state by stable key/button identity without accidental
  collisions. State is indexed by the raw Linux input code (0..0x2ff); codes
  outside that range pass through. Covered by
  `test_codes_do_not_alias_debounce_state`.
- [x] Define behavior when multiple buttons change in the same frame. input-rs
  deliberately keeps per-code state independent and does not flush other buttons
  on a button change; a single timeout dispatch emits each deferred release in
  code order. Covered by `multiple_buttons_debounce_independently`.
- [ ] Define behavior for repeated press while already pressed and repeated
  release while already released.
- [ ] Define interaction with `Frame` events: if debounce delays a release, the
  eventual release must have a documented timestamp and frame relationship.
- [ ] Define cancellation on device removal, device disable, `SYN_DROPPED`, or
  other backend reset conditions.

### Filter Ordering

- [ ] Define whether debounce runs before or after left-handed button swap.
  Completion must state whether the debounce state tracks physical buttons or
  logical swapped buttons.
- [ ] Define whether debounce runs before or after middle-button emulation.
  Chord detection should normally see debounced physical left/right transitions
  if debounce is enabled.
- [ ] Define whether debounce runs before or after button scrolling. Button
  scrolling must not lock or unlock because of a suppressed bounce event.
- [ ] Define whether debounce applies before clickpad button-area/clickfinger
  mapping or only to already synthesized logical buttons.
- [ ] Ensure reset/removal paths clear debounce state before any later filter can
  observe stale held buttons.

### Diagnostics

- [ ] Log when a device receives a non-default debounce configuration and include
  the source of that configuration.
- [ ] Log invalid debounce settings with the device identity and rejected values.
- [ ] Provide a device-inspection path that reports:
  - [ ] debounce enabled/disabled.
  - [ ] bounce timeout.
  - [ ] spurious timeout.
  - [ ] configuration source.
  - [ ] whether automatic spurious mode is active, if that mode is implemented.
- [ ] Avoid noisy logs for every suppressed event during a long faulty sequence.

### Coverage

- [x] Add deterministic coverage for disabled passthrough.
- [x] Add deterministic coverage for basic bounce suppression.
- [x] Add deterministic coverage for the start of spurious detection.
- [ ] Add deterministic coverage for complete spurious-release suppression,
  including the delayed-release path and re-press timing.
- [x] Add deterministic coverage for independent button state without modulo
  collisions (`test_codes_do_not_alias_debounce_state`).
- [x] Add deterministic coverage for keyboard key debounce
  (`test_deferred_key_release_is_deadline_driven`).
- [ ] Add deterministic coverage for zero bounce timeout and zero spurious
  timeout.
- [x] Add deterministic coverage for timeout-boundary equality
  (`test_deferred_release_is_deadline_driven` fires at exactly the deadline).
- [x] Add deterministic coverage for runtime enable/disable while neutral,
  pressed, release-delayed, and spurious-detection states. Enabled-while-held
  and deferred-release cases are covered by tests; neutral enable is exercised
  by the benchmark path.
- [ ] Add deterministic coverage for device disable/removal cleanup while debounce
  state is pending.
  - [x] Add human-like Criterion workloads for direct `DeviceRemoved` input while
    bouncy and spurious-release debounce state is pending.
  - [x] Add human-like Criterion workloads for explicit
    `Context::set_device_enabled(false)` while bouncy and spurious-release
    debounce state is pending.
  - [x] Add human-like Criterion workloads for context-level
    `Context::remove_device_by_index()` while bouncy and spurious-release
    debounce state is pending.
  - [ ] Add deterministic assertions for the final cleanup policy, including
    whether pending releases are emitted, cancelled, or reset silently.
- [x] Add deterministic coverage for filter ordering with left-handed swap and
  timeout/debounce flush traversal
  (`debounce_flush_passes_through_remaining_filters`,
  `configuration_order_does_not_change_pipeline_semantics`). The canonical
  order is debounce → touchpad recognition → middle emulation → button swap → button mapping →
  pointer/touchpad acceleration → button scroll → natural scroll → rotation.
- [ ] Add deterministic coverage for jay/backend config propagation and hotplug
  replay once user-facing debounce configuration exists.
- [x] Add a human-like Criterion workload for disabled bouncy-button passthrough.
- [x] Add a human-like Criterion workload for enabled bouncy-button suppression.
- [x] Add a human-like Criterion workload for spurious double-click suppression.
- [x] Add a human-like Criterion workload for independent left/right buttons.
- [x] Add a human-like Criterion workload for keyboard key bounce.
- [x] Add a human-like Criterion workload for zero spurious timeout.
- [x] Add human-like Criterion workloads for runtime enable, disable, and
  threshold-change paths.
- [x] Add human-like Criterion workloads for direct `DeviceRemoved`,
  context-level device removal, and explicit device disable while bouncy and
  spurious debounce states are pending.
- [x] Add a human-like Criterion workload for debounce interaction with
  middle-button emulation.

### Documentation

- [x] Add this missing-feature document.
- [ ] Update `FEATURE_PARITY.md` when debounce becomes fully user-configurable
  and deterministically covered.
- [ ] Document the user-facing jay command or config syntax once it exists.
- [ ] Document defaults, timeout units, timeout ranges, and zero-timeout behavior.
- [ ] Document that input-rs owns hardware debounce behavior and jay should
  forward configuration rather than duplicate debounce filtering above input-rs.

## Local References

- `input-rs/src/processing/debounce.rs`
- `input-rs/src/processing/mod.rs`
- `input-rs/src/state.rs`
- `src/backend.rs`
- `src/input/input_backend.rs`
- `src/input/input_rs_backend.rs`
- `src/backends/drm.rs`
- `input-rs/benches/feature_workload_bench.rs`
- `libinput/doc/user/button-debouncing.rst`
- `libinput/src/libinput-plugin-button-debounce.c`
- `libinput/src/libinput-plugin.c`
