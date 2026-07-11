# Spec: Jazz Ears — hearing chords: triads, inversions, extensions (#349)

> Founder direction 2026-07-11: RV has triads, inversions, and advanced jazz
> chords — the app must hear them, and **displaying + labeling what we hear
> ("I hear: Cmaj7") is the headline deliverable**. Three tiers, each
> independently shippable, each behind the repo's full gates.

## 1. Summary
Today the live path is monophonic (YIN): a piano triad reads as one shaky
fundamental. This spec adds (T1) an in-house chromagram chord engine with a
live "I hear: C7/E" label, (T2) chord verdicts against expected drill
material, and (T3) streamed note-level polyphony via the already-vendored
basic-pitch model — up to lifting a chord progression into the RV engine.

## 2. Problem / why
Piano and guitar are multiphonic instruments; RV's material includes block
triads, inversions, and jazz extensions. The current YIN detector cannot
represent simultaneity at all — chord instruments feel unheard the moment
they stop playing single lines. Naming the chord live is also the cheapest
trust-builder we have ("it *hears* me") — same lesson as the reveals.

## 2b. Where these ears go — the use-case layer (founder review, 2026-07-11)
The same engine stack serves four escalating surfaces. Build once, deploy
everywhere — this section is WHY the tiers are worth it in today's world:

1. **Practice feedback** (T1–T2): the drills and labels above — the core.
2. **Jam-along — "play any song, see the chords."** The chroma engine is
   SOURCE-AGNOSTIC: put on a record in the room and the strip labels the
   harmony live. This is a category consumers already know (Chord AI,
   Chordify — all cloud); ours is offline and, decisively, wired into the
   practice loop: **one tap takes the chord you just heard into a 12-key
   RV row** (the reveal → practice-this-sound gesture, extended to any
   music playing in the room). The moat isn't the hearing, it's that what
   you hear becomes what you practice.
3. **Ensemble / band-level hearing** (live, one mic — #40's long-standing
   ask): harmony, key, and groove of the WHOLE GROUP. Group-level verdicts
   are honest from a single mic: chord correctness of the ensemble sound,
   tempo spread ("the band rushed the bridge"), togetherness (onset
   scatter). A band director's rehearsal companion — and the natural
   ensemble surface for the teacher dashboard (#305/#306).
4. **Per-player band feedback** (offline): a rehearsal recording → local
   stem separation (#328's already-decided Demucs architecture) → per-stem
   basic-pitch → per-player verdict reports in the recap. "The trumpets
   were clean; the trombones missed the F# all rehearsal."

**The honesty line (what we do NOT promise):** live per-instrument
feedback from one room mic — real-time separation of a full band is
research-grade; anyone who demos it is cherry-picking. Live = group-level
only; per-player = offline via stems. This line is stated in product copy
wherever band features appear.

## 3. Model / engine decision (assessed 2026-07-11)
**Tier 1 needs no model.** Chromagram + template matching + a bass note from
the existing YIN track covers triads, inversions (slash chords), and the
jazz vocabulary at practice-feedback quality, in pure Rust, license-clean,
offline, and real-time.

**Tier 3 uses basic-pitch (Spotify)** — the lightweight instrument-agnostic
polyphonic AMT model (ICASSP'22). Decisive properties: **already vendored in
`crates/transcribe`** (ONNX, `ort` load-dynamic, the .wav import path),
Apache-2.0, ~10 MB, CPU-real-time in hopped windows, instrument-agnostic
(piano AND guitar AND anything else).

Rejected alternatives, with reasons — recorded so we don't re-litigate:
- **CREPE / SwiftF0 / pesto**: monophonic-only — wrong problem.
- **Onsets & Frames / MAESTRO models**: piano-only, heavier; RV needs
  instrument-agnostic.
- **MT3 / transformer AMT**: offline-scale compute; not a desktop live path.
- **Research chord transformers (BTC, 301-class LLM-CoT rerankers)**:
  research code, no maintained on-device runtime, unclear licenses.
- **madmom chord models**: non-commercial license terms — unusable.
- **Essentia / chordino (NNLS-chroma)**: (A)GPL — unusable in this app.
- **Consumer tools (Chord AI, Moises, Chordify…)**: cloud services, not
  libraries; violate offline-first.
If a materially better Apache/MIT on-device model appears, T3's engine seam
(§6 T3) is where it swaps in — the decision above is architecture, not
allegiance.

## 4. Non-goals (all tiers)
- No change to the <25 ms YIN live-meter path — chroma and basic-pitch run
  on the worker thread beside it, never inside it.
- No polyphonic **score following** (two-hand piano vs score alignment) —
  out of scope entirely; revisit only after T1–T4 ship.
- No LIVE per-instrument separation (§2b honesty line): live band hearing
  is group-level; per-player detail is offline via #328 stems.
- No cloud inference of any kind.
- No chord-symbol *theory pedagogy* (voicing suggestions, substitutions) —
  the coach may use labels later; this spec only hears and labels.

## 5. Shared foundations (land with T1)
### 5.1 `theory::chords` — the vocabulary
Pitch-class-set templates, root-position, as interval sets from the root:
```
maj {0,4,7}          min {0,3,7}          dim {0,3,6}         aug {0,4,8}
sus2 {0,2,7}         sus4 {0,5,7}         maj6 {0,4,7,9}      min6 {0,3,7,9}
dom7 {0,4,7,10}      maj7 {0,4,7,11}     min7 {0,3,7,10}     m7b5 {0,3,6,10}
dim7 {0,3,6,9}       minMaj7 {0,3,7,11}  7sus4 {0,5,7,10}
add9 {0,2,4,7}       dom9 {0,2,4,7,10}   maj9 {0,2,4,7,11}   min9 {0,2,3,7,10}
dom13 {0,4,7,9,10}   7b9 {0,1,4,7,10}    7#9 {0,3,4,7,10}    7#11 {0,4,6,7,10}
```
(dom11/13 shells use the jazz convention: 3rd+7th present, 5th optional,
tensions added — matching is subset-tolerant, see §5.2.) Labels via the
existing `tonic_display_name` (#335) so **chord spelling follows the key
signature**: in a flat context the same pcs label "Db7", never "C#7".

### 5.2 `ears::chroma` — the front end
- 4096-sample FFT (Hann) over the worker's existing mono window at the
  detect cadence (~45 Hz input, chroma computed at ~10 Hz — every Nth
  window; the label doesn't need frame rate).
- Spectral whitening + harmonic-weighted folding into 12 log-frequency bins
  (C1–C7), per-bin exponential smoothing (τ ≈ 250 ms) so a strum/arpeggiated
  attack settles into one reading.
- **Allocation rule**: all buffers pre-allocated at construction; `chroma()`
  is allocation-free, same contract (and same counting-allocator test
  harness) as `PitchDetector::detect` (#245).

### 5.3 `brain`/`theory` — the matcher
```rust
pub struct ChordReading {
    pub root_pc: u8,
    pub quality: ChordQuality,     // enum over §5.1
    pub bass_pc: Option<u8>,       // from YIN's low track → inversions
    pub label: String,             // "C7/E" — spelled per active signature
    pub confidence: f32,           // 0..1 match score
    pub pc_mask: u16,              // the raw sounding set, for honesty UIs
}
```
Matching: normalize chroma → score every (root × template) by weighted
dot-product with a penalty for strong non-chord bins; subset tolerance for
shells (dom13 without the 5th still matches); **hysteresis** like the key
tracker — a new label must beat the incumbent by a margin for 3 consecutive
readings (block chords change ~1–2/sec; flicker is the enemy). Inversion:
if YIN's confident fundamental maps to a chord tone ≠ root → slash label;
if it maps to a non-chord tone, no slash (silence > lies).
Gate: below `MIN_CHORD_CONF` **and** ≥3 simultaneous strong bins → the
honest fallback state "hearing several notes…" — never a guessed label,
never silence that reads as deafness.

## 6. The tiers

### Tier 1 — Chroma chord engine + live label
**Contract:** `PerceptionSnapshot` gains `chord: Option<ChordReading>`
(additive serde). The "I hear" strip renders it beside key/tempo:
`I HEAR  ~112 BPM · 🎵 Bbmaj7 · F major` — chord label uses `Label/Chip`
type and the root's RV color as accent (design system: RV Note Cell rules).
Free play only in T1 (drills stay monophonic-judged until T2).
**Plumbing:** worker thread computes chroma at ~10 Hz → matcher in
`PerceptionTracker::observe` path → rides the existing throttled
`perception` event. Zero new IPC.
**ACs (each = 1+ test that can fail for a real reason):**
1. Synthetic additive-synthesis fixtures (summed sines + harmonics, the
   deterministic equivalent of our pitch tests): C major triad → "C";
   first inversion (E bass) → "C/E"; Cmaj7, C7, Cm7, Cdim7, C7#9, F#m7b5,
   Bb13 shell → correct labels. Sweep all 12 roots for the core qualities.
2. Spelling honesty: pcs {1,5,8} in a 5-flat context labels **Db**, in a
   5-sharp context **C#** (reuses #335's `tonic_display_name`).
3. Single notes and monophonic lines produce **no** chord label (the meter
   is YIN's job); ≥3-note ambiguity below confidence → "hearing several
   notes…" state, pinned.
4. Hysteresis: an alternating two-chord vamp relabels at chord rate; a
   sustained chord with detector jitter never flickers (mutation: drop the
   dwell → test fails).
5. Allocation gate: chroma path zero-alloc under the counting allocator.
6. Latency gate: the existing `latency-bench` stays <25 ms (chroma is off
   the detect hot path; the bench proves we didn't cheat).
7. Guitar profile ships (`profiles/guitar.json`: E2–E6, tuning tolerances)
   — one JSON, no code.

### Tier 2 — Chord verdicts (RV block-chord material, judged)
**Contract:** `variations` gains `ChordVoicing` figure emission — a cell
step may be a *stack* (existing `stacked` deferral in the spec header gets
its due). `GeneratedNote` gains `chord_group: Option<u32>` (additive):
notes sharing a group sound together; CellStaff draws them as a vertical
dot stack; MusicXML emits `<chord/>` marks (emit.rs already handles the
element downstream? — verify, else add).
**Judging:** follower stays melodic; block-chord DRILLS are judged by the
T1 engine instead: expected `ChordReading` (from the drill spec) vs heard —
Hit = same root+quality (bass ignored unless the drill demands the
inversion), Near = right root wrong extension / right quality wrong
inversion when demanded, Missed = otherwise. Rides the existing
`note-verdict` event + VerdictStrip + phrase cards + score recap unchanged.
**ACs:**
1. A "C7 in all 12 keys" chord drill deals 12 stacked cells; CellStaff
   renders stacks; the grading target carries expected ChordReadings.
2. Synthetic played-chord fixtures: right chord → Hit; C7 for Cmaj7 → Near;
   Am for C → Missed; inversion-demanded drill judges bass.
3. Free play and melodic drills bit-identical to today (no regression, the
   #337 suite stays green).
4. Exercise log rows for chord drills (`source: "lesson"`, spec shape
   `"C7 block chords"` via the insights shape fn).

### Tier 3 — Streaming polyphony (basic-pitch live) + polyphonic lift
**Contract:** `ears::poly` runs the vendored basic-pitch ONNX in **2 s
windows, 1 s hop**, on the worker thread (allocation allowed there;
inference budget ≤250 ms/hop on a 2020 MacBook Air — measured, gated by a
bench). Output: note events `{midi, on_secs, off_secs, amplitude}` at ~1 s
latency, feeding:
1. **Voicing-true labels**: exact sounding midis refine T1's chroma label
   (true inversions, doublings, wide jazz voicings chroma smears).
2. **Honest polyphonic phrase evidence**: phrase cards/recap can say
   "comping: 14 chords, 11 clean".
3. **The flagship — lift a progression**: `lift_progression_from_notes`
   segments the note stream into chord events → a *progression cell*
   (sequence of ChordReadings) → rowed through 12 keys by the explore
   engine (transposition of root_pc per key; voicing templates re-realized).
   "Work on my last progression" button beside the existing lick lift.
**Engine seam:** `PolyEngine` trait with the basic-pitch impl — the swap
point if a better Apache/MIT model lands (§3).
**ACs:**
1. Synthetic 2-chord comping fixture → note events reproduce both chords'
   midis (±1 semitone tolerance per note, onset within 200 ms).
2. Inference budget bench: p95 hop ≤250 ms CPU; live meter latency bench
   unaffected.
3. Progression lift: played ii–V–I fixture lifts to 3 ChordReadings and
   rows: "Dm7 G7 Cmaj7 → through 12 keys" renders and logs (`source:
   "progression_lift"`).
4. Kill-switch honesty: if the model file is absent/failed, T3 features
   hide and T1/T2 keep working (calm degradation, no crash — #267 guard
   pattern around inference).

### Tier 4 — Room Ears: jam-along + the band (builds on T1–T3, #40, #328)
**T4a — Jam-along mode.** A deliberate capture mode ("Listen to the room")
where external music is the SIGNAL, not bleed: the strip shows live chord +
key + tempo of whatever is playing; a rolling chord lane (last ~8 labels)
replaces the phrase cards. **One tap on any chord in the lane → the RV
bridge**: that quality rowed through 12 keys (reuses `start_explore_cell`
with the chord's template as a stacked cell — T2's machinery). Session recap
becomes a chord chart sketch: the label sequence with timestamps, honest
confidence dots per label. No song identification, no audio retention beyond
the session (privacy copy states it), fully offline.
**T4b — Ensemble session (live, one mic, group-level).** A session mode for
bands/sections: group verdicts only — ensemble chord vs expected (when a
score/drill is loaded: T2 judging over the mix), tempo spread per section of
the piece (groove tracker already measures this), onset togetherness score
(std-dev of near-simultaneous onsets), balance trend (band-level dynamics).
Recap: "the band" as the player. Fulfils #40's group-level scope exactly;
teacher-dashboard surfaces consume the same recap rows later.
**T4c — Band report (offline, per-player).** Import/record a rehearsal →
#328 stem separation (local, already-decided) → per-stem basic-pitch (T3
engine, batch mode) → per-stem verdicts against the score's parts (#337
S1 part picker maps stems to parts) → a sectioned recap: per-player
accuracy + worst measures. Depends on #328 S2+ shipping; the seam is
`PolyEngine` batch mode over stem files.
**ACs:**
1. T4a: a synthetic 4-chord "record" fixture (mixed, with percussion noise
   bed) yields the 4 labels in order in the chord lane; tapping one rows
   it; recap carries the timed chord sketch. Zero network assertion.
2. T4a honesty: dense/atonal mix → "hearing several notes…" lane entries,
   never fabricated labels; confidence dots pinned.
3. T4b: two synthetic 'players' (offset onsets, one rushing) → togetherness
   score degrades and tempo-spread names the rushing section; group chord
   verdict judged over the mix.
4. T4c: a 2-stem synthetic rehearsal → per-stem reports with correct
   attribution; a stem the model can't read degrades calmly per stem.
5. Every T4 surface carries the §2b honesty copy (group vs per-player).

## 7. Edge cases & failure modes (all tiers)
Piano sustain pedal (previous chord rings under the new one — chroma
smoothing + onset-weighted update; T3's note-offs disambiguate) · guitar
strum spread (~80 ms — the 250 ms smoothing absorbs it; test fixture) ·
detuned instruments (±30 cents folding tolerance per bin) · low register
muddiness (piano LH octaves: harmonic weighting discounts octave doublings)
· two players/voices at once (out of scope — label what the mix implies,
honesty state when unstable) · enharmonic labels (always via signature
spelling, #335) · silence/decay tails (label clears with a grace period,
never sticks stale) · mic bleed of the accompaniment band (band is
click/synth on speakers — document as known limit; headphone tip already in
the strip).

## 8. Test plan
- **Fixture synthesis in-test** (no audio files): summed sine+harmonic
  renderers in `crates/ears/tests/fixtures.rs` — deterministic, license-
  free, covers every AC chord. Real-recording spot-checks live in the VA
  playbook (piano + guitar steps) not CI.
- Unit: template matcher (property test: every template at every root
  round-trips), chroma folding, hysteresis, spelling.
- Integration: worker-thread plumbing (perception snapshot carries chord),
  drill judging end-to-end, progression lift through the explore engine.
- Gates: existing latency bench (hard), new zero-alloc chroma test (hard),
  new T3 inference bench (hard), full workspace suites, adversarial review
  with live mutations per slice — the standing loop.
- VA playbook: new steps per tier (play a triad → label appears; play the
  inversion → slash label; wrong chord in a chord drill → honest verdict).

## 9. Slices (each < ~400 lines, one PR, full loop)
1. **T1a** `theory::chords` vocabulary + matcher + spelling (pure logic).
2. **T1b** `ears::chroma` front end + zero-alloc + latency gates.
3. **T1c** perception plumbing + "I hear" chord label UI + guitar profile
   + VA playbook step. ← *the founder's headline lands here*
4. **T2a** stacked figures in `variations` + CellStaff stacks + MusicXML.
5. **T2b** chord drills + T1-engine judging + verdict plumbing.
6. **T3a** `PolyEngine` seam + basic-pitch streaming + benches.
7. **T3b** voicing-true labels + polyphonic phrase evidence.
8. **T3c** progression lift + "Work on my last progression".
9. **T4a** jam-along mode: room-listening capture mode + chord lane +
   tap-to-row + chord-sketch recap.
10. **T4b** ensemble session: group verdicts, togetherness, tempo spread
    (closes #40's group-level scope).
11. **T4c** band report: stems → per-stem T3 → sectioned recap (gated on
    #328 S2).

## 10. Open questions (founder)
1. T1c label placement: inside the "I hear" strip (recommended — one
   glanceable line) or a dedicated chord card below it?
2. Chord vocabulary cap for v1 labels: the §5.1 set (~24 qualities,
   recommended) or triads+sevenths only first?
3. T3 hardware floor: is a 2020 Intel MacBook Air the oldest machine we
   promise live polyphony on? (Sets the inference budget.)
4. T4a jam-along: v1 as its own mode on the picker (recommended — it's a
   different mental posture than practicing) or a toggle inside free play?
5. T4 priority: jam-along (consumer wow, zero new engine work after T1)
   before or after T2 chord drills (practice depth)? Recommendation:
   T4a immediately after T1 — it is almost free and it demos the ears.
