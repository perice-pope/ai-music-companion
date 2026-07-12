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
