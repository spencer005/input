//! Pipeline microbenchmarks for the input-rs mouse hot path.
//!
//! Measures the actual overhead layers:
//! 1. Raw 2× f64 multiply (theoretical floor for flat profile)
//! 2. SmallVec<4> construction vs. inline single-event return
//! 3. FilterChain dispatch overhead for single-filter case
//! 4. Flat MouseFilter end-to-end
//!
//! Run with: cargo bench --bench simd_bench

use {
    criterion::{Criterion, black_box, criterion_group, criterion_main},
    input_rs::{
        events::{DevIndex, InputEvent},
        processing::{Filter, FilterChain, FilterKind, FilterResult, mouse::MouseFilter},
    },
    smallvec::smallvec,
};

// ---------------------------------------------------------------------------
// Baseline: raw 2× f64 multiply (theoretical floor)
// ---------------------------------------------------------------------------

fn raw_2mul(dx: f64, dy: f64, dpi_factor: f64) -> (f64, f64) {
    (dx * dpi_factor, dy * dpi_factor)
}

// ---------------------------------------------------------------------------
// SmallVec vs inline alternatives for single-event returns
// ---------------------------------------------------------------------------

fn smallvec_return(dev: DevIndex, time_us: u64, dx: f64, dy: f64) -> FilterResult {
    smallvec![InputEvent::PointerMotion {
        device_index: dev,
        time_us,
        dx,
        dy,
        dx_unaccelerated: dx,
        dy_unaccelerated: dy
    }]
}

fn array_return(dev: DevIndex, time_us: u64, dx: f64, dy: f64) -> [InputEvent; 1] {
    [InputEvent::PointerMotion {
        device_index: dev,
        time_us,
        dx,
        dy,
        dx_unaccelerated: dx,
        dy_unaccelerated: dy,
    }]
}

fn option_return(dev: DevIndex, time_us: u64, dx: f64, dy: f64) -> Option<InputEvent> {
    Some(InputEvent::PointerMotion {
        device_index: dev,
        time_us,
        dx,
        dy,
        dx_unaccelerated: dx,
        dy_unaccelerated: dy,
    })
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

fn bench_raw_multiply(c: &mut Criterion) {
    let mut group = c.benchmark_group("raw_ops");
    group.bench_function("2x_f64_multiply", |b| {
        let mut dx = 5.0f64;
        let mut dy = -3.0f64;
        let dpi_factor = 2.5f64;
        b.iter(|| {
            let (ndx, ndy) = raw_2mul(black_box(dx), black_box(dy), black_box(dpi_factor));
            dx = ndx;
            dy = ndy;
        })
    });
    group.finish();
}

fn bench_return_types(c: &mut Criterion) {
    let dev = DevIndex(0);
    let mut group = c.benchmark_group("return_type");

    group.bench_function("smallvec_1_event", |b| {
        b.iter(|| {
            let result = smallvec_return(
                black_box(dev),
                black_box(1000u64),
                black_box(5.0f64),
                black_box(-3.0f64),
            );
            black_box(&result);
        })
    });

    group.bench_function("array_1_event", |b| {
        b.iter(|| {
            let result = array_return(
                black_box(dev),
                black_box(1000u64),
                black_box(5.0f64),
                black_box(-3.0f64),
            );
            black_box(&result);
        })
    });

    group.bench_function("option_1_event", |b| {
        b.iter(|| {
            let result = option_return(
                black_box(dev),
                black_box(1000u64),
                black_box(5.0f64),
                black_box(-3.0f64),
            );
            black_box(&result);
        })
    });

    group.finish();
}

fn bench_filterchain_overhead(c: &mut Criterion) {
    let dev = DevIndex(0);
    let event = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 1000,
        dx: 5.0,
        dy: -3.0,
        dx_unaccelerated: 5.0,
        dy_unaccelerated: -3.0,
    };

    let mut group = c.benchmark_group("filterchain");

    // Direct call to MouseFilter — no FilterChain dispatch
    let mut mouse = MouseFilter::flat(1000);
    group.bench_function("direct_mouse_flat", |b| {
        b.iter(|| {
            let result = mouse.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    // Through FilterChain with 1 filter (typical for a mouse)
    let mut chain = FilterChain::new();
    chain.add(FilterKind::Mouse(Box::new(MouseFilter::flat(1000))));
    group.bench_function("chain_1_filter_flat", |b| {
        b.iter(|| {
            let result = chain.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    // Through FilterChain with 1 filter + debounce disabled (realistic mouse config)
    use input_rs::processing::debounce::{ButtonDebounceFilter, DebounceConfig};
    let mut chain2 = FilterChain::new();
    chain2.add(FilterKind::Mouse(Box::new(MouseFilter::flat(1000))));
    chain2.add(FilterKind::Debounce(ButtonDebounceFilter::disabled()));
    group.bench_function("chain_2_filter_flat_nodebounce", |b| {
        b.iter(|| {
            let result = chain2.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    group.finish();
}

fn bench_mouse_profiles(c: &mut Criterion) {
    let dev = DevIndex(0);
    let mut group = c.benchmark_group("mouse_filter");

    let event = InputEvent::PointerMotion {
        device_index: dev,
        time_us: 1000,
        dx: 5.0,
        dy: -3.0,
        dx_unaccelerated: 5.0,
        dy_unaccelerated: -3.0,
    };

    group.bench_function("flat_1000dpi", |b| {
        let mut mouse = MouseFilter::flat(1000);
        b.iter(|| {
            let result = mouse.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    group.bench_function("flat_400dpi", |b| {
        let mut mouse = MouseFilter::flat(400);
        b.iter(|| {
            let result = mouse.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    group.bench_function("flat_35000dpi", |b| {
        let mut mouse = MouseFilter::flat(35_000);
        b.iter(|| {
            let result = mouse.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    group.bench_function("adaptive_1000dpi", |b| {
        let mut mouse = MouseFilter::new(1000, 0.0);
        b.iter(|| {
            let result = mouse.process(black_box(event.clone()), None, None);
            black_box(&result);
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_raw_multiply,
    bench_return_types,
    bench_filterchain_overhead,
    bench_mouse_profiles,
);
criterion_main!(benches);
