use {
    super::{Filter, FilterResult},
    crate::{
        backend::DeviceInfo,
        events::{InputEvent, TouchData},
        state::Seat,
    },
    smallvec::smallvec,
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

    fn transform_touch(&self, mut data: TouchData, info: Option<DeviceInfo>) -> TouchData {
        let Some(info) = info else {
            return data;
        };
        let width = info.x.max - info.x.min;
        let height = info.y.max - info.y.min;
        if width <= 0.0 || height <= 0.0 {
            return data;
        }

        let x = (data.x - info.x.min) / width;
        let y = (data.y - info.y.min) / height;
        let (x, y) = self.matrix.transform_normalized(x, y);
        data.x = info.x.min + x * width;
        data.y = info.y.min + y * height;
        data
    }
}

impl Filter for AbsoluteTransformFilter {
    fn process(
        &mut self,
        event: InputEvent,
        _seat: Option<&Seat>,
        info: Option<DeviceInfo>,
    ) -> FilterResult {
        match event {
            InputEvent::TouchDown(data) => {
                smallvec![InputEvent::TouchDown(Box::new(
                    self.transform_touch(*data, info),
                ))]
            }
            InputEvent::TouchMotion(data) => {
                smallvec![InputEvent::TouchMotion(Box::new(
                    self.transform_touch(*data, info),
                ))]
            }
            other => smallvec![other],
        }
    }
}
