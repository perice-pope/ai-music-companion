//! #327 — lesson-drill notation ⇄ OSMD, pinned end to end.
//!
//! VA runs #324/#327 hit *"undefined is not an object (evaluating
//! 't3.StaffEntries')"* on some lesson keys (G major crashed, D# major
//! rendered) — the shape behind it is a measure OSMD parses empty. The
//! emitter's invalid tempo `<direction>` (#356) produced exactly that, and
//! lesson drills feed the same emitter. This sweep makes the whole surface
//! a CI contract instead of a per-key lottery:
//!
//! - every tonic × every drill kind at the mid-ladder rung (the shape a
//!   fresh lesson actually deals), plus
//! - the full difficulty ladder — every distinct scale/chord/interval band,
//!   enclosures, inversions, triplet rhythm, stacked block chords — at the
//!   two VA-sighted keys (G major, C#/Db major).
//!
//! The Rust layer asserts the emitted XML can never carry the known
//! OSMD-killing shapes; the committed JSON fixture bridges to
//! `apps/desktop/src/components/LessonDrillNotation.osmd.test.ts`, which
//! loads every entry into the REAL OSMD parser and walks the exact
//! `StaffEntries` chain from the crash.
//!
//! Regenerating after an intentional emitter/generator change:
//!   REGEN_LESSON_DRILL_FIXTURE=1 cargo test -p brain --test lesson_drill_notation_test
//! then re-run `pnpm test` so the OSMD sweep re-verifies the new XML.

use brain::coach::{drill_for, key_signature_for, sequence_to_score_model, DrillKind};
use brain::score::emit::score_model_to_musicxml;
use brain::score::ScoreModel;

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../apps/desktop/src/test-fixtures/emitted-lesson-drills.json"
);

const KINDS: [(DrillKind, &str); 4] = [
    (DrillKind::WarmupScale, "warmup_scale"),
    (DrillKind::ArpeggioEnclosure, "arpeggio_enclosure"),
    (DrillKind::IntervalDrill, "interval_drill"),
    (DrillKind::RunThrough, "run_through"),
];

/// The VA-sighted tonics: G major (#324's crash) and C#/Db major (#327).
const SIGHTED_TONICS: [u8; 2] = [7, 1];

/// Difficulty rungs that produce distinct notation shapes: 0 (single root,
/// simplest material), 5 (enclosures + random direction), 7 (inversions),
/// 9 (triplets, 12 roots, hardest material band).
const LADDER_RUNGS: [u8; 4] = [0, 5, 7, 9];

struct Case {
    kind: DrillKind,
    kind_name: &'static str,
    difficulty: u8,
    tonic: u8,
    polyphonic: bool,
}

/// Every (kind, difficulty, tonic) shape the sweep covers. Deterministic:
/// the per-case seed is its index, so the fixture regenerates bit-identical.
fn sweep() -> Vec<Case> {
    let mut cases = Vec::new();
    // Key axis: all 12 tonics × all kinds at rung 3 (multi-root, UpDown
    // patterns) — the first shape an adapted lesson deals a real player.
    for tonic in 0..12u8 {
        for (kind, kind_name) in KINDS {
            cases.push(Case {
                kind,
                kind_name,
                difficulty: 3,
                tonic,
                polyphonic: false,
            });
        }
    }
    // Shape axis: the full ladder at the sighted keys, one sharp-side and
    // one flat-side signature, so material bands cross both spelling tables.
    for tonic in SIGHTED_TONICS {
        for (kind, kind_name) in KINDS {
            for difficulty in LADDER_RUNGS {
                cases.push(Case {
                    kind,
                    kind_name,
                    difficulty,
                    tonic,
                    polyphonic: false,
                });
            }
        }
        // Stacked block chords (#349 T2a) engrave simultaneities — a shape
        // no melodic drill produces.
        for difficulty in [5, 9] {
            cases.push(Case {
                kind: DrillKind::ArpeggioEnclosure,
                kind_name: "arpeggio_enclosure",
                difficulty,
                tonic,
                polyphonic: true,
            });
        }
    }
    cases
}

fn sounding_per_measure(model: &ScoreModel) -> Vec<usize> {
    model
        .measures
        .iter()
        .map(|m| m.notes.iter().filter(|n| !n.is_rest).count())
        .collect()
}

struct Emitted {
    id: String,
    fifths: i8,
    sounding: Vec<usize>,
    xml: String,
}

/// The exact pipeline `drill_dto` runs (commands.rs): drill → key signature
/// → score model → MusicXML.
fn emit(case: &Case, seed: u64) -> Emitted {
    let drill = drill_for(
        case.kind,
        case.difficulty,
        case.tonic,
        case.polyphonic,
        seed,
    );
    let key = key_signature_for(drill.tonic, &drill.mode);
    let fifths = key.fifths;
    let model = sequence_to_score_model(&drill.sequence, &drill.sequence.label, key);
    let stacked = if case.polyphonic { "-stacked" } else { "" };
    Emitted {
        id: format!(
            "t{}-{}-d{}{}",
            case.tonic, case.kind_name, case.difficulty, stacked
        ),
        fifths,
        sounding: sounding_per_measure(&model),
        xml: score_model_to_musicxml(&model),
    }
}

/// One fixture entry per line so the committed file diffs per-case.
fn fixture_json() -> String {
    let lines: Vec<String> = sweep()
        .iter()
        .enumerate()
        .map(|(i, case)| {
            let e = emit(case, i as u64);
            serde_json::to_string(&serde_json::json!({
                "id": e.id,
                "fifths": e.fifths,
                "sounding_per_measure": e.sounding,
                "music_xml": e.xml,
            }))
            .expect("fixture entry serializes")
        })
        .collect();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

/// The known OSMD-killers can never come back, for ANY key × shape:
/// a `<direction>` without `<direction-type>` (blanks the measure — #356)
/// and a measure with no sounding notes at all (the empty-StaffEntries
/// shape behind the #324/#327 TypeError). Every drill fills every measure
/// of its row, so an empty one is an emission bug, not a musical choice.
#[test]
fn no_drill_shape_emits_an_osmd_killing_measure() {
    // Three decorrelated seeds per case: real lessons draw seeds from the
    // whole u64 space, so the invariant must hold across the generator's
    // randomization (root shuffle, per-root direction), not just at the
    // fixture's seed.
    let salts = [0u64, 1_000_003, 0x9E37_79B9_7F4A_7C15];
    for (i, case) in sweep().iter().enumerate() {
        for e in salts
            .iter()
            .map(|s| emit(case, (i as u64).wrapping_add(*s)))
        {
            assert!(
                !e.sounding.is_empty() && e.sounding.iter().all(|&n| n > 0),
                "{}: a measure engraved with zero sounding notes — the exact \
             shape OSMD's cursor dies on: {:?}",
                e.id,
                e.sounding
            );
            let directions =
                e.xml.matches("<direction ").count() + e.xml.matches("<direction>").count();
            let direction_types = e.xml.matches("<direction-type>").count();
            assert_eq!(
                directions, direction_types,
                "{}: every <direction> needs a <direction-type> — OSMD drops the \
             whole measure's notes otherwise",
                e.id
            );
        }
    }
}

/// The committed fixture IS the emitter's current output for the sweep. If
/// this fails, the generator or emitter changed: regenerate (see the module
/// doc) and re-run the frontend OSMD sweep so the render contract is
/// re-verified against the new XML.
#[test]
fn committed_fixture_matches_emitter_output() {
    let expected = fixture_json();
    if std::env::var_os("REGEN_LESSON_DRILL_FIXTURE").is_some() {
        std::fs::write(FIXTURE_PATH, &expected).expect("fixture regenerates");
        return;
    }
    let committed = std::fs::read_to_string(FIXTURE_PATH).unwrap_or_default();
    assert_eq!(
        committed, expected,
        "emitted-lesson-drills.json drifted from the emitter — regenerate \
         with REGEN_LESSON_DRILL_FIXTURE=1 and re-run the frontend OSMD sweep"
    );
}
