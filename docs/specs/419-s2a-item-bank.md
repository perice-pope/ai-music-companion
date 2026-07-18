# Spec: Openers item bank (#419 S2a)

## 1. Summary
The four resting bank entries go live: Intervals, Chords, Scales,
Enclosures — plus custom sequence entry. Every item compiles through the
same `composite_cell` → 12-key row as S1. Pattern directions and
live-key tonic ride S2b; My Patterns rides S3.

## 2. Problem / why
S1 shipped Notes + NoteSequence with the rest of the RV bank visibly
resting ("the honest roadmap"). The founder's MUST list (#419) names all
of them. The primitives already exist in variations/theory — the bank is
wiring, not invention.

## 3. Non-goals
- No new interval/semitone tables — chords from
  `theory::ChordQuality::intervals()`, scales from
  `variations::ScaleType::semitones()`, enclosure approaches from
  `variations::Enclosure::approach_semitones()`.
- No direction modifier, no live-key tonic (S2b), no My Patterns (S3),
  no persistence (S4).
- Not every quality/scale in the catalogs — a curated, extendable subset
  (the panel must stay a calm bank, not a spec sheet).

## 4. Contract / interface
New `StarterItem` arms (same tagged snake_case wire, purely additive):
- `interval { number: u8 }` — 2..=8, major-scale degree interval from
  the root: compiles to `[0, MAJOR_DEGREE_SEMITONES[number-1]]`.
- `chord { kind }`, kind ∈ major_triad | minor_triad | dominant_seventh
  | major_seventh | minor_seventh — ascending arpeggio of
  `ChordQuality::intervals()` (i8 cast).
- `scale { kind }`, kind ∈ major | natural_minor | major_pentatonic |
  minor_pentatonic | blues | dorian | mixolydian — the scale's
  semitones + the octave (12) as an ascending run.
- `enclosure { style }`, style ∈ one_down_one_up | one_up_one_down —
  `approach_semitones()` then the target root: e.g. `[-1, 1, 0]`.
Refusals: unknown kinds are unrepresentable (typed enums); the existing
Empty/TooLong refusals cover composites.
Panel: four new live rows (chips per kind, same add-item pattern), the
custom sequence input (digits/steps parsed client-side into the EXISTING
note_sequence wire shape — no new pitch logic in TS), COMING_SOON
shrinks to Pattern directions + My patterns.

## 5. Acceptance criteria
1. Each new item kind compiles to exactly the documented offsets
   (delegation pinned per kind against the authoritative tables).
2. Wire: the panel's literal JSON for each new kind deserializes
   (extend the panel-wire pin), and recipes with new arms round-trip.
3. Old recipes (Notes/NoteSequence JSON) still deserialize unchanged.
4. Composites mix freely: e.g. enclosure + chord concatenates in play
   order under the same cap.
5. Panel: each bank row adds its item, chip labels read musically
   ("maj7", "blues", "enclosure ↓↑"), preview refreshes, Begin rows it.
6. Custom entry: "1 5 3 2" (spaces or dashes) becomes ONE note_sequence
   item; junk input gets a calm client notice, nothing sent; backend
   refusals (degree 9) still surface calmly.
7. Preview stays pure; the S1 purity/handoff pins keep passing.

## 6. Edge cases
- Chord+scale composite near the cap → existing TooLong refusal names
  the count.
- Enclosure as the FIRST item: negative leading offsets are legal
  (descending-opener precedent).
- Custom entry empty/whitespace → no item, calm notice.
- Serde: an out-of-range interval number (0, 9) refuses by name like
  degrees do (BadInterval).

## 7. Test plan
| AC | Test |
|---|---|
| 1 | starter.rs: one delegation test per kind vs theory/variations tables |
| 2 | starter.rs: extended panel-wire JSON pin incl. all new kinds; round-trip with new arms |
| 3 | starter.rs: S1 JSON literals still parse (regression pin) |
| 4 | starter.rs: mixed composite concatenation order |
| 5 | OpenersPanel.test: one wire-shape test per new row; chip labels |
| 6 | OpenersPanel.test: custom entry parse + junk refusal client-side |
| 7 | existing S1 suites unchanged and green |

## 8. Architecture
brain::starter grows arms that DELEGATE to theory/variations (brain
already depends on both). All pitch logic in Rust; the panel sends
semantic items only. No commands.rs changes (same preview/begin wire).
