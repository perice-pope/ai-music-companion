//! One-shot fixture generator: `cargo run -p brain --example gen_eighth_fixture`
//! writes the beamed eighth-run fixture used by the OSMD render-contract
//! tests. Kept as an example so regeneration is a command, not a ritual.
use brain::score::emit::score_model_to_musicxml;
use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};

fn main() {
    let eighth = |midi: u8, start: f64| ScoreNote {
        pitch_hz: brain::score::midi_to_hz(f64::from(midi)),
        midi_number: midi,
        duration_beats: 0.5,
        start_beat: start,
        dynamic: None,
        is_rest: false,
    };
    // Beats 1-2: a four-group of eighths; beat 3: an UNBEAMED (and
    // untyped) quarter; beat 4: a pair — one measure exercising mixed
    // typed/untyped notes and both group sizes through the real OSMD parse.
    let mut notes = vec![
        eighth(60, 0.0),
        eighth(62, 0.5),
        eighth(64, 1.0),
        eighth(65, 1.5),
    ];
    notes.push(ScoreNote {
        pitch_hz: brain::score::midi_to_hz(67.0),
        midi_number: 67,
        duration_beats: 1.0,
        start_beat: 2.0,
        dynamic: None,
        is_rest: false,
    });
    notes.push(eighth(69, 3.0));
    notes.push(eighth(71, 3.5));
    let model = ScoreModel {
        title: "Eighth Run".to_string(),
        composer: None,
        instrument: Some("Melody".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 60.0,
        grand_staff: false,
        measures: vec![Measure { number: 1, notes }],
    };
    print!("{}", score_model_to_musicxml(&model));
}
