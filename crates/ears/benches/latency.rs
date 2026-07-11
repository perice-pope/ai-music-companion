//! Latency benchmark for the ears analysis path (samples → AudioEvent).
//!
//! Measures the dominant, CI-measurable portion of the <25 ms budget:
//! one call to `PitchDetector::detect()` with a window of audio samples.
//!
//! Rationale: cpal has no real audio device in CI. The analysis path is
//! the dominant latency contributor and the one we can measure
//! deterministically. IPC + render budget (~5 ms) is a separate concern,
//! enforced elsewhere.
//!
//! What this bench does:
//! 1. Pre-generates ~1 second of synthetic A4 sine at 44.1 kHz (mono, f32)
//! 2. Feeds consecutive windows through the pitch detector
//! 3. Reports per-event latency (criterion produces mean + std-err; p95
//!    derived by CI from the raw samples written to `target/criterion`).
//!
//! The CI workflow (.github/workflows/latency-bench.yml) reads the JSON
//! estimates and fails the build if the mean exceeds 25 ms.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ears::pitch::{generate_sine, PitchConfig, PitchDetector};

const SAMPLE_RATE: u32 = 44_100;
const TEST_FREQ_HZ: f64 = 440.0;
const TEST_DURATION_SECS: f64 = 1.0;

/// Build the test signal: 1 s of A4 sine at 44.1 kHz.
fn build_signal() -> Vec<f32> {
    generate_sine(
        TEST_FREQ_HZ,
        SAMPLE_RATE,
        (SAMPLE_RATE as f64 * TEST_DURATION_SECS) as usize,
    )
}

/// Build a detector with the production configuration.
fn build_detector() -> PitchDetector {
    PitchDetector::new(PitchConfig {
        sample_rate: SAMPLE_RATE,
        ..Default::default()
    })
    .expect("valid pitch config")
}

/// Slice the signal into consecutive windows of `window_size` samples.
/// Mimics the real-time analysis loop where a new window is delivered
/// every hop. For latency measurement we care about per-call wall time,
/// not hop cadence — so we walk the signal with non-overlapping windows.
fn windows(signal: &[f32], window_size: usize) -> Vec<&[f32]> {
    signal.chunks_exact(window_size).collect::<Vec<_>>()
}

fn bench_analysis_path(c: &mut Criterion) {
    let signal = build_signal();
    let detector_probe = build_detector();
    let window_size = detector_probe.window_size();
    let chunks = windows(&signal, window_size);
    assert!(
        !chunks.is_empty(),
        "signal must contain at least one full window"
    );

    let mut group = c.benchmark_group("ears_analysis_path");
    group.throughput(Throughput::Elements(1));
    // Keep CI runs under ~2 min — these are well-behaved (low variance) numbers.
    group.sample_size(60);
    group.measurement_time(std::time::Duration::from_secs(8));
    group.warm_up_time(std::time::Duration::from_secs(1));

    group.bench_function(BenchmarkId::new("latency_per_event", window_size), |b| {
        // Measure the per-window `detect()` call — the analysis-path unit of
        // work. We rotate through chunks across iterations so we aren't
        // hitting only one window of input.
        let mut detector = build_detector();
        let mut idx: usize = 0;
        b.iter(|| {
            let chunk = chunks[idx % chunks.len()];
            idx = idx.wrapping_add(1);
            criterion::black_box(detector.detect(criterion::black_box(chunk)))
        });
    });

    group.finish();
}

/// #349 AC6: the chroma reading (ring feed + full Goertzel bank + fold)
/// runs on the same worker thread as the analysis path, once per ~93 ms
/// hop. It is sequenced AFTER the live pitch emit, so it can't delay an
/// event — but it still must stay a small fraction of the hop, or the
/// worker falls behind the capture ring. CI enforces a budget on this
/// group's mean (latency-bench.yml, CHROMA_BUDGET_MS).
fn bench_chroma_reading(c: &mut Criterion) {
    let signal = build_signal();
    let chunks = windows(&signal, 1024);

    let mut group = c.benchmark_group("chroma_reading");
    group.throughput(Throughput::Elements(1));
    group.sample_size(60);
    group.measurement_time(std::time::Duration::from_secs(5));
    group.warm_up_time(std::time::Duration::from_secs(1));

    group.bench_function("reading_per_hop", |b| {
        let mut extractor = ears::chroma::ChromaExtractor::new(SAMPLE_RATE);
        // Warm the ring so every measured reading is a full compute.
        for chunk in &chunks {
            extractor.feed(chunk);
        }
        let mut idx: usize = 0;
        b.iter(|| {
            // One hop's worth of work: four ~23 ms windows fed, one reading.
            for _ in 0..4 {
                let chunk = chunks[idx % chunks.len()];
                idx = idx.wrapping_add(1);
                extractor.feed(criterion::black_box(chunk));
            }
            criterion::black_box(extractor.chroma())
        });
    });

    group.finish();
}

criterion_group!(benches, bench_analysis_path, bench_chroma_reading);
criterion_main!(benches);
