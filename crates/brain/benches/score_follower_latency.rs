use brain::follower::ScoreFollower;
use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ears::AudioEvent;

fn create_mozart_scale() -> ScoreModel {
    // A realistic Mozart-style scale: C4 to C5 (8 notes).
    let notes = vec![
        ScoreNote {
            pitch_hz: 261.63,
            midi_number: 60,
            duration_beats: 0.5,
            start_beat: 0.0,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 293.66,
            midi_number: 62,
            duration_beats: 0.5,
            start_beat: 0.5,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 329.63,
            midi_number: 64,
            duration_beats: 0.5,
            start_beat: 1.0,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 349.23,
            midi_number: 65,
            duration_beats: 0.5,
            start_beat: 1.5,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 392.0,
            midi_number: 67,
            duration_beats: 0.5,
            start_beat: 2.0,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 440.0,
            midi_number: 69,
            duration_beats: 0.5,
            start_beat: 2.5,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 493.88,
            midi_number: 71,
            duration_beats: 0.5,
            start_beat: 3.0,
            dynamic: None,
            is_rest: false,
        },
        ScoreNote {
            pitch_hz: 523.25,
            midi_number: 72,
            duration_beats: 0.5,
            start_beat: 3.5,
            dynamic: None,
            is_rest: false,
        },
    ];

    ScoreModel {
        title: "Mozart Scale".to_string(),
        composer: Some("W. A. Mozart".to_string()),
        instrument: Some("Violin".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 120.0,
        measures: vec![Measure { number: 1, notes }],
    }
}

fn benchmark_score_follower_align(c: &mut Criterion) {
    let score = create_mozart_scale();
    let mut follower = ScoreFollower::new(score).unwrap();

    let pitches = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25];
    let mut event_idx = 0;

    c.bench_function("score_follower_align_single_event", |b| {
        b.iter(|| {
            let hz = black_box(pitches[event_idx % pitches.len()]);
            let ts = black_box((event_idx as f64) * 0.5);
            let event = AudioEvent {
                pitch_hz: Some(hz),
                confidence: 0.95,
                amplitude: 0.7,
                timestamp_secs: ts,
                is_onset: true,
            };

            let _pos = follower.align(&event);
            event_idx += 1;
            if event_idx >= 8 {
                event_idx = 0;
                follower.reset();
            }
        });
    });
}

fn benchmark_score_follower_many_events(c: &mut Criterion) {
    let score = create_mozart_scale();
    let mut follower = ScoreFollower::new(score).unwrap();

    let pitches = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25];

    c.bench_function("score_follower_align_100_events", |b| {
        b.iter(|| {
            for i in 0..100 {
                let hz = black_box(pitches[i % pitches.len()]);
                let ts = black_box((i as f64) * 0.05); // ~50ms per event
                let event = AudioEvent {
                    pitch_hz: Some(hz),
                    confidence: 0.95,
                    amplitude: 0.7,
                    timestamp_secs: ts,
                    is_onset: true,
                };

                let _pos = follower.align(&event);
            }
            follower.reset();
        });
    });
}

fn benchmark_score_follower_with_unvoiced_events(c: &mut Criterion) {
    let score = create_mozart_scale();
    let mut follower = ScoreFollower::new(score).unwrap();

    let pitches = [261.63, 293.66, 329.63, 349.23, 392.0, 440.0, 493.88, 523.25];
    let mut event_idx = 0;

    c.bench_function("score_follower_align_mixed_voiced_unvoiced", |b| {
        b.iter(|| {
            // Every other event is unvoiced (silence, breath, etc.)
            let is_voiced = event_idx % 2 == 0;
            let hz = if is_voiced {
                black_box(pitches[event_idx / 2 % pitches.len()])
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
    benchmark_score_follower_align,
    benchmark_score_follower_many_events,
    benchmark_score_follower_with_unvoiced_events
);
criterion_main!(benches);
