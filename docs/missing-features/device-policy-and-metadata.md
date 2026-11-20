# Feature: Device Policy And Metadata

## Gap

Libinput has a broad device policy and metadata layer: udev properties, hidden
device-specific data, device groups, send-events modes including
disabled-on-external-mouse, LED updates, and plugin hooks. `input-rs` should use
libinput's device knowledge only as research input, not as the implementation
model.

`input-rs` now reads evdev name, vendor/product IDs, bus type, axis information,
clickpad/top-button/pointing-stick properties, LED state, and switch state into
`DeviceInfo`/`DeviceInfoRef`, classifies tablet tool/pad nodes, and composes
disable reasons in real dispatch. Internal/external classification is bus-derived
(I2C/PS2 = internal) with explicit profile overrides. Disabled-on-external-mouse
is dispatch-integrated (opt-in), and `Context::set_led_state()` writes evdev
`EV_LED` events. `DeviceProfile` provides the general explicit override layer.
Remaining gaps are sysfs-based grouping/internal detection and jay command
plumbing for the new policy/profile/LED APIs.

## Current Partial Implementation

- `EvdevBackend::add_device_fd()` reads:
  - evdev device name.
  - vendor ID.
  - product ID.
  - bus type mapped to USB, Bluetooth, I2C, PS/2, or unknown.
  - absolute axis min, max, and resolution.
  - coarse capabilities including keyboard, pointer, touchpad, touchscreen, and
    switch.
- `Context::device_info()` exposes name, vendor ID, product ID, bus type,
  capabilities, and seat name through `DeviceInfoRef`.
- Manual device enable/disable exists through `Context::set_device_enabled()`.
  Disabling a device now releases tracked keys/buttons, emits `TouchCancel` for
  tracked touches, resets stateful filters, and then drops future events while
  disabled.
- Explicit context removal exists through `Context::remove_device()` and
  `Context::remove_device_by_index()`. Removal releases the same tracked
  key/button/touch state as manual disable, resets stateful filters, asks the
  backend to forget the evdev fd/state where supported, removes the device from
  its seat, and queues one `DeviceRemoved` notification.
- Backend-originated `DeviceRemoved` events also clear the matching `Context`
  device record after tracked cleanup has been queued, so evdev removal no
  longer leaves the input-rs device table populated indefinitely.
- Device add is transactional through `Context::try_add_device_fd()`: a backend
  registration failure removes the placeholder device, emits no `DeviceAdded`,
  and returns the actual `io::Error`. `Context::add_device_fd()` remains a
  compatibility wrapper that logs the error and returns a null `DeviceId`.
- Jay's `InputDeviceConfig::SetDeviceEnabled` now forwards through
  `InputRsBackend::set_device_enabled()` into `Context::set_device_enabled()`.
  Device lookup accepts the same registered human, sysfs, backend, and sysname
  aliases used by the other runtime input-rs configuration paths.
- Jay's monitor-driven input-device removal path stores the input-rs `DevIndex`
  on the jay device state during add, then calls back into
  `InputRsBackend::remove_device()` during monitor removal so input-rs owns
  context cleanup instead of waiting for a later evdev dispatch error.
- `DevicePolicyState` can compose user-disabled, disabled-on-external-mouse,
  switch-disabled, and temporary disable reasons without losing the remaining
  active reasons when one reason clears.
- `DevicePolicyState` stores caps-lock, num-lock, and scroll-lock LED state, but
  does not yet forward LED changes to evdev devices.
- `feature_device_policy_and_metadata/metadata_policy_led_current_state` runs a
  human-like metadata, external-mouse, send-events, and LED-state sequence
  through the current policy model.
- `feature_device_policy_and_metadata` also has human-like Criterion workloads
  that carry representative metadata through the current standalone policy model:
  - USB, Bluetooth, I2C, PS/2, and unknown bus metadata;
  - internal and external metadata followed by send-events policy changes;
  - internal keyboard/touchpad/trackpoint-shaped metadata;
  - tablet tool/pad/touch-shaped metadata;
  - device-profile/override-shaped metadata for known internal devices and an
    unknown internal fallback device.
- `feature_device_policy_and_metadata/capability_inventory_candidate_current_info`
  carries representative `DeviceInfo` records for:
  - an internal laptop keyboard;
  - an external USB mouse;
  - an internal trackpoint-shaped pointer;
  - a clickpad/touchpad touch node with axis metadata;
  - a direct touchscreen with axis metadata;
  - a lid/tablet-mode switch node with supported and current switch state;
  - a tablet pen tool node;
  - a tablet pad button node.
- `feature_device_policy_and_metadata` also has context-level cleanup workloads
  for manual disable, monitor-shaped explicit removal by `DevIndex`, and
  backend-originated `DeviceRemoved` while a touch contact is active.
- These metadata/grouping workloads do not mean grouping is complete. The current
  `DevicePolicyState` records counts and a metadata hash; it does not yet assign
  stable group IDs or expose grouping to DWT, DWTP, switch policy, or tablet
  policy.
- The capability-inventory workload does not mean evdev classification is
  complete. It is a human-like baseline for the desired `DeviceInfo` shapes while
  real evdev/sysfs classification still lacks tablet, tablet-pad, trackpoint,
  clickpad, and internal/external policy-grade detection.

## Scope

This feature covers cross-device policy and metadata that multiple input
features depend on:

- Device classification.
- Device grouping.
- Built-in vs external device detection.
- Send-events modes.
- Disabled-on-external-mouse.
- LED state forwarding.
- User-visible device-profile overrides and optional plugin policy.
- Diagnostics for why a device behaves a certain way.

Palm/thumb device policy overrides are tracked in more detail in
`palm-thumb-and-quirks.md` (historical filename), but the common metadata and
matching layer belongs here.

## Requirements To Mark Complete

### Metadata Collection

- [x] Populate bus type correctly for USB, Bluetooth, I2C, PS/2, and unknown.
- [x] Populate vendor and product IDs for evdev devices where `input_id`
  provides them.
- [x] Preserve device name from evdev, with fallback to the add-device name.
- [ ] Read enough sysfs metadata to distinguish internal and external devices.
- [ ] Read physical path or stable identifiers where available.
- [x] Preserve seat assignment in `Context` and expose the seat name through
  `DeviceInfoRef`.
- [ ] Expose metadata to diagnostics without leaking unstable implementation
  details into public API unnecessarily.

### Device Classification

- [x] Classify keyboard, pointer, touchpad, touchscreen, tablet tool,
  tablet pad, gesture-capable, and switch devices accurately enough for filter
  selection (real evdev capability/property parsing).
  - [x] Add a human-like Criterion workload carrying representative `DeviceInfo`
    capability shapes for keyboard, pointer, touchpad/clickpad, touchscreen,
    switch, tablet tool, and tablet pad.
  - [ ] Add deterministic coverage for the real evdev/sysfs classification path
    that creates those capability shapes from hardware metadata.
- [x] Avoid treating all `EV_KEY` devices as keyboards when they are really
  pointer buttons, tablet pads, switches, or special keys (button-range codes are
  separated; pad nodes drop `KEYBOARD`).
- [x] Avoid treating all relative devices as ordinary mice when they are
  trackpoints: `INPUT_PROP_POINTING_STICK` sets `DeviceCaps::POINTING_STICK` and
  drives DWTP.
- [x] Classify clickpads and touchpads with physical buttons via
  `INPUT_PROP_BUTTONPAD`/`INPUT_PROP_TOPBUTTONPAD`.
- [x] Classify trackpoints/tracksticks for DWTP.
- [x] Support explicit `DeviceProfile` override when automatic classification is
  wrong (including internal classification).

### Device Grouping

- [ ] Group event nodes that belong to the same physical device.
- [ ] Group internal keyboard, touchpad, and trackpoint for DWT/DWTP and
  switch policy.
- [ ] Group tablet tool, tablet pad, and tablet touch nodes.
- [ ] Device groups must be stable across hotplug order.
- [ ] Device groups must not merge unrelated devices only because they share a
  vendor/product ID.
- [ ] Expose group identity to policy code while keeping public API stable.

### Send-Events Modes

- [x] Keep manual enabled/disabled device mode.
- [x] Manual disable must avoid stuck state for events tracked by `Context`:
  currently tracked key releases, pointer-button releases, and touch cancels are
  synthesized before the device starts dropping events.
- [x] Add disabled-on-external-mouse mode for internal touchpads
  (`Context::set_disable_internal_on_external_mouse`). It is opt-in, applies the
  external-mouse reason in real dispatch/hotplug, and ignores the internal
  touchpad when detecting external pointers.
- [x] Represent multiple disable reasons at once in real dispatch and the
  standalone policy model:
  - [x] user disabled.
  - [x] disabled on external mouse.
  - [x] switch/tablet-mode disabled.
  - [x] DWT/DWTP temporary suppression if modeled as send-events policy.
- [x] Removing one disable reason must not accidentally enable a device that is
  still disabled for another reason (real `Device::disable_reasons` composition).
- [x] Policy changes perform tracked cleanup immediately when a device becomes
  disabled; no neutral-state wait is needed for the current event classes.
  - [x] Manual disable performs tracked cleanup immediately.
  - [x] Monitor-driven device removal calls into input-rs context removal
    cleanup.
  - [x] Switch and external-mouse policy changes release tracked state through
    the same path.
- [ ] Device removal must have one owner for cleanup. Completion requires the
  monitor path, evdev error path, explicit path removal, and any future suspend
  removal path to converge on the same input-rs cleanup semantics: release
  tracked keys/buttons, cancel tracked touches, reset stateful filters, remove
  the backend fd/state, remove the device from its seat/group, and emit exactly
  one lifecycle removal notification to jay.
- [x] External mouse detection ignores internal touchpads (requires
  non-internal pointer without touchpad capability).
- [x] External mouse detection refreshes on device add and removal;
  `external_mouse_policy_composes_with_other_reasons` covers composition.

### LED Forwarding

- [x] Add a way to set keyboard LED state on evdev devices:
  `Context::set_led_state()` → `Backend::set_led_state()` →
  `EvdevBackend`, which writes `EV_LED` events + `SYN_REPORT`.
- [x] Support caps lock, num lock, and scroll lock
  (`LED_CAPSL`, `LED_NUML`, `LED_SCROLLL`).
- [ ] LED updates must target keyboard devices, not arbitrary `EV_KEY` devices.
  The current API allows any device; callers are expected to target keyboards.
- [x] Failure to update LEDs returns an `io::Error` to the caller and does not
  affect dispatch.
- [x] Initial LED state is read at device add into `DeviceInfo::led_state`.
  Explicit reapply after policy changes still belongs to the caller.

### Device Overrides And Plugin Policy

- [x] Add a data-driven device-profile/config override format.
  - [x] Add a human-like Criterion workload carrying known-device and fallback
    metadata through the current standalone policy model.
  - [x] Define a concrete profile format (`profile::DeviceProfile`) readable
    from configuration.
- [x] Define matching keys and precedence.
  - [x] Matching fields: bus, vendor ID, product ID, exact device name, and
    explicit profile name. Physical path/sysfs/capability/axis keys remain
    future work.
  - [x] Precedence: explicit selection wins; more match keys beat fewer;
    equally specific matches are rejected as ambiguous; missing/malformed
    profiles fall back to sanitized defaults.
- [x] Define supported override values: internal classification, pointer
  DPI/speed/adaptive/rotation/natural scrolling, touchpad timing/maps/click
  method/scroll method/palm/thumb/DWT/DWTP/button geometry/clickpad, and broken
  axes.
- [x] Profile application rejects invalid numeric values through
  `TouchpadConfig::sanitized()` and rejects invalid rotation angles.
- [x] Device profiles are testable without hardware (`profile::tests`,
  `device_profile_applies_defaults_on_add`,
  `explicit_profile_selection_ignores_match_keys`).
- [x] Plugin hooks are not planned; profiles are the supported extension
  mechanism.

### Diagnostics

- [ ] Provide a way to inspect device metadata, capabilities, selected filters,
  overrides, group, and active disable reasons.
- [ ] Logs must explain why a device was disabled by policy.
- [ ] Logs must explain when an override changed classification or thresholds.
- [ ] Diagnostics must be useful for fixing bad DPI, bad touchpad dimensions,
  and missing grouping.

### Coverage

- [ ] Cover metadata parsing for representative USB, Bluetooth, I2C, and PS/2
  devices.
  - [x] Add a human-like Criterion workload that carries USB, Bluetooth, I2C,
    PS/2, and unknown-bus metadata through the current standalone policy model.
  - [ ] Add deterministic coverage for real evdev/sysfs metadata parsing and
    `DeviceInfo` population for each bus type.
- [ ] Cover capability classification for representative device classes.
  - [x] Add a human-like Criterion workload carrying `DeviceInfo` capability
    inventories for keyboard, mouse, trackpoint-shaped pointer,
    touchpad/clickpad, touchscreen, switch, tablet tool, and tablet pad.
  - [ ] Add deterministic coverage that real evdev devices with equivalent
    events/keys/axes produce the expected `DeviceCaps` and do not accidentally
    inherit unrelated caps such as keyboard on tablet pads or pointer on direct
    touchscreens.
- [ ] Cover internal vs external device classification.
  - [x] Add a human-like Criterion workload that carries internal and external
    metadata plus disabled-on-external-mouse send-events changes through the
    current standalone policy model.
  - [ ] Add deterministic coverage for the real sysfs/internal-device classifier
    once it exists.
- [ ] Cover grouping of keyboard/touchpad/trackpoint.
  - [x] Add a human-like Criterion workload carrying internal
    keyboard/touchpad/trackpoint-shaped metadata through the current standalone
    policy model.
  - [ ] Add actual group identity, hotplug-stable grouping, and deterministic
    coverage for DWT/DWTP policy consumers.
- [ ] Cover grouping of tablet tool/pad/touch nodes.
  - [x] Add a human-like Criterion workload carrying tablet tool/pad/touch-shaped
    metadata through the current standalone policy model.
  - [ ] Add actual group identity, tablet node relationships, and deterministic
    coverage for tablet policy consumers.
- [x] Add a human-like Criterion workload for disabled-on-external-mouse
  add/remove state changes in the standalone policy model.
- [x] Add a human-like Criterion workload for multiple simultaneous disable
  reasons in the standalone policy model.
- [x] Add a human-like Criterion workload for LED state updates in the standalone
  policy model.
- [x] Add a human-like Criterion workload for manual device-disable cleanup
  through `Context::set_device_enabled(false)`.
- [x] Add a human-like Criterion workload for monitor-shaped context removal
  cleanup through `Context::remove_device_by_index()`.
- [x] Add a human-like Criterion workload for backend-originated
  `DeviceRemoved` cleanup through `Context::dispatch()` while a touch contact is
  active.
- [ ] Add deterministic coverage that jay `SetDeviceEnabled` config reaches the
  intended input-rs `DeviceId`, including human-readable name, sysfs name,
  evdev backend name, and raw sysname aliases.
- [ ] Add deterministic coverage that jay monitor-driven removal calls
  `InputRsBackend::remove_device()` with the stored input-rs `DevIndex`, that
  input-rs removes the matching `DeviceId`, and that repeated evdev removal
  notifications for the same device are harmless.
- [x] Cover disabled-on-external-mouse add/remove behavior in deterministic
  tests (`external_mouse_policy_composes_with_other_reasons`).
- [x] Cover multiple simultaneous disable reasons in deterministic tests
  (`user_disabled_device_stays_disabled_across_switch_changes`,
  `external_mouse_policy_composes_with_other_reasons`).
- [ ] Cover LED hardware update success and failure paths (requires a writable
  evdev device).
- [x] Cover device-profile/override matching, precedence, and invalid override
  rejection.
  - [x] Add a human-like Criterion workload carrying known internal touchpad,
    internal PS/2, and unknown internal fallback metadata through the current
    standalone policy model.
  - [x] Add deterministic coverage for actual profile matching, explicit
    user-config precedence, ambiguity rejection, and fallback-to-default
    behavior (`profile::tests`, `device_profile_applies_defaults_on_add`,
    `explicit_profile_selection_ignores_match_keys`).

### Documentation

- [x] Update `FEATURE_PARITY.md`.
- [ ] Document supported send-events modes.
- [x] Document device-profile/override format and matching (see
  `profile.rs` rustdoc and `FEATURE_PARITY.md`; jay config docs still needed).
- [ ] Document diagnostic output so bug reports can include useful data.

## Local References

- `input-rs/src/backend/linux_discovery.rs`
- `input-rs/src/backend/evdev.rs`
- `input-rs/src/backend/mod.rs`
- `input-rs/src/state.rs`
- `src/input/input_rs_backend.rs`
- `libinput/doc/user/device-configuration-via-udev.rst`
- `libinput/doc/user/lua-plugins.rst`
- `libinput/src/libinput.h`
