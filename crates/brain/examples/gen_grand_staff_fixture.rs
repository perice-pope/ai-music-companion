//! One-shot fixture generator: `cargo run -p brain --example gen_grand_staff_fixture`
//! writes the piano grand-staff fixture used by the OSMD render-contract
//! tests (#417-3). Kept as an example so regeneration is a command, not a
//! ritual. The model must stay in sync with `piano_drill_model()` in
//! `tests/emitted_notation_test.rs` (the drift check).
use brain::score::emit::score_model_to_musicxml;
use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};

fn main() {
    let note = |midi: u8, beats: f64, start: f64| ScoreNote {
        pitch_hz: brain::score::midi_to_hz(f64::from(midi)),
        midi_number: midi,
        duration_beats: beats,
        start_beat: start,
        dynamic: None,
        is_rest: false,
    };
    let rest = |beats: f64, start: f64| ScoreNote {
        pitch_hz: 0.0,
        midi_number: 0,
        duration_beats: beats,
        start_beat: start,
        dynamic: None,
        is_rest: true,
    };
    // Measure 1 exercises every grand-staff rule at once: a straddling
    // chord (whole on the bass staff), a rest that stays in the left hand,
    // and an eighth run crossing middle C (beam breaks at the staff change).
    let m1 = vec![
        note(48, 1.0, 0.0), // C3 ┐
        note(64, 1.0, 0.0), // E4 ├ chord, lowest below middle C → staff 2
        note(67, 1.0, 0.0), // G4 ┘
        rest(1.0, 1.0),     // breath inside the bass phrase → staff 2
        note(55, 0.5, 2.0), // G3 bass ┐ pair
        note(57, 0.5, 2.5), // A3 bass ┘
        note(60, 0.5, 3.0), // C4 treble ┐ pair
        note(64, 0.5, 3.5), // E4 treble ┘
    ];
    // Measure 2: all-treble — the edge where the LEFT hand is silent and
    // the bass staff must still render (empty, not broken).
    let m2 = vec![note(60, 1.0, 0.0), note(64, 1.0, 1.0), note(67, 2.0, 2.0)];
    let model = ScoreModel {
        title: "Piano Drill".to_string(),
        composer: None,
        instrument: Some("Piano".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 90.0,
        grand_staff: true,
        measures: vec![
            Measure {
                number: 1,
                notes: m1,
            },
            Measure {
                number: 2,
                notes: m2,
            },
        ],
    };
    print!("{}", score_model_to_musicxml(&model));
}
