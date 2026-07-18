//! Score-follower position tracking through a full piece — the VA #347
//! regression. The tester sang the va-kit C-major scale (4 measures, up then
//! down) for ~50 s: the cursor never left "Measure 1", phrase cards never
//! fired, and the recap counted 66 judged notes (55 missed) on a 16-note
//! score. Root causes: free backward DTW steps + lowest-index tie-breaking
//! let the descent re-trace the ascent back to the top, and the 0.3 s
//! silence "reset" fired on every breath, wiping the verdict watermark.
//!
//! These tests drive the follower the way live audio does — ~45 Hz frames,
//! detached notes with real breath gaps — and assert the honest outcome:
//! the position walks all four measures, every sung note is judged where it
//! lives in the score, and verdict counts stay proportional to notes sung.

use brain::follower::{ScoreFollower, Verdict};
use brain::score::{KeyMode, KeySignature, Measure, ScoreModel, ScoreNote, TimeSignature};
use ears::AudioEvent;

/// Frames arrive at ~45 Hz from the detect loop.
const FRAME_SECS: f64 = 0.022;
/// One detached sung quarter note ≈ 20 voiced frames.
const FRAMES_PER_NOTE: usize = 20;
/// Breath between detached notes — well over the phrase gap (0.3 s), well
/// under a rehearsal break.
const BREATH_SECS: f64 = 0.4;

/// The va-kit sample: C major scale up (m1–2) and down (m3–4), quarter
/// notes, C4 → C5 → C4. Same pitches ascending and descending — the exact
/// shape that dragged the old alignment backward.
fn asc_desc_scale() -> ScoreModel {
    let midis: [[u8; 4]; 4] = [
        [60, 62, 64, 65], // C D E F
        [67, 69, 71, 72], // G A B C5
        [72, 71, 69, 67], // C5 B A G
        [65, 64, 62, 60], // F E D C
    ];
    let measures = midis
        .iter()
        .enumerate()
        .map(|(m, row)| Measure {
            number: m + 1,
            notes: row
                .iter()
                .enumerate()
                .map(|(i, &midi)| ScoreNote {
                    pitch_hz: 440.0 * 2f64.powf((f64::from(midi) - 69.0) / 12.0),
                    midi_number: midi,
                    duration_beats: 1.0,
                    start_beat: i as f64,
                    dynamic: None,
                    is_rest: false,
                })
                .collect(),
        })
        .collect();
    ScoreModel {
        title: "Test Scale (C major)".to_string(),
        composer: None,
        instrument: None,
        time_signature: TimeSignature::default(),
        key_signature: KeySignature {
            fifths: 0,
            mode: KeyMode::Major,
        },
        tempo_bpm: 120.0,
        grand_staff: false,
        measures,
    }
}

fn voiced(hz: f64, t: f64) -> AudioEvent {
    AudioEvent {
        pitch_hz: Some(hz),
        confidence: 0.9,
        amplitude: 0.5,
        timestamp_secs: t,
        is_onset: false,
        note_info: None,
    }
}

fn midi_to_hz(midi: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(midi) - 69.0) / 12.0)
}

/// Sing the full 16-note scale detached, starting at `t0`. Returns
/// (measure the cursor reported at the end of each note, drained verdicts)
/// and the timestamp after the final note.
fn sing_pass(
    f: &mut ScoreFollower,
    t0: f64,
) -> (Vec<usize>, Vec<brain::follower::NoteVerdict>, f64) {
    let scale = asc_desc_scale();
    let midis: Vec<u8> = scale
        .measures
        .iter()
        .flat_map(|m| m.notes.iter().map(|n| n.midi_number))
        .collect();

    let mut t = t0;
    let mut end_measures = Vec::new();
    let mut verdicts = Vec::new();
    for &midi in &midis {
        let hz = midi_to_hz(midi);
        let mut pos = f.current_position();
        for k in 0..FRAMES_PER_NOTE {
            pos = f.align(&voiced(hz, t + k as f64 * FRAME_SECS));
        }
        end_measures.push(pos.measure_number);
        verdicts.extend(f.take_verdicts());
        t += FRAMES_PER_NOTE as f64 * FRAME_SECS + BREATH_SECS;
    }
    (end_measures, verdicts, t)
}

/// AC1 (#347): singing straight through the scale walks the cursor through
/// ALL four measures — the descent must land in measures 3 and 4, not
/// re-trace the ascent back to measure 1.
#[test]
fn position_walks_all_four_measures_on_a_clean_pass() {
    let mut f = ScoreFollower::new(asc_desc_scale()).unwrap();
    let (end_measures, _, _) = sing_pass(&mut f, 0.0);

    assert_eq!(&end_measures[..4], &[1, 1, 1, 1], "ascent, measure 1");
    assert_eq!(&end_measures[4..8], &[2, 2, 2, 2], "ascent, measure 2");
    // Note 8 is the repeated C5 (last of m2 / first of m3): staying on its
    // measure-2 twin is a legitimate cost tie, so measure 2 or 3 are both
    // honest there. From the B4 on, the descent must be in measure 3.
    assert!(
        end_measures[8] == 2 || end_measures[8] == 3,
        "repeated C5 may sit on either twin, got {}",
        end_measures[8]
    );
    assert_eq!(&end_measures[9..12], &[3, 3, 3], "descent, measure 3");
    assert_eq!(&end_measures[12..16], &[4, 4, 4, 4], "descent, measure 4");
}

/// AC2 (#347): a cleanly sung detached pass judges EVERY note — the breath
/// between notes is articulation, not a rehearsal break, so the verdict
/// watermark survives it. Under the old behavior this pass produced 2
/// verdicts, and a noisy real session produced 66-for-16 with phantom
/// misses. Exactly one verdict per sung note, all Hits, attributed to the
/// measures where the notes live.
#[test]
fn a_clean_detached_pass_judges_every_note_as_a_hit() {
    let mut f = ScoreFollower::new(asc_desc_scale()).unwrap();
    let (_, verdicts, _) = sing_pass(&mut f, 0.0);

    assert_eq!(
        verdicts.len(),
        16,
        "one verdict per sung note: {verdicts:?}"
    );
    assert!(
        verdicts.iter().all(|v| v.verdict == Verdict::Hit),
        "an in-tune pass has no misses to confess: {verdicts:?}"
    );
    let measures: Vec<usize> = verdicts.iter().map(|v| v.measure_number).collect();
    let expected: Vec<usize> = (1..=4).flat_map(|m| [m; 4]).collect();
    assert_eq!(
        measures, expected,
        "verdicts land in the measures the notes live in"
    );
}

/// AC4 (#347): one noisy frame must not drag the cursor backward. During
/// the descent the played C5 pitch also lives in measure 2 — the backward
/// path to that twin must stay expensive enough that a single singer's
/// scoop or octave glitch can't regress the display. (The exact-tie
/// tie-break has its own unit test:
/// `a_release_artifact_frame_does_not_flick_the_cursor_back`.)
#[test]
fn a_stray_frame_does_not_drag_the_cursor_backward() {
    let mut f = ScoreFollower::new(asc_desc_scale()).unwrap();
    let scale = asc_desc_scale();
    let midis: Vec<u8> = scale
        .measures
        .iter()
        .flat_map(|m| m.notes.iter().map(|n| n.midi_number))
        .collect();

    // Sing cleanly through the B4 of the descent (note 9, measure 3).
    let mut t = 0.0;
    for &midi in &midis[..10] {
        for k in 0..FRAMES_PER_NOTE {
            f.align(&voiced(midi_to_hz(midi), t + k as f64 * FRAME_SECS));
        }
        t += FRAMES_PER_NOTE as f64 * FRAME_SECS + BREATH_SECS;
    }
    assert_eq!(f.current_position().measure_number, 3, "descent underway");

    // One stray C5 frame — a scoop. C5 lives in measures 2 AND 3; the
    // cursor must hold its ground, not jump back to the measure-2 twin.
    let pos = f.align(&voiced(midi_to_hz(72), t));
    assert!(
        pos.measure_number >= 3,
        "a single noisy frame must not regress the cursor to measure {}",
        pos.measure_number
    );
}

/// AC3 (#347): repeating the scale after a real pause re-anchors at the top
/// and judges the second pass fully — the session total stays proportional
/// to notes sung (2 passes = 32 verdicts, never 66-for-16).
#[test]
fn a_repeat_after_a_pause_reanchors_and_judges_proportionally() {
    let mut f = ScoreFollower::new(asc_desc_scale()).unwrap();
    let (_, first, t) = sing_pass(&mut f, 0.0);
    // 2.5 s pause — a real break, past the 1.5 s relocation threshold.
    let (end_measures, second, _) = sing_pass(&mut f, t + 2.5);

    assert_eq!(end_measures[0], 1, "restart re-anchors at the top");
    assert_eq!(
        first.len() + second.len(),
        32,
        "two passes over 16 notes judge 32 notes, not 66"
    );
    assert!(
        second.iter().all(|v| v.verdict == Verdict::Hit),
        "the second pass judges cleanly from the top: {second:?}"
    );
}
