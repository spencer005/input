use criterion::{black_box, criterion_group, criterion_main, Criterion};
use input_rs::events::{InputEvent, DevIndex};
use input_rs::state::{Context, DeviceId};
use input_rs::backend::{Backend, DeviceInfo};
 use input_rs::processing::{Filter, FilterKind};
use input_rs::processing::accel::ConstantAccelFilter;
use input_rs::processing::debounce::{ButtonDebounceFilter, DebounceConfig};
use input_rs::processing::hysteresis::HysteresisFilter;
use input_rs::processing::mouse::MouseFilter;
use input_rs::processing::touchpad::TouchpadFilter;
 use input_rs::TouchData;
 use input_rs::TouchEndData;
 use input_rs::processing::touchpad_motion::TouchpadMotionFilter;
 use slotmap::SlotMap;
use std::io;
use std::os::unix::io::OwnedFd;

struct BenchBackend {
    events: Vec<(DeviceId, InputEvent)>,
}

impl Backend for BenchBackend {
    fn initialize(&mut self) -> io::Result<()> { Ok(()) }
    fn add_device_fd(&mut self, _id: DeviceId, _fd: OwnedFd, _name: &str) -> io::Result<DeviceInfo> { Ok(DeviceInfo::default()) }
    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
        for (id, ev) in &self.events {
            callback(*id, ev.clone());
        }
        Ok(())
    }
    fn get_fd(&self) -> std::os::unix::io::RawFd { -1 }
}

fn bench_slotmap(c: &mut Criterion) {
    let mut sm: SlotMap<DeviceId, u32> = SlotMap::with_key();
    c.bench_function("slotmap_insert_remove", |b| {
        b.iter(|| {
            let key = sm.insert(black_box(123));
            sm.remove(black_box(key))
        })
    });
}

fn bench_filters(c: &mut Criterion) {
    let dev = DevIndex(0);
    let event = InputEvent::PointerMotion { device_index: dev, time_us: 1000, dx: 10.5, dy: -5.2, dx_unaccelerated: 10.5, dy_unaccelerated: -5.2 };

    let mut accel = ConstantAccelFilter::new(2.0);
    c.bench_function("filter_accel", |b| {
        b.iter(|| accel.process(black_box(event.clone()), None, None))
    });

    let mut hysteresis = HysteresisFilter::new(1.0);
    c.bench_function("filter_hysteresis_move", |b| {
        b.iter(|| hysteresis.process(black_box(event.clone()), None, None))
    });

    let _ = hysteresis.process(InputEvent::PointerMotion { device_index: dev, time_us: 0, dx: 0.0, dy: 0.0, dx_unaccelerated: 0.0, dy_unaccelerated: 0.0 }, None, None);
    let small_event = InputEvent::PointerMotion { device_index: dev, time_us: 1000, dx: 0.1, dy: 0.1, dx_unaccelerated: 0.1, dy_unaccelerated: 0.1 };
    c.bench_function("filter_hysteresis_suppress", |b| {
        b.iter(|| hysteresis.process(black_box(small_event.clone()), None, None))
    });
}

fn bench_touchpad(c: &mut Criterion) {
    let mut touchpad = TouchpadFilter::new(130.0, 0.0);
    let dev = DevIndex(0);
    let event = InputEvent::PointerMotion { device_index: dev, time_us: 1000, dx: 5.0, dy: 5.0, dx_unaccelerated: 5.0, dy_unaccelerated: 5.0 };

    c.bench_function("filter_touchpad", |b| {
        b.iter(|| touchpad.process(black_box(event.clone()), None, None))
    });
}

fn bench_mouse(c: &mut Criterion) {
    let mut mouse = MouseFilter::new(1000, 0.0);
    let dev = DevIndex(0);
    let event = InputEvent::PointerMotion { device_index: dev, time_us: 1000, dx: 5.0, dy: 5.0, dx_unaccelerated: 5.0, dy_unaccelerated: 5.0 };

    c.bench_function("filter_mouse_adaptive", |b| {
        b.iter(|| mouse.process(black_box(event.clone()), None, None))
    });

    let mut mouse_flat = MouseFilter::flat(1000);
    let event2 = InputEvent::PointerMotion { device_index: dev, time_us: 2000, dx: 5.0, dy: 5.0, dx_unaccelerated: 5.0, dy_unaccelerated: 5.0 };
    c.bench_function("filter_mouse_flat", |b| {
        b.iter(|| mouse_flat.process(black_box(event2.clone()), None, None))
    });
}

fn bench_debounce(c: &mut Criterion) {
    let mut debounce_enabled = ButtonDebounceFilter::new(DebounceConfig {
        enabled: true,
        ..Default::default()
    });
    let dev = DevIndex(0);
    c.bench_function("filter_debounce_enabled", |b| {
        let mut time = 0u64;
        b.iter(|| {
            time += 1000;
            debounce_enabled.process(
                black_box(InputEvent::Key { device_index: dev, time_us: time, key: 0x110, pressed: true }),
                None, None
            )
        })
    });

    let mut debounce_disabled = ButtonDebounceFilter::disabled();
    c.bench_function("filter_debounce_disabled", |b| {
        let mut time = 0u64;
        b.iter(|| {
            time += 1000;
            debounce_disabled.process(
                black_box(InputEvent::Key { device_index: dev, time_us: time, key: 0x110, pressed: true }),
                None, None
            )
        })
    });
}

fn bench_context(c: &mut Criterion) {
    let mut ctx = Context::new(Box::new(BenchBackend {
        events: vec![],
    }));

    let seat = ctx.create_seat("bench_seat");
    let null_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let dev = ctx.add_device_fd("bench_mouse", seat, null_fd);
    let dev_index = ctx.devices.get(dev).unwrap().index;

    if let Some(device) = ctx.devices.get_mut(dev) {
        device.filters.add(FilterKind::Accel(ConstantAccelFilter::new(1.5)));
        device.filters.add(FilterKind::Hysteresis(HysteresisFilter::new(0.5)));
    }

    let events = vec![
        (dev, InputEvent::PointerMotion { device_index: dev_index, time_us: 1234, dx: 5.0, dy: 5.0, dx_unaccelerated: 5.0, dy_unaccelerated: 5.0 }),
        (dev, InputEvent::Frame { device_index: dev_index, time_us: 1234 }),
    ];

    c.bench_function("context_process_motion_frame", |b| {
        b.iter(|| {
            ctx.backend = Box::new(BenchBackend { events: events.clone() });
            ctx.dispatch().unwrap();
            while ctx.next_event().is_some() {}
        })
    });

    // Approximate a high-polling mouse workload: many motion packets per dispatch.
    let events_8khz_like: Vec<(DeviceId, InputEvent)> = (0..8)
        .map(|i| {
            (
                dev,
                InputEvent::PointerMotion {
                    device_index: dev_index,
                    time_us: 1000 + i,
                    dx: 1.0,
                    dy: -1.0,
                    dx_unaccelerated: 1.0,
                    dy_unaccelerated: -1.0,
                },
            )
        })
        .chain(std::iter::once((
            dev,
            InputEvent::Frame {
                device_index: dev_index,
                time_us: 1008,
            },
        )))
        .collect();

    c.bench_function("context_process_8khz_like_batch", |b| {
        b.iter(|| {
            ctx.backend = Box::new(BenchBackend {
                events: events_8khz_like.clone(),
            });
            ctx.dispatch().unwrap();
            while ctx.next_event().is_some() {}
        })
    });
}

criterion_group!(benches, bench_slotmap, bench_filters, bench_touchpad, bench_mouse, bench_debounce, bench_context);
criterion_main!(benches);
