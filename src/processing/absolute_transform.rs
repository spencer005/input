use {
    super::{Filter, Step},
    crate::{
        backend::DeviceInfo,
        events::{InputEvent, TouchData},
        state::Seat,
    },
};

/// Affine calibration matrix for absolute coordinates.
///
/// Values are `[a, b, c, d, e, f]`, applied to device-normalized coordinates:
/// `x' = a*x + b*y + c`, `y' = d*x + e*y + f`. The result is clamped to the
/// normalized device range and then converted back to the device's coordinate
/// units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationMatrix {
    values: [f64; 6],
}

impl CalibrationMatrix {
    pub fn identity() -> Self {
        Self {
            values: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        }
    }

    pub fn new(values: [f64; 6]) -> Option<Self> {
        if values.iter().all(|v| v.is_finite()) {
            Some(Self { values })
        } else {
            None
        }
    }

    /// The six affine values in `[a, b, c, d, e, f]` order.
    pub fn values(&self) -> [f64; 6] {
        self.values
    }

    fn transform_normalized(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.values;
        (
            (a * x + b * y + c).clamp(0.0, 1.0),
            (d * x + e * y + f).clamp(0.0, 1.0),
        )
    }
}

pub struct AbsoluteTransformFilter {
    matrix: CalibrationMatrix,
}

impl AbsoluteTransformFilter {
    pub fn new(matrix: CalibrationMatrix) -> Self {
        Self { matrix }
    }

    /// Return the installed calibration matrix.
    pub fn matrix(&self) -> CalibrationMatrix {
        self.matrix
    }

    fn transform_touch(&self, data: &mut TouchData, info: Option<&DeviceInfo>) {
        let Some(info) = info else {
            return;
        };
        let width = info.x.max - info.x.min;
        let height = info.y.max - info.y.min;
        if width <= 0.0 || height <= 0.0 {
            return;
        }

        let x = (data.x - info.x.min) / width;
        let y = (data.y - info.y.min) / height;
        let (x, y) = self.matrix.transform_normalized(x, y);
        data.x = info.x.min + x * width;
        data.y = info.y.min + y * height;
    }
}

impl Filter for AbsoluteTransformFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> Step {
        match event {
            InputEvent::TouchDown(data) | InputEvent::TouchMotion(data) => {
                self.transform_touch(data, info);
            }
            _ => {}
        }
        Step::Keep
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{backend::AxisInfo, events::DevIndex, processing::TestFilterExt},
    };

    fn info() -> DeviceInfo {
        DeviceInfo {
            x: AxisInfo {
                min: 0.0,
                max: 1000.0,
                resolution: 10.0,
            },
            y: AxisInfo {
                min: 0.0,
                max: 1000.0,
                resolution: 10.0,
            },
            ..Default::default()
        }
    }

    fn touch(x: f64, y: f64) -> InputEvent {
        InputEvent::TouchDown(Box::new(TouchData {
            device_index: DevIndex(0),
            time_us: 1,
            slot: 0,
            x,
            y,
            pressure: None,
            touch_major: None,
            touch_minor: None,
            tool_type: None,
            tracking_id: None,
        }))
    }

    fn transform(matrix: [f64; 6], x: f64, y: f64) -> (f64, f64) {
        let mut filter = AbsoluteTransformFilter::new(CalibrationMatrix::new(matrix).unwrap());
        let out = filter.run(touch(x, y), None, Some(&info()));
        match &out[0] {
            InputEvent::TouchDown(data) => (data.x, data.y),
            other => panic!("expected touch down, got {other:?}"),
        }
    }

    #[test]
    fn identity_preserves_coordinates() {
        let (x, y) = transform([1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 250.0, 500.0);
        assert!((x - 250.0).abs() < 1e-9);
        assert!((y - 500.0).abs() < 1e-9);
    }

    #[test]
    fn x_and_y_inversion() {
        assert_eq!(
            transform([-1.0, 0.0, 1.0, 0.0, 1.0, 0.0], 250.0, 500.0).0,
            750.0
        );
        assert_eq!(
            transform([1.0, 0.0, 0.0, 0.0, -1.0, 1.0], 250.0, 500.0).1,
            500.0
        );
        assert_eq!(
            transform([1.0, 0.0, 0.0, 0.0, -1.0, 1.0], 250.0, 0.0).1,
            1000.0
        );
    }

    #[test]
    fn ninety_degree_rotation() {
        let (x, y) = transform([0.0, 1.0, 0.0, -1.0, 0.0, 1.0], 250.0, 500.0);
        assert!((x - 500.0).abs() < 1e-9);
        assert!((y - 750.0).abs() < 1e-9);
    }

    #[test]
    fn out_of_range_results_are_clamped() {
        let (x, y) = transform([1.0, 0.0, 2.0, 0.0, 1.0, -2.0], 250.0, 500.0);
        assert_eq!(x, 1000.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn non_finite_matrix_is_rejected() {
        assert!(CalibrationMatrix::new([f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0]).is_none());
        assert!(CalibrationMatrix::new([1.0, 0.0, f64::INFINITY, 0.0, 1.0, 0.0]).is_none());
    }

    #[test]
    fn matrix_query_returns_installed_values() {
        let values = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let filter = AbsoluteTransformFilter::new(CalibrationMatrix::new(values).unwrap());
        assert_eq!(filter.matrix().values(), values);
    }
}
