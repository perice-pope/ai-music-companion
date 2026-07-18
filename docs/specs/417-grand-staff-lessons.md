# Spec: Grand staff for keyboard lessons (#417 item 3)

## 1. Summary
Piano/keyboard lessons render on a grand staff (treble + bass, `<staves>2</staves>`)
instead of a single treble staff. A pianist opening "Teach me a lesson" sees piano
music, not flute music.

## 2. Problem / why
Founder's piano session (#417): "The teach me a lesson is a treble clef but piano
music should be a grand staff." The lesson emitter produces one part with no clef
or staff information, so OSMD defaults to a single treble staff regardless of
instrument. Low drill notes (LH-range roots, stacked chord bass) pile up on ledger
lines below a treble staff — instantly non-credible to a pianist.

## 3. Non-goals
- **No cross-staff chords.** A stacked chord renders whole on the staff of its
  lowest note. Splitting one chord across both staves (bass note LH, rest RH)
  needs `<voice>`/`<backup>` writing and OSMD's cross-staff support is unproven —
  deferred until verified, tracked on #417.
- No grand staff for imported scores (they carry their own staves), free-play
  explore/opener cells (CellStaff SVG), or non-keyboard instruments.
- No hand-phrase intelligence (a run crossing middle C splits per note; proper
  engraving keeps a hand's phrase together — later refinement if it reads badly).

## 4. Contract / interface
- `ScoreModel` gains `#[serde(default)] pub grand_staff: bool` (false = today's
  behavior everywhere; stored models deserialize unchanged).
- `brain::coach::LessonSpec` gains `#[serde(default)] pub grand_staff: bool`,
  set at `start_lesson` from the active instrument's family (`"Keyboard"`),
  alongside the existing `polyphonic` resolution. Its own flag — `polyphonic`
  may later include guitar, which is never grand staff.
- `score_model_to_musicxml`: when `grand_staff`, first-measure `<attributes>`
  emit `<staves>2</staves>` plus `<clef number="1">` G2 and `<clef number="2">`
  F4 (after `<time>`, `staves` before `clef` per the XSD); every `<note>` carries
  `<staff>1|2</staff>` (after `<type>`, before `<beam>`).
- Split rule: pitched note → staff 1 if `midi >= 60` (middle C), else staff 2.
  Chord group → the staff of its lowest note, whole. Rest → the staff of the
  previous sounding note in the measure stream (opening rests → staff 1).

## 5. Acceptance criteria (numbered, testable)
1. A keyboard lesson drill's MusicXML contains `<staves>2</staves>`, a G clef on
   staff 1, an F clef on staff 2, and every non-rest note a `<staff>` element.
2. Notes below middle C (midi < 60) carry `<staff>2</staff>`; notes at/above
   carry `<staff>1</staff>`.
3. A stacked chord whose lowest tone is below middle C renders entirely on
   staff 2 — no chord is split across staves.
4. A non-keyboard lesson's MusicXML is byte-identical to today's output (no
   `staves`, no `clef`, no `staff` elements).
5. `start_lesson` on a Keyboard-family instrument produces drills whose
   `music_xml` satisfies AC1; on Trumpet it satisfies AC4.
6. OSMD parses the grand-staff XML into one instrument with two staves and
   renders without dropping notes (the #356 silent-drop failure mode).
7. Rests between low notes stay on the bass staff (no rest teleports to the
   empty treble staff mid-phrase).

## 6. Edge cases & failure modes
- All-treble keyboard drill (every note ≥ 60): staff 2 exists but is empty of
  notes — must still emit both clefs (an empty bass staff with whole rests is
  how real piano music shows a silent hand). OSMD tolerates staves with no notes
  in a voice — verified by AC6's fixture including such a measure.
- Chord exactly straddling middle C (e.g. C3-E4-G4): lowest note < 60 → whole
  chord staff 2 (AC3).
- Note exactly at midi 60: staff 1 (">= 60" is the pinned boundary).
- Round-trip: the MusicXML parser must not choke on `<staff>`/`<staves>`
  (ignore-unknown is expected; the existing round-trip tests plus one
  grand-staff parse test prove it).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1, AC2 | `emit.rs` unit `grand_staff_emits_two_staves_and_splits_at_middle_c` | staves/clefs present; per-note staff by midi |
| AC3 | `emit.rs` unit `a_chord_stays_whole_on_its_lowest_notes_staff` | straddling chord all-staff-2 |
| AC4 | `emit.rs` unit `non_grand_staff_output_is_unchanged` | emitted XML has no staves/clef/staff |
| AC5 | `commands.rs` test `keyboard_lesson_drills_render_a_grand_staff` | start_lesson_impl(keyboard) → XML AC1; trumpet → AC4 |
| AC6 | `EmittedNotation.osmd.test.ts` new fixture case | OSMD sheet: 2 staves, all notes present |
| AC7 | `emit.rs` unit `rests_follow_the_previous_notes_staff` | low-note/rest/low-note all staff 2 |
| parse edge | `parse.rs`/roundtrip test `grand_staff_xml_parses_back` | notes+durations survive, no error |

## 8. Architecture / approach
All pitch/staff logic in Rust (`crates/brain/src/score/emit.rs`); the frontend
only renders. Instrument identity flows session → `start_lesson` (commands.rs,
where `polyphonic` is already resolved from family) → `LessonSpec.grand_staff`
→ `drill_dto` sets `model.grand_staff` before emitting. Offline, no network,
no audio-thread involvement.

## 9. Slice breakdown
One slice (~350 lines): model flag + emitter + lesson wiring + tests + OSMD
fixture. No parallel decomposition needed.
