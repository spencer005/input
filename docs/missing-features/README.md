# input-rs Missing Feature Completion Docs

This directory tracks libinput-facing gaps that are not complete in
`input-rs`. Each document describes one feature area and the requirements that
must be met before that area can be marked complete in `FEATURE_PARITY.md`.

These documents are intentionally stricter than a loose TODO list. A feature is
not complete just because an enum variant exists or one happy path works. It is
complete only when the backend, event model, filters, runtime configuration,
jay bridge, and coverage all agree on the behavior.

## Completion Rule

A feature can be marked complete only when all applicable requirements in its
document are satisfied, or when a requirement is explicitly moved to another
linked feature document with a reason.

For each feature, completion must include:

- A public data model for all events and configuration values the compositor
  needs.
- Backend detection and event production from evdev data, not only manually
  constructed test events.
- Filter or policy state machines that handle neutral states, cancellation,
  multi-device interactions, and delayed config changes where relevant.
- Runtime configuration methods in `Context` and propagation through the jay
  input backend when the feature is user-visible.
- Device capability reporting that does not advertise unsupported behavior.
- Deterministic coverage through unit tests, synthetic event fixtures, replay
  tests, or another repeatable harness appropriate for the feature.
- Documentation updates in `FEATURE_PARITY.md` and any user-facing jay config
  docs when the feature exposes user configuration.

## Feature Documents

- [Touchpad tap and click](touchpad-tap-and-click.md)
- [Touchpad scroll methods](touchpad-scroll-methods.md)
- [Touchpad motion extensions](touchpad-motion-extensions.md)
- [Touchpad gestures](touchpad-gestures.md)
- [Middle-button emulation](middle-button-emulation.md)
- [Button debounce configuration](button-debounce.md)
- [Palm, thumb, and device policy overrides](palm-thumb-and-quirks.md)
- [Tablet tool and pad](tablet-tool-and-pad.md)
- [Switch devices](switch-devices.md)
- [Absolute device transforms](absolute-device-transforms.md)
- [Touch event completeness](touch-event-completeness.md)
- [Device policy and metadata](device-policy-and-metadata.md)
- [Pointer DPI and motion configuration](pointer-dpi-and-motion-config.md)

## Benchmark Coverage

Human-like Criterion workloads for these feature areas live in
`input-rs/benches/feature_workload_bench.rs`. The benchmark target is
registered as `feature_workload_bench` in `input-rs/Cargo.toml`.

The current benchmark groups are:

- `feature_touchpad_tap_and_click`
- `feature_touchpad_scroll_methods`
- `feature_touchpad_motion_extensions`
- `feature_touchpad_gestures`
- `feature_button_debounce`
- `feature_middle_button_emulation`
- `feature_palm_thumb_and_device_policy`
- `feature_touch_event_completeness`
- `feature_tablet_tool_and_pad`
- `feature_switch_devices`
- `feature_absolute_device_transforms`
- `feature_pointer_dpi_and_motion_config`
- `feature_device_policy_and_metadata`

Some groups run real current filters or event paths, such as
`TouchpadMotionFilter` for tap, scroll, DWT, palm, optional motion-extension
input shapes, and the initial gesture recognizer; `ButtonDebounceFilter`;
`MiddleButtonEmulationFilter`; `AbsoluteTransformFilter`; `FilterChain`; and the
current `ButtonScrollFilter` and `SwitchToggle` event model, plus current tablet
tool/pad event variants, including active removal-shaped tablet tool, pad, and
tablet-touch node streams, and `DevicePolicyState`.
The tap-and-click, scroll, motion-extension, gesture, middle-button, and
button-debounce groups include context-level removal workloads for active or
pending recognizer/filter state in addition to direct filter-level
`DeviceRemoved` workloads.
The scroll, gesture, and absolute-transform groups also include current runtime
reconfiguration workloads so config changes can be measured separately from
steady-state happy paths.
The pointer DPI and motion group exercises the current mouse filter and the
current `Context::set_mouse_dpi()` path with human-like low-DPI and high-DPI
pointer motion, parser coverage for realistic `MOUSE_DPI` property strings,
bench-local hotplug replay shapes, adaptive/custom acceleration speed ramps,
runtime acceleration-profile changes, and runtime speed-adjustment changes.
The device policy and metadata group includes standalone policy workloads plus
`DeviceInfo` capability-inventory shapes for keyboard, mouse, trackpoint,
touchpad/clickpad, touchscreen, switch, tablet tool, and tablet pad, plus
context-level manual disable, monitor-shaped removal cleanup, and
backend-originated removal cleanup.

### Benchmark Integrity Requirements

Human-like workloads are necessary, but fixed traces are not enough by
themselves. A benchmark group must not be treated as hard to game merely because
it contains realistic-looking input. Completion for benchmark coverage requires
both representative happy-path workloads and nearby variants that make exact
trace overfitting harder.

At minimum, each feature benchmark group should include at least one
anti-overfit variant appropriate for the feature:

- touch workloads should vary slot numbers, timestamps, coordinates, and
  metadata without changing the intended gesture or contact state;
- pointer workloads should vary timestamps and deltas while preserving the
  intended direction and speed class;
- tablet workloads should vary timestamps, axes, pressure, and pad values while
  preserving the tool/pad lifecycle;
- switch workloads should vary timing without changing the state sequence;
- metadata/policy workloads should vary timestamps and non-semantic metadata
  values without changing the policy scenario.

High-risk paths that are especially easy to specialize accidentally, such as
touch recognition, motion normalization, absolute transforms, and policy
composition, should also include a small multi-seed variant suite so one
Criterion iteration covers several nearby event shapes.

The current anti-overfit variant registrations are:

- `single_finger_tap_slot_timing_variant_current_filter`
- `two_finger_tap_slot_timing_variant_current_filter`
- `two_finger_tap_variant_suite_current_filter`
- `two_finger_scroll_slot_timing_variant_current_filter`
- `two_finger_scroll_variant_suite_current_filter`
- `button_scroll_timing_delta_variant_current_chain`
- `button_scroll_variant_suite_current_chain`
- `pressure_dependent_motion_slot_timing_metadata_variant_current_filter`
- `pressure_dependent_motion_variant_suite_current_filter`
- `three_finger_swipe_slot_timing_variant_current_recognizer`
- `three_finger_swipe_variant_suite_current_recognizer`
- `pinch_zoom_out_slot_timing_variant_current_recognizer`
- `pinch_zoom_out_variant_suite_current_recognizer`
- `bouncy_button_timing_variant_current_filter`
- `bouncy_button_variant_suite_current_filter`
- `left_right_chord_timing_variant_current_filter`
- `left_right_chord_variant_suite_current_filter`
- `firmware_palm_slot_timing_metadata_variant_current_filter`
- `firmware_palm_variant_suite_current_filter`
- `mt_b_slot_timing_metadata_variant_current_events`
- `mt_b_variant_suite_current_events`
- `stylus_stroke_timing_axis_variant_current_events`
- `stylus_stroke_variant_suite_current_events`
- `rapid_tablet_mode_timing_variant_current_events`
- `rapid_tablet_mode_variant_suite_current_events`
- `identity_calibration_slot_timing_variant_current_filter`
- `identity_calibration_variant_suite_current_filter`
- `high_dpi_micro_aim_timing_delta_variant_current_filter`
- `high_dpi_micro_aim_variant_suite_current_filter`
- `adaptive_profile_speed_ramp_timing_delta_variant_current_filter`
- `adaptive_profile_speed_ramp_variant_suite_current_filter`
- `metadata_policy_led_timing_metadata_variant_current_state`
- `metadata_policy_led_variant_suite_current_state`

These variants are still performance workloads, not correctness proofs. Any
feature that has observable behavior requirements still needs deterministic
assertions or replay fixtures that verify emitted events, cleanup state, and
runtime configuration effects. Criterion output alone must not be used to mark a
feature complete.

The current benchmark target should prefer real current code paths. If a future
feature area temporarily needs a raw fixture because no event model exists yet,
that fixture is only a placeholder; it is not a substitute for a real
implementation benchmark.

When a missing feature is implemented, update its benchmark group so it measures
the real backend, event model, filter, or policy path.

## Current Local References

The current implementation is centered around these files:

- `input-rs/src/backend/mod.rs`
- `input-rs/src/backend/evdev.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/processing/config.rs`
- `input-rs/src/processing/touchpad_motion.rs`
- `input-rs/src/processing/touchpad.rs`
- `input-rs/src/state.rs`
- `src/input/input_rs_backend.rs`

The libinput references used for these gaps are primarily:

- `libinput/doc/user/configuration.rst`
- `libinput/doc/user/tapping.rst`
- `libinput/doc/user/scrolling.rst`
- `libinput/doc/user/gestures.rst`
- `libinput/doc/user/tablet-support.rst`
- `libinput/doc/user/switches.rst`
- `libinput/doc/user/palm-detection.rst`
- `libinput/doc/user/touchpad-thumb-detection.rst`
- `libinput/src/libinput.h`
