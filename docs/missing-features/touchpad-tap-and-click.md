# Feature: Touchpad Tap and Click

## Gap

Libinput supports a full touchpad tap and click stack: one, two, and
three-finger taps; configurable tap button maps; tap-and-drag; drag lock;
clickpad software button areas; clickfinger; and click method selection.

`input-rs` now implements configurable tap button maps, configurable tap
timing/movement thresholds, maximum tap finger count, tap-and-drag, a real
drag-lock state machine with timeout, clickpad detection from
`INPUT_PROP_BUTTONPAD`, configurable software button geometry, configurable
clickfinger maps, and a documented left-handed clickpad policy. Runtime plumbing
carries these through `Context`/`FilterChain`; jay still needs user-facing config
for the newer knobs.

Remaining work is narrow and listed as unchecked items below: exact
libinput-style delayed tap release, a stable clickfinger finger-count decision
window, deterministic coverage for a few config-change/order combinations, and
jay config/docs propagation.

Local libinput behavior is nuanced here and must be copied deliberately or
replaced deliberately. In libinput, tap button maps and clickfinger maps are
not changed by the generic left-handed toggle; those features use their own
tap/clickfinger map settings. Clickpad software button areas do interact with
left-handed mode, but only for explicit left/right button areas. The main area
still behaves like the primary click area, and middle stays middle. `input-rs`
must encode its chosen policy explicitly instead of relying on hidden
device-specific behavior.

## Current Partial Implementation

- `TouchpadMotionFilter` recognizes:
  - one-finger tap as `BTN_LEFT`;
  - two-finger tap as `BTN_RIGHT` when `two_finger_tap_right_click` is enabled;
  - three-finger tap as `BTN_MIDDLE`.
- `TouchpadMotionFilter` records the last recognized tap and, when tap-and-drag
  is enabled, a quick second touch-down emits a logical button press with that
  tap button. The logical button is released when the final drag contact lifts.
- Tap-and-drag is intentionally documented as partial because the current path
  still emits the original tap click before the drag press; it does not yet delay
  tap release to perfectly match libinput.
- `TouchCancel` clears tap recognition state and releases the current tap-drag
  logical button if one is active for the affected contact.
- jay runtime config now reaches input-rs for:
  - tap enabled;
  - tap-and-drag enabled;
  - tap drag lock enabled;
  - click method;
  - two-finger tap right-click.
- `TouchpadConfig` stores `tap_drag_lock` and `click_method`.
- `TouchpadMotionFilter` now uses `click_method` for physical clickpad-style
  `BTN_LEFT` press/release events:
  - `ButtonAreas` maps bottom-left to `BTN_LEFT`, bottom-middle to `BTN_MIDDLE`,
    bottom-right to `BTN_RIGHT`, and the main area to `BTN_LEFT`.
  - `Clickfinger` maps one active finger to `BTN_LEFT`, two active fingers to
    `BTN_RIGHT`, and three or more active fingers to `BTN_MIDDLE`.
  - The chosen logical button is latched at physical press time so release uses
    the same logical button even if contacts move before release.
- Current left-handed handling is only a generic filter-chain button swap.
  `FilterChain::set_left_handed(true)` inserts `ButtonSwapFilter` before the
  touchpad filter. That means tap events synthesized by `TouchpadMotionFilter`
  are not swapped by the current chain, and physical clickpad `BTN_LEFT` input
  can be swapped before click-method handling has a chance to inspect button
  areas or finger count. This is documented as an incomplete ordering/policy
  issue, not as intended final behavior.
- Drag-lock behavior is implemented: the logical button is held across a lift,
  released by the next contact or `drag_lock_timeout_us`, and cleared on touch
  cancel.
- `feature_touchpad_tap_and_click` has human-like Criterion workloads for
  one-finger tap, two-finger tap, two-finger tap with right-click disabled,
  staggered two-finger tap with reverse release order, same-frame two-finger tap
  with reverse release order, three-finger tap, movement-cancelled tap,
  timeout-cancelled tap, near-timeout tap, near-movement-threshold tap,
  tap-then-drag, delayed second-touch tap-drag timing, drag-lock-shaped
  lift/re-touch, drag-lock-shaped device removal and explicit device disable, a
  physical click during a pending tap, clickpad main area, bottom-left,
  bottom-middle, bottom-right, and press-position-latched clicks, plus one-, two-,
  and three-finger clickfinger clicks, a staggered two-finger clickfinger click,
  and a clickfinger click where another finger is added while the button is
  already held.
- `feature_touchpad_tap_and_click` also has current-behavior candidate workloads
  for:
  - an alternate tap-map-shaped sequence covering one-, two-, and three-finger
    taps that a future one-left/two-middle/three-right map would remap;
  - an alternate clickfinger-map-shaped sequence covering one-, two-, and
    three-finger physical clicks that a future clickfinger map would remap.
  These workloads do not mean configurable maps are implemented.
- The tap/click workload group also carries a removal-shaped tap-drag sequence
  where `DeviceRemoved` arrives while the drag contact is active and the filter
  may own a logical button.
- `feature_touchpad_tap_and_click` also has human-like Criterion workloads for:
  - tap-to-click configuration changes before a neutral tap sequence and during an
    active tap sequence;
  - two-finger tap right-click configuration disabled, changed before a neutral
    tap sequence, and changed while a two-finger tap is active;
  - click-method changes from neutral state into button-area and clickfinger
    modes;
  - click-method changes while a touch is already active but before the physical
    click, and while a clickpad button is already held;
  - tap suppression while DWT is active;
  - tap suppression for an edge-zone palm contact;
  - tap behavior while natural scrolling is enabled;
  - left-handed current-chain interaction for one-, two-, and three-finger taps,
    clickpad main/bottom-left/bottom-middle/bottom-right clicks, and one-, two-,
    and three-finger clickfinger clicks;
  - a three-finger tap passing through middle-button emulation after the touchpad
    filter synthesizes `BTN_MIDDLE`.

## Scope

This feature covers touchpad-generated button behavior only:

- Tap-to-click.
- Tap button mapping.
- Tap-and-drag.
- Tap drag lock.
- Three-finger drag if implemented as touchpad button emulation.
- Clickpad software button areas.
- Clickfinger button selection.
- Runtime click method configuration.

This feature does not cover middle-button emulation from physical left+right
button chords. That is tracked in `middle-button-emulation.md`.

## Requirements To Mark Complete

### Public Configuration

- [x] Add a touchpad tap configuration type that can represent enabled/disabled
  tapping separately from click method settings.
- [x] Represent the supported tap finger count for a device. The implementation
  must not expose three-finger tap as available on devices that cannot track
  three fingers. `TouchpadConfig::max_tap_fingers` caps the map; evdev currently
  defaults to 3 unless a profile/backend override lowers it.
- [ ] Represent tap button maps at least for:
  - [x] one-finger left, two-finger right, three-finger middle.
  - [x] two-finger tap right-click disabled as the current one-bit partial map.
  - [x] Add a human-like Criterion workload carrying the one-, two-, and
    three-finger tap input shape through the current hardcoded map as a baseline
    for a future alternate tap map.
  - [x] one-finger left, two-finger middle, three-finger right
    (`TapButtonMap::LeftMiddleRight`).
- [x] Define tap-map interaction with left-handed mode. If matching libinput,
  the generic left-handed toggle must not rewrite tap output; one/two/three
  finger taps are controlled only by the tap button map. If `input-rs` chooses
  a different user-visible behavior, it must have an explicit configuration
  knob, documented defaults, jay config propagation, and deterministic coverage.
- [ ] Add runtime config methods on `Context` for:
  - [x] tap enabled.
  - [x] tap button map (`Context::set_tap_button_map`, plus the narrower
    `two_finger_tap_right_click` toggle).
  - [x] tap-and-drag enabled.
  - [x] tap drag lock enabled and implemented.
  - [x] click method. Clickpad detection, button areas, clickfinger, and
    deterministic policy tests exist.
  - [x] clickfinger button map (`Context::set_clickfinger_button_map`).
- [x] Represent tap timing settings in input-rs:
  - [x] maximum tap duration (`tap_timeout_us`).
  - [x] maximum tap movement distance (`tap_move_threshold_mm`).
  - [x] tap-and-drag second-touch timeout (`tap_drag_timeout_us`).
  - [x] drag-lock unlock timeout (`drag_lock_timeout_us`; zero disables the
    timeout and unlocks on the next contact).
  These must live in input-rs configuration, with jay forwarding values instead
  of applying any tap timing policy itself.
- [x] Extend the jay `InputBackend` path so compositor config changes reach
  the active `input-rs` device without requiring device re-add.
- [x] Define default values deliberately. If libinput defaults are not copied,
  the doc must state the reason and the user-visible default. Defaults:
  `tap_timeout_us = 180_000`, `tap_move_threshold_mm = 1.0`,
  `tap_drag_timeout_us = 300_000`, `drag_lock_timeout_us = 0` (unlock on next
  tap), `max_tap_fingers = 3`, tap map `LeftRightMiddle`, clickfinger map
  `LeftRightMiddle`, button-area bottom ratio `0.20`, middle-area boundaries
  `1/3` and `2/3`. These match the pre-existing filter behavior except where
  documented above.

### Event And State Machine Behavior

- [x] One-finger tap must emit left button press and release only after the tap
  is recognized, not at touch-down time.
- [x] Two-finger tap must emit the configured second button and must not emit a
  pointer motion or scroll event for the same gesture.
- [x] Three-finger tap must emit the configured third button when the hardware
  can track three fingers. `max_tap_fingers` suppresses the tap when the device
  cannot; mapping is covered by deterministic tests.
- [x] Movement beyond the configured tap movement threshold must cancel tap
  recognition.
- [x] Tap timeout must cancel tap recognition when a touch is held too long.
- [ ] Tap recognition must handle fingers going down and up in different orders
  without duplicating button events.
  - [x] Add human-like Criterion workloads for staggered two-finger down with
    reverse release and same-frame two-finger down with reverse release.
  - [ ] Add deterministic assertions that each ordering emits exactly one
    logical tap button press/release pair or the documented cancellation result.
- [ ] Tap recognition must handle `SYN_REPORT` frame boundaries. A multi-finger
  tap should count fingers observed in a coherent gesture, not accidental order
  within one evdev frame.
  - [x] Add human-like Criterion workloads for same-frame multi-finger taps and
    staggered multi-frame finger arrival.
  - [ ] Add deterministic replay coverage for evdev frame ordering and
    multi-finger tap classification.
- [x] Tap-and-drag must support the common sequence: tap, second touch-down,
  movement while holding logical button, release on final touch-up.
- [x] Tap-and-drag must use the button selected by the initial tap finger count.
- [x] Drag lock must keep the logical button pressed across a short lift and
  release it on the defined unlock gesture or timeout. Lifting the drag contact
  holds the button; the next contact releases it, and a non-zero
  `drag_lock_timeout_us` releases it without further input.
- [x] Drag lock must not leave a logical button stuck after device removal,
  disablement, palm cancellation, or touch cancel. Touch cancel releases the
  lock; disable/removal release tracked buttons through context cleanup;
  deterministic tests cover touch-cancel and timeout paths.
  - [x] Touch cancel releases the current non-locking tap-drag logical button.
  - [x] Manual device disable releases tracked logical pointer buttons through
    `Context::set_device_enabled(false)`.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    while a tap-drag sequence is active through the current touchpad filter.
  - [x] Add a human-like Criterion workload for context-level
    `Context::remove_device_by_index()` while a tap-drag sequence is active.
  - [x] Drag-lock behavior is implemented.
  - [x] Cover device-removal paths for drag lock. Evdev removal with active
    touches emits `TouchCancel`; monitor-driven and backend removal reach
    context cleanup, which releases tracked logical buttons. Filter-level
    `DeviceRemoved` clears drag-lock state.
- [ ] Physical touchpad buttons must be handled separately from taps so a
  physical click does not get duplicated by tap recognition.
  - [x] Physical pointer-button press now clears tap recognition for the active
    sequence in the current filter path.
  - [x] Add a human-like Criterion workload where a physical click happens while
    a tap candidate is still active, so the current no-duplicate-click path is
    measured.
  - [x] Deterministic coverage for physical-click/tap interaction
    (`test_clickpad_detection_gates_software_buttons`).

### Clickpad Button Areas

- [x] Detect whether a device is a clickpad, or provide a device-profile/config
  path that marks it as one. `INPUT_PROP_BUTTONPAD` sets `DeviceInfo.is_clickpad`
  and `DeviceCaps::CLICKPAD`; `clickpad_override`/profiles can override it.
- [x] Implement software button areas for left, right, and middle where the
  selected click method supports them.
  - [x] Bottom-zone mapping exists in `TouchpadMotionFilter`.
  - [x] Configurable defaults: `button_area_bottom_ratio = 0.20`,
    `button_area_left_ratio = 1/3`, `button_area_right_ratio = 2/3`.
- [x] Button area selection must be based on touch position at click time and
  must respect the configured left-handed behavior.
  - [x] Selection uses active touch position at physical press time.
  - [x] Final left-handed policy: swap bottom-left/bottom-right software areas,
    keep bottom-middle as middle, and leave the main area as the primary click
    area (matching libinput).
  - [x] Apply the policy after click-method button selection. `FilterChain`
    sets a touchpad-internal flag instead of inserting a pre-chain
    `ButtonSwapFilter`, so raw clickpad `BTN_LEFT` reaches the touchpad filter.
  - [x] Add human-like Criterion workloads for the current left-handed chain
    with clickpad main-area, bottom-left, bottom-middle, and bottom-right
    clicks.
  - [x] Add deterministic assertions for the final policy
    (`test_left_handed_swaps_only_bottom_button_areas`,
    `test_clickpad_detection_gates_software_buttons`).
- [x] A click in the main area must produce left click unless the active click
  method says otherwise.
- [x] Bottom right and middle areas are configurable from device geometry with
  documented defaults.
- [ ] Area clicks must not conflict with edge scrolling. If both features are
  enabled on a device, the precedence must be defined and covered.

### Clickfinger

- [x] Implement clickfinger selection for one, two, and three fingers.
  - [x] Active-finger-count mapping exists.
- [x] Clickfinger must emit left, right, and middle according to the configured
  clickfinger map.
  - [x] Default map emits left for one finger, right for two fingers, and
    middle for three or more fingers.
  - [x] Add a human-like Criterion workload carrying one-, two-, and
    three-finger physical click input through the current map.
  - [x] Configurable clickfinger maps (`ClickfingerButtonMap::LeftRightMiddle`
    and `LeftMiddleRight`) with `Context::set_clickfinger_button_map`.
- [x] Define clickfinger interaction with left-handed mode. Matching libinput,
  the generic left-handed toggle does not rewrite clickfinger output; the
  clickfinger map controls it. Deterministic coverage:
  `test_clickfinger_button_map`.
- [x] Add human-like Criterion workloads for one-, two-, and three-finger
  clickfinger clicks through the current left-handed chain.
- [ ] Clickfinger must wait for a stable finger count before choosing the
  button where hardware reports fingers across multiple frames.
  - [x] Add a human-like Criterion workload for a two-finger clickfinger click
    where the second finger arrives in a later frame before the physical press.
  - [ ] Deterministic coverage must define how long the implementation waits for
    a stable finger count and what happens if the physical switch arrives before
    the expected contacts are visible.
- [ ] Clickfinger must not change the logical button after the press has been
  emitted, even if another finger is added while the button is held.
  - [x] Add a human-like Criterion workload where a one-finger clickfinger press
    is held, another finger is added, and the physical release must still release
    the latched button from press time.

### Neutral State And Reconfiguration

- [ ] Changing tap or click settings while touches/buttons are active must not
  produce impossible event sequences.
- [ ] If a config change cannot apply immediately, it must apply after the
  touchpad reaches neutral state.
- [x] Device disablement must release or cancel any logical button that the
  tap/click state machine owns.
- [ ] Device removal must not leave compositor button state pressed.

### Coverage

- [x] Add human-like Criterion workloads for one, two, and three-finger taps.
- [x] Add a human-like Criterion workload for two-finger tap with the current
  `two_finger_tap_right_click` setting disabled.
- [x] Add a human-like Criterion workload for an alternate tap-map-shaped
  one-/two-/three-finger tap sequence through the current hardcoded map.
- [ ] Cover tap cancellation by movement and timeout.
  - [x] Add human-like Criterion workloads for movement cancellation and timeout
    cancellation.
  - [x] Add a human-like Criterion workload for a tap that is close to the
    current timeout but still inside the current fixed threshold.
  - [x] Add a human-like Criterion workload for a tap with movement close to the
    current fixed movement threshold but still under it.
  - [x] Add deterministic tests for cancellation boundaries
    (`test_tap_timeout_is_configurable`, `test_tap_move_threshold_is_configurable`).
- [x] Cover tap-and-drag and drag lock (Criterion workloads plus deterministic
  drag-lock tests).
  - [x] Add a human-like Criterion workload for tap-and-drag through the current
    filter.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    while tap-and-drag is active through the current touchpad filter.
  - [x] Add a human-like Criterion workload for context-level
    `Context::remove_device_by_index()` while tap-and-drag is active.
  - [x] Add a human-like Criterion workload for a drag-lock-shaped short lift and
    re-touch sequence with `tap_drag_lock` enabled.
  - [x] Add a human-like Criterion workload for direct `DeviceRemoved` input
    during a drag-lock-shaped active sequence with `tap_drag_lock` enabled.
  - [x] Add a human-like Criterion workload for explicit
    `Context::set_device_enabled(false)` during a drag-lock-shaped active
    sequence with `tap_drag_lock` enabled.
  - [x] Add a human-like Criterion workload for context-level
    `Context::remove_device_by_index()` during a drag-lock-shaped active
    sequence with `tap_drag_lock` enabled.
  - [x] Add a human-like Criterion workload where the second touch arrives after
    the current fixed tap-drag timeout, so future configurable timing can be
    compared against current behavior.
  - [x] Implement actual drag-lock hold/release behavior and add deterministic
    assertions for the unlock gesture, timeout, touch cancel, and device disable
    (`test_drag_lock_holds_button_until_next_tap`,
    `test_drag_lock_timeout_releases_button`,
    `test_drag_lock_timeout_is_dispatchable_without_input`).
- [x] Cover clickpad button areas on at least one synthetic device geometry.
  - [x] Add human-like Criterion workloads for main-area, bottom-left,
    bottom-middle, and bottom-right button-area clicks.
  - [x] Add a human-like Criterion workload where a contact presses in one
    button area, moves to a different button area while held, and releases. This
    covers the current latched-button path but does not replace deterministic
    assertions.
  - [x] Add deterministic tests for main area, bottom-left, bottom-middle,
    bottom-right, and left-handed swapping. Missing/invalid geometry falls back
    to `BTN_LEFT`; a dedicated missing-geometry test remains optional.
- [x] Cover clickfinger with one, two, and three fingers.
  - [x] Add human-like Criterion workloads for one-, two-, and three-finger
    clickfinger clicks.
  - [x] Add human-like Criterion workloads for staggered two-finger arrival and
    for adding a finger while a clickfinger button is held.
  - [x] Add a human-like Criterion workload for an alternate
    clickfinger-map-shaped one-/two-/three-finger physical-click sequence
    through the current hardcoded map.
  - [x] Add deterministic tests for one, two, and three-finger clickfinger
    clicks and both maps (`test_clickfinger_button_map`). A stable finger-count
    decision window and changing-count-during-press assertions remain open.
- [ ] Cover config changes while neutral and while active.
  - [x] Add human-like Criterion workloads for tap-to-click changes before a
    neutral tap sequence and during an active tap sequence.
  - [x] Add human-like Criterion workloads for `two_finger_tap_right_click`
    changes before a neutral tap sequence and during an active two-finger tap.
  - [x] Add human-like Criterion workloads for click-method changes from neutral
    state into `ButtonAreas` and `Clickfinger`.
  - [x] Add human-like Criterion workloads for click-method changes while a
    contact is already active but before the physical click, and while a
    clickpad button is already held.
  - [ ] Add deterministic assertions that runtime tap and click-method changes
    either apply immediately without impossible button sequences or defer until
    neutral state.
- [ ] Cover interaction with DWT, palm rejection, natural scrolling, and
  left-handed mode.
  - [x] Add human-like Criterion workloads for DWT tap suppression, edge-palm tap
    suppression, and tapping while natural scrolling is enabled.
  - [x] Add a human-like Criterion workload where a synthesized three-finger middle
    tap passes through middle-button emulation without being chorded again.
  - [x] Add human-like Criterion workloads for one-, two-, and three-finger taps
    through the current left-handed chain.
  - [x] Add human-like Criterion workloads for clickpad button-area and
    clickfinger physical-click sequences through the current left-handed chain.
  - [ ] Add deterministic coverage for DWT timing, palm/tap arbitration,
    natural-scroll no-op behavior for taps, and left-handed tap/click mappings.

### Documentation

- [x] Update `FEATURE_PARITY.md` with the implemented behavior and remaining
  gaps. The feature stays ⚠️ until jay config propagation and the remaining
  stable-finger-count/delayed-release work are complete.
- [ ] Document all user-visible config names in jay's config docs.
- [ ] Document any deliberate differences from libinput defaults.

## Local References

- `input-rs/src/processing/touchpad_motion.rs`
- `input-rs/src/processing/config.rs`
- `input-rs/src/state.rs`
- `src/input/input_rs_backend.rs`
- `libinput/doc/user/configuration.rst`
- `libinput/doc/user/tapping.rst`
- `libinput/doc/user/clickpad-softbuttons.rst`
- `libinput/src/libinput.h`
