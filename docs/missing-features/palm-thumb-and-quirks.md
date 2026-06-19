# Feature: Palm, Thumb, And Device Policy Overrides

## Gap

Libinput has device-aware palm and thumb handling. It uses firmware palm flags,
pressure, touch size, exclusion zones, disable-while-typing, disable-while-
trackpointing, stylus arbitration, and hidden device-specific data.

`input-rs` should not copy libinput's hidden device-quirks database model.
Device-specific behavior should come from explicit input-rs configuration,
device profiles, or another user-visible override mechanism with diagnostics.

`input-rs` currently has a fixed edge-zone palm check, a simple
disable-while-typing timeout, and an optional conservative thumb-rest heuristic
for contacts that start in the lower touchpad zone while another finger is
already active. It now preserves raw pressure, touch-size, and MT tool-type
metadata, and it can reject contacts marked as firmware palms at touch down.
It does not yet use pressure or size thresholds for classification, does not
handle late firmware reclassification, and does not have a device-profile or
override layer.

## Current Partial Implementation

- `TouchpadMotionFilter` can reject contacts that begin in the configured palm
  edge zone. Classification is latched for the contact.
- `TouchpadMotionFilter` can reject likely thumb-rest contacts when
  `thumb_detection` is enabled: a contact beginning in the lower
  `thumb_edge_ratio` portion of the touchpad while another non-palm contact is
  active is treated as ignored.
- The evdev data path preserves raw optional touch metadata needed by richer
  palm/thumb policy:
  - `ABS_MT_PRESSURE` on MT-B slots.
  - `ABS_PRESSURE` on the conservative single-touch fallback.
  - `ABS_MT_TOUCH_MAJOR` and `ABS_MT_TOUCH_MINOR` on MT-B slots.
  - `ABS_MT_TOOL_TYPE` on MT-B slots, represented as `TouchToolType`.
  - `ABS_MT_TRACKING_ID` on MT-B slots so higher layers can distinguish slot
    reuse from continued contact identity when needed.
- `TouchpadMotionFilter` treats `TouchToolType::Palm` as a palm when
  `palm_detection` is enabled and the tool type is present on `TouchDown`.
  The classification is latched for that contact like edge-zone palms and
  thumb-rest contacts.
- `TouchpadMotionFilter` suppresses touchpad motion during the fixed DWT timeout
  after seat keyboard activity.
- `TouchCancel` resets the affected touchpad slot, including any latched
  palm/thumb classification for that contact.
- `feature_palm_thumb_and_device_policy` has human-like Criterion workloads for
  edge-palm rejection, thumb-rest suppression, DWT suppression, and a
  firmware-palm-shaped contact carrying pressure/size/tool metadata.
- The thumb workloads also include a lower-zone single-contact sequence with
  thumb detection enabled, and a clickpad bottom-left button-area click with
  thumb detection enabled, so the current behavior is captured for the "only
  contact" and intentional clickpad-click policy questions.
- `feature_palm_thumb_and_device_policy` also has human-like workloads for a
  high-pressure/large-touch contact that is not marked as a firmware palm, a
  contact whose tool type changes from finger to palm after touch down, and a
  firmware-palm-shaped contact with `palm_detection` disabled. These measure the
  current behavior for policy gaps; they do not mark pressure/size thresholds or
  late firmware reclassification complete.
- `feature_palm_thumb_and_device_policy` also has a current-behavior workload
  where a firmware-palm-shaped contact joins while a three-finger swipe stream is
  already active. This captures the unresolved policy question of whether the
  palm should cancel, be ignored by, or otherwise arbitrate with active gestures.
- `feature_palm_thumb_and_device_policy` also has human-like or policy-shaped
  Criterion workloads for:
  - DWT after an internal-keyboard-shaped event timestamp;
  - external-keyboard-shaped input that does not update the touchpad seat
    timestamp and therefore does not suppress touchpad motion;
  - grouped keyboard/touchpad/trackpoint-shaped metadata through the current
    standalone policy model;
  - device-profile/override-shaped metadata for known internal touchpad,
    internal PS/2 device, and unknown internal fallback cases through the current
    standalone policy model;
  - stylus-proximity plus touch-contact events through the current event model.
- Pressure thresholding, size thresholding, late firmware reclassification,
  device grouping, DWTP, stylus arbitration, and configurable device-profile
  override support are not complete yet.

## Scope

This feature covers accidental touch suppression and per-device behavior
overrides:

- Palm detection.
- Thumb detection.
- Disable-while-typing.
- Disable-while-trackpointing.
- Stylus vs touchpad arbitration where applicable.
- Per-device configurable overrides and model-specific thresholds.
- Device grouping needed for internal keyboard, touchpad, trackpoint, and
  tablet relationships.

It does not cover clickpad button areas themselves. Those are tracked in
`touchpad-tap-and-click.md`.

## Requirements To Mark Complete

### Input Data Collection

- [x] Read and store `ABS_MT_PRESSURE` when available.
- [x] Read and store `ABS_PRESSURE` for devices that report pressure outside
  MT slots.
- [x] Read and store `ABS_MT_TOUCH_MAJOR` and `ABS_MT_TOUCH_MINOR` when
  available on MT slots.
- [x] Read and store `ABS_MT_TOOL_TYPE` on MT slots.
- [ ] Read and store equivalent non-MT size axes or `BTN_TOOL_*` tool
  information where available and reliable.
- [x] Preserve per-slot pressure and size values through touch down, motion,
  and frame processing.
- [x] Do not require pressure or size axes for ordinary touchpads to keep
  working.
- [ ] Normalize pressure and size values against device axis ranges, or
  explicitly document that all policy thresholds are raw device units.

### Palm Detection

- [ ] Keep edge-zone palm detection but make its geometry explicit and
  configurable per device or profile.
- [ ] Add pressure-based palm detection for devices with pressure data.
- [ ] Add size-based palm detection for devices with touch-major data.
- [x] Honor firmware palm/tool classification when provided on `TouchDown` as
  `TouchToolType::Palm` and `palm_detection` is enabled.
- [ ] Handle firmware palm/tool classification that arrives after touch down,
  including deterministic rules for whether an active finger can become a palm.
- [ ] A touch classified as palm must not emit pointer motion, scroll, tap,
  click, or unintended gesture events.
  - [x] The current filter suppresses pointer motion, scroll, tap, and click
    output for latched palm contacts.
  - [x] Add a human-like current-behavior Criterion workload where a
    firmware-palm-shaped contact appears while a three-finger swipe stream is
    active.
  - [ ] Define whether a palm contact should cancel, be ignored by, or otherwise
    arbitrate with an active gesture stream, then add deterministic assertions
    for the chosen policy.
- [x] Palm classification must be stable enough to avoid flickering between
  palm and finger during one contact.
- [ ] Define whether a palm can ever be reclassified as a finger during the
  same contact. If yes, the transition rules must be deterministic.
- [ ] Palm handling must clear any gesture/tap state that was waiting on the
  affected contact.

### Thumb Detection

- [ ] Detect likely thumbs based on pressure, size, touch position, speed, or
  relative position to active fingers.
- [x] Thumb detection must be able to ignore a resting thumb during pointer
  motion by another finger.
- [ ] Thumb detection must not break intentional clickpad bottom-area clicks.
  - [x] Add a human-like current-behavior Criterion workload for a bottom-left
    clickpad button-area click while thumb detection is enabled.
  - [ ] Define the final arbitration policy and add deterministic assertions for
    intentional button-area clicks with thumb detection enabled.
- [ ] Thumb detection must distinguish a thumb resting in a button area from an
  intentional click where possible.
- [ ] Define behavior when a thumb is the only contact.
  - [x] Add a human-like current-behavior Criterion workload for a lower-zone
    single contact while thumb detection is enabled.
  - [ ] Document whether a lone lower-zone contact is ignored, treated as an
    ordinary finger, or handled by a separate policy, then add deterministic
    assertions for the chosen behavior.

### DWT And DWTP

- [ ] Keep disable-while-typing but make its timeout configurable or documented
  as an intentional fixed policy.
- [ ] DWT must distinguish internal keyboard activity from unrelated external
  keyboards if device grouping metadata is available.
- [ ] DWT must not suppress touchpads for modifier-only or compositor-generated
  events unless that is deliberate.
- [ ] Implement disable-while-trackpointing for devices grouped with a
  trackpoint.
- [ ] DWTP must suppress touchpad motion while the trackpoint is active and
  restore touchpad input after a timeout.
- [ ] DWT and DWTP must not suppress physical buttons that are required for a
  grouped trackpoint, such as top buttons on some Lenovo clickpads.

### Stylus And Tablet Arbitration

- [ ] For devices that have touch and stylus components, suppress accidental
  touch input while a stylus is in proximity where appropriate.
- [ ] Stylus arbitration must depend on device grouping, not only global seat
  state.
- [ ] Touch input must resume when stylus proximity ends.

### Device Overrides And Metadata

- [ ] Add an explicit device-profile or override representation for
  model-specific thresholds, broken axes, device classes, button areas, and
  grouping hints.
  - [x] Add a human-like Criterion workload carrying device-profile-shaped
    metadata through the current standalone policy model. This only exercises the
    metadata fields that exist today: bus, vendor id, product id, and internal
    classification.
  - [ ] Define the actual profile schema. Completion requires named fields for
    palm edge geometry, thumb edge geometry, pressure thresholds, size
    thresholds, DWT timeout, DWTP timeout, clickpad button areas, button maps,
    device class overrides, grouping hints, and broken/ignored axes.
- [ ] Match automatic overrides using stable metadata such as
  bus/vendor/product/name and any required sysfs attributes, or require users to
  select a profile explicitly when automatic matching would be ambiguous.
  - [ ] Include enough match keys to distinguish devices that share
    bus/vendor/product but differ by name, physical path, sysfs attributes,
    capabilities, or firmware-reported axis ranges.
  - [ ] Define conflict behavior when more than one profile matches: explicit
    user selection must win, exact metadata matches must beat broad defaults,
    and ambiguous automatic matches must either be rejected with diagnostics or
    resolved by a documented priority order.
- [ ] Overrides must be data-driven enough that adding a device profile does
  not require changing core state machine code.
- [ ] Override values must be visible in debug logs or diagnostics so device
  behavior can be explained.
- [ ] A missing or malformed override must fall back to safe defaults.
  - [ ] Safe fallback must mean the device still works as a normal input device:
    no panic, no hidden hard-coded model behavior, no disabled device unless an
    explicit policy says so, and a diagnostic that explains why the override was
    ignored.

### Device Grouping

- [ ] Represent relationships between internal keyboard, internal touchpad,
  trackpoint, tablet tool, tablet pad, and tablet touch nodes.
- [ ] Grouping must be available to DWT, DWTP, stylus arbitration, and
  disabled-on-external-mouse policy.
- [ ] Device grouping must survive hotplug reorder. It cannot depend only on
  the order devices are discovered.

### Coverage

- [x] Add a human-like Criterion workload for edge-zone palm rejection.
- [ ] Cover pressure-based palm rejection.
  - [x] Add a human-like Criterion workload that carries pressure metadata
    through the event model.
  - [x] Add a human-like Criterion workload for a high-pressure contact shaped
    like a palm candidate but without a firmware palm tool flag.
  - [ ] Add actual pressure threshold policy and deterministic assertions.
- [ ] Cover size-based palm rejection.
  - [x] Add a human-like Criterion workload that carries touch-major/minor
    metadata through the event model.
  - [x] Add a human-like Criterion workload for a large touch-major/minor contact
    shaped like a palm candidate but without a firmware palm tool flag.
  - [ ] Add actual size threshold policy and deterministic assertions.
- [ ] Cover firmware palm/tool classification.
  - [x] Add a human-like Criterion workload with a palm-classified contact
    carrying pressure, size, tool type, and tracking id.
  - [x] Add a human-like Criterion workload where a contact starts as a finger
    and later reports `TouchToolType::Palm` on motion, covering the current late
    reclassification gap.
  - [x] Add a human-like Criterion workload for a firmware-palm-shaped contact
    with `palm_detection` disabled.
  - [x] Add a human-like Criterion workload where a firmware-palm-shaped contact
    appears while a three-finger swipe stream is active.
  - [ ] Add deterministic assertions for firmware-palm suppression, ordinary
    finger preservation, and runtime `palm_detection = false` behavior.
- [x] Add a human-like Criterion workload for thumb resting during pointer
  motion.
- [x] Add human-like Criterion workloads for a lower-zone single contact and an
  intentional clickpad bottom-area click while thumb detection is enabled.
- [ ] Cover DWT with internal keyboard and external keyboard cases.
- [x] Add human-like Criterion workloads for current DWT suppression after an
  internal-keyboard-shaped timestamp and non-suppression for an
  external-keyboard-shaped case that does not update the touchpad seat timestamp.
- [ ] Add deterministic assertions once internal/external keyboard grouping is
  represented in the production DWT path.
- [ ] Cover DWTP with a grouped trackpoint.
  - [x] Add a human-like Criterion workload carrying grouped
    keyboard/touchpad/trackpoint-shaped metadata through the current standalone
    policy model.
  - [ ] Implement actual DWTP suppression/restoration behavior and deterministic
    assertions for grouped trackpoint activity.
- [ ] Cover stylus proximity suppressing touch.
  - [x] Add a human-like Criterion workload carrying stylus proximity/tip/axis
    events interleaved with a touch contact through the current event model.
  - [ ] Implement actual stylus-touch arbitration and deterministic assertions
    for suppression while stylus is in proximity and restoration after proximity
    ends.
- [ ] Cover device-profile/override matching and fallback behavior.
  - [x] Add a human-like Criterion workload carrying known internal touchpad,
    internal PS/2, and unknown internal fallback metadata through the current
    standalone policy model.
  - [ ] Add actual profile matching, conflict handling, malformed-profile
    fallback, user-selected profile override, and deterministic coverage for
    every supported match key.

### Documentation

- [x] Update `FEATURE_PARITY.md` with separate statuses for edge palm, pressure
  palm, size palm, thumb, DWT, DWTP, and device overrides.
- [ ] Document supported device-profile/override keys and matching rules.
- [ ] Document any user-facing palm/thumb configuration.

## Local References

- `input-rs/src/backend/evdev.rs`
- `input-rs/src/processing/touchpad_motion.rs`
- `input-rs/src/processing/config.rs`
- `input-rs/src/state.rs`
- `libinput/doc/user/palm-detection.rst`
- `libinput/doc/user/touchpad-thumb-detection.rst`
- `libinput/doc/user/trackpoints.rst`
