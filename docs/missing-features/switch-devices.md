# Feature: Switch Devices

## Gap

Libinput supports switch devices such as lid switches and tablet-mode switches.
It emits switch toggle events and applies policy such as disabling internal
touchpads, keyboards, or trackpoints when tablet mode is active.

`input-rs` now has initial partial support: evdev devices exposing `EV_SW`
set `DeviceCaps::SWITCH`, `InputEvent::SwitchToggle` carries a typed
`SwitchKind` plus boolean state, lid/tablet-mode evdev events are forwarded,
unknown switch codes are preserved as `SwitchKind::Unknown(code)`, and the jay
bridge maps the known switch kinds to jay's existing switch event enum.

The feature is not complete because the current implementation does not read
initial switch state into switch-driven policy, does not handle switch-device
removal as policy input, and does not apply switch-driven internal-device
policy.

## Current Partial Implementation

- `input-rs/src/backend/evdev.rs` detects `EV_SW`, queues pending switch state,
  flushes it at `SYN_REPORT`, and suppresses repeated same-state switch events
  after the first observed state for that switch kind.
- `input-rs/src/backend/evdev.rs` records the supported switch codes reported by
  evdev and reads the initial switch state during device add.
- `input-rs/src/backend/mod.rs` exposes supported switch kinds and known switch
  states through `DeviceInfo` / `DeviceInfoRef`.
- `input-rs/src/events/mod.rs` defines `InputEvent::SwitchToggle` and
  `SwitchKind::{Lid, TabletMode, Unknown(u32)}`.
- `input-rs/src/state.rs` remaps switch events when device indices change.
- `src/input/input_rs_backend.rs` reports jay switch capability and maps:
  - `SwitchKind::Lid`, `enabled = false` to `LidOpened`.
  - `SwitchKind::Lid`, `enabled = true` to `LidClosed`.
  - `SwitchKind::TabletMode`, `enabled = false` to `ConvertedToLaptop`.
  - `SwitchKind::TabletMode`, `enabled = true` to `ConvertedToTablet`.
  - `SwitchKind::Unknown(_)` is ignored by jay for now because jay has no
    equivalent public switch event for unknown switch identities.
- `input-rs/benches/feature_workload_bench.rs` includes
  `feature_switch_devices/lid_and_tablet_mode_current_events` with a
  human-like lid/tablet-mode sequence.
- `input-rs/benches/feature_workload_bench.rs` also includes
  `feature_switch_devices/unknown_switch_passthrough_current_events` so unknown
  switch identities stay represented in the event model and filter path.
- `feature_switch_devices` also has human-like Criterion workloads for:
  - startup-shaped initial lid/tablet-mode switch state events;
  - active tablet-mode switch followed by switch device removal;
  - rapid tablet-mode state changes, matching convertible hinges that briefly
    move through laptop/tablet/laptop states while the user adjusts the screen;
  - an active unknown switch followed by device removal, so unknown switch
    identities remain safe even when the reporting node disappears while the
    switch is logically active;
  - switch-driven internal-device disable and restore through the current
    standalone policy model;
  - user-disabled internal devices staying disabled while switch state changes in
    the standalone policy model;
  - external devices remaining enabled when switch policy changes in the
    standalone policy model;
  - composition of switch-driven disable with disabled-on-external-mouse for an
    internal device, so the standalone policy model keeps the two disable
    reasons distinct while either or both are active.

## Scope

This feature covers hardware switch devices and switch-driven policy:

- Lid switch.
- Tablet-mode switch.
- Keypad slide switch if supported by hardware.
- Switch toggle events.
- Internal device enable/disable policy caused by switch state.
- Runtime state tracking across hotplug and startup.

This feature does not cover software configuration toggles such as manually
disabling a device. That is tracked in `device-policy-and-metadata.md`.

## Requirements To Mark Complete

### Capability Detection

- [x] Detect `EV_SW` devices and set `DeviceCaps::SWITCH`.
- [x] Detect which switch codes a device supports.
- [x] At minimum, handle:
  - [x] lid switch toggle events.
  - [x] tablet-mode switch toggle events.
- [x] Do not classify devices as switch-capable when they only expose ordinary
  keys or buttons.
- [x] Store initial switch state during device add so event processing knows the
  current state before the first toggle.

### Event Model

- [x] Add a switch event variant with device index, timestamp, switch code, and
  state.
- [x] Define typed switch codes instead of passing raw integers through every
  layer.
- [x] Preserve unknown switch codes in a way that can be logged or passed
  through without panics.
- [x] Emit switch events only for actual state changes unless a startup sync
  event is deliberately defined.
  - [x] Suppress repeated same-state events after input-rs has observed a state
    for that switch kind.
  - [x] Read startup state explicitly so the first event after device add can be
    classified as either initial sync or a real hardware state change.

### Backend Processing

- [x] Parse `EV_SW` events in the evdev backend.
- [x] Coalesce switch state according to `SYN_REPORT`.
- [x] Read initial switch state from the device where evdev exposes it.
- [ ] Handle devices that report switch state before the compositor has
  completed device configuration.
  - [x] Seed input-rs backend state from evdev before dispatching later switch
    events.
  - [ ] Feed seeded state into jay-visible policy once switch policy exists.
- [ ] Handle switch device removal without leaving policy stuck.
  - [x] Add a human-like Criterion workload for known tablet-mode removal while
    active through the current event path.
  - [x] Add a human-like Criterion workload for unknown-switch removal while
    active through the current event path.
  - [ ] Define the real policy behavior when the switch device disappears while
    its last reported state is active. Completion requires a concrete rule such
    as "clear only the state contributed by the removed switch device", "keep the
    last state until another switch in the same group reports", or "fall back to
    startup/device-profile defaults"; the chosen rule must be documented and
    deterministic coverage must prove it.

### Policy

- [ ] Define lid switch policy. If jay does not act on lid state, the event
  must still be exposed or the non-goal must be documented.
- [ ] Define tablet-mode policy for internal devices.
- [ ] Tablet mode should disable internal touchpad and trackpoint where
  appropriate.
- [ ] Tablet mode may disable internal keyboard if the hardware requires it;
  this must be device-group aware.
- [ ] External devices must not be disabled by tablet-mode policy unless
  explicitly configured.
- [ ] When tablet mode ends, devices disabled by switch policy must be restored
  without overriding user-disabled devices.
- [ ] Policy must distinguish user-disabled, switch-disabled, and
  disabled-on-external-mouse states so one policy does not erase another.
  - [x] Add a human-like Criterion workload where an internal device is disabled
    by an external mouse, then also disabled by tablet-mode policy, then restored
    one reason at a time in the standalone policy model.
  - [ ] Wire the same reason composition into jay/input-rs runtime dispatch so
    changing one policy bit does not accidentally re-enable a device still
    disabled by another policy bit.

### Device Grouping

- [ ] Identify which keyboard, touchpad, and trackpoint are internal to the
  switched chassis.
- [ ] Grouping must not rely only on discovery order.
- [ ] If grouping metadata is unavailable, the fallback policy must be safe and
  documented.

### Jay Bridge

- [x] Forward lid and tablet-mode switch events to jay's existing switch event
  path.
- [ ] If jay consumes switch events only as policy input, document that choice.
- [ ] Ensure user configuration can opt into or out of switch-driven disabling
  if such config is exposed.

### Coverage

- [x] Add a human-like Criterion workload for lid switch open/close and
  tablet-mode convert-to-tablet/convert-to-laptop events.
- [x] Add a Criterion workload that carries an unknown switch code through the
  current event/filter path without treating it as lid or tablet mode.
- [ ] Cover lid switch on/off events in deterministic tests or replay fixtures.
- [ ] Cover tablet-mode switch on/off events in deterministic tests or replay
  fixtures.
- [ ] Cover initial switch state on device add.
  - [x] Add a human-like Criterion workload carrying startup-shaped initial
    lid/tablet-mode switch state through the current event path.
  - [ ] Add deterministic coverage that real evdev initial switch state is read
    during device add and exposed to policy exactly once.
- [ ] Cover switch device removal while active.
  - [x] Add a human-like Criterion workload carrying an active tablet-mode switch
    followed by `DeviceRemoved` through the current event path.
  - [x] Add a human-like Criterion workload carrying an active unknown switch
    followed by `DeviceRemoved` through the current event path.
  - [ ] Add deterministic coverage that removal clears or reconciles switch-driven
    policy state.
- [ ] Cover internal device disable and restore.
  - [x] Add a human-like Criterion workload for switch-driven internal-device
    disable and restore in the standalone policy model.
  - [ ] Wire switch policy into real grouped devices and add deterministic
    coverage for touchpad, trackpoint, and keyboard policy choices.
- [ ] Cover user-disabled device staying disabled after switch state changes.
  - [x] Add a human-like Criterion workload where a user-disabled internal device
    remains disabled while switch policy toggles in the standalone policy model.
  - [ ] Add deterministic coverage once composed policy reasons are wired into
    real dispatch state.
- [ ] Cover external devices remaining enabled.
  - [x] Add a human-like Criterion workload where external devices remain enabled
    while switch policy toggles in the standalone policy model.
  - [ ] Add deterministic coverage once switch policy receives real internal vs
    external classification.
- [ ] Cover rapid tablet-mode changes.
  - [x] Add a human-like Criterion workload for tablet-mode true/false/true/false
    transitions over short human-adjustment intervals through the current event
    path.
  - [ ] Add deterministic coverage proving every real state change is delivered
    in order after evdev `SYN_REPORT` coalescing, while repeated same-state
    events are suppressed by the backend.
- [ ] Cover switch-disable composition with disabled-on-external-mouse.
  - [x] Add a human-like Criterion workload for an internal device disabled first
    by external mouse policy, then by both external mouse and switch policy, then
    by switch policy alone, and finally restored.
  - [ ] Add deterministic coverage against the real dispatch/device state once
    switch policy is wired into grouped devices.

### Documentation

- [x] Update `FEATURE_PARITY.md`.
- [ ] Document switch policy defaults.
- [ ] Document how switch policy composes with manual device disable and
  disabled-on-external-mouse.

## Local References

- `input-rs/src/backend/mod.rs`
- `input-rs/src/backend/evdev.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/state.rs`
- `libinput/doc/user/switches.rst`
- `libinput/src/libinput.h`
