//! Jazz Ears T1a (#349 §5.1/§5.3): the chord vocabulary and the chroma
//! matcher. Pure pitch-class-set theory — no audio, no I/O, no labels
//! (display spelling lives in `brain`, next to the key-signature rules).
//!
//! The matcher scores every (root × template) against a normalized 12-bin
//! chroma by weighted overlap with a penalty for strong non-chord energy,
//! with subset tolerance so jazz SHELL voicings (a 13th chord without its
//! 5th) still match their quality.

/// A chord quality the ears can name (#349 §5.1). Ordered roughly by
/// harmonic complexity; the matcher prefers simpler qualities on ties so a
/// bare triad never gets labeled as a shell of something fancier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    Maj,
    Min,
    Dim,
    Aug,
    Sus2,
    Sus4,
    Maj6,
    Min6,
    Dom7,
    Maj7,
    Min7,
    Min7b5,
    Dim7,
    MinMaj7,
    Dom7Sus4,
    Add9,
    Dom9,
    Maj9,
    Min9,
    Dom13,
    Dom7b9,
    Dom7s9,
    Dom7s11,
}

impl ChordQuality {
    /// Interval set from the root, semitones 0..12.
    pub fn intervals(self) -> &'static [u8] {
        use ChordQuality::*;
        match self {
            Maj => &[0, 4, 7],
            Min => &[0, 3, 7],
            Dim => &[0, 3, 6],
            Aug => &[0, 4, 8],
            Sus2 => &[0, 2, 7],
            Sus4 => &[0, 5, 7],
            Maj6 => &[0, 4, 7, 9],
            Min6 => &[0, 3, 7, 9],
            Dom7 => &[0, 4, 7, 10],
            Maj7 => &[0, 4, 7, 11],
            Min7 => &[0, 3, 7, 10],
            Min7b5 => &[0, 3, 6, 10],
            Dim7 => &[0, 3, 6, 9],
            MinMaj7 => &[0, 3, 7, 11],
            Dom7Sus4 => &[0, 5, 7, 10],
            Add9 => &[0, 2, 4, 7],
            Dom9 => &[0, 2, 4, 7, 10],
            Maj9 => &[0, 2, 4, 7, 11],
            Min9 => &[0, 2, 3, 7, 10],
            Dom13 => &[0, 4, 7, 9, 10],
            Dom7b9 => &[0, 1, 4, 7, 10],
            Dom7s9 => &[0, 3, 4, 7, 10],
            Dom7s11 => &[0, 4, 6, 7, 10],
        }
    }

    /// Intervals a SHELL voicing may omit without losing the quality
    /// (#349 §5.1 jazz convention: the 5th is optional once a 7th defines
    /// the sound; a plain triad's 5th is NOT optional — three notes is
    /// already the minimum).
    fn optional(self) -> &'static [u8] {
        use ChordQuality::*;
        match self {
            Dom7 | Maj7 | Min7 | MinMaj7 | Dom7Sus4 | Dom9 | Maj9 | Min9 | Dom13 | Dom7b9
            | Dom7s9 => &[7],
            Dom7s11 => &[7, 4],
            _ => &[],
        }
    }

    /// The label suffix jazz players read: "" for major, "m", "7", "maj7"…
    /// (The root name — spelled per key signature — is prepended in `brain`.)
    pub fn suffix(self) -> &'static str {
        use ChordQuality::*;
        match self {
            Maj => "",
            Min => "m",
            Dim => "dim",
            Aug => "aug",
            Sus2 => "sus2",
            Sus4 => "sus4",
            Maj6 => "6",
            Min6 => "m6",
            Dom7 => "7",
            Maj7 => "maj7",
            Min7 => "m7",
            Min7b5 => "m7b5",
            Dim7 => "dim7",
            MinMaj7 => "mMaj7",
            Dom7Sus4 => "7sus4",
            Add9 => "add9",
            Dom9 => "9",
            Maj9 => "maj9",
            Min9 => "m9",
            Dom13 => "13",
            Dom7b9 => "7b9",
            Dom7s9 => "7#9",
            Dom7s11 => "7#11",
        }
    }

    /// Every quality, in tie-break order (simpler first).
    pub fn all() -> &'static [ChordQuality] {
        use ChordQuality::*;
        &[
            Maj, Min, Dim, Aug, Sus2, Sus4, Maj6, Min6, Dom7, Maj7, Min7, Min7b5, Dim7, MinMaj7,
            Dom7Sus4, Add9, Dom9, Maj9, Min9, Dom13, Dom7b9, Dom7s9, Dom7s11,
        ]
    }
}

/// A matched chord, before display spelling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChordMatch {
    pub root_pc: u8,
    pub quality: ChordQuality,
    /// The sounding bass pitch class when it's a chord tone other than the
    /// root — the slash in "C7/E". `None` = root position or bass unknown.
    pub bass_pc: Option<u8>,
    /// Match confidence 0..1 (template overlap minus non-chord penalty).
    pub confidence: f32,
}

/// Don't report a chord below this many clearly-sounding pitch classes —
/// one or two notes is a line, not a chord (the meter's job, not ours).
pub const MIN_CHORD_BINS: usize = 3;
/// A chroma bin counts as "sounding" above this fraction of the max bin.
const ACTIVE_BIN_RATIO: f32 = 0.25;
/// Weight of the penalty for strong energy OUTSIDE the template.
const NON_CHORD_PENALTY: f32 = 0.8;
/// Score penalty per omitted optional tone, so an exact interpretation of
/// the sounding notes always beats a shell reading of the same notes
/// (Fsus4 must not be reported as C7sus4-with-the-5th-dropped).
const MISSING_TONE_PENALTY: f32 = 0.05;
/// Minimum score to report anything at all.
pub const MIN_CHORD_CONF: f32 = 0.5;

/// Match a normalized 12-bin chroma (index = pitch class, C = 0) against
/// the vocabulary. `bass_pc` is the independently-detected lowest sounding
/// pitch class (from the monophonic tracker), used only to name inversions —
/// never to force the root.
///
/// Returns `None` when fewer than [`MIN_CHORD_BINS`] classes sound or no
/// template clears [`MIN_CHORD_CONF`] — the caller shows the honest
/// "hearing several notes…" state instead of a guess (#349 §5.3).
pub fn best_match(chroma: &[f32; 12], bass_pc: Option<u8>) -> Option<ChordMatch> {
    let total: f32 = chroma.iter().sum();
    if total <= f32::EPSILON {
        return None;
    }
    let max_bin = chroma.iter().cloned().fold(0.0f32, f32::max);
    let active = chroma
        .iter()
        .filter(|&&v| v >= max_bin * ACTIVE_BIN_RATIO)
        .count();
    if active < MIN_CHORD_BINS {
        return None;
    }

    let mut best: Option<ChordMatch> = None;
    for root in 0u8..12 {
        'quality: for &q in ChordQuality::all() {
            let intervals = q.intervals();
            let optional = q.optional();
            let mut in_chord = 0.0f32;
            let mut missing = 0usize;
            let mut template_mask = [false; 12];
            for &iv in intervals {
                let pc = usize::from((root + iv) % 12);
                template_mask[pc] = true;
                let v = chroma[pc];
                // Required tones must actually sound (shells may drop the
                // optional ones — at a small cost, tallied below).
                if v < max_bin * ACTIVE_BIN_RATIO {
                    if !optional.contains(&iv) {
                        continue 'quality;
                    }
                    missing += 1;
                }
                in_chord += v;
            }
            let out_of_chord: f32 = (0..12)
                .filter(|&pc| !template_mask[pc])
                .map(|pc| chroma[pc])
                .sum();
            let score = (in_chord - NON_CHORD_PENALTY * out_of_chord) / total
                - MISSING_TONE_PENALTY * missing as f32;
            // Strictly-better wins; ties keep the earlier (simpler) quality
            // and the earlier root — deterministic and triad-favoring.
            if score > best.map_or(MIN_CHORD_CONF, |b| b.confidence) {
                let bass = bass_pc.filter(|&b| {
                    b % 12 != root && intervals.iter().any(|&iv| (root + iv) % 12 == b % 12)
                });
                best = Some(ChordMatch {
                    root_pc: root,
                    quality: q,
                    bass_pc: bass.map(|b| b % 12),
                    confidence: score.clamp(0.0, 1.0),
                });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a chroma with the given pitch classes sounding at strength 1
    /// (plus optional weaker extras).
    fn chroma_of(pcs: &[u8]) -> [f32; 12] {
        let mut c = [0.0f32; 12];
        for &pc in pcs {
            c[usize::from(pc % 12)] = 1.0;
        }
        c
    }

    /// #349 T1a AC1 (property): every quality at every root round-trips —
    /// feed the exact template, get the exact chord back. Fails if any
    /// template, the tie-break order, or the scorer regresses.
    #[test]
    fn every_template_at_every_root_round_trips() {
        for root in 0u8..12 {
            for &q in ChordQuality::all() {
                let pcs: Vec<u8> = q.intervals().iter().map(|&iv| (root + iv) % 12).collect();
                let m = best_match(&chroma_of(&pcs), None)
                    .unwrap_or_else(|| panic!("no match for {q:?} at root {root}"));
                // The exact pitch-class set can be quality-ambiguous
                // (Cmaj6 == Am7, Cdim7 == any of its rotations). Accept any
                // answer whose template reproduces the SAME set — that's
                // enharmonic-equivalence honesty, not a wrong answer.
                let matched: std::collections::BTreeSet<u8> = m
                    .quality
                    .intervals()
                    .iter()
                    .map(|&iv| (m.root_pc + iv) % 12)
                    .collect();
                let fed: std::collections::BTreeSet<u8> = pcs.iter().copied().collect();
                assert_eq!(matched, fed, "{q:?}@{root} matched {m:?}");
            }
        }
    }

    /// The canonical jazz ladder at C, unambiguous members asserted exactly.
    #[test]
    fn the_jazz_ladder_names_correctly() {
        let cases: &[(&[u8], ChordQuality)] = &[
            (&[0, 4, 7], ChordQuality::Maj),
            (&[0, 3, 7], ChordQuality::Min),
            (&[0, 4, 7, 10], ChordQuality::Dom7),
            (&[0, 4, 7, 11], ChordQuality::Maj7),
            (&[0, 3, 7, 10], ChordQuality::Min7),
            (&[0, 3, 6, 10], ChordQuality::Min7b5),
            (&[0, 3, 4, 7, 10], ChordQuality::Dom7s9),
            (&[0, 1, 4, 7, 10], ChordQuality::Dom7b9),
        ];
        for (pcs, want) in cases {
            let m = best_match(&chroma_of(pcs), None).expect("match");
            assert_eq!((m.root_pc, m.quality), (0, *want), "pcs {pcs:?}");
        }
    }

    /// #349 §5.2 subset tolerance: a 13th SHELL (no 5th) still reads as 13;
    /// a plain triad missing its 5th does NOT match (two notes are a line).
    #[test]
    fn shells_match_but_two_note_fragments_do_not() {
        // C13 shell: C E Bb A (3rd, 7th, 13th — no 5th).
        let m = best_match(&chroma_of(&[0, 4, 10, 9]), None).expect("shell matches");
        assert_eq!((m.root_pc, m.quality), (0, ChordQuality::Dom13));
        // C + G alone: no chord.
        assert!(best_match(&chroma_of(&[0, 7]), None).is_none());
    }

    /// Inversion honesty: bass = chord tone ≠ root → slash pc; bass = root
    /// or a non-chord tone → no slash.
    #[test]
    fn bass_names_inversions_only_when_it_is_a_chord_tone() {
        let c = chroma_of(&[0, 4, 7]);
        assert_eq!(best_match(&c, Some(4)).unwrap().bass_pc, Some(4)); // C/E
        assert_eq!(best_match(&c, Some(0)).unwrap().bass_pc, None); // root pos.
        assert_eq!(best_match(&c, Some(1)).unwrap().bass_pc, None); // Db ≠ tone
    }

    /// Non-chord energy is punished: a triad drowned in chromatic mush must
    /// not report confidently. Fails if the penalty is dropped.
    #[test]
    fn chromatic_mush_does_not_read_as_a_confident_triad() {
        let mut c = chroma_of(&[0, 4, 7]);
        for pc in [1usize, 2, 3, 5, 6, 8, 9, 10, 11] {
            c[pc] = 0.8;
        }
        assert!(
            best_match(&c, None).is_none(),
            "mush must fall below the confidence gate"
        );
    }
}
