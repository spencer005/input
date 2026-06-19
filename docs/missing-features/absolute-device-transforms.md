# Feature: Absolute Device Transforms

## Gap

Libinput supports calibration matrices for absolute devices, rotation for
relative devices, tablet pressure range configuration, and active area
configuration for tablets.

`input-rs` now has an initial `AbsoluteTransformFilter` that can apply a
validated affine calibration matrix to `TouchDown` and `TouchMotion` events
when the device has usable absolute axis bounds. jay calibration configuration is
wired into input-rs runtime device configuration and installs/replaces that
filter for the target device. Tablet-specific active area, tablet pressure range
configuration, calibration reset/query, and active-contact reconfiguration policy
are still missing.

## Current Partial Implementation

- `CalibrationMatrix` stores six affine values in libinput-style order:
  `[a, b, c, d, e, f]`.
- `CalibrationMatrix::new()` rejects non-finite matrix values.
- `AbsoluteTransformFilter` normalizes touch coordinates through `DeviceInfo`
  axis min/max, applies the affine matrix, clamps the transformed normalized
  coordinates to `[0, 1]`, and converts back to device units.
- The filter transforms `TouchDown` and `TouchMotion`; `TouchUp`, `Frame`, and
  non-touch events pass through unchanged.
- `FilterChain::set_calibration_matrix()` installs or replaces one
  `AbsoluteTransformFilter` at the front of the per-device filter chain.
- `Context::set_calibration_matrix()` updates an active input-rs device.
- jay `SetCalibrationMatrix` configuration now flows through
  `InputDeviceConfig::SetCalibrationMatrix`, `InputBackend::set_calibration_matrix()`,
  and `InputRsBackend::set_calibration_matrix()` into `Context`.
- `feature_absolute_device_transforms` has human-like Criterion workloads for:
  - identity calibration through the current transform filter;
  - X-axis inversion through the current transform filter;
  - Y-axis inversion through the current transform filter;
  - 90-degree clockwise and counter-clockwise rotation matrices through the
    current transform filter;
  - translation plus scale through the current transform filter;
  - transformed coordinates that exceed normalized device bounds and are clamped
    by the current transform filter;
  - invalid non-finite calibration matrix config values rejected by
    `CalibrationMatrix::new()`;
  - calibration replacement before a neutral touch sequence;
  - calibration replacement while a touch sequence is active;
  - an indirect-tablet stroke that starts outside, crosses through, and exits a
    plausible active area, carried through the current event model;
  - relative-pointer motion shaped like a user moving on a rotated display,
    carried through the current event model.
- Non-finite matrices are rejected before they reach the filter.

## What The Calibration Matrix Does

The calibration matrix is for absolute-position devices such as touchscreens and
tablet touch surfaces. It does not change mouse DPI, pointer acceleration,
touchpad gestures, or relative pointer motion.

The filter takes each absolute touch coordinate and:

1. Converts raw evdev device units into normalized device coordinates:
   `x = (raw_x - min_x) / (max_x - min_x)` and
   `y = (raw_y - min_y) / (max_y - min_y)`.
2. Applies the six configured matrix values:
   `x' = a*x + b*y + c` and `y' = d*x + e*y + f`.
3. Clamps `x'` and `y'` to `[0, 1]`.
4. Converts the transformed normalized coordinates back into device units before
   jay sees the touch event.

Common matrices:

- `[1, 0, 0, 0, 1, 0]` keeps coordinates unchanged.
- `[-1, 0, 1, 0, 1, 0]` flips the X axis.
- `[1, 0, 0, 0, -1, 1]` flips the Y axis.
- `[0, 1, 0, -1, 0, 1]` rotates coordinates 90 degrees clockwise.
- `[0, -1, 1, 1, 0, 0]` rotates coordinates 90 degrees counter-clockwise.

This is useful when a touchscreen or tablet reports coordinates in a different
orientation than the panel or output it is attached to. In this architecture,
input-rs owns the input coordinate transform. jay stores and forwards the
configuration, but should not apply the same calibration a second time.

## Do We Want This?

For ordinary mice, trackballs, trackpoints, and relative touchpad pointer motion:
no. A calibration matrix is the wrong tool. DPI, acceleration, scroll direction,
button mapping, and touchpad gesture settings cover those devices.

For direct absolute devices such as touchscreens: usually yes, if jay needs to
correct device coordinates before they are delivered to clients. The concrete
problem is "the physical point I touched is not the logical point clients see."
Examples are a rotated panel, an inverted axis, a touchscreen attached to a
different output orientation than the kernel reports, or a device whose active
coordinate range needs a fixed offset/scale.

For tablets: maybe, but it should not be treated as complete until tablet tool
coordinates, tablet touch coordinates, output mapping, active area, and pressure
range all have a single documented composition order. A tablet active area may be
the better user-facing control than exposing a raw six-value matrix for many
tablet cases.

Completion requires an explicit policy decision, not just the existence of the
filter:

- If input-rs supports calibration matrices, the matrix is per absolute input
  device and is applied once in input-rs before jay consumes the event.
- If jay/output mapping owns some later transform, the input-rs calibration must
  compose with that mapping in a documented order.
- If we decide not to expose raw matrices to users, the filter can still remain as
  an internal primitive used by clearer configuration such as rotate, flip,
  active area, or "map touchscreen to output."

## Scope

This feature covers coordinate and value transforms for absolute input devices:

- Touchscreen calibration matrix.
- Tablet absolute coordinate calibration.
- Tablet active area.
- Relative-device rotation where applicable.
- Coordinate normalization and output mapping.
- Composition with tablet pressure range and touch event delivery.

Tablet tool event support itself is tracked in `tablet-tool-and-pad.md`.

## Requirements To Mark Complete

### Coordinate Model

- [ ] Define the coordinate spaces used by `input-rs`:
  - [x] raw evdev units.
  - [x] device-normalized coordinates for the current calibration filter.
  - [x] transformed device coordinates after clamped normalized calibration.
  - [ ] compositor/output coordinates if handled in this layer.
- [ ] Preserve raw coordinates where callers need them for debugging or custom
  transforms.
- [x] Document whether transforms are applied in `input-rs`, jay, or both for
  the current filter: jay stores and forwards configuration, but input-rs owns
  the current coordinate transform once the filter is installed.
- [ ] Avoid applying the same transform in both `input-rs` and jay. This still
  needs runtime ownership plumbing before it can be proven.

### Calibration Matrix

- [x] Add per-device calibration matrix storage. The current matrix is stored by
  the per-device `AbsoluteTransformFilter` in the filter chain.
- [ ] Add runtime `Context` methods to set, reset, and query calibration. Current
  code can set/replace calibration; reset and query are still missing.
- [x] Apply calibration to touchscreen coordinates before emitting compositor-
  visible touch events when this layer owns calibration.
- [ ] Apply calibration to tablet absolute x/y where appropriate.
- [x] Validate matrix values and reject non-finite values.
- [x] Define behavior for coordinates transformed outside the device bounds:
  clamp, drop, or pass through with documented semantics. The current filter
  clamps transformed normalized coordinates to `[0, 1]`.
- [ ] Calibration changes during active touch contacts must not produce broken
  touch sequences. Either defer until neutral state or define remapping rules.

### Active Area

- [ ] Add active area configuration for indirect tablets.
  - [x] Add a human-like Criterion workload carrying a tablet stroke that starts
    outside a plausible active rectangle, enters it, continues through it, and
    exits it through the current event model.
  - [ ] Add an actual active-area config object with a documented coordinate
    space, per-device storage, validation, runtime set/reset/query operations,
    and a filter or mapper that applies it to tablet coordinates.
- [ ] Define the active area in device units or normalized units.
- [ ] Reject invalid rectangles: empty, inverted, non-finite, or outside allowed
  range unless clamping is deliberate.
- [ ] Map coordinates inside the active area to the full logical tablet area or
  chosen output mapping.
- [ ] Drop or clamp coordinates outside the active area according to documented
  policy.
- [ ] Active area must compose predictably with calibration and output mapping.

### Relative Rotation

- [ ] Add relative-device rotation configuration if parity with libinput is
  desired.
  - [x] Add a human-like Criterion workload carrying pointer deltas shaped like a
    user moving a relative device while the logical display is rotated.
  - [ ] Add actual rotation configuration with accepted angles, validation,
    reset/query behavior, runtime update behavior, and per-device storage.
- [ ] Rotate pointer motion deltas by the configured angle.
- [ ] Define wheel behavior for rotations near 180 degrees.
- [ ] Ensure unaccelerated deltas are either rotated or preserved according to
  documented raw-input semantics.
- [ ] Rotation must compose with acceleration in a defined order.

### Pressure Range

- [ ] Add pressure range configuration for tablet tools or link explicitly to
  `tablet-tool-and-pad.md` if implemented there.
- [ ] Clamp pressure below the configured minimum to logical zero.
- [ ] Clamp pressure above the configured maximum to logical one.
- [ ] Rescale pressure between the configured min and max.
- [ ] Define interaction with automatic pressure offset handling.

### Jay Bridge

- [ ] Ensure transformed coordinates are what jay expects for touchscreen and
  tablet input.
- [ ] Preserve enough metadata for jay to map absolute devices to outputs.
- [x] Expose configuration through jay if users or desktop components need to
  set calibration.

### Coverage

- [ ] Cover identity calibration.
  - [x] Add a human-like Criterion workload for identity calibration through the
    current transform filter.
  - [ ] Add deterministic assertions that identity preserves touch down/motion
    coordinates exactly for representative axis ranges.
- [x] Add human-like Criterion workloads that exercise X-axis inversion, Y-axis
  inversion, and 90-degree clockwise/counter-clockwise rotation matrices through
  the current transform filter.
- [ ] Cover translation, scale, and axis inversion in deterministic tests or
  replay fixtures.
  - [x] Add a human-like Criterion workload for translation plus scale through the
    current transform filter.
  - [x] Add human-like Criterion workloads for Y inversion and 90-degree
    clockwise/counter-clockwise rotation matrices through the current transform
    filter.
  - [ ] Add deterministic assertions for translation, scale, X inversion, Y
    inversion, and 90-degree rotation matrices.
- [ ] Cover invalid matrix rejection.
  - [x] Add a Criterion workload with user-config-shaped calibration matrices
    containing `NaN`, positive infinity, and negative infinity values, plus one
    valid matrix for comparison, through `CalibrationMatrix::new()`.
  - [ ] Add deterministic assertions that non-finite matrices are rejected and
    do not alter an installed filter.
- [ ] Cover coordinate clamping or dropping policy.
  - [x] Add a human-like Criterion workload for coordinates that clamp after the
    current normalized transform.
  - [ ] Add deterministic assertions for min-bound and max-bound clamping, or
    replace the current clamping behavior with a documented drop/pass-through
    policy and cover that policy.
- [ ] Cover calibration changes while active and neutral.
  - [x] Add human-like Criterion workloads for calibration replacement before a
    neutral sequence and during an active touch sequence.
  - [ ] Add deterministic assertions defining whether active-contact changes are
    immediate, deferred to neutral, or explicitly cancelled/remapped.
- [ ] Cover active area mapping.
  - [x] Add a human-like Criterion workload carrying an indirect-tablet stroke
    that crosses a plausible active area through the current event model.
  - [ ] Add deterministic assertions for inside-area mapping, outside-area
    clamp/drop behavior, invalid rectangle rejection, and interaction with
    output mapping.
- [ ] Cover relative rotation at 90, 180, and arbitrary angles if supported.
  - [x] Add a human-like Criterion workload carrying relative motion that would
    need rotation to align with a rotated display.
  - [ ] Add deterministic assertions for configured 90-degree, 180-degree, and
    arbitrary-angle rotation, including both accelerated and unaccelerated delta
    semantics.
- [ ] Cover pressure range remapping.
  - [x] Tablet pressure-range-shaped event-model coverage exists in
    `feature_tablet_tool_and_pad/pressure_range_candidate_current_events`.
  - [ ] Add actual pressure range configuration and deterministic min/max clamp
    plus rescale assertions in the layer that owns tablet pressure policy.

### Documentation

- [x] Update `FEATURE_PARITY.md`.
- [x] Document current ownership of calibration between `input-rs` and jay:
  jay forwards configuration and input-rs applies the transform in the filter
  chain.
- [x] Document current matrix format and units.
- [ ] Document active area format and units.

## Local References

- `input-rs/src/backend/evdev.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/state.rs`
- `src/input/input_rs_backend.rs`
- `libinput/doc/user/configuration.rst`
- `libinput/doc/user/absolute-axes.rst`
- `libinput/doc/user/tablet-support.rst`
- `libinput/src/libinput.h`
