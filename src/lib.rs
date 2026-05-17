pub mod backend;
pub mod events;
pub mod processing;
pub mod state;

pub use state::{Context, DeviceId, SeatId, GroupId};
pub use events::{DevIndex, TouchData, TouchEndData};
pub use backend::evdev::EvdevBackend;
pub use processing::mouse::MouseAccelProfile;
pub use processing::debounce::DebounceConfig;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, DeviceInfo};
    use crate::events::InputEvent;
    use std::io;
    use std::os::unix::io::OwnedFd;

    struct MockBackend {
        events: Vec<(DeviceId, InputEvent)>,
    }

    impl Backend for MockBackend {
        fn initialize(&mut self) -> io::Result<()> { Ok(()) }
        fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
            for (id, ev) in self.events.drain(..) {
                callback(id, ev);
            }
            Ok(())
        }
        fn add_device_fd(&mut self, _id: DeviceId, _fd: OwnedFd, _name: &str) -> io::Result<DeviceInfo> {
            Ok(DeviceInfo::default())
        }
        fn get_fd(&self) -> std::os::unix::io::RawFd { -1 }
    }

    #[test]
    fn test_architecture() {
        let mut ctx = Context::new(Box::new(MockBackend { events: vec![] }));
        let seat_id = ctx.create_seat("seat0");

        let dev_id = ctx.add_device_fd("Mouse0", seat_id, std::fs::File::open("/dev/null").unwrap().into());
        assert!(matches!(ctx.next_event(), Some(InputEvent::DeviceAdded { .. })));

        let dev_index = ctx.devices.get(dev_id).unwrap().index;
        let backend = MockBackend {
            events: vec![
                (dev_id, InputEvent::PointerMotion { device_index: DevIndex(0), time_us: 100, dx: 10.0, dy: 5.0, dx_unaccelerated: 10.0, dy_unaccelerated: 5.0 })
            ],
        };

        ctx.backend = Box::new(backend);
        ctx.dispatch().unwrap();

        match ctx.next_event() {
            Some(InputEvent::PointerMotion { dx, dy, .. }) => {
                assert_eq!(dx, 10.0);
                assert_eq!(dy, 5.0);
            }
            _ => panic!("Expected PointerMotion"),
        }
    }
}
