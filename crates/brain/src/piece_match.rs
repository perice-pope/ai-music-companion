//! Library-match identification core (#214, #417 item 5, S1a).
//!
//! "Shazam matches recordings; we match PIECES." The engine is a
//! transposition- and tempo-invariant interval n-gram index over
//! [`ScoreModel`]s (see `docs/architecture/piece-identification.md`):
//! retrieval narrows the library to candidates by shared melodic
//! n-grams with POSITIONAL COHERENCE (hits that agree on where-in-the-
//! piece), and a margin gate prefers silence over a wrong name — the
//! wrong-Beethoven lesson (#417) is the founding rule here.
//!
//! S1a is pure logic: no persistence, no audio, no UI. S1b wires it to
//! the score store at import time and to the live note stream, and adds
//! the follower-confirmation stage; until then the retrieval margin is
//! deliberately strict. S1b TODOs: a top-k finalists accessor (the
//! design's §3.3 confirmation consumes pre-gate candidates) and
//! delete-by-id via a reverse map once the index persists (remove_score
//! scans all postings today — fine in-memory).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::score::ScoreModel;

/// Intervals per n-gram window (5 notes). Long enough that random
/// noodling rarely collides positionally; near-full-window queries
/// survive a stray wrong note, short ones fail toward silence (the
/// right direction).
pub(crate) const NGRAM_INTERVALS: usize = 4;
/// The query reads the last this-many notes of the live stream.
pub(crate) const QUERY_WINDOW: usize = 20;
/// A candidate must place at least this many n-grams at ONE alignment
/// offset. 6 coherent windows ≈ 10 consecutive right notes.
pub(crate) const MIN_COHERENT_HITS: usize = 6;
/// Those coherent hits must come from at least this many DISTINCT
/// n-grams (review MF1): an ostinato loop or a chromatic run aligns the
/// SAME few windows over and over — counts without identity. Real melody
/// clears this trivially; self-similar figuration reads as silence.
pub(crate) const MIN_DISTINCT_COHERENT: usize = 6;
/// The winner must beat the runner-up by this factor on coherent hits —
/// ambiguity reads as silence, never a coin flip.
pub(crate) const MARGIN_RATIO: f64 = 2.0;
/// Interval clamp — a leap wider than an octave carries no more
/// identity than an octave (and clamping identically on both sides of
/// the index/query pair never breaks a match).
const INTERVAL_CLAMP: i16 = 12;

/// A confirmed library match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The indexed score's id.
    pub id: u64,
    /// N-gram hits agreeing on one alignment offset — the evidence.
    pub coherent_hits: usize,
    /// All hits for this candidate (coherent or not).
    pub total_hits: usize,
    /// The winning alignment offset (score interval position minus query
    /// interval position) — S1b's seed for follower confirmation and
    /// "open the score at your measure".
    pub offset: i64,
}

/// The melodic surface of a score: the TOP line per onset (a chord
/// collapses to its highest note — what a listener tracks), rests
/// skipped, in play order. Durations are never read, which is what
/// makes identification tempo- and rhythm-invariant by construction.
pub fn melody_line(model: &ScoreModel) -> Vec<u8> {
    let mut line: Vec<u8> = Vec::new();
    for measure in &model.measures {
        // Review MF2: real MusicXML (voices, <backup>, grand staves) lists
        // notes in DOCUMENT order, not time order — the parser preserves
        // that. Sort by onset first; only then does "top note per onset"
        // mean what it says. The sort is total (start_beats are finite)
        // and ties break by pitch, so the result is deterministic.
        let mut sounding: Vec<(f64, u8)> = measure
            .notes
            .iter()
            .filter(|n| !n.is_rest)
            .map(|n| (n.start_beat, n.midi_number))
            .collect();
        sounding.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut i = 0;
        while i < sounding.len() {
            let onset = sounding[i].0;
            let mut top = sounding[i].1;
            let mut j = i + 1;
            while j < sounding.len() && (sounding[j].0 - onset).abs() < 1e-6 {
                top = top.max(sounding[j].1);
                j += 1;
            }
            line.push(top);
            i = j;
        }
    }
    line
}

/// Clamped semitone deltas between consecutive melody notes.
fn intervals(line: &[u8]) -> Vec<i16> {
    line.windows(2)
        .map(|w| (i16::from(w[1]) - i16::from(w[0])).clamp(-INTERVAL_CLAMP, INTERVAL_CLAMP))
        .collect()
}

fn ngram_key(window: &[i16]) -> u64 {
    let mut h = DefaultHasher::new();
    window.hash(&mut h);
    h.finish()
}

/// The in-memory n-gram index over the library's melodic surfaces.
#[derive(Debug, Default)]
pub struct PieceIndex {
    /// n-gram hash → postings of (score id, interval position).
    postings: HashMap<u64, Vec<(u64, usize)>>,
}

impl PieceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index a score's melodic n-grams. Re-indexing the same id first
    /// removes the old postings (an edited score replaces itself).
    pub fn index_score(&mut self, id: u64, model: &ScoreModel) {
        self.remove_score(id);
        let line = melody_line(model);
        let ivs = intervals(&line);
        for (pos, window) in ivs.windows(NGRAM_INTERVALS).enumerate() {
            self.postings
                .entry(ngram_key(window))
                .or_default()
                .push((id, pos));
        }
    }

    /// Forget a score entirely.
    pub fn remove_score(&mut self, id: u64) {
        self.postings.retain(|_, posts| {
            posts.retain(|(pid, _)| *pid != id);
            !posts.is_empty()
        });
    }

    /// Identify the piece behind the recent melody, or — the honesty
    /// rule — return None when the evidence is thin or ambiguous.
    pub fn identify(&self, recent_midi: &[u8]) -> Option<Match> {
        let start = recent_midi.len().saturating_sub(QUERY_WINDOW);
        let ivs = intervals(&recent_midi[start..]);
        if ivs.len() < NGRAM_INTERVALS {
            return None; // Too little played — never a guess.
        }
        // candidate id → (alignment offset → (hit count, distinct keys)).
        // Offsets are score_pos − query_pos: hits from real playing agree.
        // Distinct keys are the MF1 defense: periodic material (ostinato,
        // chromatic run) piles the SAME windows onto one offset — counts
        // without identity.
        type OffsetEvidence = HashMap<i64, (usize, std::collections::HashSet<u64>)>;
        let mut alignments: HashMap<u64, OffsetEvidence> = HashMap::new();
        let mut totals: HashMap<u64, usize> = HashMap::new();
        for (qpos, window) in ivs.windows(NGRAM_INTERVALS).enumerate() {
            let key = ngram_key(window);
            if let Some(posts) = self.postings.get(&key) {
                for &(id, spos) in posts {
                    let slot = alignments
                        .entry(id)
                        .or_default()
                        .entry(spos as i64 - qpos as i64)
                        .or_default();
                    slot.0 += 1;
                    slot.1.insert(key);
                    *totals.entry(id).or_default() += 1;
                }
            }
        }
        // Deterministic ranking: coherent hits, then total, then id —
        // no HashMap iteration order reaches the outcome. The best offset
        // per candidate breaks count-ties by distinct keys then offset.
        let mut ranked: Vec<(Match, usize)> = alignments
            .iter()
            .map(|(&id, offsets)| {
                let (offset, count, distinct) = offsets
                    .iter()
                    .map(|(&off, (count, keys))| (off, *count, keys.len()))
                    .max_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)).then(b.0.cmp(&a.0)))
                    .unwrap_or((0, 0, 0));
                (
                    Match {
                        id,
                        coherent_hits: count,
                        total_hits: totals.get(&id).copied().unwrap_or(0),
                        offset,
                    },
                    distinct,
                )
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.coherent_hits
                .cmp(&a.0.coherent_hits)
                .then(b.0.total_hits.cmp(&a.0.total_hits))
                .then(a.0.id.cmp(&b.0.id))
        });
        let (best, best_distinct) = ranked.first()?.clone();
        if best.coherent_hits < MIN_COHERENT_HITS {
            return None; // Thin evidence — silence beats a maybe.
        }
        if best_distinct < MIN_DISTINCT_COHERENT {
            return None; // Counts without identity (review MF1) — silence.
        }
        if let Some((second, _)) = ranked.get(1) {
            if (best.coherent_hits as f64) < MARGIN_RATIO * second.coherent_hits as f64 {
                return None; // Ambiguous — silence beats a coin flip.
            }
        }
        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::{KeySignature, Measure, ScoreNote, TimeSignature};

    /// A quarter-note melody split into 4/4 measures — the smallest honest
    /// ScoreModel (the emitted-notation fixture idiom).
    fn model_from(midis: &[u8]) -> ScoreModel {
        let mut measures: Vec<Measure> = Vec::new();
        for (m, chunk) in midis.chunks(4).enumerate() {
            let notes = chunk
                .iter()
                .enumerate()
                .map(|(i, &midi)| ScoreNote {
                    pitch_hz: 440.0,
                    midi_number: midi,
                    duration_beats: 1.0,
                    start_beat: i as f64,
                    dynamic: None,
                    is_rest: false,
                })
                .collect();
            measures.push(Measure {
                number: m + 1,
                notes,
            });
        }
        ScoreModel {
            title: "t".into(),
            composer: None,
            instrument: None,
            time_signature: TimeSignature::default(),
            key_signature: KeySignature::default(),
            tempo_bpm: 100.0,
            measures,
            grand_staff: false,
        }
    }

    /// A distinctive 24-note line (Für-Elise-ish contour, not a scale).
    fn piece_a() -> Vec<u8> {
        vec![
            76, 75, 76, 75, 76, 71, 74, 72, 69, 60, 64, 69, 71, 64, 68, 71, 72, 64, 76, 75, 76, 75,
            76, 71,
        ]
    }

    /// A different distinctive line (arpeggiated with turns).
    fn piece_b() -> Vec<u8> {
        vec![
            60, 67, 64, 72, 71, 67, 69, 65, 62, 65, 69, 74, 72, 69, 66, 62, 60, 72, 67, 64, 60, 55,
            59, 62,
        ]
    }

    fn library() -> PieceIndex {
        let mut idx = PieceIndex::new();
        idx.index_score(1, &model_from(&piece_a()));
        idx.index_score(2, &model_from(&piece_b()));
        // A third piece: a lyrical stepwise-but-not-scale line.
        idx.index_score(
            3,
            &model_from(&[
                67, 69, 67, 64, 62, 64, 67, 72, 71, 69, 71, 67, 64, 60, 62, 64, 62, 59, 55, 62, 60,
                64, 67, 72,
            ]),
        );
        idx
    }

    /// AC1: a mid-piece window of real playing identifies the piece.
    #[test]
    fn identify_matches_a_library_piece_mid_stream() {
        let idx = library();
        // The player enters at note 6 of piece A and plays 14 notes.
        let played = &piece_a()[6..20];
        let m = idx.identify(played).expect("a real excerpt identifies");
        assert_eq!(m.id, 1);
        assert!(m.coherent_hits >= MIN_COHERENT_HITS);
    }

    /// AC2: the RV loop must not defeat identification — the same
    /// excerpt transposed +3 still matches (intervals, not pitches).
    #[test]
    fn transposition_never_defeats_identification() {
        let idx = library();
        let transposed: Vec<u8> = piece_a()[6..20].iter().map(|&n| n + 3).collect();
        let m = idx.identify(&transposed).expect("transposed still matches");
        assert_eq!(m.id, 1);
    }

    /// AC4 (first-class): free material stays SILENT. A scale, an
    /// arpeggio exercise, and noodling must never name a piece.
    #[test]
    fn scales_arpeggios_and_noodling_stay_silent() {
        let idx = library();
        let scale: Vec<u8> = (60..=76).collect();
        assert_eq!(idx.identify(&scale), None, "a scale is not a piece");
        let arps: Vec<u8> = vec![
            60, 64, 67, 72, 67, 64, 62, 65, 69, 74, 69, 65, 64, 68, 71, 76,
        ];
        assert_eq!(idx.identify(&arps), None, "an exercise is not a piece");
        let noodle: Vec<u8> = vec![
            62, 65, 61, 70, 66, 59, 63, 71, 58, 67, 61, 73, 60, 68, 64, 57,
        ];
        assert_eq!(idx.identify(&noodle), None, "noodling is not a piece");
    }

    /// AC4/edge: the SAME piece under two ids is ambiguous — the margin
    /// reads it as silence, never a coin flip.
    #[test]
    fn duplicate_windows_refuse_on_margin() {
        let mut idx = library();
        idx.index_score(9, &model_from(&piece_a())); // duplicate of id 1
        assert_eq!(
            idx.identify(&piece_a()[6..20]),
            None,
            "a duplicate makes the answer ambiguous — silence"
        );
    }

    /// AC5: chords collapse to the top line; rests are skipped.
    #[test]
    fn melody_line_collapses_chords_and_skips_rests() {
        let mut model = model_from(&[60, 64, 67, 72]);
        // Turn beat 0 of measure 1 into a chord (60+64+67 same onset)
        // and beat 2 into a rest.
        let m1 = &mut model.measures[0].notes;
        m1[1].start_beat = 0.0; // 64 joins the chord at onset 0
        m1[2].is_rest = true;
        m1[2].midi_number = 0;
        assert_eq!(melody_line(&model), vec![64, 72]);
    }

    /// AC4's other face (pins MIN_COHERENT_HITS itself): a SHORT real
    /// fragment — 8 true notes of piece A — is genuine but thin evidence
    /// (4 coherent windows < the floor of 6). Silence, not a guess. The
    /// same passage two notes longer clears the floor (the boundary).
    #[test]
    fn a_short_true_fragment_is_not_enough() {
        let idx = library();
        assert_eq!(
            idx.identify(&piece_a()[6..14]),
            None,
            "8 notes = 4 coherent windows — under the floor"
        );
        assert_eq!(
            idx.identify(&piece_a()[6..16]).map(|m| m.id),
            Some(1),
            "10 notes = 6 coherent windows — exactly the floor"
        );
    }

    /// AC6: a removed score can no longer match; the others still do.
    #[test]
    fn removing_a_score_forgets_it() {
        let mut idx = library();
        idx.remove_score(1);
        assert_eq!(idx.identify(&piece_a()[6..20]), None);
        assert_eq!(idx.identify(&piece_b()[4..18]).map(|m| m.id), Some(2));
    }

    /// AC7 + edge: determinism (no hash-order in the outcome), and a
    /// too-short query never guesses.
    #[test]
    fn identification_is_deterministic_and_never_guesses_short() {
        let idx = library();
        let played = &piece_a()[6..20];
        let first = idx.identify(played);
        for _ in 0..10 {
            assert_eq!(idx.identify(played), first);
        }
        assert_eq!(idx.identify(&piece_a()[..4]), None, "4 notes is a guess");
    }

    /// Review MF1: self-similar figuration must read as silence. A bare
    /// ostinato loop and a chromatic run rack up COUNTS at aligned
    /// offsets, but from the same few n-grams — no identity.
    #[test]
    fn ostinatos_and_chromatic_runs_stay_silent() {
        let mut idx = PieceIndex::new();
        let mut alberti = Vec::new();
        for _ in 0..4 {
            alberti.extend_from_slice(&[60, 64, 67, 64]);
        }
        alberti.extend_from_slice(&[72, 71, 69, 67, 65, 64, 62, 60]);
        idx.index_score(1, &model_from(&alberti));
        let mut chroma = vec![55, 59, 62];
        chroma.extend(60..73);
        chroma.extend_from_slice(&[72, 67, 64, 60]);
        idx.index_score(2, &model_from(&chroma));

        let loop_query: Vec<u8> = (0..5).flat_map(|_| [60, 64, 67, 64]).collect();
        assert_eq!(
            idx.identify(&loop_query),
            None,
            "an ostinato is not a piece"
        );
        let run_query: Vec<u8> = (60..80).collect();
        assert_eq!(idx.identify(&run_query), None, "a run is not a piece");
    }

    /// Review MF3(f): POSITIONAL coherence is the separator — disjoint
    /// fragments of one piece, shuffled, have hits but no single
    /// alignment. Coherence must refuse.
    #[test]
    fn scattered_fragments_never_cohere() {
        let idx = library();
        let a = piece_a();
        let mut scattered = Vec::new();
        scattered.extend_from_slice(&a[14..19]);
        scattered.extend_from_slice(&a[0..5]);
        scattered.extend_from_slice(&a[7..12]);
        assert_eq!(
            idx.identify(&scattered),
            None,
            "fragments without ONE alignment are not a performance"
        );
    }

    /// Review MF3(f), the ranking half: a DECOY piece stuffed with
    /// repeated copies of the true piece's fragments out-totals the true
    /// piece while cohering nowhere. Total-hits ranking picks the decoy
    /// (and dies on the distinct gate → None); coherence picks the truth.
    #[test]
    fn a_fragment_stuffed_decoy_never_outranks_the_real_piece() {
        let mut idx = PieceIndex::new();
        let run: Vec<u8> = vec![64, 62, 60, 65, 64, 67, 65, 69, 71, 72, 69, 67, 71, 74, 72];
        let mut real = run.clone();
        real.extend_from_slice(&[76, 74, 71, 67, 64]);
        idx.index_score(1, &model_from(&real));
        // The decoy: two 7-note fragments of the run, each FOUR times,
        // separated by filler — many hits, scattered offsets.
        let mut decoy = Vec::new();
        for _ in 0..4 {
            decoy.extend_from_slice(&run[0..7]);
            decoy.extend_from_slice(&[50, 53]);
        }
        for _ in 0..4 {
            decoy.extend_from_slice(&run[7..14]);
            decoy.extend_from_slice(&[52, 55]);
        }
        idx.index_score(2, &model_from(&decoy));

        let m = idx
            .identify(&run[..14])
            .expect("the real piece identifies over the decoy");
        assert_eq!(m.id, 1, "coherence beats raw totals");
        assert!(
            m.total_hits <= m.coherent_hits + 2,
            "the winner's evidence is one aligned run"
        );
    }

    /// Review MF3(h2): two LIVE candidates — a shared quotation resolved
    /// by piece-specific material — pins the deterministic ranking with
    /// real competition.
    #[test]
    fn a_quotation_resolves_to_the_piece_that_continues_it() {
        let mut idx = PieceIndex::new();
        let quote: Vec<u8> = vec![67, 65, 64, 62, 60, 62, 64, 67];
        let mut a = quote.clone();
        a.extend_from_slice(&[72, 71, 67, 64, 69, 71, 72, 76]);
        let mut b = quote.clone();
        b.extend_from_slice(&[59, 62, 65, 69, 66, 62, 59, 55]);
        idx.index_score(1, &model_from(&a));
        idx.index_score(2, &model_from(&b));
        assert_eq!(
            idx.identify(&quote),
            None,
            "a shared quotation is ambiguous"
        );
        let played = &a[2..16];
        let first = idx.identify(played);
        assert_eq!(first.as_ref().map(|m| m.id), Some(1));
        for _ in 0..10 {
            assert_eq!(idx.identify(played), first);
        }
    }

    /// Review MF2: notes arrive in DOCUMENT order in real MusicXML
    /// (voices, <backup>, grand staves) — melody is the top line per
    /// ONSET, not per list position.
    #[test]
    fn melody_line_survives_document_order_voices() {
        let mut model = model_from(&[60, 64, 67, 72]);
        model.measures[0].notes.push(ScoreNote {
            pitch_hz: 440.0,
            midi_number: 48,
            duration_beats: 2.0,
            start_beat: 0.0,
            dynamic: None,
            is_rest: false,
        });
        model.measures[0].notes.push(ScoreNote {
            pitch_hz: 440.0,
            midi_number: 55,
            duration_beats: 2.0,
            start_beat: 2.0,
            dynamic: None,
            is_rest: false,
        });
        assert_eq!(
            melody_line(&model),
            vec![60, 64, 67, 72],
            "the LH never becomes melody; onsets collapse to the top note"
        );
    }

    /// Spec §6: a leap wider than an octave clamps identically on both
    /// sides — pinned with ONE wide leap inside real melody. (All-leap
    /// material collapses to ±12 periodicity and the distinct-key gate
    /// rightly silences it — the MF1 design working, not a bug.)
    #[test]
    fn a_wide_leap_clamps_and_still_matches() {
        let mut idx = PieceIndex::new();
        let mut tune = piece_b();
        tune[7] = 91;
        idx.index_score(1, &model_from(&tune));
        assert_eq!(idx.identify(&tune[2..16]).map(|m| m.id), Some(1));
    }
}
