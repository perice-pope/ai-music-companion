# Spec: Coach's Rhythm — rhythm cells on drill material (#421 S3)

## 1. Summary
The Pocket's S3 arc: rhythm becomes an RV modifier. Any melodic figure the
generator deals — a scale pattern, an arpeggio, a lifted cell — can be re-timed
by a **rhythm cell** (a small repeating duration figure: "dotted eighth +
sixteenth", "swung eighths"), same pitches, new rhythm. The coach then learns
to DEAL the cell the player's timing data asks for, with one honest sentence of
why. This spec covers the whole S3 arc; **this PR ships S3a only** — the
generator seam.

## 2. Problem / why
Rhythm today is a uniform grid: `RhythmSpec.notes_per_beat` steps every melodic
note by the same duration. RV's founder direction (#421): "rhythm should bake
into each practice mode as a practice option", and the coach's answer to RV's
old-aged RANDOM button is dealing the rhythm your data asks for ("your eighths
rush at 120 — today's cell: dotted-eighth anchors at 100"). None of that can
exist until the generator can express a non-uniform rhythm at all.

## 3. Non-goals
- No pedagogy in this slice: nothing picks a cell from learner data (S3b).
- No UI (S3c) and no IPC changes — the field rides the existing spec wire.
- No rests/offsets inside cells (RV's rests live in the #419 starter arc).
- No lifting the player's own rhythm as a cell (a later arc, like pitch
  `cell` was for phrases).
- Stacked chords and progressions keep their whole-measure deal — a
  simultaneity has no note-to-note rhythm.

## 4. Contract / interface
- `variations::RhythmCell` (catalog enum, `catalog.rs` house style):
  `durations() -> &'static [f64]` (beats, all > 0) and `label() -> &'static str`.
  V1 catalog — the classic subdivision figures (each sums to one beat):
  `DottedEighthSixteenth` [0.75, 0.25], `SixteenthDottedEighth` [0.25, 0.75],
  `EighthTwoSixteenths` [0.5, 0.25, 0.25], `TwoSixteenthsEighth`
  [0.25, 0.25, 0.5], `SwungEighths` [2/3, 1/3]. Adding an entry needs no
  generator changes.
- `RhythmSpec.cell: Option<RhythmCell>` — additive on the wire
  (`serde(default)`), so every persisted spec (exercise log, assignments)
  keeps parsing. Precedence: when set, the cell IS the melodic grid and
  `notes_per_beat` is ignored; `None` = today's behavior, byte-identical.
- Generator: melodic figures cycle the cell's durations from each figure's
  first note; the cycle restarts per root (RV row invariance — the same shape
  gets the same rhythm in every key). One-cell-per-measure rule unchanged.
- `GeneratedSequence.label` names the cell (e.g. `"… · dotted eighth +
  sixteenth"`).

## 5. Acceptance criteria (numbered, testable)
1. A spec with a rhythm cell deals the same pitches in the same order as the
   identical spec without one — only `start_beat`/`duration_beats` change, and
   they follow the cell's durations cycled from each figure's first note.
2. The cycle restarts at every root: with figures of equal length, note *k* of
   every segment carries the same duration (row invariance).
3. Every figure still starts on a measure boundary (one cell per measure), and
   a figure ending mid-measure breathes for the remainder.
4. Stacked-chord and progression sequences are unchanged by a rhythm cell.
5. Wire compat: pre-S3 JSON (no `cell` key in `rhythm`) parses to `cell: None`;
   a spec with a cell round-trips through JSON.
6. The label names the cell; without a cell the label is unchanged.
7. Determinism/purity hold: same `(spec, seed)` → same sequence, and the cell
   consumes no randomness (root shuffle + directions identical with/without).
8. A drill with a rhythm cell adapts to a well-formed `ScoreModel` whose
   MusicXML durations are exact at DIVISIONS=480 (dotted eighth = 360,
   sixteenth = 120, swung pair = 320/160) — the drill engraves honestly.

## 6. Edge cases & failure modes
- Figure shorter than the cell (a 2-note figure under a 3-duration cell):
  cycle truncates — only the first N durations are used, next root restarts.
- Figure longer than the cell: durations wrap (cycle).
- `notes_per_beat: 0` with a cell set: cell wins; the 0-guard stays for the
  `None` path.
- Swung-eighth f64 thirds: per-note `beats_to_divs` rounds exactly at 480;
  the measure-boundary `ceil` absorbs accumulated ulp drift.
- Enclosure approach notes are part of the realized figure and take cell
  durations like any figure note.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `variations::tests::rhythm_cell_retimes_without_touching_pitches` | midi sequence identical; durations follow the cycle |
| AC2 | `variations::tests::rhythm_cell_restarts_per_root` | note k same duration in every segment |
| AC3 | `variations::tests::rhythm_cell_keeps_measure_boundaries` | each segment's first note on a measure line |
| AC4 | `variations::tests::stacked_and_progression_ignore_rhythm_cell` | sequences equal with/without cell |
| AC5 | `variations::tests::rhythm_cell_wire_compat` | old JSON → None; with-cell roundtrip; **a None-cell spec serializes byte-identically to the pre-S3 wire** (`exercise_spec_hash` FNVs those bytes under a stability contract — `"cell":null` would split identical materials across the upgrade boundary) |
| AC6 | `variations::tests::label_names_the_rhythm_cell` | suffix present iff cell set (and iff it re-timed — AC4's full-sequence equality pins the stacked/progression label) |
| AC7 | `variations::tests::rhythm_cell_consumes_no_randomness` | root_order equal with/without cell under randomize_roots |
| AC8 | `brain::coach::tests::rhythm_cell_drill_engraves_exact_durations` | full bars for dotted/mixed/swung cells; XML ticks exact (360/120, 320/160); a mid-measure figure end engraves its breath as REST |
| edges | truncation: 2-note interval figure under the 3-duration cell (in the AC1 test); wrap: 8-note figure under the same cell | truncation + wrap + cycle restart |
| catalog | `variations::catalog::tests::every_rhythm_cell_is_barline_safe` | every entry: non-empty, all > 0, prefixes ≤ 1, cycle sums to one beat — the invariants "adding an entry needs no generator changes" rides on |

## 8. Architecture / approach
Pure Rust in `crates/variations` (business logic in the core, CLAUDE.md); no
network, no hot-path contact — generation runs on demand, off the audio
thread. The downstream pipeline already survives non-uniform durations:
`cell_staff_view` attributes segments by note COUNT, the CellStaff rhythm
layer draws stems/flags from `duration_beats`, and `emit.rs` writes exact
divisions (no `<type>` on unbeamed notes, so renderers infer from duration;
dotted figures simply don't beam).

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S3a | rhythm-cell seam: catalog enum + generator + label (this PR) | `crates/variations/`, one AC8 test in `crates/brain/src/coach.rs` | — | no |
| S3b | Coach's Rhythm pick: groove/insights → cell + one-line why; "Surprise me" keeps the dice | `crates/brain/src/coach.rs`, `insights.rs` | S3a | no |
| S3c | Surface: rhythm chip on drills/openers, cell shown in the row label, Pocket dialog Rhythm tab | `apps/desktop/src/components/`, IPC DTOs | S3b | no |

## 10. Risks / open questions
- Cell catalog contents are the standard subdivision figures (RV's
  meter/division language + the founder's named "dotted-eighth anchors");
  expansion is data-only. Which cells the COACH deals, and when, is S3b —
  where the pedagogy call lives.
- CellStaff's flag glyph is binary (`duration < 1`), so a dotted eighth draws
  a plain eighth flag — position still carries the true rhythm. An augmentation
  dot in the rhythm layer is a possible S3c polish, not blocking.
- Under a single-approach enclosure the cell's FIRST duration lands on the
  ornament, not the target — "dotted-eighth anchors" then anchors the approach
  note. Spec'd behavior (§6), but S3b should sanity-check it before dealing
  cells onto enclosed rows (review observation).
- `insights::shape_of` ignores `rhythm.cell`, so a dotted and a straight row
  group as one practice-history shape. Plausibly right for melodic-shape
  analytics; S3b decides deliberately (review observation).

## 11. References
- Issue #421 (founder design, S1/S2 shipped in PRs #434/#441); this file's
  siblings `docs/specs/421-pocket-s1.md`, `421-s2-follow-handoff.md`.
- `crates/variations/src/lib.rs` (`RhythmSpec`, `generate_in_window`),
  `catalog.rs` (house style), `crates/brain/src/score/emit.rs` (DIVISIONS),
  `docs/architecture/rv-methodology.md` (cell × row × modifiers).
