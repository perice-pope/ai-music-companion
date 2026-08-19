# Spec: Follower confirmation — "You're playing" is earned, not guessed (#214 S2b)

> Design source: `docs/architecture/piece-identification.md` §3.3 + §5. The S2 ship note
> named this stage "the standing backstop upgrade for the assertion voice" — this slice
> builds it.

## 1. Summary
The reveal card's "You're playing — {title}" assertion currently rides retrieval alone
(n-gram coherence). This slice adds the design's confirmation stage: an alignment judge
fits the played interval stream against each finalist's melodic line, and the assertion
voice fires only when the identified piece's alignment clears an absolute cost bar and
leads every rival finalist by a margin. The chip's hedged "sounds like" voice stays
retrieval-tier, exactly per the honesty rules ("'sounds like' until the follower has
confirmed alignment; 'You're playing' only after").

## 2. Problem / why
§5 of the design doc is explicit: never fake precision. Retrieval coherence counts
shared n-grams — it cannot see that the *second half* of the player's window diverged
from the piece, so a half-right window can currently put the direct assertion on the
card. The wrong-Beethoven card (#417 item 2) was the founding complaint; a wrong
"You're playing X" would be strictly worse.

## 3. Non-goals
- No change to chip behavior or retrieval gates (S1a constants untouched).
- Not S3 piece-aware feedback (measure-addressed critique) — this only hardens the voice.
- No streaming/online DTW against `AudioEvent`s; the judge runs over the same note
  buffer retrieval reads, at the same ambient per-phrase cadence.
- No persistence changes; the melodic lines live beside the in-memory index.

## 4. Contract / interface
- `brain::piece_match::PieceIndex` retains each score's interval line;
  `finalists(recent_midi, k) -> Vec<Match>` exposes pre-gate top-k (the S1b TODO), and
  `confirm(recent_midi) -> Option<Match>` returns the alignment-judged winner or None.
- `PieceMatchDto` gains `confirmed: bool`. `check_piece_match_impl` sets it iff
  `confirm` picks the same score `identify` named (surfaces must never disagree).
- Frontend: `pieceMatch` store slice carries `confirmed`; `RevealCard` asserts only on
  a confirmed match. Once asserted for a score, the voice is sticky for that score
  (rule 0); the existing live-key contradiction dim still steps it down.

## 5. Acceptance criteria (numbered, testable)
1. An exact mid-piece excerpt of a library piece confirms: `confirm` returns the same
   id as `identify`, and the DTO carries `confirmed: true` end-to-end.
2. The RV promise survives the judge: the same excerpt transposed still confirms
   (alignment is over intervals, transposition-invariant by construction).
3. One stray wrong note does not break confirmation (tolerance — real playing confirms).
4. A window whose tail diverges from the piece still chips (retrieval passes) but is
   NEVER asserted: `identify` returns the match, `confirm` returns None, DTO says
   `confirmed: false`, and the reveal card keeps the hedged catalog voice.
5. Two library near-twins identical through the played window are never asserted
   (alignment lead margin refuses a photo finish).
6. Deleting a score removes its melodic line: `confirm` can no longer pick it.
7. Voice stickiness (rule 0): once a score's match arrives confirmed, a later
   retrieval-only re-sight of the same score does not step the card back down.
8. Fully offline; no new network calls; no new dependencies.

## 6. Edge cases & failure modes
Query shorter than the evidence floor → None (never a guess) · retrieval offset at the
score's start/end (alignment window clamps; a truncated window judges what exists) ·
negative alignment offset (piece entered mid-query-window) → clamped, still judged ·
finalist whose line was removed mid-flight → skipped defensively · sole finalist (no
rival) → absolute bar alone decides · determinism: repeated calls agree (no hash-order
in the outcome).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `piece_match::tests::confirm_matches_exact_playing`, commands `piece_match_lifecycle` | same id; `confirmed: true` DTO |
| AC2 | `piece_match::tests::transposition_never_defeats_confirmation` | transposed excerpt confirms |
| AC3 | `piece_match::tests::a_stray_wrong_note_still_confirms` | one altered note → Some |
| AC4 | `piece_match::tests::a_divergent_tail_chips_but_never_asserts`; commands `half_right_window_never_asserts`; `RevealCard.test.tsx` unconfirmed case | identify Some + confirm None; card hedges, chip shows |
| AC5 | `piece_match::tests::near_twins_refuse_on_alignment_lead` | confirm None on twins |
| AC6 | `piece_match::tests::removing_a_score_drops_its_line` | removed id unpickable |
| AC7 | `RevealCard.test.tsx` sticky-voice case | assertion holds across a false re-sight |
| edges | `piece_match::tests` boundary cases (short query, window clamp, determinism) | per §6 |

## 8. Architecture / approach
Pure logic in `crates/brain/src/piece_match.rs` beside the S1a engine: the index keeps
`lines: HashMap<u64, Vec<i16>>` (a few KB per library), and confirmation runs a fitting
alignment (free score-side lead/trail inside a slack window around the retrieval
offset; gap and capped-substitution costs over semitone-interval deltas, normalized per
query interval). Cheap by construction — ≤3 finalists × a ≤20×~28 DP — and nowhere
near the audio thread. Thresholds are pinned by boundary tests the same way S1a pins
its gates.
