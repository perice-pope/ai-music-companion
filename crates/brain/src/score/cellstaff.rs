//! CellStaff view (#292 slice 1): everything the frontend's dot-staff renderer
//! needs, computed HERE so the frontend does zero music theory. Each note maps
//! to a diatonic staff step (spelled per the key signature — flats in flat
//! keys, #277) plus the accidental glyph to draw, if any. The renderer is pure
//! geometry over this view.

use serde::{Deserialize, Serialize};
use variations::GeneratedSequence;

use super::emit::midi_to_pitch;
use super::KeySignature;

/// One dot on the staff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellStaffNote {
    pub midi: u8,
    pub start_beat: f64,
    pub duration_beats: f64,
    /// Diatonic staff steps above the treble staff's bottom line (E4 = 0).
    /// Line = even, space = odd; negative = below the staff (ledger territory).
    pub step: i16,
    /// Accidental glyph to draw, when the spelled alter differs from what the
    /// key signature already implies for this letter: `-1` flat, `0` natural,
    /// `1` sharp. `None` = no glyph (the signature covers it).
    pub accidental: Option<i8>,
    /// `true` when this note's pitch class is one of the sequence's roots.
    /// RV color rule (founder, 2026-07-08): only root notes carry their
    /// pitch-class color on the staff — every other note draws white, so the
    /// roots pop and the cell reads as shape-around-a-root.
    #[serde(default)]
    pub is_root: bool,
}

/// The whole staff view for one cell/drill rep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellStaffView {
    /// Key signature on the circle of fifths (negative = flats).
    pub fifths: i8,
    pub beats_per_measure: u8,
    /// Total beats including the trailing rest padding, for windowing.
    pub total_beats: f64,
    pub notes: Vec<CellStaffNote>,
}

/// Letters the sharps/flats of a signature hit, in signature order.
const SHARP_ORDER: [char; 7] = ['F', 'C', 'G', 'D', 'A', 'E', 'B'];
const FLAT_ORDER: [char; 7] = ['B', 'E', 'A', 'D', 'G', 'C', 'F'];

/// What the key signature implies for a letter: +1, -1, or 0.
fn key_alter_for(letter: char, fifths: i8) -> i8 {
    if fifths > 0 {
        let n = (fifths as usize).min(7);
        if SHARP_ORDER[..n].contains(&letter) {
            return 1;
        }
    } else if fifths < 0 {
        let n = ((-fifths) as usize).min(7);
        if FLAT_ORDER[..n].contains(&letter) {
            return -1;
        }
    }
    0
}

fn letter_index(letter: char) -> i16 {
    match letter {
        'C' => 0,
        'D' => 1,
        'E' => 2,
        'F' => 3,
        'G' => 4,
        'A' => 5,
        'B' => 6,
        _ => 0,
    }
}

/// Diatonic staff step of a midi note under `key` (E4 = 0). Shared with the
/// edit engine so gestures and rendering can never disagree.
pub fn staff_step(midi: u8, key: &KeySignature) -> i16 {
    let (letter, _, octave) = midi_to_pitch(midi, key.fifths < 0);
    i16::from(octave) * 7 + letter_index(letter) - (4 * 7 + 2)
}

/// The accidental glyph a midi note would need under `key`, if any.
pub fn accidental_for(midi: u8, key: &KeySignature) -> Option<i8> {
    let (letter, alter, _) = midi_to_pitch(midi, key.fifths < 0);
    if alter == key_alter_for(letter, key.fifths) {
        None
    } else {
        Some(alter)
    }
}

/// Build the staff view for a generated sequence under a key signature.
/// Pure and deterministic.
pub fn cell_staff_view(seq: &GeneratedSequence, key: KeySignature) -> CellStaffView {
    let notes: Vec<CellStaffNote> = seq
        .notes
        .iter()
        .map(|n| {
            let step = staff_step(n.midi, &key);
            let accidental = accidental_for(n.midi, &key);
            CellStaffNote {
                midi: n.midi,
                start_beat: n.start_beat,
                duration_beats: n.duration_beats,
                step,
                accidental,
                // Straight from the generator, which flags roots PER CELL —
                // in a 12-root row every pitch class is somebody's root, so
                // a global pitch-class check would color everything (review
                // must-fix on the first cut of this rule).
                is_root: n.is_root,
            }
        })
        .collect();
    let total_beats = seq
        .notes
        .last()
        .map(|n| n.start_beat + n.duration_beats)
        .unwrap_or(0.0)
        .max(f64::from(seq.beats_per_measure));
    CellStaffView {
        fifths: key.fifths,
        beats_per_measure: seq.beats_per_measure,
        total_beats,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use variations::GeneratedNote;

    fn seq(midis: &[u8]) -> GeneratedSequence {
        GeneratedSequence {
            notes: midis
                .iter()
                .enumerate()
                .map(|(i, &m)| GeneratedNote {
                    midi: m,
                    start_beat: i as f64,
                    duration_beats: 1.0,
                    // The generator flags roots per cell; this fixture mirrors
                    // its single-root shape (first pitch class = the root).
                    is_root: m % 12 == midis.first().copied().unwrap_or(60) % 12,
                })
                .collect(),
            target_midi: midis.to_vec(),
            root_order: vec![midis.first().copied().unwrap_or(60)],
            label: "test".to_owned(),
            tempo_bpm: 100.0,
            beats_per_measure: 4,
        }
    }

    fn key(fifths: i8) -> KeySignature {
        KeySignature {
            fifths,
            mode: super::super::KeyMode::Major,
        }
    }

    /// #292 AC: midi → staff step is anchored at E4 = bottom line and counts
    /// diatonically — middle C sits two steps below the staff, F5 on the top
    /// line. Fails if the mapping (or its E4 rebase) shifts.
    #[test]
    fn steps_are_anchored_at_the_bottom_line() {
        let v = cell_staff_view(&seq(&[64, 60, 77, 71]), key(0));
        let steps: Vec<i16> = v.notes.iter().map(|n| n.step).collect();
        // E4 = 0, C4 = -2, F5 = 8, B4 = 4.
        assert_eq!(steps, vec![0, -2, 8, 4]);
    }

    /// #292 AC (spelling): in a flat key the black key spells FLAT and needs
    /// no glyph when the signature already covers it — Bb in F major (1 flat)
    /// draws no accidental, while B natural there needs the natural sign.
    /// Fails if spelling ignores the key or the implied-alter logic breaks.
    #[test]
    fn flat_keys_spell_flat_and_dedupe_against_the_signature() {
        let v = cell_staff_view(&seq(&[70, 71]), key(-1)); // F major: Bb in sig
        let bb = &v.notes[0];
        assert_eq!(bb.step, 4, "Bb sits on the B line (middle)");
        assert_eq!(bb.accidental, None, "the signature already flats B");
        let b_nat = &v.notes[1];
        assert_eq!(b_nat.accidental, Some(0), "B natural needs the natural");
    }

    /// Sharp keys mirror the rule: F# in D major (2 sharps) needs no glyph;
    /// F natural needs a natural; C# outside the signature (in C major) draws
    /// a sharp.
    #[test]
    fn sharp_keys_mirror_the_dedupe() {
        let v = cell_staff_view(&seq(&[66, 65]), key(2)); // D major: F#, C#
        assert_eq!(v.notes[0].accidental, None, "F# is in the signature");
        assert_eq!(v.notes[1].accidental, Some(0), "F natural needs a glyph");
        let v2 = cell_staff_view(&seq(&[61]), key(0));
        assert_eq!(v2.notes[0].accidental, Some(1), "C# in C major draws #");
    }

    /// RV color rule: the view carries the generator's PER-CELL root flags
    /// through untouched — the renderer colors exactly what the generator
    /// marked (roots colored, everything else white). Fails if the view drops
    /// the flag or re-derives it with its own (global) pitch-class logic.
    #[test]
    fn the_view_carries_per_cell_root_flags_through() {
        let s = seq(&[64, 67, 76, 71]); // E4, G4, E5, B4 — root E
        let v = cell_staff_view(&s, key(0));
        let flags: Vec<bool> = v.notes.iter().map(|n| n.is_root).collect();
        assert_eq!(
            flags,
            vec![true, false, true, false],
            "E4 and E5 carry the root flag; G and B do not"
        );
    }

    /// Windowing inputs: total beats covers the last note and never reads
    /// below one measure, so an empty/short cell still draws a full bar.
    #[test]
    fn total_beats_supports_windowing() {
        let v = cell_staff_view(&seq(&[60]), key(0));
        assert_eq!(v.total_beats, 4.0, "min one measure");
        let v2 = cell_staff_view(&seq(&[60; 10]), key(0));
        assert_eq!(v2.total_beats, 10.0);
        assert_eq!(v2.beats_per_measure, 4);
    }
}
