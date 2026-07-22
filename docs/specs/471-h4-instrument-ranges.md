# Spec: Instrument ranges constrain the row — except voice (#471-4, H4)

> Founder: "instruments have ranges — they can only go so high and so low…
> not vocals tho, leave that be."

## 1. Summary

Thread a per-instrument register window into the octave fold at every
`generate()` call site that renders practice material. The window derives from
the session instrument's profile (`profiles/*.json` frequency range → MIDI),
plugs into the H4 seam #471-2 built (`fold_into_window`), and obeys the same
H2 rule: **never clamp, never bend an interval**. Voice is explicitly exempt.

## 2. Problem / why

The playable window is a fixed C2..C7 (MIDI 36..96) today. A trumpet row can
deal notes a trumpet cannot play; the 12-key row must respect the instrument's
physical range (issue #471 point 4). The lift already folds by octaves — the
row gets the same discipline, per instrument.

## 3. Profile → MIDI window table (all shipped profiles)

Conversion: `midi(hz) = 12·log2(hz/440) + 69`, then per boundary:
**snap to the nearest integer when within 0.05 semitones (5 cents)** — the
profiles store note frequencies rounded to whole Hz, and blind inward rounding
would drop real boundary notes (165.0 Hz is E3 = 164.81, midi 52.02; strict
ceil would steal the trumpet's low E) — otherwise **round inward**
(lo = ceil, hi = floor: the window never claims a note the profile doesn't
cover). Finally intersect with physical MIDI 0..=127. The 0.05 threshold
cleanly separates rounding noise (every profile boundary note lands within
0.025) from deliberate margin (every non-note boundary sits ≥ 0.06 away).

| Profile | Family | Hz range | midi(f_min)..midi(f_max) | Window | Width |
|---|---|---|---|---|---|
| Cello | strings | 65–988 | 35.892..83.004 | **36..83** (C2..B5) | 47 |
| Clarinet | woodwind | 147–1568 | 50.020..91.000 | **50..91** (D3..G6) | 41 |
| Flute | woodwind | 262–2093 | 60.025..96.000 | **60..96** (C4..C7) | 36 |
| French Horn | brass | 87–880 | 40.939..81.000 | **41..81** (F2..A5) | 40 |
| Guitar | strings | 82–1319 | 39.914..88.006 | **40..88** (E2..E6) | 48 |
| Piano | keyboard | 28–4186 | 21.312..108.000 | **22..108** (Bb0..C8) | 86 |
| Trombone | brass | 58–587 | 33.919..73.990 | **34..74** (Bb1..D5) | 40 |
| Trumpet | brass | 165–1047 | 52.020..84.008 | **52..84** (E3..C6) | 32 |
| Violin | strings | 196–3136 | 55.000..103.000 | **55..103** (G3..G7) | 48 |
| Voice | voice | 82–1047 | — | **EXEMPT → 36..96 default** | 60 |

Voice exemption (founder, explicit): any profile whose family is Voice keeps
the default 36..96 window untouched — resolution short-circuits before the
Hz conversion, pinned by test. Unknown instrument, no active session, or a
degenerate window (lo > hi after intersection) → default window.

## 4. Contract / interface

- `variations::FoldWindow { lo: u8, hi: u8 }` (new, pub, Copy). `Default` =
  `MIDI_MIN..MIDI_MAX` (36..96). The window is a **generation parameter
  alongside the seed** — it lives in no spec and no stored artifact, so
  stored specs replay under any instrument (the #419 stored-seed law:
  replay = stored spec + seed; the window is the session's, at replay time).
- `variations::generate_in_window(spec, seed, window) -> GeneratedSequence`
  (new, pub). `generate(spec, seed)` delegates with the default window
  (bit-identical to today — the voice/unknown/test path).
- `VariationSpec` is UNCHANGED — the spec stays instrument-agnostic.
- `GeneratedSequence.range_fallback: bool` (new, additive `serde(default)`):
  true when at least one segment could not fit the instrument window and was
  dealt in the default window instead (§5). Always false under the default
  window.
- `brain::coach`: each generating entry point gains a `*_windowed` variant
  taking `window: FoldWindow` (same seam pattern as
  `fold_into_range`/`fold_into_window`); the existing names delegate with the
  default window. Windowed: `build_first_windowed`, `advance_windowed`,
  `start_explore_windowed`, `apply_explore_delta_windowed`,
  `start_explore_cell_windowed`, `start_explore_chord_windowed`,
  `start_explore_progression_windowed`, `edit_explore_note_windowed`,
  `undo_explore_edit_windowed`, `resume_explore_spec_windowed`.
  `FoldWindow` is re-exported from `brain::coach`.
- `commands.rs`: `fold_window_from_hz(min, max) -> Option<FoldWindow>` (the
  table's conversion, pure), `fold_window_for(&AppState, name) -> FoldWindow`
  (catalog lookup + voice exemption + unknown default), and async
  `session_fold_window(&AppState) -> FoldWindow` (active session instrument,
  else default). Command wrappers that generate become `async` so they can
  resolve the session instrument before calling the sync `_impl` fns, which
  take the window as an argument (testable without a runtime).
- `ExploreDto.range_notice: Option<String>` (new, additive): one calm
  sentence when the dealt row used the fallback (§5); `null` otherwise. The
  frontend surfaces it through the EXISTING `exploreNotice` channel
  (PracticeSession's inline notice); no new UI.

## 5. Cant-fit policy (the H2 rule, extended — DECIDED)

Per segment: a figure FITS the window iff some whole-octave shift lands it
entirely inside — the shift is quantized to octaves, so span ≤ width is NOT
enough: the interval `[lo−min, hi−max]` must contain a multiple of 12 (every
key is guaranteed only when width − span ≥ 11; narrower headroom fits some
keys and not others — fit is judged per key, pinned by test). A fitting
segment folds INTO the instrument window (the fold minimizes overflow, so it
lands fully inside). A cant-fit segment **falls back to the default 36..96
window** — the exact placement H2 ships today — and the sequence is marked
`range_fallback`. **No note is ever clamped; the delta sequence is exact in
every key under every window** (the H2 invariant). Register honesty beats
register comfort: a too-wide figure dealt truthfully outside the horn's range
is honest; a reshaped figure is a lie.

Silent vs notice — judged: **notice, on the explore surface only.** The
explore/opener/lift/recall paths surface one calm sentence through the
existing `exploreNotice` channel ("that pattern reaches past your
instrument's range — dealing it in the full window instead"). The lesson path
has no equivalent inline channel and its catalog figures fit every shipped
window (widest catalog figure ≈ 25 semitones < 32, the narrowest window), so
a lesson fallback is unreachable in practice and stays silent rather than
inventing a new UI surface for it. Documented here; revisit if lessons ever
deal lifted material.

## 6. Threading map (every production `generate` call site)

| # | fold site | window source | path |
|---|---|---|---|
| 1 | `coach::build_drill` → `build_first_windowed` | `start_lesson` (async) → `session_fold_window` → `start_lesson_impl` | lesson start |
| 2 | `coach::build_drill` → `advance_windowed` | `submit_drill` (async) → `session_fold_window` → `submit_drill_impl` | lesson advance |
| 3 | `start_explore_windowed` | `start_explore_variation` (async) → `start_explore_variation_impl` | free-play explore |
| 4 | `apply_explore_delta_windowed` | `apply_variation_delta` (async) → `apply_variation_delta_impl` | chips |
| 5 | `start_explore_cell_windowed` | `explore_last_phrase` (async) → `explore_last_phrase_impl` | lift a lick |
| 6 | `start_explore_cell_windowed` | `explore_measure` (async) → `explore_measure_impl` | measure bridge |
| 7 | `start_explore_cell_windowed` | `preview_opener` / `begin_opener` (async) → `opener_impl` | openers |
| 8 | `start_explore_chord_windowed` | `explore_chord` (async) → `explore_chord_impl` | jam bridge |
| 9 | `start_explore_progression_windowed` | `explore_progression` (async) → `explore_progression_impl` | progression lift |
| 10 | `resume_explore_spec_windowed` | `begin_opener_recall` (async) → `begin_opener_recall_impl` | recall (stored seed) |
| 11 | `edit_explore_note_windowed` (2 internal `generate`s) | `edit_explore_note` (async) → `edit_explore_note_impl` | cell editing |
| 12 | `undo_explore_edit_windowed` | `undo_explore_edit` (async) → `undo_explore_edit_impl` | undo |

The edit engine's BAKE stays window-independent: `unfolded_figures` is the
pre-fold truth (#471-2 F2), so no window can ever smuggle a register shift
into the player's cell. `lift_cell_from_pitch_track`'s ±36 fold is cell
CAPTURE (offsets from the first note), not rendering — untouched.
`starter.rs` builds cells only (no generate) — untouched (concurrently owned
by #471-3).

## 7. Acceptance criteria (numbered, testable)

1. **Derivation:** the trumpet window derives from its profile as exactly
   52..84 (and voice resolves to the DEFAULT window — the exemption pinned;
   unknown instrument → default).
2. **Fit:** a wide cell (span ≤ 31) on trumpet renders entirely within 52..84
   in all 12 keys, delta-exact in every key (the H2 invariant holds under
   narrower windows).
3. **Voice byte-identical:** for any spec/seed, the voice path
   (default window) produces output byte-identical to `generate` today.
4. **Cant-fit fallback:** a cell wider than the trumpet window (span > 32)
   falls back to the DEFAULT window placement — byte-identical to the
   unwindowed render, delta-exact in all 12 keys, `range_fallback = true`,
   and the explore surface carries the calm notice; no note clamps.
5. **Recall stability:** a stored (spec, seed) replayed under the same
   instrument window renders byte-identically on every replay.
6. **No regression:** full workspace + src-tauri + frontend gates green;
   default-window behavior everywhere no window is known.

## 8. Test plan

| AC | Test | Where |
|---|---|---|
| 1 | `trumpet_window_derives_from_its_profile`, `voice_is_exempt_from_range_windows`, `unknown_instrument_gets_the_default_window` | commands.rs |
| 2 | `a_wide_cell_folds_inside_the_trumpet_window_in_all_12_keys` | variations |
| 3 | `default_window_generation_is_byte_identical_to_generate` | variations |
| 4 | `an_unfittable_figure_falls_back_to_the_default_window_without_clamping` (variations) + `cant_fit_fallback_surfaces_the_calm_range_notice` (commands) | both |
| 5 | `stored_seed_recall_replays_identically_under_the_same_window` | commands.rs |
| 6 | existing suites unmodified | all |

## 9. Risks / open questions

- Command wrappers turning `async` is wire-invisible (invoke is
  promise-based) but touches many signatures — mechanical.
- Mid-session instrument switch re-windows the NEXT rep (chips/edits/undo
  resolve the window at call time) — deliberate: the row follows the horn in
  your hands.
- The 5-cent snap is a judged deviation from strict inward rounding,
  documented in §3; without it four profiles lose their true boundary note
  to integer-Hz rounding noise.

## 10. References

- #471 (point 4 + investigation comment), #471-2 spec
  (`docs/specs/471-h2-rv-fidelity.md`) — the `fold_into_window` seam built
  for this slice; #419 S4 (stored-seed law); founder voice-exemption quote.
