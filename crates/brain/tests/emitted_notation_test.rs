//! #356 — the emitted-notation contract, pinned end to end.
//!
//! The VA run showed imported scores rendering HALF their notes: the emitter
//! wrote the tempo as `<direction><sound/></direction>` with no
//! `<direction-type>` child — invalid MusicXML — and OSMD 1.9.x reacts by
//! silently dropping every note of the measure containing it (always
//! measure 1). The staff was also labeled OSMD's anonymous "Music" instead
//! of the imported part's name.
//!
//! These tests pin the fix at the Rust layer; the OSMD layer is pinned by
//! `apps/desktop/src/components/EmittedNotation.osmd.test.ts`, which loads
//! the SAME committed fixtures into the real OSMD parser and counts what
//! survives. The drift test here is the bridge: it fails whenever the
//! emitter's output for the va-kit sources stops matching the committed
//! fixtures, so the frontend test can never silently test stale XML.

use brain::score::emit::score_model_to_musicxml;
use brain::score::midi::{list_midi_parts, parse_midi_bytes_track};
use brain::score::musicxml::parse_musicxml_str;
use brain::score::ScoreModel;

const BAND_MID: &[u8] = include_bytes!("../../../va-testing-kit/samples/sample-band-c-major.mid");
const SCALE_XML: &str =
    include_str!("../../../va-testing-kit/samples/sample-score-c-major-scale.musicxml");

const FIXTURE_TRUMPET: &str =
    include_str!("../../../apps/desktop/src/test-fixtures/emitted-band-trumpet.musicxml");
const FIXTURE_BASS: &str =
    include_str!("../../../apps/desktop/src/test-fixtures/emitted-band-bass.musicxml");
const FIXTURE_SCALE: &str =
    include_str!("../../../apps/desktop/src/test-fixtures/emitted-scale-score.musicxml");

/// Import one named part of the band fixture.
fn band_part(name: &str) -> ScoreModel {
    let parts = list_midi_parts(BAND_MID).expect("band fixture lists parts");
    let part = parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("band fixture has a {name} part"));
    parse_midi_bytes_track(BAND_MID, Some(part.track_index)).expect("part imports")
}

/// Sounding (non-rest) note count per measure.
fn sounding_per_measure(model: &ScoreModel) -> Vec<usize> {
    model
        .measures
        .iter()
        .map(|m| m.notes.iter().filter(|n| !n.is_rest).count())
        .collect()
}

/// The committed fixtures ARE the emitter's current output for the va-kit
/// sources. If this fails, the emitter changed: regenerate the fixtures
/// (parse each source, run `score_model_to_musicxml`, overwrite the files
/// under `apps/desktop/src/test-fixtures/`) and re-run the frontend OSMD
/// sweep so the render contract is re-verified against the new XML.
#[test]
fn committed_fixtures_match_emitter_output() {
    let cases: [(&str, String); 3] = [
        (
            "emitted-band-trumpet.musicxml",
            score_model_to_musicxml(&band_part("Trumpet")),
        ),
        (
            "emitted-band-bass.musicxml",
            score_model_to_musicxml(&band_part("Bass")),
        ),
        (
            "emitted-scale-score.musicxml",
            score_model_to_musicxml(&parse_musicxml_str(SCALE_XML).expect("scale score parses")),
        ),
    ];
    for ((name, emitted), fixture) in
        cases
            .into_iter()
            .zip([FIXTURE_TRUMPET, FIXTURE_BASS, FIXTURE_SCALE])
    {
        assert_eq!(
            emitted, fixture,
            "{name} drifted from the emitter — regenerate the fixture and \
             re-run the frontend OSMD sweep"
        );
    }
}

/// #356 AC: the band-fixture Trumpet part carries all 8 notes into the
/// emitted XML — 4 per measure, no measure emptied — and the staff is
/// labeled "Trumpet", not "Music".
#[test]
fn band_trumpet_emits_all_notes_and_its_name() {
    let model = band_part("Trumpet");
    assert_eq!(sounding_per_measure(&model), vec![4, 4]);

    let xml = score_model_to_musicxml(&model);
    assert!(
        xml.contains("<part-name>Trumpet</part-name>"),
        "the imported part names the staff:\n{xml}"
    );
    // Round-trip: nothing lost between model and XML.
    let reparsed = parse_musicxml_str(&xml).expect("emitted XML parses");
    assert_eq!(sounding_per_measure(&reparsed), vec![4, 4]);
}

/// #356 (OSMD contract): no emitted `<direction>` may lack a
/// `<direction-type>` child, in ANY fixture — that exact shape is what made
/// OSMD blank measure 1. Sweeps all three sources so a future direction
/// emission (a new dynamic, a mid-score tempo) can't reintroduce it.
#[test]
fn no_emitted_direction_lacks_a_direction_type() {
    for (name, xml) in [
        (
            "band-trumpet",
            score_model_to_musicxml(&band_part("Trumpet")),
        ),
        ("band-bass", score_model_to_musicxml(&band_part("Bass"))),
        (
            "scale-score",
            score_model_to_musicxml(&parse_musicxml_str(SCALE_XML).unwrap()),
        ),
    ] {
        let directions = xml.matches("<direction ").count() + xml.matches("<direction>").count();
        let direction_types = xml.matches("<direction-type>").count();
        assert!(directions > 0, "{name}: tempo direction expected");
        assert_eq!(
            directions, direction_types,
            "{name}: every <direction> needs a <direction-type> — OSMD drops \
             the whole measure's notes otherwise:\n{xml}"
        );
    }
}

/// The beamed-eighths contract model: one 4/4 measure of eight straight
/// eighths (C major run). Kept tiny and constructed (not imported) so the
/// fixture is stable; the frontend OSMD sweep asserts the beams parse into
/// two groups of four.
fn eighth_run_model() -> ScoreModel {
    use brain::score::{KeySignature, Measure, ScoreNote, TimeSignature};
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
    ScoreModel {
        title: "Eighth Run".to_string(),
        composer: None,
        instrument: Some("Melody".to_string()),
        time_signature: TimeSignature::default(),
        key_signature: KeySignature::default(),
        tempo_bpm: 60.0,
        grand_staff: false,
        measures: vec![Measure { number: 1, notes }],
    }
}

const FIXTURE_EIGHTH_RUN: &str =
    include_str!("../../../apps/desktop/src/test-fixtures/emitted-eighth-run.musicxml");

/// The beamed fixture IS the emitter's output for the eighth-run model —
/// same drift contract as the #356 fixtures: if this fails, regenerate the
/// fixture and re-run the OSMD sweep.
#[test]
fn beamed_fixture_matches_emitter_output() {
    assert_eq!(
        score_model_to_musicxml(&eighth_run_model()),
        FIXTURE_EIGHTH_RUN,
        "emitted-eighth-run.musicxml drifted from the emitter"
    );
}

/// #417-3: the piano grand-staff fixture's model — must stay in sync with
/// `examples/gen_grand_staff_fixture.rs` (regeneration command).
fn piano_drill_model() -> ScoreModel {
    use brain::score::{KeySignature, Measure, ScoreNote, TimeSignature};
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
    let m1 = vec![
        note(48, 1.0, 0.0),
        note(64, 1.0, 0.0),
        note(67, 1.0, 0.0),
        rest(1.0, 1.0),
        note(55, 0.5, 2.0),
        note(57, 0.5, 2.5),
        note(60, 0.5, 3.0),
        note(64, 0.5, 3.5),
    ];
    let m2 = vec![note(60, 1.0, 0.0), note(64, 1.0, 1.0), note(67, 2.0, 2.0)];
    ScoreModel {
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
    }
}

const FIXTURE_GRAND_STAFF: &str =
    include_str!("../../../apps/desktop/src/test-fixtures/emitted-piano-grand-staff.musicxml");

/// #417-3: the grand-staff fixture IS the emitter's output for the piano
/// drill model — same drift contract as the other fixtures: if this fails,
/// regenerate (`cargo run -p brain --example gen_grand_staff_fixture`) and
/// re-run the frontend OSMD sweep.
#[test]
fn grand_staff_fixture_matches_emitter_output() {
    assert_eq!(
        score_model_to_musicxml(&piano_drill_model()),
        FIXTURE_GRAND_STAFF,
        "emitted-piano-grand-staff.musicxml drifted from the emitter"
    );
}
