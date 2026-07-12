//! #349 T2a — stacked chord cells through the score path, pinned end to end:
//! variations::generate (block figures) → sequence_to_score_model →
//! score_model_to_musicxml → parse_musicxml_str. The CellStaff view is the
//! other consumer, pinned in `brain::score::cellstaff` / CellStaff.tsx.

use brain::coach::{key_signature_for, sequence_to_score_model};
use brain::score::emit::score_model_to_musicxml;
use brain::score::musicxml::parse_musicxml_str;
use variations::{
    generate, ArpeggioPattern, ChordModifier, ChordType, DirectionMode, RhythmSpec, VariationSpec,
};

fn stacked_c7_spec() -> VariationSpec {
    VariationSpec {
        roots: (60..72).collect(),
        cell: None,
        degrees: None,
        progression: None,
        scale: None,
        chord: Some(ChordModifier {
            chord: ChordType::Dominant7,
            pattern: ArpeggioPattern::Ascending,
            inversion: 0,
            stacked: true,
        }),
        interval: None,
        enclosure: None,
        direction: DirectionMode::Forward,
        rhythm: RhythmSpec::default(),
        randomize_roots: false,
    }
}

/// #349 T2a AC1 (score path): a 12-key stacked drill emits `<chord/>` marks
/// — three per four-tone cell (the 2nd+ simultaneous notes) — and each
/// measure's TIME still adds up (chord members don't advance the clock), so
/// the emitted file stays valid MusicXML. Fails if the adapter splits the
/// simultaneity or the emitter stops marking chords.
#[test]
fn a_stacked_drill_emits_chord_marks_with_honest_measure_time() {
    let seq = generate(&stacked_c7_spec(), 7);
    let model = sequence_to_score_model(&seq, "C7 block chords", key_signature_for(0, "major"));
    assert_eq!(model.measures.len(), 12, "one cell per measure");

    let xml = score_model_to_musicxml(&model);
    let chord_marks = xml.matches("<chord/>").count();
    assert_eq!(chord_marks, 12 * 3, "2nd..4th tone of every cell is marked");

    // Time integrity: per measure, the durations of notes that ADVANCE time
    // (rests + the first note of each simultaneity) sum to the bar.
    for measure in &model.measures {
        let mut advancing = 0.0;
        let mut prev_start: Option<f64> = None;
        for n in &measure.notes {
            let is_chord_member =
                !n.is_rest && prev_start.is_some_and(|p| (n.start_beat - p).abs() < 1e-6);
            if !is_chord_member {
                advancing += n.duration_beats;
            }
            if !n.is_rest {
                prev_start = Some(n.start_beat);
            }
        }
        assert!(
            (advancing - 4.0).abs() < 1e-6,
            "measure {} advances {advancing} beats, not the bar",
            measure.number
        );
    }
}

/// Round trip: the emitted stacked drill reparses with every simultaneity
/// intact — four sounding notes sharing each measure's downbeat. Fails if
/// the parser or emitter loses `<chord/>` semantics in either direction.
#[test]
fn a_stacked_drill_survives_the_round_trip() {
    let seq = generate(&stacked_c7_spec(), 7);
    let model = sequence_to_score_model(&seq, "C7 block chords", key_signature_for(0, "major"));
    let xml = score_model_to_musicxml(&model);
    let back = parse_musicxml_str(&xml).expect("emitted stacked drill reparses");

    assert_eq!(back.measures.len(), 12);
    for measure in &back.measures {
        let downbeat_notes = measure
            .notes
            .iter()
            .filter(|n| !n.is_rest && n.start_beat.abs() < 1e-6)
            .count();
        assert_eq!(
            downbeat_notes, 4,
            "measure {}: the whole stack sounds on the downbeat",
            measure.number
        );
    }
}
