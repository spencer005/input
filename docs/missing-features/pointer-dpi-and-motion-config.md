# Feature: Pointer DPI And Motion Configuration

## Gap

Libinput treats pointer DPI as device metadata, not as a guessed constant. Local
libinput references show that relative pointer devices may provide `MOUSE_DPI`
through udev properties, and libinput parses values such as a single DPI
(`400@125`), a list of supported DPIs, and a selected entry marked with `*`.
That DPI feeds motion normalization before pointer acceleration, so an otherwise
correct acceleration curve can still feel wrong when the DPI is wrong.

`input-rs` has the core math for DPI normalization in `MouseFilter`, plus
runtime hooks from jay's backend configuration layer into
`Context::set_mouse_dpi()`, `Context::set_mouse_profile()`, and
`Context::set_mouse_speed_adjustment()`. The current implementation is still not
full libinput parity because the `MOUSE_DPI` parser is not wired to device-add
metadata discovery, it does not expose a general user-facing jay config command
for DPI or custom acceleration curves, it does not report the effective DPI and
motion configuration back through device inspection, and it does not have
production jay-side coverage for every hotplug alias path used when pending
pointer configuration is replayed onto newly added devices.

## Current Partial Implementation

- `MouseFilter` normalizes relative pointer motion with `dpi_factor = 1000 / dpi`.
- `MouseConfig` carries the three pieces of relative-pointer motion policy that
  input-rs owns today: sensor DPI, speed adjustment, and acceleration profile.
- `MouseAccelProfile::Flat` applies DPI normalization plus constant speed
  scaling; `MouseAccelProfile::Adaptive` applies the current velocity-based
  built-in curve; `MouseAccelProfile::Custom` applies a piecewise-linear
  speed-to-factor curve.
- Custom profile points are represented as `(speed_mm_per_s, factor)` pairs and
  are evaluated by linear interpolation. Speeds below the first point use the
  first factor; speeds above the last point use the last factor.
- Direct `MouseFilter` construction normalizes non-positive DPI values to the
  documented 1000 DPI fallback, and runtime `FilterChain::set_mouse_dpi()` ignores
  non-positive changes.
- `parse_mouse_dpi_property()` parses libinput-style DPI metadata strings:
  selected entries with `*`, comma or whitespace separated lists, optional
  sampling-rate suffixes with `@`, first-selected-entry behavior, and invalid
  token rejection.
- `Context::set_mouse_dpi()` updates the mouse filter for a live input-rs
  `DeviceId`.
- `Context::set_mouse_profile()` updates a live input-rs device to flat,
  adaptive, or custom acceleration by replacing the mouse filter with one built
  from the existing DPI and speed adjustment.
- `Context::set_mouse_speed_adjustment()` updates a live input-rs device by
  replacing the mouse filter with one built from the existing DPI and profile.
- `InputBackend::set_mouse_dpi()` and `InputDeviceConfig::SetMouseDpi` can now
  forward a backend-level DPI request into `InputRsBackend`.
- jay's existing acceleration profile config currently exposes only flat and
  adaptive profiles. The input-rs custom profile implementation is not complete
  from a user-configurability perspective until jay can send a custom curve.
- jay's existing acceleration speed config forwards to input-rs and is stored for
  hotplug replay by device alias.
- `InputRsBackend::set_mouse_dpi()` stores requested DPI values by device name so
  they can be applied to devices that appear later.
- Pending DPI, acceleration profile, and acceleration speed settings are matched
  against the same device aliases registered at hotplug time:
  - jay's human-readable device name.
  - the sysfs name read from `/sys/class/input/<sysname>/device/name`.
  - the evdev backend-reported name.
  - the raw kernel input sysname such as `event7`.
- `InputRsBackend::add_device_fd()` applies pending DPI before the device-specific
  fallback override. This means an explicit pending rule wins over the built-in
  Razer Viper V3 Pro DPI override.
- A built-in Razer Viper V3 Pro fallback override remains in
  `InputRsBackend::dpi_override_for_device()` so that hotplugged matching devices
  use 35000 DPI even when jay was started before the mouse was plugged in.
- `DualSubpixelAccumulator` preserves subpixel precision when normalized `f64`
  deltas are converted into jay's `Fixed` coordinates.
- `feature_pointer_dpi_and_motion_config/hotplug_dpi_replay_current_context`
  queues a bench-local pending DPI rule, adds a matching pointer, applies the
  replayed DPI through the current input-rs context hook, and then dispatches
  human-like high-DPI pointer motion.
- `feature_pointer_dpi_and_motion_config` also has bench-local hotplug replay
  workloads for:
  - a generic add-device name such as `event7` followed by the real evdev device
    name from `DeviceInfo`;
  - two matching pointer nodes with the same human-readable name;
  - a re-add-shaped sequence where the same matching name appears again;
  - a matching non-pointer device name, which must not receive a DPI filter
    change.
- `feature_pointer_dpi_and_motion_config` has human-like acceleration workloads
  for:
  - an adaptive speed ramp through the current mouse filter;
  - a custom profile speed ramp through the current mouse filter;
  - changing from flat to adaptive while representative pointer motion is being
    processed by the current filter chain;
  - changing from adaptive to a custom profile while representative pointer
    motion is being processed by the current filter chain;
  - changing speed adjustment while representative pointer motion is being
    processed by the current filter chain.

## Local Libinput References

- `libinput/doc/user/device-configuration-via-udev.rst` documents `MOUSE_DPI` as
  hardware resolution and sampling frequency metadata for relative pointer
  devices.
- `libinput/test/test-utils.c` has parser coverage for DPI lists, selected
  entries marked with `*`, optional sampling-rate suffixes using `@`, invalid
  strings, negative values, zero values, and empty values.
- `libinput/test/litest-device-mouse-low-dpi.c` defines a low-DPI mouse fixture
  with `MOUSE_DPI=400@125`.
- `libinput/test/test-pointer.c` adjusts expected relative motion by
  `1000.0 / dpi` when `MOUSE_DPI` is present.

## Scope

This feature covers relative pointer motion normalization and runtime motion
configuration for mice, trackballs, trackpoints, and other relative pointer
devices:

- DPI metadata discovery.
- DPI parser behavior.
- Runtime DPI override.
- Runtime acceleration profile override.
- Runtime acceleration speed adjustment.
- Custom user-defined acceleration curves.
- Hotplug persistence for pending runtime configuration.
- Interaction between DPI, acceleration profile, acceleration speed, and
  unaccelerated deltas.
- Diagnostics for the effective DPI and the source of that value.

It does not cover absolute coordinate calibration. Touchscreen, tablet, and
absolute touchpad calibration is tracked in `absolute-device-transforms.md`.

## Requirements To Mark Complete

### Data Model

- [ ] Store the effective DPI on the input-rs device or filter state in a way that
  can be queried after hotplug and after runtime changes.
- [ ] Store the source of the effective DPI:
  - [ ] built-in default.
  - [ ] parsed metadata.
  - [ ] user/runtime override.
  - [ ] device-specific fallback override.
- [x] Preserve the configured DPI value across filter replacement when changing
  acceleration profile or speed adjustment. The current filter-chain update
  methods rebuild `MouseFilter` from `MouseConfig::default_from_existing()`.
  Deterministic coverage for this preservation is still tracked below.
- [x] Prevent non-positive DPI values from creating an invalid `MouseFilter`
  normalization factor. Runtime DPI changes are rejected before filter
  replacement, and direct filter construction falls back to 1000 DPI.
- [ ] Decide and document the valid maximum DPI. The implementation must state
  whether very high values such as 35000 are accepted directly or clamped.

### Metadata Parsing

- [ ] Read `MOUSE_DPI` or the chosen replacement metadata source during device
  add for relative pointer devices.
- [x] Parse a single DPI value such as `400`.
- [x] Parse a DPI value with sampling-rate suffix such as `400@125`, preserving
  the DPI and either storing or explicitly ignoring the sampling rate.
- [x] Parse a list of values such as `450 1800 3200`.
- [x] Select the value marked with `*` when present.
- [x] If no value is marked, match libinput-compatible behavior for the selected
  default and document that behavior.
- [x] Reject malformed tokens such as non-numeric DPI, missing DPI around `*`, and
  malformed sampling-rate suffixes.
- [x] Reject zero and negative DPI values.
- [x] Define behavior for multiple `*` entries. If the first selected value wins,
  cover that explicitly. If the entire property is rejected, cover that
  explicitly.
- [ ] Define behavior when metadata is absent. The current fallback is 1000 DPI;
  completion requires this to be documented and observable.

### Runtime Configuration

- [x] Provide `Context::set_mouse_dpi()` for live input-rs devices.
- [x] Provide an input backend hook so jay's backend configuration layer can send
  a DPI request to input-rs.
- [x] Persist backend-level DPI requests by device name so they apply to matching
  devices that are plugged in later.
- [ ] Expose a user-facing jay config API for setting DPI by input device.
- [ ] Expose a way to clear a runtime DPI override and return to metadata/default
  behavior.
- [ ] Ensure runtime DPI changes apply only to relative pointer devices and do not
  alter touchpads, touchscreens, tablets, or keyboards unless explicitly
  supported.
- [ ] Define whether a runtime DPI change is immediate or waits for neutral
  pointer state.
- [x] Define how runtime DPI interacts with acceleration profile replacement:
  the current filter-chain replacement preserves `MouseFilter::dpi()` through
  `MouseConfig::default_from_existing()`, so profile changes do not intentionally
  reset DPI to 1000.
- [x] Define how runtime DPI interacts with acceleration speed changes: the
  current filter-chain replacement preserves `MouseFilter::dpi()` through
  `MouseConfig::default_from_existing()`, so speed changes do not intentionally
  reset DPI to 1000.
- [x] Define how runtime DPI interacts with flat/adaptive/custom acceleration
  profiles: all three profiles receive the same DPI-normalized deltas first, and
  then apply profile-specific acceleration or scaling.
- [ ] Define whether runtime profile and speed changes are immediate or wait for
  neutral pointer state. Current filter replacement is immediate and resets the
  mouse filter's velocity history; completion requires either documenting that
  as intentional or replacing it with a neutral/deferred policy.
- [ ] Expose a user-facing jay config API for custom acceleration curves.
- [ ] Define the accepted custom curve format, units, maximum number of points,
  monotonic-speed validation, non-finite value rejection, duplicate speed
  behavior, and factor bounds.
- [ ] Define whether custom acceleration curves are persisted and replayed on
  hotplug by the same device alias map as DPI, flat/adaptive profile, and speed.
- [ ] Define whether acceleration speed adjustment applies to custom profiles.
  Current custom profile evaluation ignores `speed_adjustment`; completion
  requires documenting that as intentional or composing speed adjustment with the
  custom curve in a specified order.
- [ ] Expose a way to clear runtime acceleration profile and speed overrides and
  return to metadata/default behavior.
- [ ] Ensure runtime profile and speed changes apply only to relative pointer
  devices and do not alter touchpads, touchscreens, tablets, or keyboards unless
  explicitly supported.

### Hotplug Behavior

- [x] Apply the built-in Razer Viper V3 Pro DPI fallback when the matching pointer
  is hotplugged after jay starts.
- [x] Apply pending backend-level DPI configuration when a matching pointer is
  hotplugged after the configuration request was received.
- [x] Match pending DPI configuration against human, sysfs, evdev backend, and raw
  kernel sysname aliases.
- [ ] Cover name changes where the sysfs name is generic at first but evdev later
  reports the real model name.
  - [x] Add a bench-local human-like Criterion workload where the add-device
    name is generic but the `DeviceInfo` name matches the pending DPI rule.
  - [ ] Add production jay-side coverage for the actual sysfs-name to evdev-name
    alias update path.
- [ ] Cover multiple evdev nodes with the same human-readable name.
  - [x] Add a bench-local human-like Criterion workload where two pointer nodes
    with the same `DeviceInfo` name both receive the pending DPI rule before
    representative motion is dispatched.
  - [ ] Add production jay-side coverage for duplicate human-readable names and
    stable per-node aliases.
- [ ] Cover removal and re-addition of the same device.
  - [x] Add a bench-local human-like Criterion workload where a matching pointer
    name appears again and the pending DPI rule is replayed again.
  - [ ] Add production removal/re-add coverage once input-rs exposes or models
    device removal in the test harness.
- [ ] Cover a non-pointer device with a matching name to prove DPI configuration
  does not attach to the wrong device class.
  - [x] Add a bench-local human-like Criterion workload where a keyboard-shaped
    device has the matching name and the pending DPI rule is ignored.
  - [ ] Add production jay-side coverage that the alias map keeps capability
    filtering for non-pointer devices.

### Motion Semantics

- [x] Apply DPI normalization before acceleration in the current `MouseFilter`.
- [x] Preserve unaccelerated deltas in the documented current unit:
  `MouseFilter` emits DPI-normalized unaccelerated deltas, not raw device counts.
  Completion still needs this surfaced in user-facing/device-inspection docs.
- [ ] Preserve subpixel accumulation separately for accelerated and unaccelerated
  streams.
- [ ] Avoid cursor drift from repeated fractional normalized deltas.
- [ ] Ensure changing DPI does not reset unrelated button, key, scroll, or gesture
  state.
- [ ] Ensure changing DPI cannot create a sudden synthetic motion event by itself.

### Diagnostics

- [ ] Log the selected DPI for each pointer device and the source of that value.
- [ ] Log invalid metadata with enough detail to identify the device and rejected
  string.
- [ ] Provide a device-inspection path that reports:
  - [ ] effective DPI.
  - [ ] DPI source.
  - [ ] raw metadata string, if available.
  - [ ] current acceleration profile.
  - [ ] current acceleration speed.
- [ ] Avoid noisy repeated warnings for the same invalid metadata on every event.

### Coverage

- [x] Add deterministic parser coverage matching the local libinput parser cases:
  selected values, unselected lists, sampling-rate suffixes, malformed strings,
  zero values, negative values, and empty strings.
- [x] Add deterministic coverage that direct `MouseFilter` construction with
  non-positive DPI falls back to 1000 DPI.
- [ ] Add deterministic coverage that `Context::set_mouse_dpi()` updates a live
  mouse filter without replacing unrelated filter state.
- [ ] Add deterministic coverage that acceleration profile changes preserve the
  effective DPI.
- [ ] Add deterministic coverage that acceleration speed changes preserve the
  effective DPI.
- [ ] Add deterministic coverage for hotplugged pending DPI configuration by each
  supported alias type.
- [ ] Add deterministic coverage that pending DPI does not apply to non-pointer
  devices.
- [x] Add a human-like Criterion workload for low-DPI pointer movement such as a
  slow 400 DPI mouse drag.
- [x] Add a human-like Criterion workload for high-DPI pointer movement such as a
  35000 DPI gaming mouse micro-aim sequence.
- [x] Add a human-like Criterion workload for adaptive acceleration over a slow
  to fast pointer speed ramp.
- [x] Add a human-like Criterion workload for a custom piecewise-linear
  acceleration profile over the same speed ramp.
- [x] Add a human-like Criterion workload for changing from flat to adaptive
  acceleration during representative pointer movement.
- [x] Add a human-like Criterion workload for changing from adaptive to a custom
  profile during representative pointer movement.
- [x] Add a human-like Criterion workload for changing acceleration speed during
  representative pointer movement.
- [x] Add a human-like Criterion workload for changing DPI while pointer motion is
  otherwise idle.
- [x] Add a Criterion workload for parsing realistic `MOUSE_DPI` property strings,
  including selected entries, lists, sampling rates, and invalid values.
- [x] Add a human-like Criterion workload for hotplug configuration replay: queue
  DPI, add a matching pointer, then process representative motion.
  - [x] Current Criterion coverage models the input-rs half of replay by applying
    the queued DPI through `Context::set_mouse_dpi()` before representative
    motion is dispatched.
  - [x] Current Criterion coverage also models generic add names, duplicate
    matching pointer names, re-add-shaped matching, and matching non-pointer
    names through bench-local input-rs context hooks.
  - [ ] Add jay-side integration or deterministic coverage for the production
    `InputRsBackend` alias map, including human name, sysfs name, evdev backend
    name, and raw kernel sysname matching.

### Documentation

- [x] Add this missing-feature document.
- [x] Update `FEATURE_PARITY.md` to stop marking runtime DPI as complete.
- [ ] Document the user-facing jay config name once it exists.
- [ ] Document accepted DPI formats and examples.
- [ ] Document precedence between runtime override, metadata, built-in
  device-specific fallback, and default 1000 DPI.
- [ ] Document accepted acceleration profile names and the custom-curve syntax
  once jay exposes custom profiles.
- [ ] Document the speed adjustment range, clamp behavior, and whether speed
  adjustment affects flat, adaptive, and custom profiles.
- [ ] Document that input-rs owns relative pointer normalization and acceleration;
  jay should forward configuration and must not apply a second acceleration or
  DPI transform.

## Local References

- `input-rs/src/processing/mouse.rs`
- `input-rs/src/processing/mod.rs`
- `input-rs/src/state.rs`
- `src/input/input_backend.rs`
- `src/input/input_rs_backend.rs`
- `src/backends/drm.rs`
- `src/backend.rs`
- `libinput/doc/user/device-configuration-via-udev.rst`
- `libinput/test/test-utils.c`
- `libinput/test/test-pointer.c`
- `libinput/test/litest-device-mouse-low-dpi.c`
