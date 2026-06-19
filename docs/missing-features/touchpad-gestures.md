# Feature: Touchpad Gestures

## Gap

Libinput supports swipe, pinch, and hold gestures on touchpads and other
indirect touch devices. It emits begin, update, and end/cancel events with
finger counts and gesture-specific data such as motion, scale, and rotation.

`input-rs` now defines swipe, pinch, and hold event variants and has an initial
touchpad recognizer for common human-like swipe, pinch, and stationary hold
motions. The implementation is still not full libinput parity: it lacks
deterministic coverage, complete device-removal path coverage, complete
finger-count-change semantics, and the more subtle disambiguation rules needed
for difficult hardware.

## Current Partial Implementation

- `InputEvent` has boxed cold-path variants for:
  - `GestureSwipeBegin`, `GestureSwipeUpdate`, and `GestureSwipeEnd`.
  - `GesturePinchBegin`, `GesturePinchUpdate`, and `GesturePinchEnd`.
  - `GestureHoldBegin` and `GestureHoldEnd`.
- `TouchpadMotionFilter` recognizes:
  - three-or-more-finger swipe when the active contact center moves by at least
    `GESTURE_MOTION_THRESHOLD_MM`;
  - two-finger pinch when relative spacing changes by at least
    `PINCH_SCALE_THRESHOLD` or relative angle changes by at least
    `PINCH_ROTATION_THRESHOLD_RAD`;
  - one-or-more-finger stationary hold after `HOLD_TIMEOUT_US`.
- Active gestures suppress pointer motion and two-finger scroll output for the
  same physical movement.
- Lifting a participating finger ends the current gesture. Adding a new contact
  during an active gesture cancels it.
- `TouchCancel` for a participating contact cancels the active gesture stream
  and clears the affected touchpad slot.
- Explicit input-rs device disable calls `FilterChain::reset_runtime_state()`,
  which clears `TouchpadMotionFilter` gesture, hold, scroll, tap, palm, and slot
  state.
- The jay input-rs bridge maps swipe, pinch, and hold events into jay's internal
  `InputEvent` variants.
- `feature_touchpad_gestures` has human-like Criterion workloads for:
  - three-finger and four-finger swipe sequences;
  - pinch scale-out and scale-in sequences;
  - pinch rotation in both directions;
  - stationary hold, hold cancelled by later movement, and hold cancelled by
    touch cancel;
  - finger-count change cancellation during an active swipe and during an active
    pinch;
  - competition cases where two-finger scroll, tap, and one-finger pointer motion
    should not also become high-level gestures;
  - explicit runtime filter reset while a gesture is active, as the current
    input-rs cleanup path used for device disable;
  - explicit `Context::set_device_enabled(false)` while swipe-shaped,
    pinch-shaped, and hold-shaped gesture streams are active, using the current
    context reset path;
  - direct `DeviceRemoved` input while swipe-shaped, pinch-shaped, and
    hold-shaped gesture streams are active through the current touchpad filter.

## Scope

This feature covers high-level gesture recognition from touchpad contacts:

- Swipe begin/update/end.
- Pinch begin/update/end.
- Hold begin/end.
- Gesture cancellation.
- Finger-count tracking.
- Gesture data sent through the jay bridge.

It does not cover touchscreen gestures. Libinput does not provide touchscreen
gesture recognition; toolkits usually own that layer.

## Requirements To Mark Complete

### Event Model

- [x] Define hold begin and hold end/cancel event variants.
- [ ] Ensure swipe, pinch, and hold events carry the device index, time, finger
  count, and cancellation state where applicable. Current hold begin/end carry
  finger count, but swipe/pinch end events still use `GestureEndData` without the
  final finger count.
- [ ] Pinch update events must include:
  - [x] center motion delta.
  - [x] scale relative to gesture start.
  - [x] accumulated rotation relative to gesture start.
- [x] Swipe update events must include center motion delta.
- [x] Event fields must be sufficient for the jay bridge to map into existing
  compositor input events without guessing.
- [x] The event enum must remain conscious of hot-path size. If gesture events
  are boxed, that choice should be documented.

### Gesture Recognition

- [ ] Gesture recognition must run on indirect touch devices that are classified
  as touchpads or tablet-touch pointer surfaces, not on direct touchscreens.
- [ ] A gesture must begin only after movement or finger placement is
  unambiguous enough to distinguish it from pointer motion, scrolling, tapping,
  and holding.
- [x] A gesture must end when the first finger belonging to the gesture is
  lifted, or when the gesture is cancelled by a conflicting interaction.
- [x] Adding a finger mid-gesture cancels the current gesture. Removing a finger
  ends the current gesture.
- [ ] Gesture state must be cleared on touch cancel, device disable, and device
  removal.
  - [x] Clear gesture state on `TouchCancel`.
  - [x] Clear gesture state on explicit input-rs device disable.
  - [ ] Cover every device-removal path. A human-like Criterion workload now
    carries direct `DeviceRemoved` input while swipe, pinch, and hold streams are
    active. Human-like context workloads now remove the device by `DevIndex`
    while swipe, pinch, and hold streams are active, matching the current
    monitor-driven cleanup path. Evdev removal with active touches emits
    `TouchCancel`, but deterministic coverage for backend-originated and
    monitor-driven removal is not complete.
- [x] Gesture recognition must not emit scroll and gesture updates for the same
  physical movement unless that is an explicit compatibility decision.

### Swipe

- [x] Recognize three-or-more-finger synchronous movement as swipe.
- [x] Use the logical center of the active fingers for swipe deltas.
- [x] Preserve arbitrary x/y direction; do not reduce to only cardinal
  directions.
- [x] Emit begin once, update for movement frames, and end once for the current
  recognized happy path.
- [ ] Emit cancelled end when the gesture is interrupted by finger-count change,
  palm cancellation, touch cancel, or device disable.
  - [x] Emit cancelled end for added fingers during an active gesture.
  - [x] Emit cancelled end for `TouchCancel`.
  - [x] Add a human-like Criterion workload for `TouchCancel` after a hold has
    begun.
  - [ ] Decide whether explicit device disable should synthesize a cancelled
    gesture end before the filter reset, or whether cleanup-only reset is the
    intended policy.

### Pinch

- [ ] Recognize two-or-more-finger changes in relative distance or angle as
  pinch. Current code recognizes two-finger pinch only.
- [x] Track gesture center.
- [x] Track scale as a multiplier relative to initial finger spacing.
- [x] Track rotation from relative finger angle changes.
- [x] Handle uneven finger movement by updating both center motion and scale or
  rotation as appropriate.
- [x] Avoid false pinch detection for normal two-finger scroll in the current
  recognizer by requiring spacing or angle change. The full
  disambiguation rule must be documented and covered.

### Hold

- [x] Recognize one-or-more-finger stationary contact after a defined threshold.
- [x] Cancel hold when movement transitions to pointer motion, scrolling, swipe,
  or pinch.
- [x] End hold when a participating finger is lifted.
- [x] Include finger count in begin/end events.
- [ ] Define whether a one-finger hold is enabled by default or only higher
  finger counts are reported.

### Jay Bridge

- [x] Map all gesture event variants into jay's internal input event model.
- [x] Preserve scale, rotation, deltas, finger count, timestamp, and cancelled
  state.
- [ ] Ensure clients receive exactly one coherent begin/update/end stream for
  each gesture.
- [x] If jay lacks a matching event for hold, add one or document why hold is
  intentionally not forwarded. Jay already had `HoldBegin` and `HoldEnd`; the
  input-rs bridge now forwards to them.

### Coverage

- [ ] Cover swipe begin/update/end for three and four fingers.
  - [x] Add human-like Criterion workloads for three-finger and four-finger
    swipes through the current recognizer.
  - [ ] Add deterministic assertions that exactly one begin, at least one update,
    and exactly one non-cancelled end are emitted for both finger counts.
- [ ] Cover pinch scale in and scale out.
  - [x] Add human-like Criterion workloads for pinch zoom-out and pinch zoom-in
    through the current recognizer.
  - [ ] Add deterministic assertions for scale direction, accumulated scale, and
    end/cancel behavior.
- [ ] Cover pinch rotation both directions.
  - [x] Add human-like Criterion workloads for clockwise and counter-clockwise
    two-finger rotation through the current recognizer.
  - [ ] Add deterministic assertions for rotation sign, accumulated angle, and
    interaction with simultaneous center movement.
- [ ] Cover hold begin/end and hold cancellation.
  - [x] Add human-like Criterion workloads for a stationary hold and a hold
    cancelled by later movement.
  - [x] Add a human-like Criterion workload for a hold cancelled by `TouchCancel`
    after the hold has begun.
  - [ ] Add deterministic assertions for hold timeout, one begin, one end, and the
    cancelled flag for motion/touch-cancel/device-reset cases.
- [ ] Cover finger-count changes during active gestures.
  - [x] Add a human-like Criterion workload where an active swipe receives an
    additional finger and is cancelled by the current recognizer.
  - [x] Add a human-like Criterion workload where an active pinch receives an
    additional finger and is cancelled by the current recognizer.
  - [ ] Add deterministic assertions for adding and removing fingers during swipe,
    pinch, and hold streams.
- [ ] Cover competition with two-finger scroll, tap, and pointer motion.
  - [x] Add human-like Criterion workloads that send two-finger scroll, tap, and
    one-finger pointer-motion sequences through the gesture-capable touchpad
    filter.
  - [ ] Add deterministic assertions that these competition cases emit the
    intended scroll, tap/click, or pointer-motion events and no gesture stream.
- [ ] Cover device removal or disable during an active gesture.
  - [x] Add a human-like Criterion workload that resets the current touchpad
    filter while a swipe is active, matching the current explicit device-disable
    cleanup path.
  - [x] Add a human-like Criterion workload that sends an active swipe-shaped
    touchpad stream through `Context`, then disables the device with
    `Context::set_device_enabled(false)`.
  - [x] Add a human-like Criterion workload that sends an active pinch-shaped
    touchpad stream through `Context`, then disables the device with
    `Context::set_device_enabled(false)`.
  - [x] Add a human-like Criterion workload that sends an active hold-shaped
    touchpad stream through `Context`, then disables the device with
    `Context::set_device_enabled(false)`.
  - [x] Add a human-like Criterion workload that sends an active swipe-shaped
    touchpad stream through `Context`, then removes the device with
    `Context::remove_device_by_index()`.
  - [x] Add a human-like Criterion workload that sends an active pinch-shaped
    touchpad stream through `Context`, then removes the device with
    `Context::remove_device_by_index()`.
  - [x] Add a human-like Criterion workload that sends an active hold-shaped
    touchpad stream through `Context`, then removes the device with
    `Context::remove_device_by_index()`.
  - [x] Add a human-like Criterion workload that sends direct `DeviceRemoved`
    input to the current touchpad filter while a swipe-shaped gesture is active.
  - [x] Add a human-like Criterion workload that sends direct `DeviceRemoved`
    input to the current touchpad filter while a pinch-shaped gesture is active.
  - [x] Add a human-like Criterion workload that sends direct `DeviceRemoved`
    input to the current touchpad filter while a hold-shaped gesture is active.
  - [ ] Add deterministic coverage for evdev removal, monitor-driven removal,
    explicit disable, and the final policy decision about synthesizing cancelled
    gesture-end events before reset. Monitor-driven removal now reaches
    input-rs context cleanup, but the gesture-specific cancellation policy still
    needs assertions.

### Documentation

- [x] Update `FEATURE_PARITY.md` so swipe and pinch are not marked complete
  until generation and bridge forwarding both work.
- [ ] Document gesture defaults and any unsupported gesture classes.

## Local References

- `input-rs/src/events/mod.rs`
- `input-rs/src/processing/touchpad_motion.rs`
- `src/input/input_rs_backend.rs`
- `libinput/doc/user/gestures.rst`
- `libinput/src/libinput.h`
