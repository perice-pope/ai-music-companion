# Spec: the thin-session recap — words scale to evidence (#445 pt 6b, F4)

## 1. Summary
"I can play basically nothing and the session feedback is saying a
lot." Every CLAUSE of the recap is already evidence-gated, but the
QUANTITY isn't: 30 seconds of noodling still earns a full paragraph +
three lists. Silence > lies applies to word count — a page of coaching
over nothing reads as fabrication even when each sentence is true.

## 2. Contract
- New evidence bar in `coaching.rs`:
  `is_thin_session(input)` — true when the session HAS phrases but
  fewer than `THIN_SESSION_MIN_PHRASES` (3) OR less than
  `THIN_SESSION_MIN_PLAYED_SECS` (20.0) of summed phrase time.
  Zero phrases stays with the existing (founder-praised) empty-state
  path — untouched.
- `thin_session_recap(input)`: the honest short form, in the voice the
  founder praised (#445 pt 7 — technical, warm, no filler):
  one- or two-sentence assessment naming exactly what happened (n
  phrases, ~duration, instrument) and saying plainly there isn't
  enough to read; NO strengths/areas padding; exactly ONE suggestion
  (settle in for a few minutes of continuous playing). If one
  fingerprint dimension genuinely cleared its gate, its single
  strongest fact may ride along — measured facts are never suppressed.
- ONE choke point, both generators: `generate_recap` (the LLM engine)
  returns the thin recap BEFORE building any prompt (thin sessions
  never reach the model — the LLM must not inflate), and
  `grounded_offline_recap` short-circuits identically.

## 3. ACs
1. 2 phrases (any length) → thin recap: short assessment, empty
   strengths/areas, exactly one suggestion.
2. ≥3 phrases but <20s summed → thin. ≥3 phrases and ≥20s → FULL
   (boundary pinned).
3. 0 phrases → NOT thin (existing empty-state path byte-identical).
4. The ONLINE engine with a thin input returns the thin recap without
   touching the HTTP client (the no-inflation pin).
5. A thin session where intonation cleared its gate still surfaces
   that one fact (measured truth never suppressed).

## 4. Test map
| AC | Test |
|---|---|
| 1 | thin by count: shape + single suggestion |
| 2 | thin by seconds + the ≥3/≥20 boundary stays full |
| 3 | empty input → existing recap unchanged |
| 4 | online engine + thin input → no http call, thin shape |
| 5 | thin + clear intonation → the fact rides along |
