# Spec: CellStaff engraving — real margins, real systems (#445 pts 1+5, F1)

## 1. Summary
Two founder reports, one layout root cause. (1) The key signature and
first/last notes collide with barlines. (2) The staff pages horizontally
(4 measures + pager); the founder reads music in LINES: "groups of 4
measures per line, no scroll — people don't scroll through music."

## 2. Root causes (from the code, not guesses)
- Key sig glyphs sit at `38 + i*8` while the first barline is pinned at
  `LEFT_PAD = 56`: five or more accidentals spill PAST the barline.
- Notes and barlines use DIFFERENT scales: barlines divide
  `innerWidth - 6`, note beats divide `innerWidth - 20` (+14 offset), so
  the drift grows with measure index — by measure 4 the first note sits
  left of its own barline.

## 3. Contract
- **Dynamic left pad**: `leftPad = CLEF_ZONE + |fifths| * KEYSIG_STEP +
  KEYSIG_GAP` — the first barline never starts until the key signature
  has fully ended.
- **Per-measure local placement**: each measure's notes are laid out
  inside `[barlineX(i) + NOTE_INSET_L, barlineX(i+1) - NOTE_INSET_R]`
  (single shared barline grid; no second scale). First and last note
  columns are inset by construction, accidentals (drawn at `x-11`)
  included — an accidental never touches the barline either.
- **Systems, not pages**: measures render in systems of 4 per line,
  stacked vertically in ONE svg (per-system y-offset), each system
  carrying its own staff lines, clef, and key signature (standard
  engraving). The `< >` pager is REMOVED. All measures always visible;
  height grows with system count (h-auto keeps it proportional).
- **Editing survives**: tap-select, drag (staff_steps quantization), and
  the selection halo work on every system — y math uses the note's own
  system offset.
- Rhythm-layer rule unchanged: stems/flags never move noteheads.
- No DTO changes; backend untouched. Frontend-layout-only slice.

## 4. ACs
1. With 6 sharps (F# major), every key-sig glyph ends before the first
   barline (a geometry assertion, not a snapshot).
2. In every measure of every system, min(note x - accidental width) >
   its left barline and max(note x + head rx) < its right barline.
3. 12 measures render as exactly 3 systems of 4; no pager buttons in
   the DOM; a 2-measure staff renders one system of 2.
4. Notes on system 2+ carry the system's y-offset (a note's cy in
   system k ≥ system k's staff top), and each system draws its own clef
   + key signature.
5. Tap-select and drag-edit work on a note in the LAST system
   (regression: the existing editing tests keep passing + one new test
   targeting a system-2+ note).
6. Rhythm toggle still reflows nothing (existing pin holds).

## 5. Test map
| AC | Test |
|---|---|
| 1 | key sig fully left of barline at fifths=6 and fifths=-6 |
| 2 | note/accidental insets hold in first + last measure of each system |
| 3 | 12 measures → 3 systems, no pager; 2 measures → 1 system |
| 4 | system y-offsets + per-system clef/key-sig count |
| 5 | select + drag on a system-3 note emits the right edit |
| 6 | existing rhythm/edit suites green unchanged |
