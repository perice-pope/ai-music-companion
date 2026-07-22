# Spec: RV-simple starter — the 12-note chromatic picker (#471 pt 3)

## 1. Summary
The OPEN panel's default face becomes as simple as the RV screenshot that
started it all: twelve note buttons, a live preview, and Begin. Everything
else the builder grew — sequences, intervals, chords, scales, enclosures,
directions, custom entry — folds behind one collapsed "More options"
disclosure. Backend, the one load-bearing wrong 8 falls: `NoteSequence`
degrees extend 1..=12.

## 2. Problem / why
Founder, verbatim: "how did we go from a 2-button RV screenshot to a
complicated calculator… it should be as simple as RV… if I want to just
start with 12 notes I can. 12-tone rows are the basis of RV."

**The two-button philosophy:** RV's front door is two gestures — pick
notes, go. Every additional control on the default surface is a tax on
the player who just wants their hands moving. Power stays; it just stops
standing between the player and Begin.

The #471 findings comment's caps inventory names the one wrong 8: the
starter's degree vocabulary (`starter.rs` `1..=8` + the panel's `DEGREES`
mirror). Degrees are DIATONIC (steps of the major scale); a 12-tone row
is CHROMATIC (all 12 pitch classes). Raising the degree cap to 12 does
NOT make degrees chromatic — degree 9 is the ninth (octave + 2nd), the
octave-extension reading `starter.rs` already documents with 8 = the
octave. The chromatic "just give me 12 notes" gesture speaks the
`Notes{offsets}` wire, which already supports it. Two vocabularies, both
honest, never conflated.

## 3. Non-goals
- No new wire shapes, commands, or store surgery — the picker compiles to
  ONE existing `Notes{offsets}` item through the existing
  preview/begin/save paths.
- No chromatic DEGREES — the degree table stays diatonic (major scale).
- No cap changes beyond degrees: `LIFT_MAX_NOTES` stays 17.
- No behavior change to My Patterns / Recipes / Yesterday (the #419 arc)
  beyond where they sit on screen — they stay visible (they ARE the
  one-tap RV simplicity).
- No removal of any bank row or testid — everything lives on, foldered.

## 4. Contract / interface
### Backend (`crates/brain/src/starter.rs`)
- `StarterItem::NoteSequence { degrees }` accepts `1..=12` (was `1..=8`).
  8 = the octave; 9..=12 extend through the second octave with the
  compound-interval reading: 9th = 14, 10th = 16, 11th = 17, 12th = 19
  semitones. `MAJOR_DEGREE_SEMITONES` grows to
  `[0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19]`.
- `StarterError::BadDegree` message becomes "scale degrees run 1 to 12".
- `Interval` stays `2..=8` (an interval item is "a 3rd", "a 5th" — the
  compound intervals are not in this slice's brief).
- Wire shape unchanged (`note_sequence` / `degrees`) — old recipes parse.

### Frontend (`apps/desktop/src/components/OpenersPanel.tsx`)
- **Default view** (everything on screen when the panel opens):
  - The chromatic picker: 12 buttons, one per pitch class relative to
    the root, labeled `1 ♭2 2 ♭3 3 4 ♭5 5 ♭6 6 ♭7 7`, testid
    `opener-pc-{0..11}`. Tap to add (in tap order), tap again to remove
    (toggle). Up to 12 notes — each pitch class once.
  - The live preview (`opener-preview`), chips, notices, Begin — all
    existing testids.
  - My Patterns + Recipes + Yesterday strips (existing testids).
  - One `More options ▾` disclosure button, testid `opener-more-toggle`,
    `aria-expanded` honest, **collapsed by default**.
- **Inside the disclosure** (all existing testids preserved verbatim):
  Notes (degree buttons `opener-note-{1..12}` — the mirror grows with the
  backend), Note sequence presets + custom entry, Intervals, Chords,
  Scales, Enclosures, Pattern direction.
- **Picker → wire** (the design decision, stated honestly): the picker
  builds ONE `{type:"notes", offsets}` item. Button k's raw value is k
  semitones above the root (0..11). The sent offsets are **re-based to
  the first tap**: `offsets[i] = tap[i] − tap[0]`, so the first offset is
  always 0 — honoring the documented `Notes` convention ("offsets from
  the CELL's first note") — and taps below the first tap go negative
  (descending openers are legal). Simplest alternative (send k raw) was
  rejected: it would make the cell's first note land off-root and the
  same visual shape sound transposed depending on which button led.
  Re-basing means: the first note you tap is where the row starts.
- The picker item keeps its position among other items (play order); the
  chips row still shows and removes it; removing it (chip or Begin's
  reset or a recipe tap) clears the picker's lit state.

## 5. Acceptance criteria
1. Tapping picker buttons 4, 0, 7 (in that order) previews ONE item
   `{type:"notes", offsets:[0, -4, 3]}` — tap order = note order, first
   tap re-based to 0. Toggling 0 off re-previews `[0, 3]`.
2. Tapping all 12 buttons builds a 12-note cell that previews and Begins
   through the existing `preview_opener`/`begin_opener` flow (the
   12-tone row, two gestures).
3. The disclosure is collapsed by default — none of the folded testids
   render — and after one tap on `opener-more-toggle` every folded
   control is reachable (existing testids unchanged).
4. Backend accepts degrees 9..=12 with compound offsets
   (`[8,9,10,11,12]` → `[12,14,16,17,19]`); refuses 0 and 13 by name
   with the "1 to 12" message.
5. All 27 existing OpenersPanel tests stay green (churn limited to
   opening the disclosure via a shared helper + the degree-range copy).
6. A picker-built cell round-trips as a recipe: save sends the
   `Notes{offsets}` item; the saved recipe re-applies through the
   existing preview path.
7. My Patterns, Recipes, and Yesterday remain visible in the default
   (collapsed) view.

## 6. Edge cases & failure modes
- Tap → toggle same button off → empty picker: the picker item leaves
  `openerItems`; preview clears (existing empty-items path).
- Removing the FIRST tap re-bases the remaining taps to the new first —
  the preview refresh shows exactly what will play.
- Begin / recall / recipe-apply reset `openerItems`: the picker's lit
  state must follow the store (no ghost-lit buttons over an empty
  builder).
- Picker + bank items mixed: the composite cap (17) still refuses
  calmly via the existing `TooLong` message.
- Old saved recipes (degrees ≤ 8) parse and compile unchanged.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `OpenersPanel.test.tsx` "the picker builds ONE notes item in tap order, re-based to the first tap" | preview wire `[0,-4,3]`; toggle-off re-previews `[0,3]` |
| AC2 | "all 12 notes make a 12-tone cell that previews and Begins" | 12 offsets over `preview_opener`, then `begin_opener`, explore lands |
| AC3 | "More options is collapsed by default and folds the whole bank" | folded testids absent, then present after one toggle tap |
| AC4 | `starter::tests::degrees_extend_through_the_second_octave` | `[8..12]` → `[12,14,16,17,19]` |
| AC4 | `starter::tests::out_of_range_degrees_refuse_by_name` (updated) | 0 and 13 refuse; message says "1 to 12" |
| AC5 | the 27 existing tests, via the `openFull()` helper | unchanged behavior |
| AC6 | "a picker-built cell saves as a recipe over the notes wire" | `save_opener_recipe` carries the re-based offsets |
| AC7 | "the default view keeps the one-tap strips visible" | My patterns / Recipes / yesterday testids render collapsed |
| edge: ghost-lit | "Begin clears the picker's lit state" | after Begin, no pressed picker button |
| edge: commands | `commands::tests` degree-9 pins updated to 13 / "1 to 12" | refusal still verbatim |

## 8. Architecture / approach
Pure fold + vocabulary growth. The picker is component-local tap state
synced into `openerItems` via the existing `applyOpenerRecipe` path (set
items + pure preview refresh); no new store fields, no new commands, no
pitch math in TS beyond the re-base subtraction (the offsets ARE the
wire's semantics — the degree→semitone table stays in Rust). Offline,
no network, no audio-thread proximity.

## 9. Slice breakdown
One slice (this PR): `crates/brain/src/starter.rs`,
`apps/desktop/src-tauri/src/commands.rs` (two test pins),
`apps/desktop/src/components/OpenersPanel.tsx` + its test file, this
spec.

## 10. Risks / open questions
- Re-base on first-tap removal changes the remaining notes' sound — the
  live preview makes this visible immediately; accepted as honest.
- Degree buttons 1..12 inside More options widen that row; it is behind
  the fold, where density is acceptable.

## 11. References
- #471 (founder quote + findings comment caps inventory), #419 S1–S4
  specs (`docs/specs/419-*.md`), `docs/specs/445-openers-fix.md`,
  `crates/brain/src/starter.rs`, `apps/desktop/src/components/OpenersPanel.tsx`.
