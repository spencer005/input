//! Explicit, data-driven device profiles.
//!
//! input-rs deliberately does not copy libinput's hidden quirks database. A
//! profile is user-visible configuration: it matches a device using stable
//! metadata and supplies defaults that are applied before the compositor's
//! explicit configuration. Explicit configuration always wins because it is
//! applied afterwards.
//!
//! Matching rules:
//! - every configured match key must match;
//! - the profile with the most configured match keys wins;
//! - a tie between equally specific profiles is ambiguous, so automatic
//!   matching is rejected with a diagnostic and the device keeps safe
//!   defaults;
//! - an explicitly selected profile by name ignores match keys.

use crate::{
    backend::{BusType, DeviceInfo},
    processing::config::{
        ClickfingerButtonMap, TapButtonMap, TouchpadClickMethod, TouchpadConfig,
        TouchpadScrollMethod,
    },
};

/// Touchpad defaults supplied by a device profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TouchpadProfile {
    pub tap_to_click: Option<bool>,
    pub natural_scrolling: Option<bool>,
    pub two_finger_tap_right_click: Option<bool>,
    pub tap_drag: Option<bool>,
    pub tap_drag_lock: Option<bool>,
    pub click_method: Option<TouchpadClickMethod>,
    pub scroll_method: Option<TouchpadScrollMethod>,
    pub edge_scroll_edge_ratio: Option<f64>,
    pub dwt: Option<bool>,
    pub dwt_timeout_us: Option<u64>,
    pub dwtp: Option<bool>,
    pub dwtp_timeout_us: Option<u64>,
    pub palm_detection: Option<bool>,
    pub palm_edge_ratio: Option<f64>,
    pub palm_pressure_threshold: Option<f64>,
    pub palm_size_threshold: Option<f64>,
    pub thumb_detection: Option<bool>,
    pub thumb_edge_ratio: Option<f64>,
    pub thumb_pressure_threshold: Option<f64>,
    pub stylus_arbitration: Option<bool>,
    pub tap_button_map: Option<TapButtonMap>,
    pub clickfinger_button_map: Option<ClickfingerButtonMap>,
    pub tap_timeout_us: Option<u64>,
    pub tap_move_threshold_mm: Option<f64>,
    pub tap_drag_timeout_us: Option<u64>,
    pub drag_lock_timeout_us: Option<u64>,
    pub max_tap_fingers: Option<u8>,
    pub clickpad: Option<bool>,
    pub button_area_bottom_ratio: Option<f64>,
    pub button_area_left_ratio: Option<f64>,
    pub button_area_right_ratio: Option<f64>,
    pub left_handed: Option<bool>,
    pub hold_timeout_us: Option<u64>,
    pub gesture_motion_threshold_mm: Option<f64>,
    pub pinch_scale_threshold: Option<f64>,
    pub pinch_rotation_threshold_rad: Option<f64>,
}

impl TouchpadProfile {
    /// Apply profile defaults, keeping any value not specified by the profile.
    pub fn apply(&self, config: &mut TouchpadConfig) {
        macro_rules! apply {
            ($($field:ident),* $(,)?) => {
                $( if let Some(value) = self.$field { config.$field = value; } )*
            };
        }

        apply!(
            tap_to_click,
            natural_scrolling,
            two_finger_tap_right_click,
            tap_drag,
            tap_drag_lock,
            click_method,
            scroll_method,
            edge_scroll_edge_ratio,
            dwt,
            dwt_timeout_us,
            dwtp,
            dwtp_timeout_us,
            palm_detection,
            palm_edge_ratio,
            thumb_detection,
            thumb_edge_ratio,
            stylus_arbitration,
            tap_button_map,
            clickfinger_button_map,
            tap_timeout_us,
            tap_move_threshold_mm,
            tap_drag_timeout_us,
            drag_lock_timeout_us,
            max_tap_fingers,
            button_area_bottom_ratio,
            button_area_left_ratio,
            button_area_right_ratio,
            left_handed,
            hold_timeout_us,
            gesture_motion_threshold_mm,
            pinch_scale_threshold,
            pinch_rotation_threshold_rad,
        );

        if let Some(value) = self.palm_pressure_threshold {
            config.palm_pressure_threshold = Some(value);
        }
        if let Some(value) = self.palm_size_threshold {
            config.palm_size_threshold = Some(value);
        }
        if let Some(value) = self.thumb_pressure_threshold {
            config.thumb_pressure_threshold = Some(value);
        }
        if let Some(value) = self.clickpad {
            config.clickpad_override = Some(value);
        }
    }
}

/// Mouse/pointer defaults supplied by a device profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointerProfile {
    pub dpi: Option<i32>,
    pub speed_adjustment: Option<f64>,
    /// `true` selects the adaptive profile; `false` selects flat.
    pub adaptive: Option<bool>,
    pub rotation_degrees: Option<i32>,
    pub natural_scrolling: Option<bool>,
    /// Independent scroll scaling factors; negative values invert an axis.
    pub scroll_scale_x: Option<f64>,
    pub scroll_scale_y: Option<f64>,
    /// Arbitrary pointer-button remapping.
    pub button_map: Option<crate::processing::button_map::ButtonMap>,
}

/// Declarative device profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceProfile {
    /// Human-readable profile name. Explicit selection uses this name.
    pub name: String,
    pub match_bus: Option<BusType>,
    pub match_vendor_id: Option<u16>,
    pub match_product_id: Option<u16>,
    /// Exact device name match.
    pub match_name: Option<String>,
    /// Override the conservative internal/external classification.
    pub internal: Option<bool>,
    /// Device-class overrides for diagnostics and future policy.
    pub pointer: PointerProfile,
    pub touchpad: TouchpadProfile,
    /// Axes the profile declares unusable. Backend/event consumers can consult
    /// this through the matched profile for diagnostics.
    pub broken_axes: BrokenAxes,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BrokenAxes {
    pub x: bool,
    pub y: bool,
    pub pressure: bool,
}

impl DeviceProfile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Number of match keys configured on this profile.
    pub fn specificity(&self) -> u8 {
        self.match_bus.is_some() as u8
            + self.match_vendor_id.is_some() as u8
            + self.match_product_id.is_some() as u8
            + self.match_name.is_some() as u8
    }

    pub fn matches(&self, info: &DeviceInfo) -> bool {
        if self.specificity() == 0 {
            return false;
        }
        self.match_bus.is_none_or(|bus| bus == info.bus)
            && self
                .match_vendor_id
                .is_none_or(|vendor| vendor == info.vendor_id)
            && self
                .match_product_id
                .is_none_or(|product| product == info.product_id)
            && self
                .match_name
                .as_deref()
                .is_none_or(|name| name == info.name)
    }
}

/// Result of automatic profile matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileMatch {
    Matched(String),
    NoMatch,
    /// More than one equally specific profile matched. `names` is sorted for
    /// deterministic diagnostics.
    Ambiguous(Vec<String>),
}

/// Find the best profile for `info` following the documented matching rules.
pub fn match_profile<'a, I>(profiles: I, info: &DeviceInfo) -> ProfileMatch
where
    I: IntoIterator<Item = &'a DeviceProfile>,
{
    let mut best_specificity = 0;
    let mut best: Vec<String> = Vec::new();

    for profile in profiles {
        if !profile.matches(info) {
            continue;
        }
        let specificity = profile.specificity();
        if specificity > best_specificity {
            best_specificity = specificity;
            best.clear();
            best.push(profile.name.clone());
        } else if specificity == best_specificity {
            best.push(profile.name.clone());
        }
    }

    match best.len() {
        0 => ProfileMatch::NoMatch,
        1 => ProfileMatch::Matched(best.remove(0)),
        _ => {
            best.sort();
            ProfileMatch::Ambiguous(best)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(bus: BusType, vendor: u16, product: u16, name: &str) -> DeviceInfo {
        DeviceInfo {
            bus,
            vendor_id: vendor,
            product_id: product,
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn specific_profile_beats_broad_profile() {
        let broad = DeviceProfile {
            match_vendor_id: Some(1),
            name: "broad".into(),
            ..Default::default()
        };
        let exact = DeviceProfile {
            match_vendor_id: Some(1),
            match_product_id: Some(2),
            name: "exact".into(),
            ..Default::default()
        };
        let device = info(BusType::Usb, 1, 2, "X");
        assert_eq!(
            match_profile([&broad, &exact], &device),
            ProfileMatch::Matched("exact".into())
        );
    }

    #[test]
    fn equally_specific_match_is_ambiguous() {
        let a = DeviceProfile {
            match_vendor_id: Some(1),
            match_name: Some("X".into()),
            name: "a".into(),
            ..Default::default()
        };
        let b = DeviceProfile {
            match_product_id: Some(2),
            match_name: Some("X".into()),
            name: "b".into(),
            ..Default::default()
        };
        let device = info(BusType::Usb, 1, 2, "X");
        assert_eq!(
            match_profile([&a, &b], &device),
            ProfileMatch::Ambiguous(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn profile_apply_keeps_unspecified_values() {
        let mut config = TouchpadConfig::default();
        config.tap_timeout_us = 123;
        let profile = TouchpadProfile {
            tap_timeout_us: Some(456),
            palm_pressure_threshold: Some(70.0),
            ..Default::default()
        };
        profile.apply(&mut config);
        assert_eq!(config.tap_timeout_us, 456);
        assert_eq!(config.palm_pressure_threshold, Some(70.0));
        // Unspecified value is untouched.
        assert_eq!(
            config.tap_move_threshold_mm,
            TouchpadConfig::default().tap_move_threshold_mm
        );
    }
}
