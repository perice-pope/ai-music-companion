# Spec: Upload & Practice v1 — the listening loop on stable formats (#337)

> Founder direction 2026-07-10: uploading your own music and having the app listen
> while you practice it becomes a first-class loop, anchored on the most stable
> formats. Stable tier = **MIDI + MusicXML**. PDF (OMR) and audio (transcription)
> stay beta-gated — they are the *least* stable paths, not the anchor.

## 1. Summary
A player drags in a piece (.mid/.midi/.musicxml/.mxl/.xml), plays it, and the app
listens: the cursor tracks them (#333), notes light up right/near/missed as they pass,
phrase cards name the measures that need work, the recap ranks the worst measures —
and any struggled measure becomes a 12-key RV row in one tap.

## 2. Problem / why
Import, render, and the score follower all work (4 straight VA tests; follower emits
positions — display fix is #333). But the *listening* is invisible: no live feedback
against the piece, no score-aware phrase cards (#210 open since June), a recap that
barely knows a score was involved, and zero connection from repertoire struggle to the
RV method — the product's core loop. "Practice with your music" currently means
"look at your music while the app free-plays."

## 3. Non-goals
- No PDF/OMR or .wav/.mp3 hardening (beta gates stay; audio honesty is #328/#331).
- No accompaniment/backing (that's #212's epic), no "hear it fixed" (#213).
- No multi-instrument score alignment: v1 follows the single imported part.
- No new notation renderer — score display stays OSMD read-only (standing decision);
  the RV bridge hands off to the existing CellStaff explore surface.
- No tempo-adaptive cursor changes — the follower's existing behavior stands.

## 4. Contract / interface
- **Format tiers (frontend copy + gate):** drop-zone and errors name the tiers —
  stable: MIDI/MusicXML; beta: PDF, audio. No new IPC.
- **Live note verdicts (slice 2):** the follower already computes
  `ScorePosition { measure_number, beat, expected_note }`. New event
  `note-verdict { measure_number, beat, verdict: "hit" | "near" | "missed" }`,
  emitted by the existing follower thread by comparing detected pitch to
  `expected_note` (semitone tolerance from the instrument profile; "near" = within
  1 semitone or >25 cents off per the profile's vibrato/attack tolerances). Emitted
  only while follower confidence is high — no verdict when lost (silence > lies).
  ScoreView paints passed noteheads by verdict class.
- **Phrase cards in score mode (slice 3, absorbs #210):** the existing offline phrase
  pipeline runs during score sessions; cards carry `measures: (start, end)` from the
  follower positions bracketing the phrase. Copy pattern: "Measures 5–8 — rushed
  (~+9 BPM), 2 missed notes." Reuses the free-play card surface.
- **Score recap (slice 4):** `SessionRecap` gains additive
  `score_summary: Option<ScorePracticeSummary>` —
  `{ score_title, accuracy_pct, worst_measures: Vec<MeasureVerdict>, tempo_delta_bpm }`
  (`#[serde(default)]`, wire-compat like fingerprint). Logged to `exercise_log`
  with `source: "score_practice"`, spec_json = a score-ref pseudo-spec, so
  insights/teacher surfaces see repertoire work.
- **RV bridge (slice 5):** recap worst-measure rows get "row this through 12 keys" →
  new command `explore_measure(score_id, measure_number)`: reads the measure's notes
  from the stored ScoreModel, converts to semitone offsets from its first note, and
  calls the existing `start_explore_cell` (17-note cap, LIFT_MIN_ROOTS floor apply).
  Refusal copy when a measure is empty/rest-only or over the cap.

## 5. Acceptance criteria (numbered, testable)
1. **Stable-tier robustness:** every fixture in a new real-world MIDI corpus
   (multi-track with part picking, tempo changes, pickup bar, overlapping/legato
   notes, a percussion track to skip, type-0 single-track) imports to a playable
   score or fails with a calm, named reason — never a panic, never a silent
   half-score. Same sweep for MusicXML fixtures incl. .mxl compression.
2. **Tier copy:** drop zone labels stable vs beta formats; a PDF/.wav import shows
   its beta label. (RTL test.)
3. **Live verdicts:** feeding a synthetic session (known score + scripted pitch
   stream) produces the expected hit/near/missed sequence; no verdicts while the
   follower reports lost/low confidence; verdict noteheads render by class.
   (Rust test on the verdict function; frontend test on the paint.)
4. **Phrase cards in score mode:** a score session's phrase card names the measure
   range its follower positions bracket; free-play sessions are unchanged. (#210's
   AC, finally.)
5. **Score recap:** after a synthetic session with known errors in measures 3 and 7,
   the recap's worst_measures names 3 and 7 with per-measure accuracy; accuracy_pct
   matches the scripted hit rate ±1pt; an `exercise_log` row lands with
   source "score_practice". Legacy recaps (no score_summary) still parse.
6. **RV bridge:** `explore_measure` on a fixture score returns an exploration whose
   cell is the measure's notes as offsets, rowed through ≥3 roots; the staff view
   renders it; an empty measure refuses with calm copy. Round-trip: the bridge's
   exploration logs to `exercise_log` like any explore.
7. **Offline:** the entire loop runs with zero network (existing zero-network test
   pattern extended to a score session).

## 6. Edge cases & failure modes
Transposed-instrument MusicXML (respect written vs sounding pitch as the current
importer does — pin it) · MIDI files with no note events → calm refusal · tempo = 0 /
absurd tempos → clamp with note · follower never locks (player plays something else)
→ no verdicts, phrase cards fall back to free-play copy, recap says "couldn't follow
along — was this the right piece?" · measure with 1 note (bridge still works — cell
of 1 offset + roots) · very long pieces (verdict/paint state bounded per visible page).

## 7. Test plan
- `crates/brain` (or score module): MIDI corpus sweep (fixtures committed, small,
  license-clean); verdict function unit tests; recap summary aggregation; bridge
  conversion incl. cap/refusal.
- Frontend: tier copy, verdict painting, phrase-card measure text, recap
  worst-measures render + bridge button invoke.
- Integration: golden score session (synthetic pitch stream against a fixture score)
  through session → recap → exercise_log, alongside golden_session.rs.
- Manual-verify (playbook step): drag a real .mid in, play it badly on purpose in one
  measure, confirm the cursor + verdicts + that measure tops the recap and rows
  through 12 keys.

## 8. Slices (each < ~400 lines, one PR each)
1. Stable-tier hardening + corpus fixtures + tier copy (#337-S1).
2. Live note verdicts (needs #333 merged) (#337-S2).
3. Score-mode phrase cards — closes #210 (#337-S3).
4. Score recap + exercise_log (#337-S4).
5. `explore_measure` RV bridge (#337-S5, the flagship).

## 9. Open questions (founder)
1. "Near" tolerance: per-instrument profile values or one global default to start?
   (Spec assumes profile-driven; global fallback fine for v1.)
2. Should the RV bridge live on the recap only, or also as a long-press on any
   measure during practice? (Spec assumes recap-only for v1 — smaller surface.)
