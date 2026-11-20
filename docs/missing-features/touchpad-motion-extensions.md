# Feature: Touchpad Motion Extensions

## Gap

Some touchpad behavior that users expect from older configurable stacks is not
part of libinput's public configuration surface: pressure-dependent pointer
motion, edge-motion continuation, active touchpad rectangles, and 180-degree
left-handed coordinate rotation.

These are not required for strict libinput parity. They are tracked because
`input-rs` is expected to own input behavior and expose explicit configuration
instead of relying on hidden device-specific policy. If any of these features are
implemented, jay should forward configuration into input-rs; jay should not apply
parallel touchpad motion transforms itself.

## Current Partial Implementation

- `TouchData` can carry raw pressure, touch-major, touch-minor, tool type, and
  tracking-id metadata.
- `TouchpadMotionFilter` uses touch coordinates and device axis resolution to
  emit pointer motion, tap, scroll, gesture, palm, and thumb behavior.
- `TouchpadMotionFilter` does not currently:
  - scale pointer motion based on pressure;
  - synthesize continued pointer motion after a finger reaches the pad edge;
  - ignore or remap a configurable active rectangle for touchpad pointer motion;
  - rotate touchpad coordinates 180 degrees for left-handed touchpad use.
- `feature_touchpad_motion_extensions` has human-like Criterion workloads for:
  - one-finger pointer motion with changing pressure and touch-size metadata;
  - a finger moving to the right edge, then remaining at the edge with small
    jitter where edge-motion continuation would synthesize continued motion;
  - a stroke that starts outside a plausible active rectangle, crosses through
    it, and exits it;
  - a diagonal stroke shaped like a 180-degree left-handed touchpad coordinate
    rotation candidate;
  - direct `DeviceRemoved` input while each of those four candidate streams has
    an active contact;
  - explicit `Context::set_device_enabled(false)` while each of those four
    candidate streams has an active contact;
  - context-level `Context::remove_device_by_index()` while each of those four
    candidate streams has an active contact.
- These workloads are current-behavior baselines only. They do not mean any of
  the extension features are implemented.

## Scope

This feature covers optional extensions to touchpad pointer-motion behavior:

- Pressure-dependent pointer speed.
- Edge-motion continuation.
- Touchpad active rectangle / ignored regions.
- 180-degree touchpad coordinate rotation for left-handed operation.
- Runtime input-rs configuration and jay forwarding for those behaviors.

This document does not cover:

- Palm/thumb classification thresholds, which are tracked in
  `palm-thumb-and-quirks.md`.
- Tablet active area, pressure range, or absolute-device calibration, which are
  tracked in `absolute-device-transforms.md` and `tablet-tool-and-pad.md`.
- Generic mouse DPI or pointer acceleration configuration, which is tracked in
  `pointer-dpi-and-motion-config.md`.

## Requirements To Mark Complete

### Ownership And Configuration

- [x] Decide which extensions are supported and which are explicitly not planned.
  Decision for this pass: all four touchpad motion extensions
  (pressure-dependent motion, edge-motion continuation, active rectangle, and
  180-degree touchpad coordinate rotation) are **not planned**. They are
  synaptics-era extensions rather than libinput parity. Relative-device rotation
  is implemented separately in `absolute-device-transforms.md`; generic
  left-handed mouse/clickpad policy is implemented in the tap/click path.
  `FEATURE_PARITY.md` records the decision.
- [ ] Every supported extension must have input-rs-owned configuration. jay may
  store and forward config, but input-rs must own the behavior.
- [ ] Every supported extension must have per-device configuration and a defined
  default.
- [ ] Runtime config must define whether changes apply immediately or wait until
  the touchpad reaches neutral state.
- [ ] Runtime config must reject invalid values with diagnostics instead of
  silently changing behavior.
- [ ] Device-profile overrides may provide defaults, but explicit user config
  must win over profile defaults.

### Pressure-Dependent Pointer Motion

- [ ] Define whether pressure affects pointer speed, acceleration input, or a
  separate post-acceleration multiplier.
  - [x] Add a human-like Criterion workload carrying one-finger pointer motion
    with changing pressure and touch-size metadata through the current filter.
- [ ] Define the pressure input units: raw device units, normalized `[0, 1]`, or
  profile-calibrated units.
- [ ] Define the transfer function from pressure to motion scale, including
  minimum pressure, maximum pressure, default curve, clamping behavior, and
  behavior when pressure is missing.
- [ ] Pressure-dependent motion must not affect scroll, tap, click, palm, or
  gesture decisions unless that interaction is explicitly documented.
- [ ] Pressure-dependent motion must compose predictably with pointer
  acceleration and touchpad resolution normalization.
- [ ] Add runtime input-rs configuration, jay forwarding, diagnostics, and
  deterministic coverage for low pressure, high pressure, missing pressure,
  malformed config, and live reconfiguration.

### Edge-Motion Continuation

- [ ] Define edge zones where continuation may start.
  - [x] Add a human-like Criterion workload where a finger moves to the right
    edge and then remains near the edge with small jitter.
- [ ] Define when continued motion starts: immediately at the edge, after a
  dwell timeout, after continued pressure, or after a configured gesture state.
- [ ] Define direction and speed of synthesized motion, including whether speed
  depends on distance into the edge zone, pressure, incoming velocity, or a fixed
  configured value.
- [ ] Define cancellation: leaving the edge, lifting the finger, adding another
  finger, palm/thumb classification, button press, gesture start, touch cancel,
  device disable, device removal, and seat reset must all stop continuation.
- [ ] Timer-backed synthesized motion must integrate with input-rs dispatch
  without busy loops or event bursts.
- [ ] Add runtime input-rs configuration, jay forwarding, diagnostics, and
  deterministic coverage for start, steady continuation, cancellation, and
  disabled-device cleanup.

### Active Rectangle / Ignored Regions

- [ ] Define an active rectangle coordinate space.
  - [x] Add a human-like Criterion workload carrying a stroke that starts
    outside a plausible active rectangle, crosses through it, and exits it.
- [ ] Define whether the active rectangle drops outside contacts, clamps outside
  motion, remaps the rectangle to the full touchpad area, or only suppresses
  pointer motion outside the rectangle.
- [ ] Reject invalid rectangles: empty, inverted, non-finite, outside device
  bounds unless documented clamping is intended, and rectangles too small to be
  usable.
- [ ] Define interaction with palm/thumb exclusion zones, clickpad button areas,
  edge scrolling, circular scrolling, tap recognition, gestures, and DWT/DWTP.
- [ ] Define whether contacts that start outside and move inside can become
  active, and whether contacts that start inside and move outside are cancelled,
  clamped, or continue.
- [ ] Add runtime input-rs configuration, jay forwarding, diagnostics, and
  deterministic coverage for inside-only, outside-only, crossing-in, crossing-out,
  invalid config, and live reconfiguration.

### Left-Handed Touchpad Coordinate Rotation

- [ ] Define whether left-handed touchpad rotation is a separate setting or part
  of generic left-handed input configuration.
  - [x] Add a human-like Criterion workload carrying a diagonal stroke shaped
    like a 180-degree coordinate rotation candidate.
- [ ] Define the transform exactly. A 180-degree rotation should map normalized
  `x` to `1 - x` and normalized `y` to `1 - y` before pointer-motion deltas are
  derived, or document a different behavior.
- [ ] Define whether rotation affects pointer motion only, or also tap geometry,
  clickpad button areas, edge scrolling, circular scrolling, gestures,
  palm/thumb zones, and active rectangles.
- [ ] Define the composition order with active rectangle, palm/thumb zones,
  clickpad areas, edge/circular scrolling, and pointer acceleration.
- [ ] Runtime changes while a touch is active must not produce impossible motion
  deltas or stuck button state. Either defer until neutral state or cancel/remap
  active contacts with deterministic rules.
- [ ] Add runtime input-rs configuration, jay forwarding, diagnostics, and
  deterministic coverage for neutral changes, active changes, clickpad-area
  interaction, edge-scroll interaction, and gesture interaction.

### Coverage

- [x] Add human-like Criterion workloads for pressure-dependent motion,
  edge-motion continuation, active rectangle, and left-handed rotation candidate
  input shapes.
- [ ] Add deterministic assertions for every extension that is implemented.
- [ ] Add device-disable and device-removal coverage for every extension that
  owns state or synthesizes events.
  - [x] Add human-like current-behavior Criterion workloads for direct
    `DeviceRemoved` input while pressure-dependent motion, edge-motion
    continuation, active-rectangle, and left-handed-rotation candidate streams
    have active contacts.
  - [x] Add human-like current-behavior Criterion workloads for explicit
    `Context::set_device_enabled(false)` while pressure-dependent motion,
    edge-motion continuation, active-rectangle, and left-handed-rotation
    candidate streams have active contacts.
  - [x] Add human-like current-behavior Criterion workloads for context-level
    `Context::remove_device_by_index()` while pressure-dependent motion,
    edge-motion continuation, active-rectangle, and left-handed-rotation
    candidate streams have active contacts.
  - [ ] Add deterministic assertions for final cleanup semantics once any of
    these extensions owns state, timers, or synthesized output.
- [ ] Add jay-side deterministic coverage proving user-facing config reaches the
  input-rs runtime setters once those setters exist.

### Documentation

- [ ] Update `FEATURE_PARITY.md` when each extension is implemented or
  explicitly marked not planned.
- [ ] Document user-facing jay config names, allowed values, defaults, and
  diagnostics for every supported extension.
- [ ] Document deliberate differences from libinput and synaptics behavior.

## Local References

- `input-rs/src/processing/touchpad_motion.rs`
- `input-rs/src/processing/config.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/state.rs`
- `src/input/input_rs_backend.rs`
