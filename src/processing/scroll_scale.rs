use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

/// Scales scroll deltas independently on each axis.
///
/// This is an explicit input-rs extension: libinput deliberately does not
/// accelerate or scale scroll deltas. A positive factor scales, a negative
/// factor inverts that axis, and zero suppresses it. Non-finite factors fall
/// back to 1.0 so malformed configuration cannot produce NaN motion.
#[derive(Debug, Clone, Copy)]
pub struct ScrollScaleFilter {
    x: f64,
    y: f64,
}

impl ScrollScaleFilter {
    pub fn new(x: f64, y: f64) -> Self {
        Self::sanitized(x, y)
    }

    fn sanitized(x: f64, y: f64) -> Self {
        Self {
            x: if x.is_finite() { x } else { 1.0 },
            y: if y.is_finite() { y } else { 1.0 },
        }
    }

    pub fn factors(&self) -> (f64, f64) {
        (self.x, self.y)
    }

    pub fn set_factors(&mut self, x: f64, y: f64) {
        *self = Self::sanitized(x, y);
    }
}

impl Filter for ScrollScaleFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if let InputEvent::PointerScroll { dx, dy, .. } = event {
            *dx *= self.x;
            *dy *= self.y;
        }
        Step::Keep
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            events::{DevIndex, ScrollSource},
            processing::TestFilterExt,
        },
    };

    fn scroll(dx: f64, dy: f64) -> InputEvent {
        InputEvent::PointerScroll {
            device_index: DevIndex(0),
            time_us: 1,
            source: ScrollSource::Finger,
            dx,
            dy,
        }
    }

    #[test]
    fn scales_axes_independently() {
        let mut filter = ScrollScaleFilter::new(2.0, 0.5);
        let result = filter.run(scroll(3.0, 8.0), None, None);
        assert!(matches!(
            result[0],
            InputEvent::PointerScroll {
                dx: 6.0,
                dy: 4.0,
                ..
            }
        ));
    }

    #[test]
    fn negative_factor_inverts_only_that_axis() {
        let mut filter = ScrollScaleFilter::new(-1.0, 1.0);
        let result = filter.run(scroll(3.0, 8.0), None, None);
        assert!(matches!(
            result[0],
            InputEvent::PointerScroll {
                dx: -3.0,
                dy: 8.0,
                ..
            }
        ));
    }

    #[test]
    fn non_finite_factors_fall_back_to_identity() {
        let mut filter = ScrollScaleFilter::new(f64::NAN, f64::INFINITY);
        let result = filter.run(scroll(3.0, 8.0), None, None);
        assert!(matches!(
            result[0],
            InputEvent::PointerScroll {
                dx: 3.0,
                dy: 8.0,
                ..
            }
        ));
    }
}
