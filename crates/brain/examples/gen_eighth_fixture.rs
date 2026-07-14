//! One-shot fixture generator: `cargo run -p brain --example gen_eighth_fixture`
//! writes the beamed eighth-run fixture used by the OSMD render-contract
//! tests. Kept as an example so regeneration is a command, not a ritual.
use brain::score::emit::score_model_to_musicxml;
use brain::score::{KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};

fn main() {
    let notes = (0..8u8)
        .map(|i| {
            let midi = [60, 62, 64, 65, 67, 69, 71, 72][i as usize];
            ScoreNote {
                pitch_hz: brain::score::midi_to_hz(f64::from(midi)),
                midi_number: midi,
                duration_beats: 0.5,
                start_beat: f64::from(i) * 0.5,
                dynamic: None,
                is_rest: false,
            }
        })
        .collect();
    let model = ScoreModel {
        title: "Eighth Run".to_string(),
        composer: None,
        instrument: Some("Melody".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 60.0,
        measures: vec![Measure { number: 1, notes }],
    };
    print!("{}", score_model_to_musicxml(&model));
}
