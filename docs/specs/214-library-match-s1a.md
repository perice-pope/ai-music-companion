# Spec: Library-match core (#214/#417-5, S1a)

## 1. Summary
The pure matching engine from docs/architecture/piece-identification.md:
a transposition- and tempo-invariant interval n-gram index over
ScoreModels, retrieval with positional coherence, and a margin gate that
prefers silence over a wrong identification. No store, no UI, no audio —
those are S1b's wiring. Everything here is deterministic, offline logic.

## 2. Problem / why
The wrong-Beethoven reveal (#417). Identification must exist before any
surface can use it, and the engine must be provably resistant to false
positives BEFORE it meets a user — the doc's honesty rules make silence
a feature.

## 3. Non-goals
- No SQLite persistence, no import hooks, no backfill (S1b).
- No follower/DTW confirmation stage (S1b, where the live AudioEvent
  buffer exists — the retrieval margin here is deliberately stricter to
  compensate until then).
- No session chip, no reveal integration (S1b/S2 of the doc).
- No shipped corpus (S4 of the doc).

## 4. Contract / interface (crates/brain/src/piece_match.rs)
- `melody_line(model: &ScoreModel) -> Vec<u8>` — the top line per onset
  (chords collapse to their highest note), rests skipped, in play order.
- `PieceIndex::new()`, `index_score(&mut self, id: ScoreId-like u64,
  model: &ScoreModel)`, `remove_score(&mut self, id)` — sliding windows
  of NGRAM_INTERVALS=4 consecutive intervals (semitone deltas clamped
  ±12) hashed to postings of (id, position).
- `identify(&self, recent_midi: &[u8]) -> Option<Match>` with
  `Match { id, coherent_hits, total_hits, offset }` (offset = the
  winning alignment, S1b's seed for follower confirmation and
  open-at-your-measure; the margin is a GATE, not a datum):
  - query n-grams from the last QUERY_WINDOW=20 notes;
  - per candidate, positional coherence = the largest bucket of
    (score_pos − query_pos) alignment offsets;
  - accept iff coherent_hits >= MIN_COHERENT_HITS=6 AND those hits
    come from >= MIN_DISTINCT_COHERENT=6 DISTINCT n-grams (review MF1:
    ostinatos/chromatic runs pile the same windows — counts without
    identity) AND the best candidate's coherent_hits >= MARGIN_RATIO=2.0
    × the runner-up's (sole candidates pass on the floors alone).
- Constants are pub(crate) and named — calibration knobs for S1b.

## 5. Acceptance criteria
1. Playing ≥ ~12 consecutive melody notes of an indexed piece identifies
   it (mid-piece entry included — any window, not just the opening).
2. Transposition-invariant: the piece played +3 semitones still matches.
3. Tempo/rhythm-invariant by construction (durations never read).
4. FALSE-POSITIVE FLOOR (first-class): a major scale, an arpeggio
   exercise, and random noodling against a 3-piece index → None. A
   window shared verbatim by two pieces (ambiguous) → None (margin).
5. Chords collapse to the top line; rests are skipped (melody_line unit).
6. remove_score: the deleted piece can no longer match; others still do.
7. Determinism: same index + same query → same Match (no hash-order
   dependence in scoring).

## 6. Edge cases
- Query shorter than a full n-gram window → None (never a guess).
- Score shorter than the window → indexes nothing, matches nothing.
- Two copies of the SAME piece indexed under different ids → ambiguous
  → None (margin catches the honest duplicate case).
- Interval clamp: a two-octave leap clamps to ±12 on both sides
  (index and query agree, so clamping never breaks matching).

## 7. Test plan
| AC | Test |
|---|---|
| 1 | identify_matches_a_library_piece_mid_stream |
| 2 | transposition_never_defeats_identification |
| 3 | (by construction; documented in melody_line docs) |
| 4 | scales_arpeggios_and_noodling_stay_silent + duplicate_windows_refuse_on_margin |
| 5 | melody_line_collapses_chords_and_skips_rests |
| 6 | removing_a_score_forgets_it |
| 7 | identification_is_deterministic (repeat 10×, same result) |

## 8. Architecture
Pure brain module, no new dependencies (std HashMap; hashing via
DefaultHasher is fine — the index is rebuilt in-memory, never persisted
in S1a, so hash stability across releases is NOT relied upon; S1b's
persisted index must re-derive, noted there). Fixtures built from small
hand-written ScoreModels (the emitted-notation test idiom).
