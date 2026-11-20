use {
    super::{Filter, Step},
    crate::{
        backend::DeviceInfo,
        events::{InputEvent, TabletToolAxisData, TabletToolProximityData},
        state::Seat,
    },
};

/// Normalized rectangle in the range `[0, 1]` on each axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveArea {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl ActiveArea {
    pub fn new(x_min: f64, x_max: f64, y_min: f64, y_max: f64) -> Option<Self> {
        let area = Self {
            x_min,
            x_max,
            y_min,
            y_max,
        };
        area.is_valid().then_some(area)
    }

    fn is_valid(self) -> bool {
        self.x_min.is_finite()
            && self.x_max.is_finite()
            && self.y_min.is_finite()
            && self.y_max.is_finite()
            && self.x_min >= 0.0
            && self.x_max <= 1.0
            && self.y_min >= 0.0
            && self.y_max <= 1.0
            && self.x_min < self.x_max
            && self.y_min < self.y_max
    }
}

/// Tablet-specific value transforms.
///
/// Coordinates are expected in raw evdev units and are mapped through the
/// device axis bounds. The active area is a normalized rectangle: input outside
/// it is clamped to the nearest edge, and the rectangle is mapped to the full
/// logical `[0, 1]` range.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TabletConfig {
    /// Raw pressure range mapped to logical `[0, 1]`. `None` leaves pressure
    /// untouched.
    pub pressure_range: Option<(f64, f64)>,
    /// Optional normalized active area.
    pub active_area: Option<ActiveArea>,
}

impl TabletConfig {
    pub fn sanitized(self) -> Self {
        let pressure_range = self
            .pressure_range
            .filter(|(min, max)| min.is_finite() && max.is_finite() && *min < *max);
        let active_area = self.active_area.filter(|area| area.is_valid());
        Self {
            pressure_range,
            active_area,
        }
    }
}

pub struct TabletTransformFilter {
    config: TabletConfig,
}

impl TabletTransformFilter {
    pub fn new(config: TabletConfig) -> Self {
        Self {
            config: config.sanitized(),
        }
    }

    pub fn config(&self) -> TabletConfig {
        self.config
    }

    pub fn set_config(&mut self, config: TabletConfig) {
        self.config = config.sanitized();
    }

    fn map_axis(value: f64, min: f64, max: f64, low: f64, high: f64) -> f64 {
        if !(max > min) {
            return value;
        }
        let normalized = ((value - min) / (max - min)).clamp(low, high);
        let scaled = (normalized - low) / (high - low);
        min + scaled * (max - min)
    }

    fn transform_position(&self, x: &mut f64, y: &mut f64, info: Option<&DeviceInfo>) {
        let (Some(area), Some(info)) = (self.config.active_area, info) else {
            return;
        };
        if info.x.max <= info.x.min || info.y.max <= info.y.min {
            return;
        }
        *x = Self::map_axis(*x, info.x.min, info.x.max, area.x_min, area.x_max);
        *y = Self::map_axis(*y, info.y.min, info.y.max, area.y_min, area.y_max);
    }

    fn transform_pressure(&self, pressure: &mut f64) {
        let Some((min, max)) = self.config.pressure_range else {
            return;
        };
        *pressure = ((*pressure - min) / (max - min)).clamp(0.0, 1.0);
    }

    fn transform_proximity(&self, data: &mut TabletToolProximityData, info: Option<&DeviceInfo>) {
        self.transform_position(&mut data.x, &mut data.y, info);
    }

    fn transform_axis(&self, data: &mut TabletToolAxisData, info: Option<&DeviceInfo>) {
        self.transform_position(&mut data.x, &mut data.y, info);
        self.transform_pressure(&mut data.pressure);
    }
}

impl Filter for TabletTransformFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        info: Option<&DeviceInfo>,
    ) -> Step {
        match event {
            InputEvent::TabletToolProximity(data) => {
                self.transform_proximity(data, info);
            }
            InputEvent::TabletToolAxis(data) => {
                self.transform_axis(data, info);
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
        crate::{backend::AxisInfo, events::DevIndex},
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
                max: 2000.0,
                resolution: 10.0,
            },
            ..Default::default()
        }
    }

    fn axis(x: f64, y: f64, pressure: f64) -> InputEvent {
        InputEvent::TabletToolAxis(Box::new(TabletToolAxisData {
            device_index: DevIndex(0),
            time_us: 1,
            x,
            y,
            pressure,
            tilt_x: 0.0,
            tilt_y: 0.0,
        }))
    }

    #[test]
    fn pressure_range_maps_to_unit_interval() {
        let mut filter = TabletTransformFilter::new(TabletConfig {
            pressure_range: Some((100.0, 900.0)),
            ..Default::default()
        });
        let mut event = axis(500.0, 500.0, 500.0);
        filter.process(&mut event, None, Some(&info()));
        let InputEvent::TabletToolAxis(data) = event else {
            panic!()
        };
        assert!((data.pressure - 0.5).abs() < 1e-12);
    }

    #[test]
    fn pressure_outside_range_clamps() {
        let mut filter = TabletTransformFilter::new(TabletConfig {
            pressure_range: Some((100.0, 900.0)),
            ..Default::default()
        });
        for (raw, expected) in [(0.0, 0.0), (1000.0, 1.0)] {
            let mut event = axis(500.0, 500.0, raw);
            filter.process(&mut event, None, Some(&info()));
            let InputEvent::TabletToolAxis(data) = event else {
                panic!()
            };
            assert!((data.pressure - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn active_area_maps_and_clamps() {
        let mut filter = TabletTransformFilter::new(TabletConfig {
            active_area: ActiveArea::new(0.25, 0.75, 0.5, 1.0),
            ..Default::default()
        });

        // Inside: normalized (0.25, 0.5) maps to device (250, 1000).
        let mut event = axis(250.0, 1000.0, 0.0);
        filter.process(&mut event, None, Some(&info()));
        let InputEvent::TabletToolAxis(data) = event else {
            panic!()
        };
        assert!((data.x - 0.0).abs() < 1e-9);
        assert!((data.y - 0.0).abs() < 1e-9);

        // Outside: clamp to the area edge before mapping.
        let mut event = axis(0.0, 0.0, 0.0);
        filter.process(&mut event, None, Some(&info()));
        let InputEvent::TabletToolAxis(data) = event else {
            panic!()
        };
        assert!((data.x - 0.0).abs() < 1e-9);
        assert!((data.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_config_is_rejected() {
        let config = TabletConfig {
            pressure_range: Some((500.0, 500.0)),
            active_area: Some(ActiveArea {
                x_min: 0.8,
                x_max: 0.2,
                y_min: 0.0,
                y_max: 1.0,
            }),
        }
        .sanitized();
        assert!(config.pressure_range.is_none());
        assert!(config.active_area.is_none());
    }
}
