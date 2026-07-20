//! Integrated analysis-path latency benchmark:
//! `samples → AudioEvent → phrase → recap-input`.
//!
//! Where the other two benches isolate single stages of the <25 ms
//! mic-to-screen budget (architecture-v2 §10) — `crates/ears` measures
//! `samples → AudioEvent` (pitch ~6 ms) and `score_follower_latency`
//! measures `AudioEvent → score position` (alignment ~3 ms) — this one
//! chains the whole *deterministic Rust analysis pipeline* end to end:
//!
//!   raw samples
//!     → PitchDetector::detect()           (pitch detection)
//!     → PhraseAggregator::push()           (phrasing + score-follower align)
//!     → take_new_phrases() / RecapInput    (phrase summary handed to coaching)
//!
//! It answers a question the per-stage benches can't: does the *composed*
//! per-event cost (detect + phrase bookkeeping + alignment) stay within
//! budget, including the realloc-free steady state? We measure per-window
//! cost so the number is directly comparable to the §10 stage budgets.
//!
//! HONESTY / SCOPE — what this DOES NOT cover:
//!   * Real-mic capture (cpal): no audio device in CI.
//!   * Tauri IPC serialization (AudioEvent → JSON → JS): no Tauri runtime.
//!   * GPU/SVG render of the cursor: no display.
//!   * LLM recap generation: intentionally excluded — coaching is NOT in
//!     the latency-critical path (§10) and is a network call. We stop at
//!     building the `RecapInput`, which is the last purely-local step.
//!
//! Those uncovered stages are validated by hand per `docs/qa-runbook.md`.
//!
//! CI gate: `.github/workflows/latency-bench.yml` reads the criterion JSON
//! for `integrated_analysis_path/latency_per_event` and fails the build if
//! the mean exceeds the documented budget.

use brain::follower::ScoreFollower;
use brain::phrase::{PhraseAggregator, PhraseConfig, PhraseSummary};
use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};
use brain::session::{PracticeMode, RecapInput};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ears::pitch::{generate_sine, PitchConfig, PitchDetector};

const SAMPLE_RATE: u32 = 44_100;

/// C-major scale used both for the score and the synthesized audio, so the
/// follower stays aligned while we drive the integrated path.
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
        composer: None,
        instrument: Some("Violin".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 120.0,
        grand_staff: false,
        measures: vec![Measure { number: 1, notes }],
    }
}

fn build_detector() -> PitchDetector {
    PitchDetector::new(PitchConfig {
        sample_rate: SAMPLE_RATE,
        ..Default::default()
    })
    .expect("valid pitch config")
}

/// Pre-generate one window of sine per scale pitch so the measured loop
/// performs no signal synthesis — only the analysis path.
fn build_windows(window_size: usize) -> Vec<Vec<f32>> {
    SCALE_PITCHES
        .iter()
        .map(|&hz| generate_sine(hz, SAMPLE_RATE, window_size))
        .collect()
}

/// GATED BENCH: `samples → AudioEvent → phrase → recap-input` per window.
fn bench_integrated_analysis(c: &mut Criterion) {
    let window_size = build_detector().window_size();
    let windows = build_windows(window_size);

    let mut group = c.benchmark_group("integrated_analysis_path");
    group.throughput(Throughput::Elements(1));
    group.sample_size(60);
    group.measurement_time(std::time::Duration::from_secs(10));
    group.warm_up_time(std::time::Duration::from_secs(1));

    group.bench_function(BenchmarkId::new("latency_per_event", window_size), |b| {
        // Build the pipeline once; the measured closure reuses it so we see
        // the realloc-free steady state, matching the live processing loop.
        let mut detector = build_detector();
        let mut aggregator =
            PhraseAggregator::new(PhraseConfig::default()).expect("valid phrase config");
        aggregator
            .set_score_follower(ScoreFollower::new(create_scale_score()).expect("valid score"));

        let mut idx: usize = 0;
        b.iter(|| {
            let window = &windows[idx % windows.len()];

            // Stage 1: samples → AudioEvent (pitch detection).
            let event = detector.detect(black_box(window));

            // Stage 2: AudioEvent → phrase (incl. score-follower alignment,
            // which runs inside push() when a follower is attached).
            aggregator.push(&event);

            // Stage 3: phrase → recap-input. Drain any phrase that just
            // closed and fold it into a RecapInput — the exact handoff the
            // app makes to the (out-of-band) coaching layer.
            let new_phrases: Vec<PhraseSummary> = aggregator.take_new_phrases();
            let recap = RecapInput {
                instrument: "violin".to_string(),
                instrument_family: String::new(),
                duration_secs: event.timestamp_secs,
                practice_mode: PracticeMode::Practice,
                phrases: new_phrases,
                tips: Vec::new(),
                score_title: Some("C Major Scale".to_string()),
                note_verdicts: Vec::new(),
                idiom_notes: Vec::new(),
                taste_profile: None,
                history_suggestions: Vec::new(),
            };

            idx = idx.wrapping_add(1);
            // Periodically flush + rebuild so the aggregator's internal
            // buffers don't grow unboundedly across the long bench run, and
            // so phrases actually close (exercising stage 3 repeatedly).
            if idx.is_multiple_of(windows.len() * 4) {
                aggregator.flush();
                aggregator =
                    PhraseAggregator::new(PhraseConfig::default()).expect("valid phrase config");
                aggregator.set_score_follower(
                    ScoreFollower::new(create_scale_score()).expect("valid score"),
                );
            }

            black_box(recap)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_integrated_analysis);
criterion_main!(benches);
