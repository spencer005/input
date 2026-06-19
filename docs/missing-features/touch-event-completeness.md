# Feature: Touch Event Completeness

## Gap

Libinput supports touch down, up, motion, cancel, and frame events. It also
handles a wider range of touch hardware protocols than the current `input-rs`
evdev path.

`input-rs` handles MT-B slot tracking with `ABS_MT_SLOT`,
`ABS_MT_TRACKING_ID`, `ABS_MT_POSITION_X`, and `ABS_MT_POSITION_Y`. It has an
initial conservative single-touch fallback for devices that expose `ABS_X`,
`ABS_Y`, and `BTN_TOUCH` without MT position axes, relative pointer events, or
common tablet-tool buttons. It now also has a `TouchCancel` event that can be
forwarded to jay and used by touchpad filters to clean up active contact state.

The feature is not complete because per-contact metadata is only preserved as
raw optional event data, deterministic replay coverage is still missing, and
the single-touch fallback policy still needs explicit documentation for
indirect touchpads and pointer-emulation devices.

## Current Partial Implementation

- MT-B contacts use the real hardware slot number and emit `TouchDown`,
  `TouchMotion`, `TouchUp`, and `Frame` from `SYN_REPORT`.
- Single-touch fallback uses synthetic slot `0`.
- Single-touch fallback emits:
  - `TouchDown` after `BTN_TOUCH = 1` once both `ABS_X` and `ABS_Y` are known.
  - `TouchMotion` when `ABS_X` or `ABS_Y` changes while the contact is active.
  - `TouchUp` after `BTN_TOUCH = 0` if the synthetic contact was active.
- `InputEvent::TouchCancel` carries the device index, timestamp, and affected
  slot/contact id using the same slot id model as `TouchUp`.
- `TouchData` carries optional raw per-contact metadata:
  - `pressure` for pressure-like axes.
  - `touch_major` and `touch_minor` for contact-size axes.
  - `tool_type` for Linux MT tool classification such as finger, pen, palm, or
    dial.
  - `tracking_id` for callers that need the kernel tracking id in addition to
    the active slot number.
- The evdev backend populates MT-B metadata from `ABS_MT_PRESSURE`,
  `ABS_MT_TOUCH_MAJOR`, `ABS_MT_TOUCH_MINOR`, `ABS_MT_TOOL_TYPE`, and
  `ABS_MT_TRACKING_ID`.
- The evdev backend clears optional MT-B metadata when a slot receives
  `ABS_MT_TRACKING_ID = -1`, so reused slots do not intentionally carry stale
  pressure, size, tool, or tracking-id values into the next contact.
- The single-touch fallback preserves raw `ABS_PRESSURE` as optional pressure
  on the synthetic slot-zero contact. It does not yet preserve single-touch
  size or tool metadata.
- The evdev backend emits `TouchCancel` for active MT-B or single-touch contacts
  when a `SYN_DROPPED` event is observed or a device is removed while contacts
  are still active.
- `Context::set_device_enabled(false)`, `Context::remove_device()`, and
  backend-originated `DeviceRemoved` cleanup emit `TouchCancel` for any contacts
  that are still tracked as active at the context layer.
- The touchpad motion filter consumes `TouchCancel` for touchpad contacts,
  cancels any active gesture stream, releases an active tap-drag button, clears
  pending motion, and resets the affected slot so later `TouchUp` events do not
  act on stale state.
- The jay bridge forwards `TouchCancel` to jay's existing `InputEvent::TouchCancel`
  path for touchscreen-class devices.
- The fallback is intentionally restricted to avoid common false positives:
  - The device must not expose MT position axes.
  - The device must not expose relative pointer events.
  - The device must not expose common tablet-tool buttons such as pen, stylus,
    rubber, brush, pencil, airbrush, mouse, or lens tools.
- `input-rs/benches/feature_workload_bench.rs` includes
  `feature_touch_event_completeness/single_touch_fallback_current_events` with a
  human-like drag sequence represented through the current touch event model.
- `input-rs/benches/feature_workload_bench.rs` includes
  `feature_touch_event_completeness/touch_cancel_current_events` and
  `feature_touch_event_completeness/touchpad_cancel_cleanup_current_filter`.
- `feature_touch_event_completeness` also has human-like Criterion workloads for:
  - direct MT-B down, motion, up, and frame event streams;
  - multiple active slots changing in the same frame;
  - reusing a slot after the previous contact on that slot ended;
  - pressure, size, tool-type, and tracking-id metadata passing through the
    current event model;
  - tracking-id-change-shaped metadata on one active slot, carried through the
    current event model as a candidate workload for hardware that changes
    identity without an explicit intermediate `-1`;
  - context-level `Context::set_device_enabled(false)` after a single-touch
    down/motion stream has left an active contact;
  - direct `DeviceRemoved` input while MT-B, same-frame multi-slot, and
    single-touch contacts are active;
  - backend-originated `DeviceRemoved` through `Context::dispatch()` while a
    touch contact is active, tracked in the device-policy benchmark group.

## Scope

This feature covers low-level touch event completeness for direct touchscreens
and touchpad contacts before higher-level touchpad gestures are recognized:

- MT-B slot tracking correctness.
- Single-touch fallback.
- Touch cancel.
- Touch frame semantics.
- Per-contact metadata needed by palm/gesture/tablet features.

It does not cover high-level touchpad gestures. Those are tracked in
`touchpad-gestures.md`.

## Requirements To Mark Complete

### Event Model

- [x] Add a touch cancel event or explicitly document why jay cannot consume
  one and how cancellation is represented instead.
- [x] If touch cancel exists, include device index, timestamp, and affected
  slot/contact information.
- [x] Preserve touch frame events as a boundary for one hardware sample.
- [x] Define whether frame events are emitted after cancel events. Current
  behavior emits cancel directly and does not synthesize a frame after cancel;
  normal `SYN_REPORT` frames still produce `Frame`.
- [x] Add optional per-contact metadata needed by other features:
  - [x] pressure.
  - [x] touch major/minor.
  - [x] tool type.
  - [x] tracking id if callers need identity beyond slot number.
- [ ] Define normalized units for optional metadata. Current values are raw
  kernel axis values, not normalized physical units.

### MT-B Handling

- [x] Continue supporting `ABS_MT_SLOT`.
- [x] Continue supporting `ABS_MT_TRACKING_ID`.
- [x] Continue supporting `ABS_MT_POSITION_X` and `ABS_MT_POSITION_Y`.
- [x] Handle slots that update position before tracking id in the same frame.
- [ ] Handle tracking id changes without an explicit intermediate -1 where
  hardware behaves that way, or document that such devices need an explicit
  device-profile/config override.
  - [x] Add a human-like Criterion workload carrying a contact whose
    `tracking_id` changes while the slot remains active through the current event
    model.
  - [ ] Define the real backend policy for this case. Completion requires a
    deterministic rule: either synthesize up/down for the old/new contact,
    synthesize cancel/down, treat it as invalid hardware that requires an
    explicit device profile, or preserve it as one contact with updated identity.
    The chosen rule must have real evdev replay coverage.
- [x] Emit down before motion for a newly active contact.
- [x] Emit up once for an ended contact.
- [x] Do not emit motion for inactive slots.
- [x] Reset slot state on device removal.

### Single-Touch Fallback

- [x] Detect devices that expose `ABS_X`, `ABS_Y`, and `BTN_TOUCH` without MT
  position axes in the current evdev backend.
- [x] Emit touch down when `BTN_TOUCH` becomes active and valid x/y are known.
- [x] Emit touch motion when x/y changes while `BTN_TOUCH` is active.
- [x] Emit touch up when `BTN_TOUCH` becomes inactive.
- [x] Use a stable synthetic slot, currently slot zero.
- [ ] Do not misclassify ordinary absolute tablets as direct touchscreens. The
  current code excludes common tablet-tool buttons, but this still needs replay
  coverage and likely udev/device-group metadata before it can be considered
  complete.
- [ ] Define how single-touch fallback interacts with pointer emulation from
  touchpads that expose both touch and pointer-style events.

### Touch Cancel

- [ ] Generate cancel events when the kernel or higher layer reports a
  cancellation condition.
  - [x] Generate cancels for active contacts if `SYN_DROPPED` is observed.
  - [ ] Define and wire a higher-layer cancel request path if jay needs one.
- [ ] Generate cancel or equivalent cleanup on device disable/removal while
  contacts are active.
  - [x] Generate cancels when evdev device removal happens with active contacts.
  - [x] Generate cancels for contacts tracked by `Context` on input-rs device
    disable.
  - [x] Generate cancels for contacts tracked by `Context` on explicit context
    removal and backend-originated `DeviceRemoved`.
- [x] Cancel must clear touchpad gesture, tap, scroll, and palm state for the
  affected contacts.
- [x] Cancel must not be followed by ordinary up events for the same cancelled
  contacts unless the event model explicitly requires it.

### Coordinate And Metadata

- [ ] Populate axis min, max, and resolution for touch devices.
- [ ] Preserve coordinate values in a documented unit.
- [ ] Prepare the data path for calibration transforms from
  `absolute-device-transforms.md`.
- [x] Preserve raw MT-B pressure, size, tool-type, and tracking-id metadata in
  touch down and touch motion events when present.
- [x] Preserve raw single-touch pressure in touch down and touch motion events
  when `ABS_PRESSURE` is present.
- [ ] Preserve single-touch size/tool metadata where the kernel exposes a
  reliable non-MT source for that data.
- [ ] Normalize or document pressure and size units per device, including how
  callers should interpret devices with missing axis min/max/resolution.
- [ ] Feed pressure/size metadata into palm/thumb detection policy. Firmware
  palm tool classification is handled in `TouchpadMotionFilter`, but pressure
  and size thresholding are tracked in `palm-thumb-and-quirks.md`.

### Coverage

- [ ] Cover MT-B down, motion, up, and frame.
  - [x] Add a human-like Criterion workload carrying direct MT-B down, motion, up,
    and frame events through the current event path.
  - [ ] Add deterministic replay coverage for real evdev MT-B slot parsing and
    frame grouping.
- [ ] Cover multiple active slots changing in the same frame.
  - [x] Add a human-like Criterion workload with three active slots updated before
    one frame.
  - [ ] Add deterministic assertions that all changed slots emit exactly once in
    the expected frame order.
- [ ] Cover slot reuse after up.
  - [x] Add a human-like Criterion workload where slot 0 is released and later
    reused for a new contact.
  - [ ] Add deterministic assertions that reused slots do not inherit stale
    coordinates, pressure, size, or higher-level touchpad state.
- [ ] Cover tracking id changes without an explicit intermediate `-1`.
  - [x] Add a human-like Criterion workload carrying a tracking-id change on an
    active slot through the current event model.
  - [ ] Add deterministic evdev replay coverage for the chosen backend behavior:
    up/down, cancel/down, explicit profile requirement, or single-contact
    identity update.
- [x] Add a human-like Criterion workload for single-touch down, motion, and up
  through the current touch event model.
- [x] Add a human-like Criterion workload for touchscreen-style touch cancel
  through the current touch event model.
- [x] Add a human-like Criterion workload for touchpad cancel cleanup through
  `TouchpadMotionFilter`.
- [ ] Cover single-touch down, motion, and up in deterministic tests or replay
  fixtures.
- [ ] Cover active contacts during device removal or disablement.
  - [x] Add human-like Criterion coverage for active touch cleanup during manual
    device disable through a real touch down/motion stream followed by
    `Context::set_device_enabled(false)`.
  - [x] Add human-like Criterion coverage for direct `DeviceRemoved` input while
    MT-B, same-frame multi-slot, and single-touch contacts are active.
  - [x] Add human-like Criterion coverage for backend-originated
    `DeviceRemoved` cleanup through `Context::dispatch()` while a touch contact
    is active.
  - [ ] Add deterministic tests or replay fixtures for removal and disablement.
- [x] Add Criterion coverage for cancel clearing higher-level touchpad state.
  Deterministic tests or replay fixtures are still required before completion.
- [ ] Cover pressure/size/tool metadata propagation where implemented.
  - [x] Add a human-like Criterion workload carrying pressure, touch major,
    touch minor, tool type, and tracking id through the current event model.
  - [ ] Add deterministic replay coverage proving evdev metadata parsing,
    frame grouping, and stale-metadata clearing on slot reuse.

### Documentation

- [x] Update `FEATURE_PARITY.md`.
- [ ] Document which touch protocols are supported.
- [x] Document cancellation semantics in this feature file.
- [ ] Document coordinate units before and after transforms.

## Local References

- `input-rs/src/backend/evdev.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/processing/touchpad_motion.rs`
- `libinput/src/libinput.h`
