# Spec: Accidental collision resolution on the CellStaff (#471 point 1)

## 1. Summary
On dense bars the CellStaff engraves accidentals on top of neighboring noteheads.
This slice makes every accidental legible at any column density by hoisting only the
colliding glyphs above their own column (cue-size, editorial style) — noteheads never
move, so the beat grid keeps telling the truth about rhythm.

## 2. Problem / why
Founder feedback (issue #471, 2026-07-21): "accidentals collide with noteheads" on
dense bars. Mechanism, measured from the #445 systems layout constants
(`apps/desktop/src/components/CellStaff.tsx`):

- Note span per measure = `measureWidth − NOTE_INSET_L − NOTE_INSET_R`
  = `(640 − leftPad − 12)/4 − 22 − 24` = **98.5px** at fifths=0 (86.5px at 6 sharps).
- Columns are placed at raw beat fractions of that span (`xFor`).
- An in-line accidental anchors `textAnchor="end"` at `colX − 11` with ~8px of glyph
  body: box `[colX−19, colX−11]`. A notehead is `cx ± 5.5`.
- Therefore adjacent columns **touch at Δx = 24.5px** and need **Δx ≥ 26.5px for 2px
  clearance**. Quarter columns sit at 24.625px (0.125px clearance — visually
  touching); eighth columns at **12.3125px → the accidental overlaps the previous
  notehead by ~12.2px** (the founder's screenshot). The accidental never overlaps its
  *own* head (right edge `x−11` vs head left edge `x−5.5`: 5.5px gap).

## 3. Non-goals
- **No weighted/re-spaced columns.** Rejected on two grounds:
  1. *Honesty:* the default RV view is stemless dots — horizontal position is the
     ONLY rhythm signal. Widening accidental-bearing columns makes straight eighths
     render unevenly, i.e. the staff would lie about rhythm to make room for a glyph.
  2. *Geometry:* it cannot work anyway. 8 eighth columns with full in-line clearance
     need 7 × 26.5 = 185.5px; the span is 98.5px (fixed — founder's no-scroll systems,
     4 measures per line, constant measure width). Even one in-line accidental in a
     full 8-column bar exceeds the span (6×11 + 26.5 = 92.5px of minimum gaps vs the
     ~91px the bare heads already consume). Chromatic 12-tone material — the core RV
     content — is exactly the case it fails.
- No change to measure width, `MEASURES_PER_SYSTEM`, insets, or notehead positions.
- No accidental-vs-accidental resolution *within one chord column* (pre-existing;
  chord accidentals at distinct steps rarely co-collide at cell densities).
- Sub-eighth columns (< ~10.5px pitch) where *noteheads themselves* overlap: that is
  a different, pre-existing condition, out of scope here.

## 4. Contract / interface
`CellStaff` props unchanged. Rendering contract:

- Every accidental still renders with `data-testid="staff-accidental"` inside its
  note's `<g>`.
- **In-line form (unchanged bytes):** `x = colX − 11`, `textAnchor="end"`,
  `fontSize={13}` — used whenever its box clears every other notehead box by ≥ 2px
  in at least one axis.
- **Hoisted form (new):** when the in-line box would come within 2px of any other
  notehead box in *both* axes (2D check — vertical separation keeps a glyph
  in-line), the glyph renders cue-size (`fontSize={10}`), `textAnchor="middle"`,
  centered on its **column** x (never the stacked-second head offset), risen
  `ACC_DODGE_RISE = 12px` above the column's topmost head; multiple hoisted glyphs
  in one column stack upward by `ACC_DODGE_STACK = 11px` (higher note's glyph
  nearest the heads). Marked `data-dodged="true"`.
- The viewBox grows upward when a hoisted glyph would exceed it (no-vanish rule).

Chosen convention: hoisting an accidental above its note is the established
*editorial / musica-ficta* placement — the honest engraver's move when the line may
not widen. The alternative (engraver's chord-stack: pushing glyphs further LEFT)
intrudes into earlier columns and makes dense bars strictly worse.

## 5. Acceptance criteria (numbered, testable)
1. **Dense-measure clearance:** in a measure of 8 eighth-note columns containing
   accidentals, every rendered accidental glyph box clears every other notehead box
   by ≥ 2px in at least one axis.
2. **Noteheads never move:** the same dense measure renders every notehead at
   exactly the position it has with all accidentals stripped (beat-grid positions,
   equal pitch between eighth columns).
3. **Sparse no-regression:** when nothing collides, accidentals render byte-identical
   to today — `textAnchor="end"`, `fontSize=13`, `x = dot cx − 11`, no
   `data-dodged` — and vertical separation alone (≥ 2px axis gap) keeps a glyph
   in-line even when columns are horizontally tight.
4. **Stacked seconds + accidentals combined:** a dense bar containing a stacked
   second (offset head +9) with accidentals satisfies AC1, and a hoisted glyph
   centers on the column x, not the offset head.
5. **No-vanish:** a hoisted glyph above a high column extends the viewBox; its box
   stays inside `viewBox.minY`.
6. All pre-existing CellStaff tests stay green (stacked-second offsets, #445 inset
   pins, rhythm-layer no-reflow, editing).

## 6. Edge cases & failure modes
- Accidental on the first column of a measure: `NOTE_INSET_L = 22` still covers the
  in-line box (extends to `colX−19`); cross-measure gap (insets sum 46px) always
  clears — no hoist needed from barline proximity alone.
- Chromatic repeated step (C, C# on adjacent eighths): 0px vertical separation →
  hoist (the founder case).
- Adjacent quarters with ≥ 3 staff-steps between notes: 0.125px horizontal but
  4.3px vertical clearance → stays in-line.
- Chromatic second in one column (C + C#, offset head): own-column boxes clear by
  5.5px → in-line, accidental left of the whole chord (existing pin).
- Multiple hoisted glyphs in one column: stacked upward, no glyph-on-glyph.
- Drag ghost preview: a hoisted glyph follows its note's ghost offset like the
  in-line form does.
- Rhythm-layer toggle: hoisting depends only on notehead geometry — toggling
  stems/flags changes nothing (no-reflow rule intact).

## 7. Test plan
All in `apps/desktop/src/components/CellStaff.test.tsx`
(describe `CellStaff — accidental spacing (#471-1)`), geometry computed from
rendered attributes:
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `dense chromatic bars keep every accidental clear of every notehead` | min 2D clearance acc-box vs every dot-box ≥ 2px (fails before fix: −6.8px) |
| AC2 | `resolving collisions never moves a notehead` | dot cx/cy identical with and without accidentals |
| AC3 | `sparse measures keep the in-line engraving untouched` | anchor end, size 13, x = cx−11, no data-dodged |
| AC3 | `vertical separation keeps tight columns in-line` | quarters 3+ steps apart: all in-line |
| AC4 | `stacked second with accidentals stays clear in a dense bar` | AC1 clearance + hoisted x = column x (not +9 offset) |
| AC5 | `the viewBox grows to keep a hoisted accidental visible` | viewBox minY ≤ glyph top |
| AC6 | existing suite | unchanged, green |

## 8. Architecture / approach
Pure geometry inside `CellStaff.tsx` (no music theory, no Rust): after `visible`,
`xFor`, `secondOffset` are known, one O(n²) pass computes, per accidental, whether
its in-line box 2D-collides (< 2px in both axes) with any same-system notehead box
(head boxes include the stacked-second +9 offset). Colliding indices get a hoisted
y (`column top head − 12 − k·11`); the map feeds `Dot` via a new optional
`accDodgeY` prop; viewBox min extends to cover hoisted glyphs. Constants
(`ACC_ANCHOR/ACC_W/ACC_HALF_H/HEAD_RX/HEAD_RY/ACC_CLEAR/ACC_CUE_SIZE/ACC_DODGE_*`)
document the measured model. No network, no audio-thread involvement.

## 9. Slice breakdown
Single slice (one PR): spec + tests + `CellStaff.tsx`. Footprint:
`apps/desktop/src/components/CellStaff.tsx`, `CellStaff.test.tsx`, this spec.

## 10. Risks / open questions
- Glyph ink metrics are modeled (8px body @13, 6px @10), not measured from fonts —
  same model the existing #445 pins use; conservative half-heights chosen.
- A hoisted glyph above a *chord* is ambiguous about which tone it modifies; RV
  cells are melodic-first and the hoist only triggers under collision, so accepted
  (noted for a future chord-vocabulary pass).
- If sub-eighth densities arrive, noteheads themselves collide first — needs its
  own decision (fewer measures per system?), explicitly out of scope.

## 11. References
- Issue #471 point 1; #445 systems layout (`docs/specs/445-staff-layout.md`).
- #349 T2a stacked-second offset; #292 CellStaff architecture (glyph-layer rule).
- `apps/desktop/src/components/CellStaff.tsx` — `NOTE_INSET_L/R`, `xFor`, `Dot`.
