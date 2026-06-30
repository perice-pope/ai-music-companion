# Spec: Adaptive guided coach — RV routine + per-note scoring + difficulty ramp (#254)

> Part of epic #252. The headline "private teacher" feature. Builds **on** the two shared
> foundations: F1 (RV generator engine `crates/variations`) and F2 (Learner Model `brain::learner`).
> It does **not** invent its own generator or learner state — it composes them in `brain::coach`.

## 1. Summary
On demand ("Give me a lesson"), the AI assembles a short **adaptive RV routine** (warmup scale in
random keys → arpeggios + enclosures → interval drill → run-through), walks the player through each
drill on the existing ScoreView + cursor, scores execution **per-note against the drill's exact
`target_notes`** using the existing ears/scoring path, ramps difficulty exactly one bounded step
up/down per drill from accuracy thresholds, and writes mastery deltas + the new difficulty to the
Learner Model. The Explore→Coach dial enters this mode.

## 2. Problem / why
Free play hears the player but offers no structure, accountability, or progression (epic #252, §2).
A "lesson" is the reason a parent pays. Today there is no engine that (a) assembles a sequence of
graded exercises, (b) grades what was actually played against what was asked, and (c) moves the
difficulty in response. F1 gives deterministic exercise content; F2 gives durable per-key mastery and
a difficulty step; this feature is the loop that joins them into a taught lesson. Note: `scoring.rs`
deprecated per-note green/yellow/red verdicts under "coach, don't judge" for *free play* — but a drill
has an **explicit, user-accepted target**, so per-note grading against `target_notes` is the correct,
honest signal here (see §8, §10).

## 3. Non-goals
- No text/voice input; interaction is play + tap "Next"/"Repeat"/"End lesson" (epic interaction rule).
- No new pitch/onset detection or audio-thread work — consume existing `AudioEvent`/perception output.
- No LLM call required for the loop; routine assembly, scoring, ramp, and persistence are fully local
  and offline. (LLM narration of the recap, if any, reuses the existing opt-in path — out of scope here.)
- No new instruments, no MusicXML authoring beyond what F1's `GeneratedSequence.ticks` already emits.
- No multi-session lesson planning / spaced repetition scheduling (later epic slice).
- Difficulty model is a single scalar step (F2's `difficulty: u8`); no per-dimension difficulty vector.

## 4. Contract / interface

### Backend — `brain::coach` (new module in `crates/brain`)
All business logic lives here (CLAUDE.md: no business logic in the frontend). Pure + deterministic
except where it reads the persisted Learner Model.

```rust
/// Kinds of drill in the canonical routine, in play order.
pub enum DrillKind { WarmupScale, ArpeggioEnclosure, IntervalDrill, RunThrough }

/// What the user asked for. Seed makes the whole routine reproducible/testable.
pub struct LessonSpec {
    pub seed: u64,
    pub drill_count: u8,        // K, clamped to 3..=4 (default 4)
    pub start_difficulty: u8,   // taken from LearnerModel.difficulty at lesson start
}

/// One drill: an F1 spec + its generated sequence, tagged with the difficulty it was built at.
pub struct Drill {
    pub index: u8,
    pub kind: DrillKind,
    pub difficulty: u8,                 // bounded 0..=MAX_DIFFICULTY
    pub key_scale: KeyScale,            // F2 mastery key this drill trains
    pub spec: VariationSpec,            // F1 input
    pub sequence: GeneratedSequence,    // F1 generate(&spec, drill_seed): ticks + target_notes + label
}

/// The assembled lesson. Drill N+1 is built from drill N's score (adaptive), so the
/// routine is produced incrementally — `build_first` then `advance` — not all up front.
pub struct Routine { pub drills: Vec<Drill>, pub difficulty: u8 }

/// Build drill 0 from the lesson spec + current learner state. Deterministic for a fixed
/// (LessonSpec, LearnerModel). Maps difficulty → VariationSpec knobs (keys count, tempo,
/// enclosures on/off, scale hardness) via a pure ladder, then calls F1 `generate`.
pub fn build_first(lesson: &LessonSpec, model: &LearnerModel) -> Drill;

/// Given the completed drill + its score, decide the next difficulty (bounded, ±1) and
/// build the next drill of the canonical kind sequence — or None when the routine is done.
pub fn advance(prev: &Drill, score: &DrillScore, lesson: &LessonSpec) -> Option<Drill>;

/// Per-note grade of one played execution against the drill's exact target_notes.
pub struct NoteGrade {
    pub target: Pitch,
    pub played: Option<Pitch>,      // None = note missed/not detected
    pub cents_deviation: Option<f32>,
    pub timing_error_ms: Option<f32>,
    pub correct: bool,              // within pitch + timing tolerance
}
pub struct DrillScore {
    pub per_note: Vec<NoteGrade>,
    pub accuracy: f32,              // 0..1 = correct notes / target_notes.len()
    pub pitch_accuracy: f32,        // 0..1
    pub timing_accuracy: f32,       // 0..1
}
/// Align played notes to `target_notes` (reuse follower/scoring alignment, not free-play
/// phrase scoring) and grade per-note. Deterministic for fixed inputs.
pub fn score_drill(target: &[Pitch], played: &[PlayedNote]) -> DrillScore;

/// Thresholds + the single bounded ramp rule. Pure.
pub struct RampThresholds { pub high: f32, pub low: f32 } // default high=0.85, low=0.60
pub const MAX_DIFFICULTY: u8 = /* from F2 */;
/// >= high → +1, <= low → -1, else unchanged; result clamped to 0..=MAX_DIFFICULTY.
pub fn next_difficulty(current: u8, accuracy: f32, t: &RampThresholds) -> u8;

/// Recap of a finished lesson; the per-drill scores + the deltas written to F2.
pub struct LessonRecap {
    pub drill_scores: Vec<DrillScore>,
    pub start_difficulty: u8,
    pub end_difficulty: u8,
    pub mastery_deltas: Vec<(KeyScale, f32)>, // accuracy applied per drill key_scale
}
/// Fold every drill result through F2 `apply_drill_result` (which updates key_mastery EWMA)
/// and set the final difficulty. Returns the new model + the recap. Pure given inputs.
pub fn finish_lesson(model: &LearnerModel, drills: &[(Drill, DrillScore)]) -> (LearnerModel, LessonRecap);
```

`DrillResult` (the F2 input) is constructed from `(Drill.key_scale, DrillScore.accuracy)` —
`coach` does not re-implement the mastery EWMA; it calls F2 `apply_drill_result`.

### IPC (Tauri commands/events, thin JSON)
- `start_lesson(seed?: u64) -> DrillDto` — begins a lesson, returns drill 0 (label, tempo, target,
  `music_xml` from `sequence.ticks`). Seed optional; absent → time-seeded but echoed back for replay.
- `submit_drill(played: PlayedNoteDto[]) -> DrillStepDto` — scores the just-played drill, returns its
  `DrillScore` + the next `DrillDto` (or `recap` when done).
- `lesson-progress` event — `{ drill_index, drill_count, difficulty }` for the header.
- Reuses the existing `phrase-detected`/score-follow stream to feed `played` notes; no new audio IPC.

### Frontend (`apps/desktop/src/components/`)
- `GuidedCoach.tsx` — orchestrates the lesson; renders the existing `ScoreView`
  (`musicXml` = current drill, `cursorPosition` = live score position) plus a minimal
  `DrillHeader` (drill X of K, kind label, target/key, tempo) and `LessonRecap` (per-drill bars +
  difficulty change). No scoring/ramp logic in TS — it only calls commands and renders DTOs.
- Entered by the **Explore→Coach dial** in `PracticeSession.tsx`: at the Coach end, the score pane is
  replaced by `GuidedCoach`. New `brain.ts` types: `DrillDto`, `DrillScoreDto`, `LessonRecapDto`,
  `PlayedNoteDto` (hand-mirrored, with a roundtrip Vitest per the file's drift rule).

## 5. Acceptance criteria (numbered, testable)
1. `start_lesson` / `build_first`+`advance` produce a routine of **K drills** (K clamped to 3..=4,
   default 4) whose `DrillKind`s follow the canonical order WarmupScale → ArpeggioEnclosure →
   IntervalDrill → RunThrough, each carrying a non-empty `sequence.target_notes` from F1.
2. For a fixed `(LessonSpec, LearnerModel)` the **entire routine is deterministic** — same drills,
   same specs, same target_notes, same labels (seed-driven via F1).
3. `score_drill` grades each target note: a played stream matching `target_notes` within pitch +
   timing tolerance yields `accuracy == 1.0` and every `NoteGrade.correct == true`; a fully wrong /
   silent stream yields `accuracy == 0.0`.
4. If a drill's `accuracy >= high` (default 0.85) the **next** drill's `difficulty` is exactly
   `prev + 1`; if `accuracy <= low` (0.60) it is exactly `prev - 1`; otherwise it is unchanged.
5. Difficulty is **bounded**: `next_difficulty` never returns `> MAX_DIFFICULTY` (already at max +
   high accuracy stays at max) nor `< 0` (already at 0 + low accuracy stays at 0).
6. A difficulty increase actually changes the generated content one step harder (more keys / faster
   tempo / enclosures added / harder scale) and a decrease one step easier — i.e. drill N+1's
   `VariationSpec` differs from drill N's in exactly the difficulty-mapped knobs.
7. `finish_lesson` writes one mastery update per drill via F2 `apply_drill_result` (each drill's
   `key_scale` accuracy_ewma moves toward that drill's accuracy) and sets the model's
   `difficulty` to the final ramped value; `LessonRecap` lists per-drill scores and start/end difficulty.
8. The frontend renders the current drill in `ScoreView` with a following cursor and shows a recap on
   completion; entering at the Coach end of the dial mounts `GuidedCoach`, the Explore end does not.

## 6. Edge cases & failure modes
- **First run / empty Learner Model**: `start_difficulty` defaults to F2's defined empty value (0/low);
  no key has mastery yet → routine seeds keys from the spec, not from `owned` keys; no crash.
- **K out of range**: `drill_count` 0/1/2 or >4 is clamped to 3..=4 (never produces a 0-drill lesson).
- **Already at MAX_DIFFICULTY** with high accuracy → stays at max (AC5); **at 0** with low accuracy →
  stays at 0.
- **Player stops mid-lesson / ends early**: lesson finalizes with only completed drills; `finish_lesson`
  applies mastery only for drills that were actually scored (no phantom updates for unplayed drills).
- **Empty / silent played stream** for a drill → `accuracy == 0.0`, every `NoteGrade.played == None`,
  ramp moves down one step (bounded), no panic, no divide-by-zero on `target_notes.len()`.
- **More played notes than targets** (extra/inserted notes): grading is anchored to `target_notes`
  length; extras don't raise accuracy above 1.0.
- **Offline**: entire loop (generate → follow → score → ramp → persist) works with zero network.
- **Determinism guard**: identical inputs to `build_first`/`advance`/`score_drill`/`next_difficulty`
  give identical outputs (no time/RNG leakage except the explicit seed).
- **F2 schema drift**: writing through `apply_drill_result` preserves unknown blob fields (F2 invariant);
  this feature adds no new persisted field of its own beyond what F2 owns.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `brain::coach::tests::routine_has_k_drills_in_canonical_order` | K∈3..=4, kinds in order, target_notes non-empty |
| AC2 | `brain::coach::tests::routine_is_deterministic_for_seed` | same (LessonSpec,LearnerModel) → identical drills |
| AC3 (perfect) | `brain::coach::tests::perfect_play_scores_full_accuracy` | matching stream → accuracy 1.0, all correct |
| AC3 (wrong) | `brain::coach::tests::wrong_play_scores_zero` | mismatched/silent stream → accuracy 0.0 |
| AC4 (up) | `brain::coach::tests::high_accuracy_ramps_up_one` | acc≥high → next = prev+1 |
| AC4 (down) | `brain::coach::tests::low_accuracy_ramps_down_one` | acc≤low → next = prev-1 |
| AC4 (hold) | `brain::coach::tests::mid_accuracy_holds` | low<acc<high → unchanged |
| AC5 | `brain::coach::tests::difficulty_is_bounded_at_min_and_max` | never <0 or >MAX at extremes |
| AC6 | `brain::coach::tests::ramp_changes_generated_content` | drill N+1 spec differs one step in mapped knobs |
| AC7 | `brain::coach::tests::finish_lesson_writes_mastery_and_difficulty` | per-drill EWMA moves; model.difficulty=end |
| AC7 (early end) | `brain::coach::tests::early_end_only_updates_completed_drills` | unplayed drills don't update F2 |
| AC8 | `GuidedCoach.test.tsx` | drill renders in ScoreView w/ cursor; recap on completion |
| AC8 (dial) | `PracticeSession.test.tsx` | Coach end mounts GuidedCoach; Explore end does not |
| edge: empty stream | `brain::coach::tests::silent_drill_no_panic` | accuracy 0, played None, bounded down-ramp |
| edge: clamp K | `brain::coach::tests::drill_count_is_clamped` | 0/1/2/9 → 3..=4 |
| edge: extra notes | `brain::coach::tests::extra_notes_cap_accuracy_at_one` | accuracy ≤ 1.0 |
| types drift | `brain.types.test.ts` | DrillDto/DrillScoreDto roundtrip Rust↔TS |
| Manual | manual-verify checklist | dial→lesson→play→ramp→recap in the running app, offline |

## 8. Architecture / approach
`brain::coach` is a **composition** module: it owns the difficulty→`VariationSpec` ladder, the drill
kind sequence, the per-note grader, and the ramp rule — and it **calls** F1 `variations::generate`
and F2 `brain::learner::apply_drill_result` rather than duplicating either. F1 stays a leaf crate
(coach depends on it); F2 lives beside coach in `brain`.

Per-note grading reuses the **follower/scoring alignment** that already maps played `AudioEvent`s to
expected notes (`crates/brain/src/follower.rs` produces `ScorePosition.expected_note`); `score_drill`
aligns the played stream to `target_notes` and applies a pitch (cents) + timing (ms) tolerance to
produce `NoteGrade`s. This is deliberately distinct from `scoring.rs`'s phrase-level "coach, don't
judge" path: a drill is an explicit, accepted exercise with a known answer, so per-note grading is the
honest signal — but it is scoped to lesson mode only and does not resurrect per-note verdicts in free
play. The grader and ramp are pure (no I/O, no allocation on the audio thread — grading runs at drill
end, off the hot path).

Frontend reuses `ScoreView` unchanged (`musicXml` from `sequence.ticks` → MusicXML via the existing
`score::musicxml` emitter, `cursorPosition` from the live follower). `GuidedCoach` is a thin
read-model + command caller; the Explore→Coach dial in `PracticeSession` swaps the score pane for it.
Offline-first: no network anywhere in the loop, so nothing to disclose in `ConnectionsPrivacy.tsx`.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S1 | `brain::coach` contract: types (`Drill`, `DrillScore`, `NoteGrade`, `LessonSpec`, `Routine`, `LessonRecap`) + `next_difficulty` ramp (pure, bounded) + difficulty→`VariationSpec` ladder, all behind stubs | `crates/brain/src/coach/mod.rs`, `coach/ramp.rs` | F1, F2 (types) | no |
| S2 | Routine assembly: `build_first` + `advance` calling F1 `generate`; canonical kind order; seed determinism | `crates/brain/src/coach/routine.rs` | S1, F1 | yes |
| S3 | Per-note grader `score_drill` reusing follower alignment + pitch/timing tolerance | `crates/brain/src/coach/grade.rs`, reads `follower.rs` | S1, existing follower | yes |
| S4 | `finish_lesson` folding through F2 `apply_drill_result` + final difficulty; `LessonRecap` | `crates/brain/src/coach/recap.rs` | S1, S2, S3, F2 | no |
| S5 | IPC: `start_lesson` / `submit_drill` commands + `lesson-progress` event + DTOs | `apps/desktop/src-tauri/src/commands.rs`, `coach` DTOs | S2, S3, S4 | no |
| S6 | Frontend `GuidedCoach` + `DrillHeader` + `LessonRecap` reusing `ScoreView`; `brain.ts` DTO types + roundtrip test | `apps/desktop/src/components/GuidedCoach*`, `DrillHeader*`, `LessonRecap*`, `types/brain.ts` | S5 | no |
| S7 | Explore→Coach dial wiring; mount `GuidedCoach` at Coach end | `apps/desktop/src/components/PracticeSession.tsx`, dial control | S6 | no |

Suggested waves: S1 alone → {S2, S3} parallel (disjoint files) → S4 → S5 → S6 → S7.

## 10. Risks / open questions
- **Per-note vs "coach, don't judge"**: grading drills per-note is a deliberate, scoped exception
  (explicit accepted target). Confirm with the product owner that it stays lesson-only and never leaks
  per-note verdicts into free play. (Leaning: keep `score_drill` in `coach`, not `scoring.rs`.)
- **Difficulty ladder mapping**: which knob moves at each step (keys → tempo → enclosures → scale
  hardness) and the exact `MAX_DIFFICULTY` are F2-owned; this spec assumes F2 exposes the bound and
  the ladder is defined in S1. Needs the F2 difficulty semantics finalized.
- **Alignment reuse**: `follower.rs` aligns to a loaded `ScoreModel`; confirm `target_notes` can drive
  the same aligner (it should, since F1 emits the same `StaffTick` shape ScoreView consumes) vs. a
  simpler positional grader for short drills. Decide in S3.
- **Thresholds** high=0.85 / low=0.60 are starting values; may need tuning against real kid play.
- **Tempo source for run-through**: whether the final drill grades timing against a click/F1 rhythm or
  is pitch-only — resolve in S3.

## 11. References
- Epic + foundations: `docs/specs/252-rv-practice-coach.md` (F1 `crates/variations` `generate`,
  F2 `brain::learner` `apply_drill_result`); style example `docs/specs/253-reveal-loop.md`.
- Existing code: `apps/desktop/src/components/ScoreView.tsx` (cursor reuse),
  `apps/desktop/src/components/PracticeSession.tsx` (dial host),
  `crates/brain/src/follower.rs` (`ScorePosition.expected_note`, alignment),
  `crates/brain/src/scoring.rs` (deprecated per-note verdicts — the "coach, don't judge" context),
  `crates/brain/src/score/musicxml.rs` (ticks→MusicXML for ScoreView),
  `apps/desktop/src/types/brain.ts` (`ScorePosition`, DTO drift rule).
- GitHub issue #254.
