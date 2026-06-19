use criterion::{Criterion, black_box, criterion_group, criterion_main};

#[derive(Clone, Copy)]
struct DeviceFloatCoords {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct PointerTracker {
    delta: DeviceFloatCoords,
    time: u64,
    dir: u32,
}

struct PointerTrackers {
    trackers: [PointerTracker; 16],
    ntrackers: usize,
    cur_tracker: usize,
}

#[inline(always)]
fn calculate_trackers_velocity(tracker: &PointerTracker, time: u64) -> f64 {
    let tdelta = time - tracker.time + 1;
    ((tracker.delta.x.powi(2) + tracker.delta.y.powi(2)).sqrt()) / tdelta as f64
}

fn trackers_velocity(trackers: &PointerTrackers, time: u64) -> f64 {
    let mut result = 0.0;
    let mut initial_velocity = 0.0;
    const MOTION_TIMEOUT: u64 = 1000000;
    const MAX_VELOCITY_DIFF: f64 = 1.0 / 1000.0;

    let mut dir = trackers.trackers[trackers.cur_tracker].dir;

    for offset in 1..trackers.ntrackers {
        let index = (trackers.cur_tracker + trackers.ntrackers - offset) % trackers.ntrackers;
        let tracker = &trackers.trackers[index];

        if tracker.time > time {
            break;
        }
        if time - tracker.time > MOTION_TIMEOUT {
            break;
        }

        let velocity = calculate_trackers_velocity(tracker, time);

        dir &= tracker.dir;
        if dir == 0 {
            if offset == 1 {
                result = velocity;
            }
            break;
        }

        if initial_velocity == 0.0 || offset <= 2 {
            result = velocity;
            initial_velocity = velocity;
        } else {
            if (initial_velocity - velocity).abs() > MAX_VELOCITY_DIFF {
                break;
            }
            result = velocity;
        }
    }
    result
}

fn bench_trackers_velocity(c: &mut Criterion) {
    let mut trackers = PointerTrackers {
        trackers: [PointerTracker {
            delta: DeviceFloatCoords { x: 10.0, y: 10.0 },
            time: 0,
            dir: 0xFFFFFFFF,
        }; 16],
        ntrackers: 16,
        cur_tracker: 15,
    };
    for i in 0..16 {
        trackers.trackers[i].time = i as u64 * 1000;
    }

    c.bench_function("pure_trackers_velocity", |b| {
        b.iter(|| trackers_velocity(black_box(&trackers), black_box(20000)))
    });
}

criterion_group!(benches, bench_trackers_velocity);
criterion_main!(benches);
