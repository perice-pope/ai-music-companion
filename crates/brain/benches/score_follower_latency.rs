//! Latency benchmark for the score-follower alignment step
//! (`AudioEvent → cursor/score-position update`).
//!
//! This is the second measurable stage of the <25 ms mic-to-screen budget
//! (architecture-v2 §10): **score alignment, budget ~3 ms**. It picks up
//! where `crates/ears/benches/latency.rs` (`samples → AudioEvent`) leaves
//! off and measures one call to [`ScoreFollower::align`] — the Online-DTW
//! step that turns a played note into a measure/beat position the OSMD
//! cursor follows.
//!
//! HONESTY / SCOPE: this measures the Rust alignment path only. It does
//! NOT measure real-mic capture, Tauri IPC serialization, or the GPU/SVG
//! render that paints the cursor. Those three are not deterministically
//! measurable in CI (no audio device, no display) and are covered by the
//! manual hardware runbook (`docs/qa-runbook.md`).
//!
//! What this bench does:
//! 1. Builds a realistic 8-note scale score and a [`ScoreFollower`].
//! 2. Feeds consecutive voiced [`AudioEvent`]s through `align()`.
//! 3. Reports per-event latency. The CI workflow
//!    (`.github/workflows/latency-bench.yml`) reads the criterion JSON for
//!    the `score_align_path/latency_per_event` benchmark and fails the
//!    build if the mean exceeds the documented budget.
//!
//! The two descriptive benches below (`many_events`, `mixed_voiced…`) are
//! kept for local profiling and trend tracking; only the gated
//! `score_align_path/latency_per_event` benchmark drives the CI budget.

use brain::follower::ScoreFollower;
use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ears::AudioEvent;

/// C-major scale, C4..C5, one note per half-beat. The pitches double as the
/// played-event sequence so alignment lands on a fresh score note each call.
const SCALE_PITCHES: [f64; 8] = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25];
const SCALE_MIDI: [u8; 8] = [60, 62, 64, 65, 67, 69, 71, 72];

fn create_scale_score() -> ScoreModel {
    let notes = SCALE_PITCHES
        .iter()
        .zip(SCALE_MIDI.iter())
        .enumerate()
        .map(|(i, (&hz, &midi))| ScoreNote {
            pitch_hz: hz,
            midi_number: midi,
            duration_beats: 0.5,
            start_beat: i as f64 * 0.5,
            dynamic: None,
            is_rest: false,
        })
        .collect();

    ScoreModel {
        title: "C Major Scale".to_string(),
        composer: Some("W. A. Mozart".to_string()),
        instrument: Some("Violin".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 120.0,
        grand_staff: false,
        measures: vec![Measure { number: 1, notes }],
    }
}

fn voiced_event(hz: f64, ts: f64) -> AudioEvent {
    AudioEvent {
        pitch_hz: Some(hz),
        confidence: 0.95,
        amplitude: 0.7,
        timestamp_secs: ts,
        is_onset: true,
        note_info: None,
    }
}

/// GATED BENCH: `AudioEvent → score-position update`, budget ~3 ms (§10).
///
/// Named `score_align_path/latency_per_event` so the CI parser can locate
/// its `estimates.json` / `sample.json` exactly like the ears gate locates
/// `latency_per_event`.
fn bench_score_align_path(c: &mut Criterion) {
    let score = create_scale_score();
    let mut group = c.benchmark_group("score_align_path");
    group.throughput(Throughput::Elements(1));
    // Keep CI under ~2 min; alignment is low-variance.
    group.sample_size(60);
    group.measurement_time(std::time::Duration::from_secs(8));
    group.warm_up_time(std::time::Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::new("latency_per_event", SCALE_PITCHES.len()),
        |b| {
            // Fresh follower per benchmark; rotate through the scale so we
            // exercise alignment to different score notes across iterations.
            let mut follower = ScoreFollower::new(score.clone()).expect("valid score");
            let mut idx: usize = 0;
            b.iter(|| {
                let hz = SCALE_PITCHES[idx % SCALE_PITCHES.len()];
                // Tight spacing (<silence threshold) so we stay aligned and
                // measure the steady-state DTW step, not repeated resets.
                let ts = idx as f64 * 0.05;
                let event = voiced_event(hz, ts);
                let pos = follower.align(black_box(&event));
                idx = idx.wrapping_add(1);
                // Wrap before the 0.3s silence-reset window would trip.
                if idx.is_multiple_of(SCALE_PITCHES.len()) {
                    follower.reset();
                }
                black_box(pos)
            });
        },
    );

    group.finish();
}

fn benchmark_score_follower_many_events(c: &mut Criterion) {
    let score = create_scale_score();
    let mut follower = ScoreFollower::new(score).unwrap();

    c.bench_function("score_follower_align_100_events", |b| {
        b.iter(|| {
            for i in 0..100 {
                let hz = black_box(SCALE_PITCHES[i % SCALE_PITCHES.len()]);
                let ts = black_box((i as f64) * 0.05); // ~50ms per event
                let event = voiced_event(hz, ts);
                let _pos = follower.align(&event);
            }
            follower.reset();
        });
    });
}

fn benchmark_score_follower_with_unvoiced_events(c: &mut Criterion) {
    let score = create_scale_score();
    let mut follower = ScoreFollower::new(score).unwrap();
    let mut event_idx = 0;

    c.bench_function("score_follower_align_mixed_voiced_unvoiced", |b| {
        b.iter(|| {
            // Every other event is unvoiced (silence, breath, etc.)
            let is_voiced = event_idx % 2 == 0;
            let hz = if is_voiced {
                black_box(SCALE_PITCHES[event_idx / 2 % SCALE_PITCHES.len()])
            } else {
                0.0
            };
            let confidence = if is_voiced { 0.95 } else { 0.1 };

            let event = AudioEvent {
                pitch_hz: if is_voiced { Some(hz) } else { None },
                confidence,
                amplitude: if is_voiced { 0.7 } else { 0.0 },
                timestamp_secs: black_box((event_idx as f64) * 0.05),
                is_onset: is_voiced,
                note_info: None,
            };

            let _pos = follower.align(&event);
            event_idx += 1;
            if event_idx >= 16 {
                event_idx = 0;
                follower.reset();
            }
        });
    });
}

criterion_group!(
    benches,
    bench_score_align_path,
    benchmark_score_follower_many_events,
    benchmark_score_follower_with_unvoiced_events
);
criterion_main!(benches);
