# Feature: Tablet Tool And Pad

## Gap

Libinput has dedicated support for graphics tablet tools and tablet pads.
Tablet tools provide proximity, tip, axis, pressure, tilt, distance, wheel, and
button events. Tablet pads provide buttons, keys, rings, strips, and dials.

`input-rs` defines tablet and tablet-pad capability flags. It now also has
tablet tool and tablet pad event variants, plus a human-like benchmark sequence
using those current event types. Evdev detection still does not populate tablet
capabilities, evdev processing does not emit tablet events, and jay-side tablet
protocol bridging is not wired.

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

- [ ] Detect tablet tool devices from evdev capabilities instead of only
  defining `DeviceCaps::TABLET`.
- [ ] Detect tablet pad devices instead of only defining
  `DeviceCaps::TABLET_PAD`.
- [ ] Avoid classifying ordinary touchpads or touchscreens as tablet tools.
- [ ] Support devices where tool and pad are separate event nodes.
- [ ] Support devices where tablet tool and pad capabilities appear on the same
  node.
- [ ] Preserve vendor, product, bus, name, and grouping metadata for tablet
  devices.

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
- [ ] Tool events must carry device index, timestamp, tool identity, and all
  changed axes. The current variants carry device index, timestamp, and basic
  axis/state data, but not tool identity or changed-axis masks.
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

### Proximity And Tip Handling

- [ ] Emit proximity-in before axis/tip events that depend on an active tool.
- [ ] Emit proximity-out when the tool leaves range.
- [ ] Release any pressed tool buttons before proximity-out if required to keep
  compositor state coherent.
  - [x] Add a human-like Criterion workload where a tool button remains pressed
    through proximity-out in the current event model.
- [ ] Define tool-device removal cleanup. If the tablet tool node disappears
  while a tool is in proximity, the final policy must say whether input-rs emits
  synthetic tip-up, tool-button-up, and proximity-out events before
  `DeviceRemoved`, or whether the compositor bridge consumes `DeviceRemoved` as
  the cleanup boundary and releases all per-tool state there.
  - [x] Add a human-like Criterion workload where direct `DeviceRemoved` arrives
    while proximity, tip, and a tool button are active in the current event model.
- [ ] Derive logical tip down/up from pressure, distance, button state, or
  device-specific rules.
- [ ] Avoid phantom tip events from worn tools or pressure offsets.
- [ ] Handle tools that are always in proximity.

### Tool Identity

- [ ] Represent tool type, such as pen, eraser, brush, pencil, airbrush, mouse,
  lens, or unknown.
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
- [ ] Pad events must not require a tool to be in proximity.
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

- [ ] Add pressure range configuration for tablet tools.
- [ ] Add eraser button remapping where hardware reports a firmware eraser
  button.
- [ ] Add active area configuration for indirect tablets.
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
  - [ ] Add actual pressure range configuration and deterministic assertions for
    min/max clamping and rescaling.
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
- [ ] Cover tablet events in deterministic tests or replay fixtures.
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
- [ ] Document which tablet axes are supported and in what units.
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
