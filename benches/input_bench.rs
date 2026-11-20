use {
    criterion::{Criterion, black_box, criterion_group, criterion_main},
    input_rs::{
        TouchData, TouchEndData,
        backend::{Backend, DeviceInfo},
        events::{DevIndex, InputEvent},
        processing::{
            Filter, FilterChain, FilterKind, FilterResult,
            accel::ConstantAccelFilter,
            debounce::{ButtonDebounceFilter, DebounceConfig},
            hysteresis::HysteresisFilter,
            mouse::MouseFilter,
            touchpad::TouchpadFilter,
            touchpad_motion::TouchpadMotionFilter,
        },
        state::{Context, DeviceId},
    },
    slotmap::SlotMap,
    std::{io, os::unix::io::OwnedFd},
};

struct BenchBackend {
    events: Vec<(DeviceId, InputEvent)>,
}

impl Backend for BenchBackend {
    fn initialize(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn add_device_fd(
        &mut self,
        _id: DeviceId,
        _fd: OwnedFd,
        _name: &str,
    ) -> io::Result<DeviceInfo> {
        Ok(DeviceInfo::default())
    }
    fn dispatch(&mut self, callback: &mut dyn FnMut(DeviceId, InputEvent)) -> io::Result<()> {
        for (id, ev) in &self.events {
            callback(*id, ev.clone());
        }
        Ok(())
    }
    fn get_fd(&self) -> std::os::unix::io::RawFd {
        -1
    }
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
    let event = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 1000,
        dx: 10.5,
        dy: -5.2,
        dx_unaccelerated: 10.5,
        dy_unaccelerated: -5.2,
    };

    let mut accel = ConstantAccelFilter::new(2.0);
    c.bench_function("filter_accel", |b| {
        b.iter(|| accel.run(black_box(event.clone()), None, None))
    });

    let mut hysteresis = HysteresisFilter::new(1.0);
    c.bench_function("filter_hysteresis_move", |b| {
        b.iter(|| hysteresis.run(black_box(event.clone()), None, None))
    });

    let _ = hysteresis.run(
        InputEvent::PointerMotion {
            device_index: dev,
            time_us: 0,
            dx: 0.0,
            dy: 0.0,
            dx_unaccelerated: 0.0,
            dy_unaccelerated: 0.0,
        },
        None,
        None,
    );
    let small_event = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 1000,
        dx: 0.1,
        dy: 0.1,
        dx_unaccelerated: 0.1,
        dy_unaccelerated: 0.1,
    };
    c.bench_function("filter_hysteresis_suppress", |b| {
        b.iter(|| hysteresis.run(black_box(small_event.clone()), None, None))
    });
}

fn bench_touchpad(c: &mut Criterion) {
    let mut touchpad = TouchpadFilter::new(130.0, 0.0);
    let dev = DevIndex(0);
    let event = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 1000,
        dx: 5.0,
        dy: 5.0,
        dx_unaccelerated: 5.0,
        dy_unaccelerated: 5.0,
    };

    c.bench_function("filter_touchpad", |b| {
        b.iter(|| touchpad.run(black_box(event.clone()), None, None))
    });
}

fn bench_mouse(c: &mut Criterion) {
    let mut mouse = MouseFilter::new(1000, 0.0);
    let dev = DevIndex(0);
    let event = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 1000,
        dx: 5.0,
        dy: 5.0,
        dx_unaccelerated: 5.0,
        dy_unaccelerated: 5.0,
    };

    c.bench_function("filter_mouse_adaptive", |b| {
        b.iter(|| mouse.run(black_box(event.clone()), None, None))
    });

    let mut mouse_flat = MouseFilter::flat(1000);
    let event2 = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 2000,
        dx: 5.0,
        dy: 5.0,
        dx_unaccelerated: 5.0,
        dy_unaccelerated: 5.0,
    };
    c.bench_function("filter_mouse_flat", |b| {
        b.iter(|| mouse_flat.run(black_box(event2.clone()), None, None))
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
            debounce_enabled.run(
                black_box(InputEvent::Key {
                    device_index: dev,
                    time_us: time,
                    key: 0x110,
                    pressed: true,
                }),
                None,
                None,
            )
        })
    });

    let mut debounce_disabled = ButtonDebounceFilter::disabled();
    c.bench_function("filter_debounce_disabled", |b| {
        let mut time = 0u64;
        b.iter(|| {
            time += 1000;
            debounce_disabled.run(
                black_box(InputEvent::Key {
                    device_index: dev,
                    time_us: time,
                    key: 0x110,
                    pressed: true,
                }),
                None,
                None,
            )
        })
    });
}

fn bench_context(c: &mut Criterion) {
    let mut ctx = Context::new(Box::new(BenchBackend { events: vec![] }));

    let seat = ctx.create_seat("bench_seat");
    let null_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let dev = ctx.add_device_fd("bench_mouse", seat, null_fd);
    let dev_index = ctx.devices.get(dev).unwrap().index;

    if let Some(device) = ctx.devices.get_mut(dev) {
        device
            .filters
            .add(FilterKind::Accel(ConstantAccelFilter::new(1.5)));
        device
            .filters
            .add(FilterKind::Hysteresis(HysteresisFilter::new(0.5)));
    }

    let events = vec![
        (
            dev,
            InputEvent::PointerMotion {
                device_index: dev_index,
                time_us: 1234,
                dx: 5.0,
                dy: 5.0,
                dx_unaccelerated: 5.0,
                dy_unaccelerated: 5.0,
            },
        ),
        (
            dev,
            InputEvent::Frame {
                device_index: dev_index,
                time_us: 1234,
            },
        ),
    ];

    // Install the backend once. The old benchmark allocated a Box and cloned
    // the event vectors inside every measured iteration, which dominated the
    // numbers it reported.
    ctx.backend = Box::new(BenchBackend {
        events: events.clone(),
    });
    c.bench_function("context_process_motion_frame", |b| {
        b.iter(|| {
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

    ctx.backend = Box::new(BenchBackend {
        events: events_8khz_like.clone(),
    });
    c.bench_function("context_process_8khz_like_batch", |b| {
        b.iter(|| {
            ctx.dispatch().unwrap();
            while ctx.next_event().is_some() {}
        })
    });
}

fn bench_dispatch_breakdown(c: &mut Criterion) {
    let events: Vec<(DeviceId, InputEvent)> = (0..8)
        .map(|i| {
            (
                DeviceId::default(),
                InputEvent::PointerMotion {
                    device_index: DevIndex(0),
                    time_us: 1000 + i,
                    dx: 1.0,
                    dy: -1.0,
                    dx_unaccelerated: 1.0,
                    dy_unaccelerated: -1.0,
                },
            )
        })
        .chain(std::iter::once((
            DeviceId::default(),
            InputEvent::Frame {
                device_index: DevIndex(0),
                time_us: 1008,
            },
        )))
        .collect();

    // Backend read loop + callback delivery only.
    let mut backend = BenchBackend {
        events: events.clone(),
    };
    c.bench_function("callback_delivery_8khz", |b| {
        b.iter(|| {
            backend
                .dispatch(&mut |_id, event| {
                    black_box(event);
                })
                .unwrap();
        })
    });

    // The same filters without Context lookup, remapping, tracking, or queue.
    let mut chain = FilterChain::new();
    chain.add(FilterKind::Accel(ConstantAccelFilter::new(1.5)));
    chain.add(FilterKind::Hysteresis(HysteresisFilter::new(0.5)));

    let event = events[0].1.clone();
    let mut empty_chain = FilterChain::new();
    c.bench_function("filter_chain_empty", |b| {
        b.iter(|| black_box(empty_chain.process(event.clone(), None, None)))
    });

    let mut accel_chain = FilterChain::new();
    accel_chain.add(FilterKind::Accel(ConstantAccelFilter::new(1.5)));
    c.bench_function("filter_chain_accel", |b| {
        b.iter(|| black_box(accel_chain.process(event.clone(), None, None)))
    });

    c.bench_function("filter_chain_accel_hysteresis", |b| {
        b.iter(|| black_box(chain.process(event.clone(), None, None)))
    });

    let mut chain_out = FilterResult::new();
    c.bench_function("filter_chain_accel_hysteresis_into", |b| {
        b.iter(|| {
            chain_out.clear();
            chain.process_into(event.clone(), None, None, &mut chain_out);
            black_box(chain_out.len())
        })
    });

    c.bench_function("input_event_clone", |b| b.iter(|| black_box(event.clone())));

    c.bench_function("filter_chain_8khz", |b| {
        b.iter(|| {
            for (_, event) in &events {
                black_box(chain.process(event.clone(), None, None));
            }
        })
    });

    // Same chain, but passing the device info like Context::dispatch does.
    // FilterChain clones Info once per filter per event.
    let info = DeviceInfo {
        name: "bench_mouse".into(),
        ..Default::default()
    };
    c.bench_function("filter_chain_with_info_8khz", |b| {
        b.iter(|| {
            for (_, event) in &events {
                black_box(chain.process(event.clone(), None, Some(&info)));
            }
        })
    });

    // Context path with an empty filter chain: device lookup, key/motion
    // bookkeeping, info clone, event remap, tracking, queue, and the post-
    // dispatch policy scan.
    let mut empty_ctx = Context::new(Box::new(BenchBackend { events: vec![] }));
    let seat = empty_ctx.create_seat("bench_empty");
    let null_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
    let empty_dev = empty_ctx.add_device_fd("bench_mouse", seat, null_fd);
    while empty_ctx.next_event().is_some() {}
    let empty_events: Vec<(DeviceId, InputEvent)> = events
        .iter()
        .map(|(_, event)| {
            let mut event = event.clone();
            if let InputEvent::PointerMotion { device_index, .. } = &mut event {
                *device_index = DevIndex(0);
            }
            if let InputEvent::Frame { device_index, .. } = &mut event {
                *device_index = DevIndex(0);
            }
            (empty_dev, event)
        })
        .collect();
    empty_ctx.backend = Box::new(BenchBackend {
        events: empty_events,
    });
    c.bench_function("context_dispatch_empty_filters_8khz", |b| {
        b.iter(|| {
            empty_ctx.dispatch().unwrap();
            while empty_ctx.next_event().is_some() {}
        })
    });
}

criterion_group!(
    benches,
    bench_slotmap,
    bench_filters,
    bench_touchpad,
    bench_mouse,
    bench_debounce,
    bench_context,
    bench_dispatch_breakdown
);
criterion_main!(benches);

trait ChainProcessExt {
    fn process(
        &mut self,
        event: input_rs::events::InputEvent,
        seat: Option<&input_rs::state::Seat>,
        info: Option<&input_rs::backend::DeviceInfo>,
    ) -> input_rs::processing::FilterResult;
}

impl ChainProcessExt for input_rs::processing::FilterChain {
    fn process(
        &mut self,
        event: input_rs::events::InputEvent,
        seat: Option<&input_rs::state::Seat>,
        info: Option<&input_rs::backend::DeviceInfo>,
    ) -> input_rs::processing::FilterResult {
        let mut out = input_rs::processing::FilterResult::new();
        self.process_into(event, seat, info, &mut out);
        out
    }
}

trait FilterRunExt {
    fn run(
        &mut self,
        event: input_rs::events::InputEvent,
        seat: Option<&input_rs::state::Seat>,
        info: Option<&input_rs::backend::DeviceInfo>,
    ) -> input_rs::processing::FilterResult;
}

impl<T: input_rs::processing::Filter> FilterRunExt for T {
    fn run(
        &mut self,
        mut event: input_rs::events::InputEvent,
        seat: Option<&input_rs::state::Seat>,
        info: Option<&input_rs::backend::DeviceInfo>,
    ) -> input_rs::processing::FilterResult {
        use input_rs::processing::Step;
        let mut out = input_rs::processing::FilterResult::new();
        match input_rs::processing::Filter::process(self, &mut event, seat, info) {
            Step::Keep => out.push(event),
            Step::Suppress => {}
            Step::FanOut(events) => out.extend(*events),
        }
        out
    }
}
