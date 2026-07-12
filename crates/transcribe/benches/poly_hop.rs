//! #349 T3 AC2 — the streaming-polyphony hop budget.
//!
//! One `poll()` = one basic-pitch inference over a 2 s window (plus the
//! 1 s hop's resample + bookkeeping). The spec's budget: **≤250 ms per hop
//! on CPU** — the worker thread runs a hop every second, so anything near
//! the hop period would starve the pipeline. CI enforces the budget from
//! this bench's criterion output (`latency-bench.yml`, POLY_HOP_BUDGET_MS).
//!
//! Requires ONNX Runtime (`ORT_DYLIB_PATH`); skipped when absent unless
//! `TRANSCRIBE_REQUIRE_ORT=1` (CI) makes absence a hard failure.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use transcribe::{PolyEngine, StreamingBasicPitch};

const SR: u32 = 22_050;

fn runtime_present() -> bool {
    std::env::var("ORT_DYLIB_PATH")
        .ok()
        .map(|p| std::path::Path::new(&p).exists())
        .unwrap_or(false)
}

/// A busy two-chord second — the realistic per-hop workload.
fn one_second_of_comping(offset: usize) -> Vec<f32> {
    let n = SR as usize;
    let mut out = vec![0.0f32; n];
    for (k, &m) in [60, 64, 67, 72].iter().enumerate() {
        let f = 440.0 * 2f64.powf(((m + (offset % 5) as i32) as f64 - 69.0) / 12.0);
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f64 / SR as f64;
            *o += (0.4 / (k + 1) as f64 * (2.0 * std::f64::consts::PI * f * t).sin()) as f32;
        }
    }
    out
}

fn bench_poly_hop(c: &mut Criterion) {
    if !runtime_present() {
        if std::env::var("TRANSCRIBE_REQUIRE_ORT").as_deref() == Ok("1") {
            panic!("ONNX Runtime required (TRANSCRIBE_REQUIRE_ORT=1) but ORT_DYLIB_PATH unset");
        }
        eprintln!("skipping poly_hop bench: ONNX Runtime unavailable");
        return;
    }

    let mut group = c.benchmark_group("poly_hop");
    group.throughput(Throughput::Elements(1));
    // Each iteration is a real inference (~100 ms class): keep CI time sane.
    group.sample_size(20);
    group.measurement_time(std::time::Duration::from_secs(20));
    group.warm_up_time(std::time::Duration::from_secs(3));

    group.bench_function("hop_per_second", |b| {
        let mut engine = StreamingBasicPitch::new().expect("runtime present");
        // Prime one full window so every measured poll is a real hop.
        engine.feed(&one_second_of_comping(0), SR);
        engine.feed(&one_second_of_comping(1), SR);
        let mut i = 2usize;
        b.iter(|| {
            // Steady state: feed one second, poll one hop.
            engine.feed(&one_second_of_comping(i), SR);
            i = i.wrapping_add(1);
            criterion::black_box(engine.poll().expect("inference runs"))
        });
    });

    group.finish();
}

criterion_group!(benches, bench_poly_hop);
criterion_main!(benches);
