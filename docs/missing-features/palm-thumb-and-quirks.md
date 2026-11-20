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
- [x] Normalize pressure and size values against device axis ranges, or
  explicitly document that all policy thresholds are raw device units. Current
  `palm_pressure_threshold`, `palm_size_threshold`, and
  `thumb_pressure_threshold` are documented as raw evdev units.

### Palm Detection

- [x] Keep edge-zone palm detection but make its geometry explicit and
  configurable per device or profile (`palm_edge_ratio`, profiles).
- [x] Add pressure-based palm detection for devices with pressure data.
- [x] Add size-based palm detection for devices with touch-major data.
- [x] Honor firmware palm/tool classification when provided on `TouchDown` as
  `TouchToolType::Palm` and `palm_detection` is enabled.
- [x] Handle firmware palm/tool classification that arrives after touch down.
  A late `Palm` on `TouchMotion` latches the palm, decrements the tap finger
  count, drops pending motion, and cancels the gesture when no finger remains.
  Palms are never reclassified back to fingers during the same contact.
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

- [x] Detect likely thumbs based on pressure and touch position relative to
  active fingers (size/speed heuristics remain future work).
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

- [x] Keep disable-while-typing and make its timeout configurable
  (`dwt_timeout_us`, `Context::set_touchpad_dwt_timeout`).
- [x] DWT distinguishes internal keyboard activity from external keyboards when
  bus-derived internal classification is available (default
  `Context::dwt_internal_only = true`).
- [ ] DWT must not suppress touchpads for modifier-only or compositor-generated
  events unless that is deliberate.
- [x] Implement disable-while-trackpointing using `POINTING_STICK` capability
  (sysfs grouping remains future work).
- [x] DWTP suppresses touchpad motion while the trackpoint is active and restores
  after `dwtp_timeout_us` (`test_dwtp_suppresses_touchpad_while_trackpoint_active`).
- [ ] DWT and DWTP must not suppress physical buttons that are required for a
  grouped trackpoint, such as top buttons on some Lenovo clickpads. Current
  suppression applies to pointer motion/taps, not physical button passthrough;
  top-button clickpad capability is detected but has no policy yet.

### Stylus And Tablet Arbitration

- [x] For devices that have touch and stylus components, suppress accidental
  touch input while a stylus is in proximity (`stylus_arbitration`).
- [ ] Stylus arbitration depends on seat-level `stylus_in_proximity`; per-group
  stylus/touch relationships remain future work.
- [x] Touch input resumes when stylus proximity ends
  (`test_stylus_proximity_suppresses_touchpad`).

### Device Overrides And Metadata

- [x] Add an explicit device-profile or override representation for
  model-specific thresholds, broken axes, device classes, button areas, and
  grouping hints.
  - [x] Add a human-like Criterion workload carrying device-profile-shaped
    metadata through the standalone policy model.
  - [x] Define the actual profile schema (`profile::DeviceProfile`,
    `TouchpadProfile`, `PointerProfile`, `BrokenAxes`): palm/thumb geometry,
    pressure/size thresholds, DWT/DWTP timeouts, clickpad button geometry/maps,
    tap timing, pointer DPI/speed/rotation, device-class/internal override, and
    broken-axis declarations.
- [x] Match automatic overrides using stable metadata such as
  bus/vendor/product/name, or require users to explicitly select a profile.
  - [x] Match keys distinguish bus, vendor, product, and exact name.
    Physical-path/sysfs/capability/firmware-range keys remain future work.
  - [x] Conflict behavior: explicit selection by name wins; more match keys beat
    fewer; equally specific matches are rejected as ambiguous with a
    `tracing::warn` diagnostic and safe defaults.
- [x] Overrides are data-driven and do not require changing core state-machine
  code to add a device profile.
- [x] Override application is visible through `Context::device_profile_name()`
  and ambiguity/invalid-config warnings.
- [x] A missing or malformed override falls back to safe defaults (no profile
  match, sanitized touchpad config, device still works).
  - [x] Safe fallback means the device still works as a normal input device:
    no panic, no hidden hard-coded model behavior, no disabled device unless an
    explicit policy says so, and a diagnostic for rejected/ambiguous profiles.

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
  - [x] Add pressure threshold policy and deterministic assertions
    (`test_pressure_and_size_palm_thresholds`).
- [ ] Cover size-based palm rejection.
  - [x] Add a human-like Criterion workload that carries touch-major/minor
    metadata through the event model.
  - [x] Add a human-like Criterion workload for a large touch-major/minor contact
    shaped like a palm candidate but without a firmware palm tool flag.
  - [x] Add size threshold policy and deterministic assertions
    (`test_pressure_and_size_palm_thresholds`).
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
  - [x] Add deterministic assertions for firmware-palm suppression and late
    reclassification (`test_late_firmware_palm_reclassification_suppresses_motion`).
    Runtime `palm_detection = false` remains covered by benchmark-shaped input.
- [x] Add a human-like Criterion workload for thumb resting during pointer
  motion.
- [x] Add human-like Criterion workloads for a lower-zone single contact and an
  intentional clickpad bottom-area click while thumb detection is enabled.
- [ ] Cover DWT with internal keyboard and external keyboard cases.
- [x] Add human-like Criterion workloads for current DWT suppression after an
  internal-keyboard-shaped timestamp and non-suppression for an
  external-keyboard-shaped case that does not update the touchpad seat timestamp.
- [x] Add deterministic assertions for the production DWT path
  (`test_dwt` covers timeout; `Context::dwt_internal_only` gates external
  keyboards). Sysfs grouping remains future work.
- [ ] Cover DWTP with a grouped trackpoint.
  - [x] Add a human-like Criterion workload carrying grouped
    keyboard/touchpad/trackpoint-shaped metadata through the current standalone
    policy model.
  - [x] Implement DWTP suppression/restoration and deterministic assertions
    (`test_dwtp_suppresses_touchpad_while_trackpoint_active`). Capability-based,
    not sysfs-group-based yet.
- [ ] Cover stylus proximity suppressing touch.
  - [x] Add a human-like Criterion workload carrying stylus proximity/tip/axis
    events interleaved with a touch contact through the current event model.
  - [x] Implement stylus-touch arbitration and deterministic assertions for
    suppression and restoration
    (`test_stylus_proximity_suppresses_touchpad`).
- [ ] Cover device-profile/override matching and fallback behavior.
  - [x] Add a human-like Criterion workload carrying known internal touchpad,
    internal PS/2, and unknown internal fallback metadata through the current
    standalone policy model.
  - [x] Add profile matching, conflict handling, fallback, explicit selection,
    and deterministic coverage for bus/vendor/product/name specificity
    (`profile::tests`, `device_profile_applies_defaults_on_add`,
    `explicit_profile_selection_ignores_match_keys`).

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
