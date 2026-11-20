use {
    super::{Filter, Step},
    crate::{backend::DeviceInfo, events::InputEvent, state::Seat},
};

/// Arbitrary pointer-button remapping.
///
/// Each source button code maps to a target button code. Unmapped buttons pass
/// through unchanged. Duplicate source entries resolve to the last insertion.
/// The map is an explicit input-rs extension; libinput does not expose
/// arbitrary button remapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ButtonMap {
    entries: Vec<(u32, u32)>,
}

impl ButtonMap {
    pub fn new(entries: impl IntoIterator<Item = (u32, u32)>) -> Self {
        let mut map = Self::default();
        for (from, to) in entries {
            map.insert(from, to);
        }
        map
    }

    pub fn insert(&mut self, from: u32, to: u32) {
        if let Some(entry) = self.entries.iter_mut().find(|(source, _)| *source == from) {
            entry.1 = to;
        } else {
            self.entries.push((from, to));
        }
    }

    pub fn remove(&mut self, from: u32) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(source, _)| *source != from);
        self.entries.len() != before
    }

    pub fn get(&self, from: u32) -> Option<u32> {
        self.entries
            .iter()
            .find_map(|(source, target)| (*source == from).then_some(*target))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[(u32, u32)] {
        &self.entries
    }
}

pub struct ButtonMapFilter {
    map: ButtonMap,
}

impl ButtonMapFilter {
    pub fn new(map: ButtonMap) -> Self {
        Self { map }
    }

    pub fn set_map(&mut self, map: ButtonMap) {
        self.map = map;
    }

    pub fn map(&self) -> &ButtonMap {
        &self.map
    }
}

impl Filter for ButtonMapFilter {
    fn process(
        &mut self,
        event: &mut InputEvent,
        _seat: Option<&Seat>,
        _info: Option<&DeviceInfo>,
    ) -> Step {
        if let InputEvent::PointerButton { button, .. } = event {
            *button = self.map.get(*button).unwrap_or(*button);
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

    fn button(button: u32, pressed: bool) -> InputEvent {
        InputEvent::PointerButton {
            device_index: DevIndex(0),
            time_us: 1,
            button,
            pressed,
        }
    }

    #[test]
    fn maps_press_and_release_consistently() {
        let mut filter = ButtonMapFilter::new(ButtonMap::new([(0x113, 0x110)]));
        assert!(matches!(
            filter.run(button(0x113, true), None, None)[0],
            InputEvent::PointerButton {
                button: 0x110,
                pressed: true,
                ..
            }
        ));
        assert!(matches!(
            filter.run(button(0x113, false), None, None)[0],
            InputEvent::PointerButton {
                button: 0x110,
                pressed: false,
                ..
            }
        ));
    }

    #[test]
    fn unmapped_buttons_pass_through() {
        let mut filter = ButtonMapFilter::new(ButtonMap::new([(0x113, 0x110)]));
        assert!(matches!(
            filter.run(button(0x114, true), None, None)[0],
            InputEvent::PointerButton {
                button: 0x114,
                pressed: true,
                ..
            }
        ));
    }

    #[test]
    fn duplicate_source_uses_last_insertion() {
        let map = ButtonMap::new([(0x113, 0x110), (0x113, 0x111)]);
        assert_eq!(map.get(0x113), Some(0x111));
    }
}
