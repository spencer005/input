# Feature: Tablet Tool And Pad

## Gap

Libinput has dedicated support for graphics tablet tools and tablet pads.
Tablet tools provide proximity, tip, axis, pressure, tilt, distance, wheel, and
button events. Tablet pads provide buttons, keys, rings, strips, and dials.

`input-rs` defines tablet and tablet-pad capability flags, tablet tool/pad event
variants, and now populates those capabilities and emits events from real evdev
tool and pad nodes. Evdev parsing covers proximity, tip, x/y, pressure, tilt,
tool buttons, pad buttons, ring (`ABS_WHEEL`), strip (`ABS_MISC`), and dial
(`REL_DIAL`). Jay-side tablet protocol bridging, tool identity, pressure-range
normalization, and grouped-node cleanup policy remain open.

## Current Partial Implementation

- `InputEvent` includes tablet tool variants for:
  - proximity in/out.
  - axis update with x/y, pressure, and tilt x/y.
  - tip down/up.
  - tool button press/release.
- `InputEvent` includes tablet pad variants for:
  - pad button press/release.
  - ring position.
  - strip position.
  - dial delta.
- The variants are boxed cold-path events and participate in device-index
  remapping and timestamp/device-index helper methods.
- The jay bridge currently drops these tablet events deliberately because the
  compositor-side tablet protocol path is not wired to input-rs yet.
- `feature_tablet_tool_and_pad/stylus_stroke_and_pad_current_events` runs a
  human-like stylus stroke, tool button use, and pad control sequence through
  the current event model.
- `feature_tablet_tool_and_pad` also has human-like Criterion workloads for:
  - pressure-range-shaped axis data through the current tablet event model;
  - repeated proximity cycles for one logical tool, even though tool identity is
    not represented yet;
  - a tool button held until after proximity-out, covering the current event
    model shape for button/proximity ordering;
  - direct `DeviceRemoved` input while a stylus tool is still in proximity with
    the tip and a tool button logically down;
  - pad controls used without any tool in proximity;
  - direct `DeviceRemoved` input while pad buttons, ring, strip, and dial inputs
    are active;
  - separate tool, pad, and touch node-shaped event streams using distinct device
    indices;
  - direct removal of the tablet-touch node while related tool and pad nodes in
    the same future physical group are still active.

## Scope

This feature covers graphics tablets, not tablet-mode laptop switches and not
generic touchscreen input:

- Tablet tool capability detection.
- Tablet pad capability detection.
- Tool proximity.
- Tool tip state.
- Tool absolute and relative axes.
- Tool pressure, tilt, distance, rotation, slider, and wheel data where
  available.
- Tool buttons.
- Pad buttons, keys, rings, strips, and dials.
- Tool identity and serial tracking where available.
- Tablet-specific configuration such as pressure range, eraser button mapping,
  and active area.

Switch devices are tracked in `switch-devices.md`. Touchscreen transforms are
tracked in `absolute-device-transforms.md`.

## Requirements To Mark Complete

### Capability Detection

- [x] Detect tablet tool devices from evdev capabilities (`BTN_TOOL_PEN` and
  friends + `ABS_X/Y`).
- [x] Detect tablet pad devices from evdev capabilities (`BTN_0..BTN_9`,
  `ABS_WHEEL`).
- [x] Avoid classifying ordinary touchpads or touchscreens as tablet tools
  (touchpad/finger tool markers are excluded).
- [x] Support devices where tool and pad are separate event nodes (each node is
  classified independently).
- [ ] Support devices where tablet tool and pad capabilities appear on the same
  node (tool classification wins today).
- [x] Preserve vendor, product, bus, and name metadata for tablet devices.
  Physical grouping metadata is still missing.

### Event Model

- [x] Add tablet tool event variants for:
  - [x] proximity in/out.
  - [x] axis update.
  - [x] tip down/up.
  - [x] tool button press/release.
- [x] Add tablet pad event variants for:
  - [x] pad button press/release.
  - [ ] pad key press/release.
  - [x] ring update.
  - [x] strip update.
  - [x] dial update.
- [x] Tool events carry device index, timestamp, and tool identity
  (`TabletToolKind`) on proximity events. Changed-axis masks are still absent.
- [ ] Pad events must carry device index, timestamp, control index, mode/group
  where supported, and value/state. The current variants carry device index,
  timestamp, control index, and value/state, but not mode/group metadata.
- [ ] Event field semantics must be documented: absolute units, normalized
  units, relative deltas, and button numbering. Current field names are present,
  but complete unit semantics still need documentation.

### Tablet Tool Axes

- [x] Represent absolute x/y coordinates in the current event model.
- [ ] Report relative dx/dy for tools that can be used in relative mode, or
  provide enough absolute data for the compositor to calculate it.
- [ ] Normalize pressure to a documented range. The current event carries a
  pressure value but does not define or enforce normalization.
- [ ] Normalize distance to a documented range.
- [x] Represent tilt x/y in the current event model.
- [ ] Report wheel, slider, rotation, or other tool axes where evdev exposes
  them.
- [ ] Track which axes changed per event so callers do not infer stale data as
  new input.

### Units

Tablet event fields currently carry raw evdev units:

- `TabletToolProximityData.x/y`, `TabletToolAxisData.x/y`: raw `ABS_X`/`ABS_Y`.
- `TabletToolAxisData.pressure`: raw `ABS_PRESSURE` (no normalization or
  min/max range yet).
- `TabletToolAxisData.tilt_x/tilt_y`: raw `ABS_TILT_X`/`ABS_TILT_Y`.
- `TabletPadValueData.position`: raw `ABS_WHEEL` (ring) or `ABS_MISC` (strip).
- `TabletPadDialData.delta`: accumulated `REL_DIAL` counts for the frame.

Normalization and pressure-range configuration are tracked below.

### Proximity And Tip Handling

- [x] Emit proximity-in before axis/tip events. `flush_tablet_tool` emits
  proximity, then axis, then tip per `SYN_REPORT`.
- [x] Emit proximity-out when the tool leaves range.
- [ ] Release any pressed tool buttons before proximity-out if required to keep
  compositor state coherent. Tool buttons are forwarded immediately and are not
  synthesized on proximity-out yet.
  - [x] Add a human-like Criterion workload where a tool button remains pressed
    through proximity-out in the current event model.
- [x] Define tool-device removal cleanup: evdev removal/`SYN_DROPPED` emits a
  synthetic proximity-out from `cancel_active_touches` before `DeviceRemoved`;
  tool buttons are not synthesized (the compositor bridge must release them on
  `DeviceRemoved`).
  - [x] Add a human-like Criterion workload where direct `DeviceRemoved` arrives
    while proximity, tip, and a tool button are active in the current event model.
- [ ] Derive logical tip down/up from pressure, distance, button state, or
  device-specific rules.
- [ ] Avoid phantom tip events from worn tools or pressure offsets.
- [ ] Handle tools that are always in proximity.

### Tool Identity

- [x] Represent tool type: pen, eraser/rubber, brush, pencil, airbrush, mouse,
  lens, or unknown (`TabletToolKind`).
- [ ] Track serial or unique identifiers when hardware provides them.
- [ ] Keep enough identity stable across proximity cycles for clients or the
  compositor to apply tool-specific state.
- [ ] Define fallback behavior for non-unique tools.

### Tablet Pad

- [x] Represent pad buttons with stable indices in the current event model.
- [ ] Map pad keys to key codes where the kernel provides semantic keys.
- [ ] Report ring position and source state. The current event carries ring
  position but not source state.
- [ ] Report strip position and source state. The current event carries strip
  position but not source state.
- [x] Represent dial deltas in the current event model.
- [ ] Support pad mode groups if needed for common tablets.
- [x] Pad events do not require a tool to be in proximity; pad nodes are
  processed independently.
  - [x] Add a human-like Criterion workload for pad button, ring, strip, and dial
    input without any tool proximity events.
- [ ] Define pad-device removal cleanup. If the pad node disappears with pad
  buttons pressed or relative controls mid-update, input-rs must either emit
  coherent synthetic pad-button releases before removal or document that
  `DeviceRemoved` is the only cleanup signal and the jay tablet-pad bridge must
  release per-pad state from it.
  - [x] Add a human-like Criterion workload where direct `DeviceRemoved` arrives
    while multiple pad controls are active in the current event model.

### Configuration

- [x] Add pressure range configuration for tablet tools
  (`TabletConfig::pressure_range`, `Context::set_tablet_config`).
- [ ] Add eraser button remapping where hardware reports a firmware eraser
  button.
- [x] Add active area configuration for indirect tablets
  (`TabletConfig::active_area`, normalized rectangle with clamp-and-map policy).
- [ ] Add left-handed or rotation handling for tablets if jay wants parity with
  libinput's tablet behavior.
- [ ] Configuration must be per device or per tool as appropriate.
- [ ] Runtime config changes must not corrupt an active stroke.
- [ ] Runtime config changes and device removal must compose with each other:
  if active area, pressure range, button remap, rotation, or eraser policy is
  changed while a tablet node is removed, input-rs must define whether pending
  config is dropped, applied to a re-added matching node, or retained as an
  explicit user rule for future devices.

### Jay Bridge

- [ ] Map tablet tool events into jay's input model. The current input-rs backend
  ignores these events until jay-side tablet protocol support is wired.
- [ ] Map tablet pad events into jay's input model. The current input-rs backend
  ignores these events until jay-side tablet protocol support is wired.
- [ ] Preserve pressure, tilt, distance, tool identity, serial, and pad control
  identity.
- [ ] Do not degrade tablet tool events into generic pointer events unless that
  is an explicit compatibility mode.
- [ ] Device removal must not leave jay-side tablet state stuck. Completion
  requires a bridge rule for:
  - tool node removal with proximity active.
  - tool node removal with tip down.
  - tool node removal with one or more tool buttons pressed.
  - pad node removal with one or more pad buttons pressed.
  - tablet-touch node removal while the related tool and pad nodes remain.
  - removal of one node in a grouped physical tablet followed by continued
    events from the remaining nodes.
- [ ] If jay lacks tablet protocol support for any event class, document the
  missing compositor-side work before marking this feature complete.

### Coverage

- [x] Add a human-like Criterion workload covering proximity in/out.
- [x] Add a human-like Criterion workload covering tip down/up with pressure
  changes.
- [x] Add a human-like Criterion workload covering axis updates during a stroke.
- [x] Add a human-like Criterion workload covering tool button press/release.
- [x] Add a human-like Criterion workload covering a tool button held until
  after proximity-out.
- [x] Add a human-like Criterion workload covering direct device removal while a
  tablet tool is in proximity with tip and a tool button active.
- [ ] Cover pressure range configuration.
  - [x] Add a human-like Criterion workload carrying pressure-range-shaped tablet
    axis data through the current event model.
  - [x] Add actual pressure range configuration and deterministic assertions for
    min/max clamping and rescaling (`processing::tablet` tests).
- [ ] Cover tool identity across proximity cycles.
  - [x] Add a human-like Criterion workload with repeated proximity-in/out cycles
    for one logical tool.
  - [ ] Add tool identity fields, serial/type handling, and deterministic coverage
    proving identity remains stable across proximity cycles.
- [x] Add a human-like Criterion workload covering pad button, ring, strip, and
  dial events.
- [x] Add a human-like Criterion workload covering pad controls without any tool
  in proximity.
- [x] Add a human-like Criterion workload covering direct device removal while
  pad buttons and controls are active.
- [x] Resynchronize tablet tool proximity/axis/tip and pad ring/strip after `SYN_DROPPED` from `EVIOCGKEY`/`EVIOCGABS`.
- [x] Cover tablet events in deterministic tests
  (`backend::evdev::tests` covers proximity/axis/tip ordering, proximity-out
  suppression, pad ring/strip/dial, and switch-state seeding).
- [ ] Cover separate tool and pad event nodes grouped as one physical device.
  - [x] Add a human-like Criterion workload carrying separate tool, pad, and touch
    node-shaped events with distinct device indices through the current event
    model.
  - [x] Add a human-like Criterion workload carrying removal of the tablet-touch
    node while related tool and pad node-shaped streams remain active.
  - [ ] Add real device grouping and deterministic coverage that separate event
    nodes are identified as one physical tablet where appropriate.
- [ ] Cover active removal cleanup deterministically. The required assertions
  must prove the final chosen policy for tool tip/button/proximity cleanup,
  pad-button cleanup, tablet-touch contact cleanup, and grouped-node continuation
  after one node disappears.

### Documentation

- [x] Update `FEATURE_PARITY.md`.
- [x] Document which tablet axes are supported and in what units (see
  "Units" above).
- [ ] Document tablet configuration names and defaults.
- [ ] Document unsupported tablet hardware classes if any remain.

## Local References

- `input-rs/src/backend/mod.rs`
- `input-rs/src/backend/evdev.rs`
- `input-rs/src/events/mod.rs`
- `input-rs/src/state.rs`
- `src/input/input_rs_backend.rs`
- `libinput/doc/user/tablet-support.rst`
- `libinput/src/libinput.h`
