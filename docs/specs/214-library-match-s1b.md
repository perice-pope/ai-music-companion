# Spec: Library-match wiring + the chip (#214 S1b)

## 1. Summary
S1a's engine meets the app: the index builds over the user's score
library at startup and follows imports/deletes; identification runs at
PHRASE cadence through the same frontend-triggered command pattern
coaching tips use; a confirmed match surfaces as the calm session chip —
"sounds like {title} from your library" — obeying rule 0.

## 2. Contract (as shipped)
- `AppState.piece_matcher` (S1a `PieceIndex` + u64-key → (ScoreId string,
  title) map; ScoreIds are UUIDs, keys are their string hashes).
- `rebuild_piece_index()` at startup (each entry parses in ms; a bad
  file is skipped with a warn, never a startup break); `index_entry()`
  after every import path (MusicXML method, MIDI method, command-level
  import); `unindex_score()` on delete.
- `check_piece_match` command: reads the backend's own phrase buffer
  (last 3 phrases → `midi_track_from_pitch_track`, GAP-SEPARATED repeats kept — a re-struck
  note is a real 0-interval; legato/pedal repeats collapse (the honest
  best from a pitch track alone)), queries S1a's gates, returns
  `Option<PieceMatchDto{score_id, title, coherent_hits}>`. None is the
  common answer and never an error. Invoked by the frontend's existing
  phrase-detected handler (the requestCoachingTip pattern) — ambient
  cadence, zero new threads, nothing on the audio thread.
- Store: `pieceMatch` (sticky — replaced in place, never cleared by a
  miss), `dismissedPieceIds` (session-scoped quiet list), both reset at
  session end. `PieceMatchChip`: informational only; appears on a gated
  match, HOLDS (rule 0), dismiss quiets that score.
- DEFERRED with reasons: follower confirmation (S1a's retrieval margin
  is deliberately stricter until it lands — module TODO records the
  top-k seam); "Open score at your measure" + reveal integration ride
  #214 S2 (queued as epic E4).

- Cost bound: the startup rebuild parses each stored score in ~5-8 ms
  (release) on the main thread — bounded to LIBRARY scale (dozens).
  S4's ~500-work corpus requires the arch doc's persisted index + lazy
  rebuild; recorded there as open question 3.

## 3. ACs (all tested)
1. An imported score is identifiable in the same session; a deleted one
   falls silent immediately — through the real import path and the same
   seam the delete command calls.
2. Free noodling never surfaces the chip (S1a's gates, end to end
   through the phrase seam).
3. Rule 0: the chip appears, HOLDS through misses, is replaced in place
   by a newer match; dismissal quiets that score id for the session and
   a DIFFERENT score can still surface.
4. Identification errors are silent (never a crash or notice).
5. A corrupt stored score is skipped calmly at (re)index time.
6. Session end clears the match and the quiet list.

## 4. Test map
| AC | Test |
|---|---|
| 1,2,5 | commands `imported_scores_identify_and_deleted_ones_fall_silent` |
| 3,4 | PieceMatchChip.test (4 cases) |
| 6 | store session-end reset block (piece fields) |
