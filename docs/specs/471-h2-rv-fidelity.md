# Spec: RV transposition fidelity — never emit a wrong interval (#471-2)

> Founder's bar: "it functions exactly like RV." The method is *the same cell, exactly,
> through 12 keys* — a wrong interval anywhere is the product lying about itself.

## 1. Summary

Fix the three pattern-integrity breaks the #471-2 investigation proved (issue comment,
2026-07-21): the octave-fold's silent per-note clamp (generation), the edit engine baking
folded/clamped contours back into the cell (edits), and the one-global-signature spelling
that makes correct bars *look* wrong (display). The rule for all three: **never emit a
wrong interval** — extreme registers and calm refusals beat lying intervals.

## 2. Problem / why

From the investigation (ground truth, #471 comment):

- **Break 1 (generation).** The playable window is 61 semitones (MIDI 36–96) but
  `fold_into_range` shifts in whole octaves and then **per-note clamps** the overflow.
  Any figure wider than 49 semitones cannot fit some keys' octave shifts, and the clamp
  flattens the intervals. Whether a key breaks depends on `(root + lowest offset) mod 12`,
  so the same cell is perfect in some keys and wrong in others. Law: broken keys ≈
  span − 49 (span 55 → 6 of 12 keys wrong; span 72 → all 12). Lifted/edited cells may
  legally span 72. Enclosures widen the figure pre-fold, so "it broke when I added the
  enclosure" is a real path.
- **Break 2 (edits, worse).** `edit_explore_note`'s bake path derives cell offsets from
  the **folded** `target_midi` (plus a ±36 clamp), so the first edit on a wrong-rendered
  cell bakes the clamped contour back as the new cell — a zero-move nudge on the
  wrong-looking bar permanently corrupts all 12 keys.
- **Break 3 (display).** One global key signature spells all 12 segments — chromatic
  cells render 4 different staff shapes across keys (same-line notes differing only by
  accidentals), so correct bars can LOOK wrong.

## 3. Non-goals

- **No starter/OpenersPanel changes** (degree caps, the 2-button redesign) — that is the
  #471-3 (H3) redesign.
- **No instrument ranges** (#471-4 / H4). The fold window becomes an *argument* with the
  36..96 default so H4 can plug per-instrument windows in, but nothing reads profiles here.
- **No CellStaff.tsx column-layout changes** — `fix/hbatch-accidental-spacing` (#471-1)
  owns that file's layout concurrently. This slice is Rust-side spelling only; the
  `CellStaffView` DTO shape is unchanged.
- No change to the RV grid, shuffle, chips, grading, or seed derivation.

## 4. Contract / interface

- `variations::fold_into_window(figure: &mut [i16], lo: u8, hi: u8)` (new, pub — the H4
  seam). `fold_into_range` delegates with `MIDI_MIN..MIDI_MAX`. **Behavior change:** no
  per-note clamp, ever. The window is a register *preference*; MIDI 0..=127 is the only
  hard bound, enforced as a loud validation panic (unreachable for any UI-constructible
  spec — proven by test), never a clamp.
- `variations::unfolded_figures(spec, seed) -> Vec<Vec<i16>>` (new, pub): the exact
  pre-fold melodic figures `generate` deals, per segment in play order, consuming the
  seed identically (shuffle + per-root direction coins). Empty for stacked/progression
  rows. This is the truth the edit engine bakes from.
- `brain::coach::MAX_CELL_OFFSET` becomes the one shared ±36 cap (pub): the lift fold's
  literal ±36 now reads it. With F1's no-clamp fold, ±36 stays VALID — a cell may span
  72 semitones and every key still renders the exact interval sequence.
- `brain::coach::explore_material(spec) -> String` (new, pub): the material label the
  key signature derives from (scale > chord family > progression anchor > "major"),
  moved from the desktop layer so the edit engine and staff view share one derivation.
- `brain::coach::edit_explore_note(state, index, edit)` — **drops the `key` parameter**
  (breaking, workspace-internal only). The gesture's staff-step math now uses the edited
  *segment's own* key, which is what the staff draws for that bar after F3 — the
  documented invariant "gestures and rendering can never disagree" requires it.
- `brain::score::cellstaff::cell_staff_view(seq, key, material: &str)` — **adds the
  material label** (breaking, workspace-internal only). `key` remains the DRAWN
  (tonic's) signature; spelling is per segment. Wire DTO (`CellStaffView`) unchanged.

## 5. Acceptance criteria (numbered, testable)

1. **F1 exactness:** for the investigation's repro cells — `[0,25,-25]` (span 50, the
   root-61 break), `[0,16,28,36,26,14,2,-19]` (span 55), and a span-72 lifted-style
   cell — `generate` yields an identical interval (delta) sequence in all 12 keys
   (roots 60..71), for Forward AND Reversed AND with an enclosure, and that sequence is
   the spec's true realized shape.
2. **F1 physical proof:** every 2-note figure of span exactly 72, at every placement
   (low offset −72..=0) and every root 60..=71, renders entirely within MIDI 0..=127
   with the span preserved exactly — the "impossible to overflow" claim, proven.
3. **F1 loud failure:** a figure that cannot fit 0..=127 under any whole-octave shift
   (span > 127, hostile wire only) panics with a clear message — it never clamps.
4. **F1 replay stability:** a figure the old fold placed *without clamping* keeps its
   exact legacy placement (pinned: the root-95 major run still renders 83..95).
5. **F2 lossless bake:** span-55 cell, `DirectionMode::Reversed`, 12 roots — a zero-move
   nudge (`Semitones { by: 0 }`) on ANY bar leaves `target_midi` identical in all 12 keys
   and bakes `spec.cell` byte-exact to the true realized figure (the reversed original
   offsets — direction folds into the bake by #292 design), with no folded or clamped
   offset anywhere. (Today this corrupts all 12 keys.)
6. **F2 refusal over clamp:** a gesture whose true offset exceeds ±`MAX_CELL_OFFSET`
   refuses with the existing calm voice and mutates nothing (existing pins keep passing);
   the removed 0..127 clamps can no longer turn "+1 octave" into a different interval.
7. **F3 per-segment spelling:** the chromatic `[0,1,2,3]` probe across 12 roots renders
   at most 3 staff-step shapes (down from 4), enharmonically-equal-signature roots pair
   up exactly, and the per-root shape table is pinned. Residual (documented): shapes can
   still differ by where the staff's two natural semitones (E–F, B–C) fall inside the
   figure — a 7-letter staff has no letter between them; this is how RV itself engraves.
8. **F3 drawn-signature honesty:** the staff still draws the tonic's signature
   (`view.fifths` unchanged); every accidental glyph is computed against that DRAWN
   signature, so a reader applying the signature plus glyphs always recovers the true
   pitch (pinned: F-natural in a sharp-key row gets its natural sign; a Db-rooted
   segment over a C-major staff draws a flat, matching the row's "Db" cell name).
9. **No regression:** full workspace + src-tauri + frontend gates green; existing regen
   determinism and stored-seed replay suites pass unmodified except where a test pinned
   the old *clamped* (lying) output, each such change justified inline.

## 6. Edge cases & failure modes

- Empty figure → no-op fold (unchanged). Empty roots → empty sequence (unchanged).
- Stacked-chord and progression rows: still refuse edits calmly; `unfolded_figures`
  returns empty for them by design.
- Ragged sequences (`len % roots != 0`): staff view falls back to the global key
  (today's exact behavior); edit already refuses.
- Hostile wire (roots ≥ 128-span figures, degree-pattern offsets beyond i8): generation
  panics loudly (AC3) or the edit refuses calmly — never a silent clamp.
- A true (unfolded) pitch outside 0..=127 under a staff-step drag: calm refusal (cannot
  spell an unphysical pitch); unreachable for UI-built specs.
- `chord_group` present but no matching `chord_target`: fall back to the drawn key.

## 7. Test plan

| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `variations::tests::wide_cells_transpose_exactly_in_every_key` | delta-sequence identity across 12 roots × {Forward, Reversed} × {no enclosure, OneDownOneUp}, equal to the independently-computed true shape |
| AC2 | `variations::tests::span_72_always_fits_physical_midi` | all notes 0..=127, span exactly preserved, for every placement × root |
| AC3 | `variations::tests::an_unfittable_figure_fails_loudly_instead_of_lying` | `#[should_panic(expected = "refusing to clamp")]` |
| AC4 | `variations::tests::fitting_figures_keep_their_legacy_placement` | root-95 major run renders 83..95 exactly |
| F2 seam | `variations::tests::unfolded_figures_mirror_generates_segments` | fold(unfolded[i]) == generate's segment i, RandomPerRoot + enclosure |
| AC5 | `brain::coach::tests::baking_a_wide_reversed_cell_is_lossless` | zero-move nudge on every one of 12 bars: rendered row unchanged, baked cell == exact reversed offsets |
| AC6 | existing `founder_range_reachable_and_boundaries_refuse` + `net_zero_nudges_never_corrupt_the_row` | refusal + no mutation, net-zero identity |
| AC7 | `brain::score::cellstaff::tests::chromatic_probe_spells_per_segment` | pinned 12-root shape table, ≤3 distinct shapes, signature-pairs identical |
| AC8 | `brain::score::cellstaff::tests` (updated spelling pins) | glyphs vs the DRAWN signature; Db-rooted segment draws flat over C-major staff |
| AC9 | full suites (`just ci` equivalents), golden_session | everything else byte-identical |

## 8. Architecture / approach

All in the existing crates, no new deps, no network, nothing near the audio thread.

- **F1** (`crates/variations/src/lib.rs`): `fold_into_window` picks the whole-octave
  shift minimizing the figure's worst poke past the window (max per-end overflow; 0 when
  it fits), ties → smallest |shift|, then the lower octave. For every figure that fits,
  this reproduces the legacy placement exactly (replay stability); for wider figures it
  balances the overflow around the window instead of clamping. If the top/bottom then
  exceeds physical MIDI, the shift steps back inside 0..=127 (register beats centering;
  intervals never bend); if no shift fits, panic loudly.
- **F2** (`crates/brain/src/coach.rs`): the bake path calls
  `variations::unfolded_figures` (which shares `generate`'s exact figure/direction/
  enclosure code and RNG consumption) instead of reading folded `target_midi`; the ±36
  cap refuses instead of clamping; the 0..127 clamps are removed (audited inline).
- **F3** (`crates/brain/src/score/cellstaff.rs`): per-note segment attribution (uniform
  melodic figure length, or `chord_group` → `chord_targets`), per-segment
  `key_signature_for(segment_root, material)` for the *spelling*, the drawn (tonic)
  signature for the *glyph*. `commands.rs` passes the material from the same
  `explore_material` derivation `explore_key` uses.

### Stored-seed-law disclosure (prominent, also in the PR)

F1 changes `generate()` output **by design** for any (spec, seed) whose old render was
clamped — i.e. exactly the broken keys. Judged against the stored-seed law (#419:
*replay = stored spec + seed; `generate()` output may change*): recall of a
previously-logged broken-render exercise now renders **correctly-differently** — the
replay contract replays the spec and seed, and those now produce the TRUE pattern in
every key. **Replays of previously-broken keys now play the true pattern; that is a
fidelity fix, not a replay break.** Keys the old fold rendered without clamping replay
bit-identically (AC4 pins the placement rule that guarantees it). Audit result: no
existing test pinned a clamped output (the fold suite asserted range + contour only);
one display pin changed (`C# in C major` now honestly draws Db for a Db-rooted segment,
justified at the assertion).

## 9. Slice breakdown

One slice — the three fixes share the fold/bake/spelling seam and land together
(< ~400 changed lines outside tests).

## 10. Risks / open questions

- The `edit_explore_note` signature change touches every caller/test — mechanical.
- Per-segment spelling changes some accidental glyphs users have seen; the drawn
  signature and every glyph remain individually truthful (AC8).
- H4 will pass instrument windows into `fold_into_window`; voice stays unconstrained
  (explicitly excluded by the founder) — out of scope here.

## 11. References

- #471 (issue + investigation-findings comment — ground truth), #292 (cell editing),
  #335 (one-voice spelling), #419 S4 (stored-seed law), #277 (display honesty family).
- `docs/architecture/rv-methodology.md` — the cell × row × modifiers north star.
