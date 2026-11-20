pub mod absolute_transform;
pub mod accel;
pub mod button_map;
pub mod button_scroll;
pub mod button_swap;
pub mod config;
pub mod debounce;
pub mod hysteresis;
pub mod middle_button;
pub mod mouse;
pub mod natural_scroll;
pub mod rotation;
pub mod scroll_scale;
pub mod tablet;
pub mod touchpad;
pub mod touchpad_motion;

pub use smallvec::SmallVec;
use {
    self::{
        absolute_transform::{AbsoluteTransformFilter, CalibrationMatrix},
        accel::ConstantAccelFilter,
        button_map::{ButtonMap, ButtonMapFilter},
        button_scroll::ButtonScrollFilter,
        button_swap::ButtonSwapFilter,
        config::{TouchpadClickMethod, TouchpadScrollMethod},
        debounce::{ButtonDebounceFilter, DebounceConfig},
        hysteresis::HysteresisFilter,
        middle_button::MiddleButtonEmulationFilter,
        mouse::{MouseAccelProfile, MouseConfig, MouseConfigError, MouseFilter},
        natural_scroll::NaturalScrollFilter,
        rotation::{RelativeRotationFilter, RotationError},
        scroll_scale::ScrollScaleFilter,
        tablet::{TabletConfig, TabletTransformFilter},
        touchpad::TouchpadFilter,
        touchpad_motion::TouchpadMotionFilter,
    },
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

// Filters can return multiple events (e.g. a tap emits Press + Release)
pub type FilterResult = SmallVec<InputEvent, 2>;

/// Result of one in-place filter step.
#[derive(Debug)]
pub enum Step {
    /// Event is unchanged or was mutated in place.
    Keep,
    /// Event is dropped.
    Suppress,
    /// Rare path: filter produced multiple events.
    FanOut(Box<FilterResult>),
}

impl Step {
    #[inline]
    pub fn from_result(event: &mut InputEvent, result: FilterResult) -> Self {
        match result.len() {
            0 => Step::Suppress,
            1 => {
                *event = result.into_iter().next().unwrap();
                Step::Keep
            }
            _ => Step::FanOut(Box::new(result)),
        }
    }
}

pub trait Filter {
    /// Process one event in place.
    ///
    /// Filters mutate the event directly and return `Keep`; filters that emit a
    /// different event variant assign it through the reference. Only filters
    /// that genuinely emit multiple events use `FanOut`.
    fn process(
        &mut self,
        event: &mut InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> Step;

    /// Emit any output whose deadline has passed at `now_us`.
    ///
    /// Filters that delay output (middle-button emulation, drag lock) expose
    /// their deadline through [`Filter::next_timeout_us`]. The compositor can
    /// call `Context::dispatch_timeouts` from its event loop timer so those
    /// deadlines do not depend on unrelated input arriving.
    fn dispatch_timeout(&mut self, now_us: u64) -> FilterResult {
        let _ = now_us;
        FilterResult::new()
    }

    /// Absolute microsecond deadline for the next delayed output, if any.
    fn next_timeout_us(&self) -> Option<u64> {
        None
    }
}

pub enum FilterKind {
    Accel(ConstantAccelFilter),
    AbsoluteTransform(AbsoluteTransformFilter),
    Hysteresis(HysteresisFilter),
    TabletTransform(TabletTransformFilter),
    Touchpad(Box<TouchpadFilter>),
    Mouse(Box<MouseFilter>),
    TouchpadMotion(Box<TouchpadMotionFilter>),
    Debounce(ButtonDebounceFilter),
    ButtonMap(ButtonMapFilter),
    ButtonScroll(ButtonScrollFilter),
    ButtonSwap(ButtonSwapFilter),
    MiddleButton(MiddleButtonEmulationFilter),
    NaturalScroll(NaturalScrollFilter),
    ScrollScale(ScrollScaleFilter),
    Rotation(RelativeRotationFilter),
}

impl FilterKind {
    #[inline]
    pub fn process_event(
        &mut self,
        event: &mut InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> Step {
        match self {
            FilterKind::Accel(f) => f.process(event, seat, info),
            FilterKind::AbsoluteTransform(f) => f.process(event, seat, info),
            FilterKind::Hysteresis(f) => f.process(event, seat, info),
            FilterKind::TabletTransform(f) => f.process(event, seat, info),
            FilterKind::Touchpad(f) => f.process(event, seat, info),
            FilterKind::Mouse(f) => f.process(event, seat, info),
            FilterKind::TouchpadMotion(f) => f.process(event, seat, info),
            FilterKind::Debounce(f) => f.process(event, seat, info),
            FilterKind::ButtonScroll(f) => f.process(event, seat, info),
            FilterKind::ButtonSwap(f) => f.process(event, seat, info),
            FilterKind::ButtonMap(f) => f.process(event, seat, info),
            FilterKind::MiddleButton(f) => f.process(event, seat, info),
            FilterKind::NaturalScroll(f) => f.process(event, seat, info),
            FilterKind::ScrollScale(f) => f.process(event, seat, info),
            FilterKind::Rotation(f) => f.process(event, seat, info),
        }
    }

    pub fn dispatch_timeout(&mut self, now_us: u64) -> FilterResult {
        match self {
            FilterKind::TouchpadMotion(f) => f.dispatch_timeout(now_us),
            FilterKind::MiddleButton(f) => f.dispatch_timeout(now_us),
            FilterKind::Debounce(f) => f.dispatch_timeout(now_us),
            _ => FilterResult::new(),
        }
    }

    pub fn next_timeout_us(&self) -> Option<u64> {
        match self {
            FilterKind::TouchpadMotion(f) => f.next_timeout_us(),
            FilterKind::MiddleButton(f) => f.next_timeout_us(),
            FilterKind::Debounce(f) => f.next_timeout_us(),
            _ => None,
        }
    }

    pub fn reset_runtime_state(&mut self) {
        match self {
            FilterKind::Hysteresis(f) => f.reset_runtime_state(),
            FilterKind::Touchpad(f) => f.reset_runtime_state(),
            FilterKind::Mouse(f) => f.reset_runtime_state(),
            FilterKind::TouchpadMotion(f) => f.reset_runtime_state(),
            FilterKind::Debounce(f) => f.reset_runtime_state(),
            FilterKind::ButtonScroll(f) => f.reset_runtime_state(),
            FilterKind::MiddleButton(f) => f.reset_runtime_state(),
            _ => {}
        }
    }
}

/// Owned snapshot of a device's effective configuration.
///
/// This is intended for diagnostics and user-facing inspection. It reports the
/// active filters and their current settings, not just the last values passed
/// to setters.
#[derive(Debug, Clone)]
pub struct DeviceConfigSnapshot {
    pub filter_order: Vec<&'static str>,
    pub touchpad: Option<config::TouchpadConfig>,
    pub mouse: Option<MouseConfig>,
    pub debounce: Option<DebounceConfig>,
    pub middle_button_emulation: bool,
    pub button_scroll: Option<(u32, bool, bool)>,
    pub natural_scroll_mouse: bool,
    pub scroll_scale: Option<(f64, f64)>,
    pub button_map: Option<ButtonMap>,
    pub rotation_degrees: Option<i32>,
    pub calibration: Option<CalibrationMatrix>,
    pub left_handed: bool,
    pub tablet: Option<TabletConfig>,
}

pub struct FilterChain {
    pub(crate) filters: SmallVec<FilterKind, 8>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self {
            filters: SmallVec::new(),
        }
    }

    /// Append a filter explicitly.
    ///
    /// This preserves the caller's ordering. The configuration setters below
    /// call [`FilterChain::normalize_order`] so equivalent configurations
    /// produce the same executable pipeline regardless of the order in which
    /// they were requested.
    pub fn add(&mut self, filter: FilterKind) {
        self.filters.push(filter);
    }

    /// Canonical filter order for configuration-driven chains.
    ///
    /// The order is: calibration → debounce → motion-precision filters →
    /// touchpad recognition → middle-button emulation → button swap →
    /// button mapping → pointer/touchpad acceleration → button scrolling → natural scrolling →
    /// rotation. This makes button scrolling happen before natural-scroll
    /// inversion, keeps debounce ahead of chord detection, and lets delayed
    /// middle-button output still traverse the swap.
    fn order_rank(filter: &FilterKind) -> u8 {
        match filter {
            FilterKind::AbsoluteTransform(_) => 0,
            FilterKind::TabletTransform(_) => 0,
            FilterKind::Debounce(_) => 1,
            FilterKind::Hysteresis(_) => 2,
            FilterKind::Accel(_) => 3,
            FilterKind::TouchpadMotion(_) => 4,
            // Middle-button emulation sees physical (touchpad-mapped) buttons
            // and its delayed output must still traverse later button
            // transforms.
            FilterKind::MiddleButton(_) => 5,
            FilterKind::ButtonSwap(_) => 6,
            FilterKind::ButtonMap(_) => 7,
            FilterKind::Mouse(_) => 8,
            FilterKind::Touchpad(_) => 9,
            FilterKind::ButtonScroll(_) => 10,
            FilterKind::NaturalScroll(_) => 11,
            FilterKind::ScrollScale(_) => 12,
            FilterKind::Rotation(_) => 13,
        }
    }

    fn normalize_order(&mut self) {
        self.filters.sort_by_key(Self::order_rank);
    }

    /// Process an event into a caller-owned result buffer.
    ///
    /// The event is owned once and borrowed mutably by each filter.
    #[inline]
    pub fn process_into(
        &mut self,
        event: InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
        out: &mut FilterResult,
    ) {
        out.clear();
        self.process_remaining(0, event, seat, info, out);
    }

    fn process_remaining(
        &mut self,
        start: usize,
        mut event: InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
        out: &mut FilterResult,
    ) {
        for index in start..self.filters.len() {
            match self.filters[index].process_event(&mut event, seat, info) {
                Step::Keep => {}
                Step::Suppress => return,
                Step::FanOut(events) => {
                    for event in *events {
                        self.process_remaining(index + 1, event, seat, info, out);
                    }
                    return;
                }
            }
        }
        out.push(event);
    }

    pub fn reset_runtime_state(&mut self) {
        for filter in &mut self.filters {
            filter.reset_runtime_state();
        }
    }

    /// Emit output for every filter deadline that has passed.
    ///
    /// Timer-generated events are not special: each one is pushed through the
    /// remaining filters in the chain, exactly like an event originating from
    /// the backend. This keeps downstream button swaps, debouncing, and
    /// scrolling behavior identical for timed and real events.
    pub fn dispatch_timeouts(
        &mut self,
        now_us: u64,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult {
        let mut out = FilterResult::new();
        for index in 0..self.filters.len() {
            out.extend(self.dispatch_timeouts_at(index, now_us, seat, info.clone()));
        }
        out
    }

    /// Dispatch one filter's deadlines and send generated events through the
    /// filters that follow it.
    fn dispatch_timeouts_at(
        &mut self,
        index: usize,
        now_us: u64,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult {
        let mut out = FilterResult::new();
        let generated = self.filters[index].dispatch_timeout(now_us);
        for event in generated {
            self.process_remaining(index + 1, event, seat, info, &mut out);
        }
        out
    }

    /// Flush pending middle-button emulation output before removing or
    /// replacing the filter, so a held/delayed chord does not become a stuck or
    /// spurious button.
    pub fn flush_middle_button(
        &mut self,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult {
        let index = self
            .filters
            .iter()
            .position(|filter| matches!(filter, FilterKind::MiddleButton(_)));
        match index {
            Some(index) => self.dispatch_timeouts_at(index, u64::MAX, seat, info),
            None => FilterResult::new(),
        }
    }

    /// Flush deferred debounce transitions (for example before disabling
    /// debounce).
    pub fn flush_debounce(
        &mut self,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult {
        let index = self
            .filters
            .iter()
            .position(|filter| matches!(filter, FilterKind::Debounce(_)));
        match index {
            Some(index) => self.dispatch_timeouts_at(index, u64::MAX, seat, info),
            None => FilterResult::new(),
        }
    }

    /// Build an owned snapshot of the effective configuration.
    pub fn config_snapshot(&self) -> DeviceConfigSnapshot {
        let mut snapshot = DeviceConfigSnapshot {
            filter_order: Vec::with_capacity(self.filters.len()),
            touchpad: None,
            mouse: None,
            debounce: None,
            middle_button_emulation: false,
            button_scroll: None,
            natural_scroll_mouse: false,
            scroll_scale: None,
            button_map: None,
            rotation_degrees: None,
            calibration: None,
            left_handed: false,
            tablet: None,
        };

        for filter in &self.filters {
            snapshot.filter_order.push(match filter {
                FilterKind::Accel(_) => "accel",
                FilterKind::AbsoluteTransform(_) => "absolute-transform",
                FilterKind::Hysteresis(_) => "hysteresis",
                FilterKind::TabletTransform(_) => "tablet-transform",
                FilterKind::Touchpad(_) => "touchpad-accel",
                FilterKind::Mouse(_) => "mouse",
                FilterKind::TouchpadMotion(_) => "touchpad-motion",
                FilterKind::Debounce(_) => "debounce",
                FilterKind::ButtonScroll(_) => "button-scroll",
                FilterKind::ButtonSwap(_) => "button-swap",
                FilterKind::ButtonMap(_) => "button-map",
                FilterKind::MiddleButton(_) => "middle-button",
                FilterKind::NaturalScroll(_) => "natural-scroll",
                FilterKind::ScrollScale(_) => "scroll-scale",
                FilterKind::Rotation(_) => "rotation",
            });

            match filter {
                FilterKind::Mouse(mouse) => {
                    snapshot.mouse = Some(MouseConfig {
                        dpi: mouse.dpi(),
                        speed_adjustment: mouse.speed_adjustment(),
                        profile: mouse.profile(),
                    });
                }
                FilterKind::Debounce(debounce) => {
                    snapshot.debounce = Some(debounce.config());
                }
                FilterKind::MiddleButton(middle) => {
                    snapshot.middle_button_emulation = middle.is_enabled();
                }
                FilterKind::ButtonScroll(scroll) => {
                    snapshot.button_scroll = Some((
                        scroll.scroll_button(),
                        scroll.lock_enabled(),
                        scroll.is_enabled(),
                    ));
                }
                FilterKind::NaturalScroll(natural) => {
                    snapshot.natural_scroll_mouse = natural.is_enabled();
                }
                FilterKind::ScrollScale(scale) => {
                    snapshot.scroll_scale = Some(scale.factors());
                }
                FilterKind::ButtonMap(map) => {
                    snapshot.button_map = Some(map.map().clone());
                }
                FilterKind::Rotation(rotation) => {
                    snapshot.rotation_degrees = Some(rotation.angle_degrees());
                }
                FilterKind::AbsoluteTransform(transform) => {
                    snapshot.calibration = Some(transform.matrix());
                }
                FilterKind::TabletTransform(transform) => {
                    snapshot.tablet = Some(transform.config());
                }
                _ => {}
            }
        }

        snapshot.touchpad = self.touchpad_config().copied();
        snapshot.left_handed = self.left_handed();
        snapshot
    }

    /// Earliest absolute deadline among filters, if any.
    pub fn next_timeout_us(&self) -> Option<u64> {
        self.filters
            .iter()
            .filter_map(FilterKind::next_timeout_us)
            .min()
    }
}

// ---------------------------------------------------------------------------
// Runtime configuration methods on FilterChain
// ---------------------------------------------------------------------------
// These delegate to the appropriate filter, replacing it in-place
// when structural changes are needed (e.g., profile switch).
// ---------------------------------------------------------------------------

impl FilterChain {
    /// Change the DPI normalization for the mouse filter.
    /// No-op if no mouse filter is present.
    pub fn set_mouse_dpi(&mut self, dpi: i32) {
        if dpi <= 0 {
            return;
        }
        for filter in &mut self.filters {
            if let FilterKind::Mouse(m) = filter {
                let config = MouseConfig {
                    dpi,
                    ..MouseConfig::default_from_existing(m)
                };
                *m = Box::new(MouseFilter::with_config(config));
            }
        }
    }

    /// Change the acceleration profile for the mouse filter.
    /// Replaces the filter in-place.
    ///
    /// Invalid custom curves leave the existing filter untouched and are
    /// logged. Use [`FilterChain::try_set_mouse_profile`] to handle the error.
    pub fn set_mouse_profile(&mut self, profile: MouseAccelProfile) {
        if let Err(error) = self.try_set_mouse_profile(profile) {
            tracing::warn!(%error, "input-rs: leaving mouse profile unchanged");
        }
    }

    /// Change the acceleration profile, validating it before replacing the
    /// filter.
    pub fn try_set_mouse_profile(
        &mut self,
        profile: MouseAccelProfile,
    ) -> Result<(), MouseConfigError> {
        // Validate against every mouse filter first so a rejected curve cannot
        // partially update a chain.
        for filter in &self.filters {
            if let FilterKind::Mouse(mouse) = filter {
                MouseConfig {
                    profile: profile.clone(),
                    ..MouseConfig::default_from_existing(mouse)
                }
                .validate()?;
            }
        }

        for filter in &mut self.filters {
            if let FilterKind::Mouse(mouse) = filter {
                let config = MouseConfig {
                    profile: profile.clone(),
                    ..MouseConfig::default_from_existing(mouse)
                };
                *mouse = Box::new(MouseFilter::try_with_config(config)?);
            }
        }
        Ok(())
    }

    /// Change the speed adjustment for the mouse filter.
    /// Speed is in [-1.0, 1.0]; 0.0 is the default.
    pub fn set_mouse_speed_adjustment(&mut self, speed: f64) {
        let speed = if speed.is_finite() {
            speed.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        for filter in &mut self.filters {
            if let FilterKind::Mouse(m) = filter {
                let config = MouseConfig {
                    speed_adjustment: speed,
                    ..MouseConfig::default_from_existing(m)
                };
                *m = Box::new(MouseFilter::with_config(config));
            }
        }
    }

    /// Enable or disable natural scrolling.
    pub fn set_natural_scroll(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_natural_scrolling(enabled);
            }
        }
    }

    /// Enable or disable tap-to-click.
    pub fn set_tap_to_click(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_tap_to_click(enabled);
            }
        }
    }

    /// Enable or disable two-finger tap right-click.
    pub fn set_two_finger_tap_right_click(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_two_finger_tap_right_click(enabled);
            }
        }
    }

    /// Enable or disable tap-and-drag.
    pub fn set_tap_drag(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_tap_drag(enabled);
            }
        }
    }

    /// Enable or disable tap drag lock.
    pub fn set_tap_drag_lock(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_tap_drag_lock(enabled);
            }
        }
    }

    /// Select a touchpad click method.
    pub fn set_click_method(&mut self, method: TouchpadClickMethod) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_click_method(method);
            }
        }
    }

    /// Select a touchpad scroll method.
    pub fn set_scroll_method(&mut self, method: TouchpadScrollMethod) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_scroll_method(method);
            }
        }
    }

    /// Enable, disable, or replace button scrolling for a pointer device.
    ///
    /// Disabling an active button-scroll filter leaves it in place in a
    /// disengaged state so the eventual release of the swallowed scroll button
    /// is also swallowed. Removing it immediately would emit a release for a
    /// button clients never saw pressed.
    pub fn set_button_scroll(&mut self, enabled: bool, button: u32, lock_enabled: bool) {
        if enabled {
            self.filters
                .retain(|f| !matches!(f, FilterKind::ButtonScroll(_)));
            self.filters.push(FilterKind::ButtonScroll(
                ButtonScrollFilter::new(button).with_lock(lock_enabled),
            ));
        } else if let Some(filter) = self.filters.iter_mut().find_map(|filter| match filter {
            FilterKind::ButtonScroll(scroll) => Some(scroll),
            _ => None,
        }) {
            filter.set_enabled(false);
        }
        self.normalize_order();
    }

    /// Whether middle-button emulation is currently installed.
    pub fn middle_button_emulation_enabled(&self) -> bool {
        self.filters.iter().any(|filter| match filter {
            FilterKind::MiddleButton(middle) => middle.is_enabled(),
            _ => false,
        })
    }

    /// Set or replace an absolute-device calibration matrix.
    pub fn set_calibration_matrix(&mut self, matrix: CalibrationMatrix) {
        self.filters
            .retain(|f| !matches!(f, FilterKind::AbsoluteTransform(_)));
        self.filters.insert(
            0,
            FilterKind::AbsoluteTransform(AbsoluteTransformFilter::new(matrix)),
        );
        self.normalize_order();
    }

    /// Remove any calibration matrix from this device. Returns whether one was
    /// installed.
    pub fn clear_calibration_matrix(&mut self) -> bool {
        let before = self.filters.len();
        self.filters
            .retain(|f| !matches!(f, FilterKind::AbsoluteTransform(_)));
        self.filters.len() != before
    }

    /// Return the installed calibration matrix, if any.
    pub fn calibration_matrix(&self) -> Option<CalibrationMatrix> {
        self.filters.iter().find_map(|filter| match filter {
            FilterKind::AbsoluteTransform(f) => Some(f.matrix()),
            _ => None,
        })
    }

    /// Set or replace tablet transforms (pressure range and active area).
    pub fn set_tablet_config(&mut self, config: TabletConfig) {
        self.filters
            .retain(|filter| !matches!(filter, FilterKind::TabletTransform(_)));
        if config != TabletConfig::default() {
            self.filters
                .push(FilterKind::TabletTransform(TabletTransformFilter::new(
                    config,
                )));
        }
        self.normalize_order();
    }

    /// Current tablet transform configuration, if installed.
    pub fn tablet_config(&self) -> Option<TabletConfig> {
        self.filters.iter().find_map(|filter| match filter {
            FilterKind::TabletTransform(transform) => Some(transform.config()),
            _ => None,
        })
    }

    /// Set or replace the relative-device rotation angle.
    ///
    /// Invalid angles are rejected and leave the existing configuration
    /// untouched.
    pub fn set_rotation(&mut self, angle_degrees: i32) -> Result<(), RotationError> {
        // Validate before touching the chain so a rejected update cannot
        // accidentally remove the current filter.
        let rotation = RelativeRotationFilter::new(angle_degrees)?;
        let existing = self
            .filters
            .iter()
            .position(|f| matches!(f, FilterKind::Rotation(_)));
        match existing {
            Some(pos) => self.filters[pos] = FilterKind::Rotation(rotation),
            // Rotation runs after acceleration and other motion processing.
            None => self.filters.push(FilterKind::Rotation(rotation)),
        }
        self.normalize_order();
        Ok(())
    }

    /// Remove relative-device rotation. Returns whether a rotation filter was
    /// installed.
    pub fn clear_rotation(&mut self) -> bool {
        let before = self.filters.len();
        self.filters
            .retain(|f| !matches!(f, FilterKind::Rotation(_)));
        self.filters.len() != before
    }

    /// Return the configured rotation angle, if any.
    pub fn rotation_angle(&self) -> Option<i32> {
        self.filters.iter().find_map(|filter| match filter {
            FilterKind::Rotation(f) => Some(f.angle_degrees()),
            _ => None,
        })
    }

    /// Set left-handed mode.
    ///
    /// For non-touchpad pointer devices this installs the generic
    /// `ButtonSwapFilter`. For touchpads it applies the touchpad-specific
    /// policy instead: bottom-left/bottom-right software button areas are
    /// swapped, while tap and clickfinger maps stay controlled by their own
    /// maps. This also means the raw `BTN_LEFT` reported by a clickpad reaches
    /// `TouchpadMotionFilter` before any button swap, so button-area and
    /// clickfinger selection still see the physical click.
    pub fn set_left_handed(&mut self, enabled: bool) {
        // Remove any existing generic swap
        self.filters
            .retain(|f| !matches!(f, FilterKind::ButtonSwap(_)));

        let mut has_touchpad = false;
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_left_handed(enabled);
                has_touchpad = true;
            }
        }

        if enabled && !has_touchpad {
            self.filters
                .insert(0, FilterKind::ButtonSwap(ButtonSwapFilter::new()));
        }
        self.normalize_order();
    }

    /// Whether the chain currently applies left-handed button behavior.
    pub fn left_handed(&self) -> bool {
        if let Some(touchpad) = self.touchpad_config() {
            touchpad.left_handed
        } else {
            self.filters
                .iter()
                .any(|filter| matches!(filter, FilterKind::ButtonSwap(_)))
        }
    }

    /// Effective touchpad configuration, if this chain has a touchpad filter.
    pub fn touchpad_config(&self) -> Option<&config::TouchpadConfig> {
        self.filters.iter().find_map(|filter| match filter {
            FilterKind::TouchpadMotion(tm) => Some(tm.config()),
            _ => None,
        })
    }

    /// Apply touchpad defaults from a device profile.
    pub fn apply_touchpad_profile(&mut self, profile: &crate::profile::TouchpadProfile) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.apply_profile(profile);
            }
        }
    }

    /// Enable or disable left+right button chord emulation for middle click.
    ///
    /// Disabling keeps a disengaged filter in the chain so any physical
    /// left/right releases that belong to a chord are swallowed until the
    /// physical buttons are neutral.
    pub fn set_middle_button_emulation(&mut self, enabled: bool) {
        let index = self
            .filters
            .iter()
            .position(|filter| matches!(filter, FilterKind::MiddleButton(_)));
        match (enabled, index) {
            (true, Some(index)) => {
                if let FilterKind::MiddleButton(filter) = &mut self.filters[index] {
                    filter.set_enabled(true);
                }
            }
            (true, None) => {
                self.filters
                    .push(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
            }
            (false, Some(index)) => {
                if let FilterKind::MiddleButton(filter) = &mut self.filters[index] {
                    filter.set_enabled(false);
                }
            }
            (false, None) => {}
        }
        self.normalize_order();
    }

    /// Enable or disable natural scrolling for a mouse/pointer device.
    /// Adds or removes a NaturalScrollFilter that inverts scroll deltas.
    pub fn set_natural_scroll_mouse(&mut self, enabled: bool) {
        // Remove existing natural scroll filter for mice
        self.filters
            .retain(|f| !matches!(f, FilterKind::NaturalScroll(_)));
        if enabled {
            self.filters
                .push(FilterKind::NaturalScroll(NaturalScrollFilter::new(true)));
        }
        self.normalize_order();
    }

    /// Select the two/three-finger tap button map.
    pub fn set_tap_button_map(&mut self, map: config::TapButtonMap) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_tap_button_map(map);
            }
        }
    }

    /// Select the clickfinger button map.
    pub fn set_clickfinger_button_map(&mut self, map: config::ClickfingerButtonMap) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_clickfinger_button_map(map);
            }
        }
    }

    /// Replace touchpad tap timing and movement thresholds.
    pub fn set_tap_timing(
        &mut self,
        tap_timeout_us: u64,
        tap_move_threshold_mm: f64,
        tap_drag_timeout_us: u64,
        drag_lock_timeout_us: u64,
    ) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_tap_timing(
                    tap_timeout_us,
                    tap_move_threshold_mm,
                    tap_drag_timeout_us,
                    drag_lock_timeout_us,
                );
            }
        }
    }

    /// Set the maximum finger count that may produce a tap.
    pub fn set_max_tap_fingers(&mut self, fingers: u8) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_max_tap_fingers(fingers);
            }
        }
    }

    /// Override clickpad detection for this device.
    pub fn set_clickpad_override(&mut self, clickpad: Option<bool>) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_clickpad_override(clickpad);
            }
        }
    }

    /// Set raw pressure/size palm thresholds and the thumb pressure threshold.
    pub fn set_touchpad_palm_thresholds(
        &mut self,
        pressure: Option<f64>,
        size: Option<f64>,
        thumb_pressure: Option<f64>,
    ) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_palm_thresholds(pressure, size, thumb_pressure);
            }
        }
    }

    /// Enable or disable disable-while-trackpointing.
    pub fn set_touchpad_dwtp(&mut self, enabled: bool, timeout_us: u64) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_dwtp(enabled, timeout_us);
            }
        }
    }

    /// Enable or disable stylus-vs-touchpad arbitration.
    pub fn set_touchpad_stylus_arbitration(&mut self, enabled: bool) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_stylus_arbitration(enabled);
            }
        }
    }

    /// Replace touchpad gesture timing and thresholds.
    pub fn set_touchpad_gesture_config(
        &mut self,
        hold_timeout_us: u64,
        motion_threshold_mm: f64,
        pinch_scale_threshold: f64,
        pinch_rotation_threshold_rad: f64,
    ) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_gesture_config(
                    hold_timeout_us,
                    motion_threshold_mm,
                    pinch_scale_threshold,
                    pinch_rotation_threshold_rad,
                );
            }
        }
    }

    /// Set the disable-while-typing suppression timeout.
    pub fn set_touchpad_dwt_timeout(&mut self, timeout_us: u64) {
        for filter in &mut self.filters {
            if let FilterKind::TouchpadMotion(tm) = filter {
                tm.set_dwt_timeout(timeout_us);
            }
        }
    }

    /// Set or clear arbitrary pointer-button remapping.
    ///
    /// An empty map removes the filter.
    pub fn set_button_map(&mut self, map: ButtonMap) {
        self.filters
            .retain(|filter| !matches!(filter, FilterKind::ButtonMap(_)));
        if !map.is_empty() {
            self.filters
                .push(FilterKind::ButtonMap(ButtonMapFilter::new(map)));
        }
        self.normalize_order();
    }

    /// Current button map, if installed.
    pub fn button_map(&self) -> Option<&ButtonMap> {
        self.filters.iter().find_map(|filter| match filter {
            FilterKind::ButtonMap(map) => Some(map.map()),
            _ => None,
        })
    }

    /// Set independent scroll scaling/inversion factors.
    ///
    /// Positive factors scale, negative factors invert, and zero suppresses
    /// that axis. Non-finite factors fall back to identity.
    pub fn set_scroll_scale(&mut self, x: f64, y: f64) {
        if let Some(filter) = self.filters.iter_mut().find_map(|filter| match filter {
            FilterKind::ScrollScale(scale) => Some(scale),
            _ => None,
        }) {
            filter.set_factors(x, y);
        } else {
            self.filters
                .push(FilterKind::ScrollScale(ScrollScaleFilter::new(x, y)));
        }
        self.normalize_order();
    }

    /// Current scroll scale factors, if installed.
    pub fn scroll_scale(&self) -> Option<(f64, f64)> {
        self.filters.iter().find_map(|filter| match filter {
            FilterKind::ScrollScale(scale) => Some(scale.factors()),
            _ => None,
        })
    }

    /// Set debounce configuration.
    ///
    /// An existing debounce filter is updated in place so deferred transitions
    /// keep their deadlines. Callers should flush pending deadlines after
    /// disabling debounce.
    pub fn set_debounce(&mut self, config: DebounceConfig) {
        if let Some(filter) = self.filters.iter_mut().find_map(|filter| match filter {
            FilterKind::Debounce(debounce) => Some(debounce),
            _ => None,
        }) {
            filter.set_config(config);
        } else {
            self.filters
                .push(FilterKind::Debounce(ButtonDebounceFilter::new(config)));
        }
        self.normalize_order();
    }

    /// Seed debounce state for a code that is already held, used when
    /// debounce is enabled while a button or key is down.
    pub fn seed_debounce_state(&mut self, code: u32, is_key: bool) {
        if let Some(filter) = self.filters.iter_mut().find_map(|filter| match filter {
            FilterKind::Debounce(debounce) => Some(debounce),
            _ => None,
        }) {
            filter.seed_down(code, is_key);
        }
    }
}

#[cfg(test)]
pub trait TestFilterExt {
    fn run(
        &mut self,
        event: InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult;
}

#[cfg(test)]
impl<T: Filter> TestFilterExt for T {
    fn run(
        &mut self,
        mut event: InputEvent,
        seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> FilterResult {
        let mut out = FilterResult::new();
        match Filter::process(self, &mut event, seat, info) {
            Step::Keep => out.push(event),
            Step::Suppress => {}
            Step::FanOut(events) => out.extend(*events),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::events::DevIndex};

    trait ChainExt {
        fn collect(
            &mut self,
            event: InputEvent,
            seat: Option<&Seat>,
            info: Option<&DeviceInfo>,
        ) -> FilterResult;
    }

    impl ChainExt for FilterChain {
        fn collect(
            &mut self,
            event: InputEvent,
            seat: Option<&Seat>,
            info: Option<&DeviceInfo>,
        ) -> FilterResult {
            let mut out = FilterResult::new();
            self.process_into(event, seat, info, &mut out);
            out
        }
    }

    fn middle_press(time_us: u64) -> InputEvent {
        InputEvent::PointerButton {
            device_index: DevIndex(0),
            time_us,
            button: 0x110, // BTN_LEFT
            pressed: true,
        }
    }

    fn scroll_chain(natural_first: bool) -> FilterChain {
        let mut chain = FilterChain::new();
        if natural_first {
            chain.set_natural_scroll_mouse(true);
            chain.set_button_scroll(true, 0x112, false);
        } else {
            chain.set_button_scroll(true, 0x112, false);
            chain.set_natural_scroll_mouse(true);
        }
        chain
    }

    fn button_scroll_result(chain: &mut FilterChain) -> (f64, f64) {
        // Hold the middle button to enter button scrolling.
        chain.collect(
            InputEvent::PointerButton {
                device_index: DevIndex(0),
                time_us: 0,
                button: 0x112,
                pressed: true,
            },
            None,
            None,
        );
        let events = chain.collect(
            InputEvent::PointerMotion {
                device_index: DevIndex(0),
                time_us: 1,
                dx: 10.0,
                dy: 5.0,
                dx_unaccelerated: 10.0,
                dy_unaccelerated: 5.0,
            },
            None,
            None,
        );
        match &events[0] {
            InputEvent::PointerScroll { dx, dy, .. } => (*dx, *dy),
            other => panic!("expected scroll, got {other:?}"),
        }
    }

    #[test]
    fn configuration_order_does_not_change_pipeline_semantics() {
        let mut natural_first = scroll_chain(true);
        let mut button_first = scroll_chain(false);
        assert_eq!(
            button_scroll_result(&mut natural_first),
            (-10.0, -5.0),
            "button-scroll output must be inverted by natural scrolling"
        );
        assert_eq!(button_scroll_result(&mut button_first), (-10.0, -5.0));
    }

    #[test]
    fn disabling_button_scroll_does_not_leak_a_release() {
        let mut chain = FilterChain::new();
        chain.set_button_scroll(true, 0x112, false);

        let press = |pressed| InputEvent::PointerButton {
            device_index: DevIndex(0),
            time_us: 1,
            button: 0x112,
            pressed,
        };

        // The scroll button press is swallowed by design.
        assert!(chain.collect(press(true), None, None).is_empty());
        chain.set_button_scroll(false, 0x112, false);

        // Its physical release must still be swallowed because clients never
        // saw the press.
        assert!(chain.collect(press(false), None, None).is_empty());

        // A later press passes through once the scroll button is neutral.
        let events = chain.collect(press(true), None, None);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                button: 0x112,
                pressed: true,
                ..
            }
        ));
    }

    fn debounced_chain() -> FilterChain {
        let mut chain = FilterChain::new();
        chain.set_debounce(DebounceConfig {
            enabled: true,
            bounce_timeout_us: 25_000,
            spurious_timeout_us: 12_000,
        });
        chain.set_left_handed(true);
        chain
    }

    fn press_left(pressed: bool) -> InputEvent {
        InputEvent::PointerButton {
            device_index: DevIndex(0),
            time_us: 0,
            button: 0x110,
            pressed,
        }
    }

    fn prime_deferred_release(chain: &mut FilterChain) {
        let events = chain.collect(press_left(true), None, None);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                button: 0x111,
                pressed: true,
                ..
            }
        ));
        assert!(chain.collect(press_left(false), None, None).is_empty());
    }

    #[test]
    fn debounce_flush_emits_the_deferred_release() {
        let mut chain = debounced_chain();
        prime_deferred_release(&mut chain);

        let index = chain
            .filters
            .iter()
            .position(|f| matches!(f, FilterKind::Debounce(_)))
            .unwrap();
        let direct = match &mut chain.filters[index] {
            FilterKind::Debounce(debounce) => debounce.dispatch_timeout(u64::MAX),
            _ => unreachable!(),
        };
        assert_eq!(direct.len(), 1);
    }

    #[test]
    fn debounce_flush_passes_through_remaining_filters() {
        let mut chain = debounced_chain();
        prime_deferred_release(&mut chain);
        assert!(chain.next_timeout_us().is_some(), "release must be pending");

        let events = chain.flush_debounce(None, None);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                button: 0x111,
                pressed: false,
                ..
            }
        ));
    }

    #[test]
    fn config_snapshot_reports_effective_settings() {
        let mut chain = FilterChain::new();
        chain.set_debounce(DebounceConfig {
            enabled: true,
            bounce_timeout_us: 30_000,
            spurious_timeout_us: 9_000,
        });
        chain.set_left_handed(true);
        chain.set_button_map(ButtonMap::new([(0x113, 0x110)]));
        chain.set_scroll_scale(2.0, -1.0);
        chain.set_rotation(90).unwrap();
        chain.set_middle_button_emulation(true);

        let snapshot = chain.config_snapshot();
        assert_eq!(snapshot.debounce.unwrap().bounce_timeout_us, 30_000);
        assert!(snapshot.left_handed);
        assert_eq!(snapshot.button_map.unwrap().get(0x113), Some(0x110));
        assert_eq!(snapshot.scroll_scale, Some((2.0, -1.0)));
        assert_eq!(snapshot.rotation_degrees, Some(90));
        assert!(snapshot.middle_button_emulation);
        assert!(snapshot.filter_order.contains(&"button-map"));
        assert!(snapshot.filter_order.contains(&"rotation"));
    }

    #[test]
    fn button_map_runs_after_left_handed_swap() {
        let mut chain = FilterChain::new();
        chain.set_left_handed(true);
        chain.set_button_map(ButtonMap::new([(0x111, 0x113)])); // right → side

        let events = chain.collect(middle_press(0), None, None);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                button: 0x113,
                pressed: true,
                ..
            }
        ));
    }

    #[test]
    fn canonical_order_swaps_timeout_generated_middle_fallback() {
        let mut chain = FilterChain::new();
        chain.set_left_handed(true);
        chain.set_middle_button_emulation(true);

        // Left press is held for chord detection.
        assert!(chain.collect(middle_press(0), None, None).is_empty());

        // Timeout emits the original left press, which must still be swapped
        // to right because button swap follows middle emulation.
        let events = chain.dispatch_timeouts(60_000, None, None);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                button: 0x111,
                pressed: true,
                ..
            }
        ));
    }

    #[test]
    fn timeout_generated_events_pass_through_remaining_filters() {
        let mut chain = FilterChain::new();
        chain.add(FilterKind::MiddleButton(MiddleButtonEmulationFilter::new()));
        chain.add(FilterKind::ButtonSwap(ButtonSwapFilter::new()));

        // Middle-button emulation holds the press for chord detection.
        assert!(chain.collect(middle_press(0), None, None).is_empty());

        // On timeout it emits BTN_LEFT. That event must still flow through the
        // downstream button swap, becoming BTN_RIGHT.
        let events = chain.dispatch_timeouts(60_000, None, None);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            InputEvent::PointerButton {
                button: 0x111, // BTN_RIGHT
                pressed: true,
                ..
            }
        ));
    }
}
