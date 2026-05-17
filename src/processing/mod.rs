pub mod accel;
pub mod button_swap;
pub mod natural_scroll;
pub mod config;
pub mod debounce;
pub mod hysteresis;
pub mod mouse;
pub mod touchpad;
pub mod touchpad_motion;

use crate::events::InputEvent;
use crate::state::Seat;
use crate::backend::DeviceInfo;
pub use smallvec::SmallVec;
use self::accel::ConstantAccelFilter;
use self::button_swap::ButtonSwapFilter;
use self::natural_scroll::NaturalScrollFilter;
use self::debounce::ButtonDebounceFilter;
use self::debounce::DebounceConfig;
use self::hysteresis::HysteresisFilter;
use self::mouse::{MouseFilter, MouseConfig, MouseAccelProfile};
use self::touchpad::TouchpadFilter;
use self::touchpad_motion::TouchpadMotionFilter;

// Filters can return multiple events (e.g. a tap emits Press + Release)
pub type FilterResult = SmallVec<InputEvent, 4>;

pub trait Filter {
    fn process(&mut self, event: InputEvent, seat: Option<&Seat>, info: Option<DeviceInfo>) -> FilterResult;
}

pub enum FilterKind {
    Accel(ConstantAccelFilter),
    Hysteresis(HysteresisFilter),
    Touchpad(Box<TouchpadFilter>),
    Mouse(Box<MouseFilter>),
    TouchpadMotion(Box<TouchpadMotionFilter>),
    Debounce(ButtonDebounceFilter),
    ButtonSwap(ButtonSwapFilter),
    NaturalScroll(NaturalScrollFilter),
}

impl FilterKind {
    #[inline(always)]
    pub fn process_event(&mut self, event: InputEvent, seat: Option<&Seat>, info: Option<DeviceInfo>) -> FilterResult {
        match self {
            FilterKind::Accel(f) => f.process(event, seat, info),
            FilterKind::Hysteresis(f) => f.process(event, seat, info),
            FilterKind::Touchpad(f) => f.process(event, seat, info),
            FilterKind::Mouse(f) => f.process(event, seat, info),
            FilterKind::TouchpadMotion(f) => f.process(event, seat, info),
            FilterKind::Debounce(f) => f.process(event, seat, info),
            FilterKind::ButtonSwap(f) => f.process(event, seat, info),
            FilterKind::NaturalScroll(f) => f.process(event, seat, info),
        }
    }
}

pub struct FilterChain {
    pub(crate) filters: SmallVec<FilterKind, 8>,
}

impl FilterChain {
    pub fn new() -> Self {
        Self { filters: SmallVec::new() }
    }

    pub fn add(&mut self, filter: FilterKind) {
        self.filters.push(filter);
    }

    /// Process an event through all filters in the chain.
    ///
    /// Reuses internal buffers across calls to avoid per-event heap allocation.
    /// For single-filter chains containing only a Mouse filter, the fast path
    /// skips the fan-out loop entirely.
    pub fn process(&mut self, event: InputEvent, seat: Option<&Seat>, info: Option<DeviceInfo>) -> FilterResult {
        // Fast path: single filter, no fan-out needed.
        // Most mouse devices have exactly one filter.
        if self.filters.len() == 1 {
            return self.filters[0].process_event(event, seat, info);
        }

        let mut current = FilterResult::new();
        current.push(event);

        for filter in &mut self.filters {
            let mut next = FilterResult::new();
            for ev in current {
                next.extend(filter.process_event(ev, seat, info.clone()));
            }
            current = next;
            if current.is_empty() {
                break;
            }
        }

        current
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
    pub fn set_mouse_profile(&mut self, profile: MouseAccelProfile) {
        for filter in &mut self.filters {
            if let FilterKind::Mouse(m) = filter {
                let config = MouseConfig {
                    profile: profile.clone(),
                    ..MouseConfig::default_from_existing(m)
                };
                *m = Box::new(MouseFilter::with_config(config));
            }
        }
    }

    /// Change the speed adjustment for the mouse filter.
    /// Speed is in [-1.0, 1.0]; 0.0 is the default.
    pub fn set_mouse_speed_adjustment(&mut self, speed: f64) {
        let speed = speed.clamp(-1.0, 1.0);
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

    /// Set left-handed mode (swaps left/right mouse buttons).
    pub fn set_left_handed(&mut self, enabled: bool) {
        // Remove any existing swap
        self.filters.retain(|f| !matches!(f, FilterKind::ButtonSwap(_)));
        if enabled {
            self.filters.insert(0, FilterKind::ButtonSwap(ButtonSwapFilter::new()));
        }
    }

    /// Enable or disable natural scrolling for a mouse/pointer device.
    /// Adds or removes a NaturalScrollFilter that inverts scroll deltas.
    pub fn set_natural_scroll_mouse(&mut self, enabled: bool) {
        // Remove existing natural scroll filter for mice
        self.filters.retain(|f| !matches!(f, FilterKind::NaturalScroll(_)));
        if enabled {
            // Natural scroll filter goes near the end, after motion processing
            self.filters.push(FilterKind::NaturalScroll(NaturalScrollFilter::new(true)));
        }
    }

    /// Set debounce configuration.
    pub fn set_debounce(&mut self, config: DebounceConfig) {
        let new_filter = FilterKind::Debounce(ButtonDebounceFilter::new(config));
        let pos = self.filters.iter().position(|f| matches!(f, FilterKind::Debounce(_)));
        if let Some(pos) = pos {
            self.filters[pos] = new_filter;
        } else {
            self.filters.push(new_filter);
        }
    }
}
