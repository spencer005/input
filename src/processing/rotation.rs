use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

/// Validation error for relative-device rotation configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationError {
    /// The angle is outside the accepted `0..=359` degree range.
    OutOfRange(i32),
}

impl std::fmt::Display for RotationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RotationError::OutOfRange(angle) => {
                write!(f, "rotation angle {angle} is outside 0..=359 degrees")
            }
        }
    }
}

impl std::error::Error for RotationError {}

/// Rotates relative pointer motion deltas by a configured angle.
///
/// The angle is expressed in degrees, clockwise in screen coordinates (x to the
/// right, y down). libinput's rotation configuration is intended for relative
/// devices such as trackballs whose physical orientation does not match the
/// display; the same semantics apply here.
///
/// Rotation is applied after acceleration and to both the accelerated and
/// unaccelerated delta streams, so raw-input clients see the same orientation
/// as normal clients. Scroll deltas are not rotated; libinput does not rotate
/// wheel events, and rotating a wheel axis would silently swap vertical and
/// horizontal scrolling.
#[derive(Debug, Clone, Copy)]
pub struct RelativeRotationFilter {
    angle_degrees: i32,
    cos: f64,
    sin: f64,
}

impl RelativeRotationFilter {
    pub fn new(angle_degrees: i32) -> Result<Self, RotationError> {
        let angle_degrees = Self::validate(angle_degrees)?;
        Ok(Self {
            angle_degrees,
            cos: (angle_degrees as f64).to_radians().cos(),
            sin: (angle_degrees as f64).to_radians().sin(),
        })
    }

    fn validate(angle_degrees: i32) -> Result<i32, RotationError> {
        if (0..=359).contains(&angle_degrees) {
            Ok(angle_degrees)
        } else {
            Err(RotationError::OutOfRange(angle_degrees))
        }
    }

    pub fn angle_degrees(&self) -> i32 {
        self.angle_degrees
    }

    /// Replace the rotation angle.
    ///
    /// An invalid angle leaves the existing filter configuration untouched so a
    /// rejected runtime update cannot silently change pointer behavior.
    pub fn set_angle(&mut self, angle_degrees: i32) -> Result<(), RotationError> {
        let angle_degrees = Self::validate(angle_degrees)?;
        self.angle_degrees = angle_degrees;
        self.cos = (angle_degrees as f64).to_radians().cos();
        self.sin = (angle_degrees as f64).to_radians().sin();
        Ok(())
    }

    #[inline]
    fn rotate(&self, dx: f64, dy: f64) -> (f64, f64) {
        (dx * self.cos - dy * self.sin, dx * self.sin + dy * self.cos)
    }
}

impl Filter for RelativeRotationFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if let InputEvent::PointerMotion {
            dx,
            dy,
            dx_unaccelerated,
            dy_unaccelerated,
            ..
        } = event
        {
            let (new_dx, new_dy) = self.rotate(*dx, *dy);
            let (new_ux, new_uy) = self.rotate(*dx_unaccelerated, *dy_unaccelerated);
            *dx = new_dx;
            *dy = new_dy;
            *dx_unaccelerated = new_ux;
            *dy_unaccelerated = new_uy;
        }
        Step::Keep
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{events::DevIndex, processing::TestFilterExt},
    };

    fn motion(dx: f64, dy: f64) -> InputEvent {
        InputEvent::PointerMotion {
            device_index: DevIndex(0),
            time_us: 1,
            dx,
            dy,
            dx_unaccelerated: dx,
            dy_unaccelerated: dy,
        }
    }

    fn run(filter: &mut RelativeRotationFilter, dx: f64, dy: f64) -> (f64, f64, f64, f64) {
        let result = filter.run(motion(dx, dy), None, None);
        match &result[0] {
            InputEvent::PointerMotion {
                dx,
                dy,
                dx_unaccelerated,
                dy_unaccelerated,
                ..
            } => (*dx, *dy, *dx_unaccelerated, *dy_unaccelerated),
            other => panic!("expected motion, got {other:?}"),
        }
    }

    #[test]
    fn identity_rotation_preserves_motion() {
        let mut filter = RelativeRotationFilter::new(0).unwrap();
        let (dx, dy, ux, uy) = run(&mut filter, 3.5, -2.0);
        assert_eq!((dx, dy, ux, uy), (3.5, -2.0, 3.5, -2.0));
    }

    #[test]
    fn ninety_degree_rotation_clockwise() {
        let mut filter = RelativeRotationFilter::new(90).unwrap();
        let (dx, dy, ux, uy) = run(&mut filter, 1.0, 0.0);
        assert!(dx.abs() < 1e-12);
        assert!((dy - 1.0).abs() < 1e-12);
        assert!(ux.abs() < 1e-12);
        assert!((uy - 1.0).abs() < 1e-12);
    }

    #[test]
    fn one_eighty_degree_rotation_inverts_motion() {
        let mut filter = RelativeRotationFilter::new(180).unwrap();
        let (dx, dy, ux, uy) = run(&mut filter, 2.0, -4.0);
        assert!((dx + 2.0).abs() < 1e-12);
        assert!((dy - 4.0).abs() < 1e-12);
        assert!((ux + 2.0).abs() < 1e-12);
        assert!((uy - 4.0).abs() < 1e-12);
    }

    #[test]
    fn arbitrary_rotation_preserves_magnitude() {
        let mut filter = RelativeRotationFilter::new(37).unwrap();
        let (dx, dy, ux, uy) = run(&mut filter, 3.0, 4.0);
        assert!((dx.hypot(dy) - 5.0).abs() < 1e-12);
        assert!((ux.hypot(uy) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn scroll_is_not_rotated() {
        let mut filter = RelativeRotationFilter::new(90).unwrap();
        let result = filter.run(
            InputEvent::PointerScroll {
                device_index: DevIndex(0),
                time_us: 1,
                source: crate::events::ScrollSource::Wheel,
                dx: 1.0,
                dy: 2.0,
            },
            None,
            None,
        );
        assert!(matches!(
            result[0],
            InputEvent::PointerScroll {
                dx: 1.0,
                dy: 2.0,
                ..
            }
        ));
    }

    #[test]
    fn invalid_angle_is_rejected_and_does_not_change_existing_config() {
        assert_eq!(
            RelativeRotationFilter::new(360).unwrap_err(),
            RotationError::OutOfRange(360)
        );
        assert_eq!(
            RelativeRotationFilter::new(-1).unwrap_err(),
            RotationError::OutOfRange(-1)
        );

        let mut filter = RelativeRotationFilter::new(90).unwrap();
        assert_eq!(
            filter.set_angle(400).unwrap_err(),
            RotationError::OutOfRange(400)
        );
        assert_eq!(filter.angle_degrees(), 90);
        let (dx, dy, _, _) = run(&mut filter, 1.0, 0.0);
        assert!(dx.abs() < 1e-12);
        assert!((dy - 1.0).abs() < 1e-12);
    }
}
