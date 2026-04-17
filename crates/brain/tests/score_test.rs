//! Integration tests for the score parsing module.

use brain::score::{KeyMode, ScoreError, ScoreParser};
use std::io::Write;
use std::path::PathBuf;

/// Path to the test fixtures directory.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn musicxml_fixture_produces_valid_score_model() {
    let path = fixture_path("simple_scale.musicxml");
    let model = ScoreParser::parse(&path).expect("parse MusicXML fixture");

    assert_eq!(model.title, "C Major Scale");
    assert_eq!(model.composer.as_deref(), Some("Test Composer"));
    assert_eq!(model.instrument.as_deref(), Some("Trumpet"));
    assert_eq!(model.time_signature.beats, 4);
    assert_eq!(model.time_signature.beat_type, 4);
    assert_eq!(model.key_signature.fifths, 0);
    assert_eq!(model.key_signature.mode, KeyMode::Major);
    assert!((model.tempo_bpm - 120.0).abs() < 0.1);
    assert_eq!(model.measures.len(), 2, "Fixture has 2 measures");
}

#[test]
fn score_model_note_count_matches_expected() {
    let path = fixture_path("simple_scale.musicxml");
    let model = ScoreParser::parse(&path).expect("parse fixture");

    let total_notes: usize = model.measures.iter().map(|m| m.notes.len()).sum();
    assert_eq!(
        total_notes, 8,
        "C major scale should have 8 notes, got {total_notes}"
    );

    // 4 notes per measure
    assert_eq!(model.measures[0].notes.len(), 4);
    assert_eq!(model.measures[1].notes.len(), 4);
}

#[test]
fn score_model_pitch_values_are_correct() {
    let path = fixture_path("simple_scale.musicxml");
    let model = ScoreParser::parse(&path).expect("parse fixture");

    // Expected: C4, D4, E4, F4, G4, A4, B4, C5
    let expected_hz: &[f64] = &[261.63, 293.66, 329.63, 349.23, 392.00, 440.00, 493.88, 523.25];
    let expected_midi: &[u8] = &[60, 62, 64, 65, 67, 69, 71, 72];

    let all_notes: Vec<_> = model
        .measures
        .iter()
        .flat_map(|m| m.notes.iter())
        .collect();

    assert_eq!(all_notes.len(), expected_hz.len());

    for (i, note) in all_notes.iter().enumerate() {
        assert!(
            (note.pitch_hz - expected_hz[i]).abs() < 0.1,
            "Note {i} Hz: expected {}, got {}",
            expected_hz[i],
            note.pitch_hz
        );
        assert_eq!(
            note.midi_number, expected_midi[i],
            "Note {i} MIDI: expected {}, got {}",
            expected_midi[i], note.midi_number
        );
        assert!(!note.is_rest, "Note {i} should not be a rest");
    }
}

#[test]
fn score_model_serialization_roundtrip() {
    let path = fixture_path("simple_scale.musicxml");
    let model = ScoreParser::parse(&path).expect("parse fixture");

    let json = serde_json::to_string(&model).expect("serialize ScoreModel");
    let roundtrip: brain::score::ScoreModel =
        serde_json::from_str(&json).expect("deserialize ScoreModel");

    assert_eq!(roundtrip.title, model.title);
    assert_eq!(roundtrip.measures.len(), model.measures.len());

    let original_notes: usize = model.measures.iter().map(|m| m.notes.len()).sum();
    let roundtrip_notes: usize = roundtrip.measures.iter().map(|m| m.notes.len()).sum();
    assert_eq!(original_notes, roundtrip_notes);
}

#[test]
fn auto_detect_routes_xml_extension() {
    // Copy the .musicxml fixture content to a .xml temp file to exercise
    // the .xml branch of the auto-detect match (previously this test used
    // the .musicxml file and tested the wrong extension).
    let src = fixture_path("simple_scale.musicxml");
    let xml = std::fs::read(&src).expect("read fixture");

    let mut tmp = tempfile::Builder::new()
        .prefix("score-xml-ext-")
        .suffix(".xml")
        .tempfile()
        .expect("create .xml temp file");
    tmp.write_all(&xml).expect("write fixture to temp");

    // sanity: temp path really ends in .xml (not .musicxml)
    let path = tmp.path();
    assert_eq!(
        path.extension().and_then(|e| e.to_str()),
        Some("xml"),
        "temp file should have .xml extension, got: {path:?}"
    );

    let model = ScoreParser::parse(path).expect("parse .xml via auto-detect");
    assert_eq!(model.title, "C Major Scale");
    assert_eq!(model.measures.len(), 2);
}

#[test]
fn auto_detect_rejects_mxl_without_zip_support() {
    // .mxl is compressed MusicXML and would need a ZIP dep we haven't taken
    // on. Auto-detect should refuse it up-front with UnsupportedFormat
    // rather than trying read_to_string (which would fail opaquely on
    // compressed bytes).
    let path = PathBuf::from("/tmp/never-created.mxl");
    let err = ScoreParser::parse(&path).expect_err("mxl should be unsupported");
    assert!(
        matches!(err, ScoreError::UnsupportedFormat(ref ext) if ext == "mxl"),
        "Expected UnsupportedFormat(\"mxl\"), got: {err}"
    );
}

#[test]
fn multi_part_fixture_selects_part_zero_by_default() {
    let path = fixture_path("two_part_duet.musicxml");
    let model = ScoreParser::parse(&path).expect("parse duet fixture");

    // Part 0 is Trumpet: C4 D4 E4 F4
    assert_eq!(model.instrument.as_deref(), Some("Trumpet"));
    let midi: Vec<u8> = model.measures[0]
        .notes
        .iter()
        .map(|n| n.midi_number)
        .collect();
    assert_eq!(
        midi,
        vec![60, 62, 64, 65],
        "Part 0 (Trumpet) should be C4 D4 E4 F4"
    );
}

#[test]
fn multi_part_fixture_selects_part_one() {
    let path = fixture_path("two_part_duet.musicxml");
    let model = ScoreParser::parse_musicxml_part(&path, 1).expect("parse part 1");

    // Part 1 is Trombone: C3 B2 A2 G2
    assert_eq!(model.instrument.as_deref(), Some("Trombone"));
    let midi: Vec<u8> = model.measures[0]
        .notes
        .iter()
        .map(|n| n.midi_number)
        .collect();
    assert_eq!(
        midi,
        vec![48, 47, 45, 43],
        "Part 1 (Trombone) should be C3 B2 A2 G2"
    );
}

#[test]
fn multi_part_fixture_out_of_range_is_part_not_found() {
    let path = fixture_path("two_part_duet.musicxml");
    let err = ScoreParser::parse_musicxml_part(&path, 2).expect_err("part 2 does not exist");
    assert!(
        matches!(err, ScoreError::PartNotFound(2)),
        "Expected PartNotFound(2), got: {err}"
    );
}

#[test]
fn list_parts_returns_expected_names() {
    let path = fixture_path("two_part_duet.musicxml");
    let names = ScoreParser::list_musicxml_parts(&path).expect("list parts");
    assert_eq!(
        names,
        vec!["Trumpet".to_string(), "Trombone".to_string()],
        "Duet fixture should list Trumpet then Trombone"
    );
}

#[test]
fn part_zero_and_part_one_produce_different_notes() {
    // Regression test for the multi-part fix: the old code always returned
    // the first part regardless of the requested index.
    let path = fixture_path("two_part_duet.musicxml");
    let p0 = ScoreParser::parse_musicxml_part(&path, 0).expect("parse part 0");
    let p1 = ScoreParser::parse_musicxml_part(&path, 1).expect("parse part 1");

    let midi0: Vec<u8> = p0.measures[0].notes.iter().map(|n| n.midi_number).collect();
    let midi1: Vec<u8> = p1.measures[0].notes.iter().map(|n| n.midi_number).collect();

    assert_ne!(
        midi0, midi1,
        "Part 0 and Part 1 should have different notes (got {midi0:?} for both)"
    );
    assert_eq!(p0.instrument.as_deref(), Some("Trumpet"));
    assert_eq!(p1.instrument.as_deref(), Some("Trombone"));
}
