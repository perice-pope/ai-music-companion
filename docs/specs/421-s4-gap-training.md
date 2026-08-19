# Spec: Gap training — the click that trusts you (#421 S4)

## 1. Summary
The Pocket's classic pro drill, graded for the first time: the click plays
N bars, drops out for M bars, and returns — and because the app LISTENS,
the silence is scoreable. This spec covers the whole S4 arc; **S4a (this
slice) is the gap engine in `crates/ears`**: bar-windowed silence in the
`Metronome` that keeps the beat grid honest, stays out of the #445 click
gate's way, and can be re-sized live over a lock-free channel.

## 2. Problem / why
#421 option 3 (founder-blessed "ship 1, 2, 3"): every deaf metronome app
has gap training as an ungraded toy; ours can grade the re-entry. S1
(Anchor + count-in) and S2 (Follow/Handoff) shipped the click and its
listening wiring; nothing today can silence the click for a stretch
WITHOUT losing the grid — stopping the Pocket forgets the phase, and a
volume hack would keep reporting click fires, making the #445 gate eat
the player's own re-entry onsets as "our click".

## 3. Non-goals (S4a)
- No commands / IPC surface (S4b wires `start_pocket` + a
  `set_pocket_gaps` command through the channel added here).
- No UI (Gaps tab, dimmed-pulse gap rendering, re-entry drift line) and
  no grading policy — the frontend grades re-entry from perception vs
  the frozen grid exactly like Handoff's drift line (S2's
  routed-measurement pattern), in S4b.
- No adaptive-length POLICY ("nail it and the silences grow") — policy
  is frontend; the engine only needs live re-sizing, which S4a ships.
- No partial-bar gaps, no per-beat dropout patterns.

## 4. Contract / interface
- `Metronome::with_gaps(play_bars: u8, gap_bars: u8) -> Self` — builder,
  like `with_count_in`. `gap_bars == 0` disables (today's behavior);
  `play_bars` is clamped to ≥ 1.
- `Metronome::set_gap_cycle(&mut self, play_bars: u8, gap_bars: u8)` —
  control-thread setter behind the builder; same clamping. Applies
  immediately: bar audibility derives from `live bars started so far`
  modulo the CURRENT cycle length.
- `Metronome::is_gap_bar(&self) -> bool` — true while the current live
  bar is silent (count-in is never a gap bar). For the UI's dimmed
  pulse (rule 0: the pulse keeps breathing, dimmed — never unmounts).
- Gap semantics inside `next_sample` (hot path, no allocation):
  - Count-in bars are always audible and do NOT count toward the cycle.
  - A silent bar's beats still advance the grid: `current_beat()`,
    beat spacing, and the fire timeline are IDENTICAL to the audible
    case; only the envelope and the `ClickFire` report are suppressed.
  - No `ClickFire` is pushed for a silent beat — nothing sounded, so
    the #445 gate must not reject mic onsets near the phantom grid
    (the player's re-entry IS the signal we grade).
- `TempoFedMetronome::with_gap_channel(rx: HeapCons<GapCycle>) -> Self`
  plus `pocket_gap_channel(capacity) -> (HeapProd<GapCycle>, HeapCons<GapCycle>)`
  where `GapCycle { play_bars: u8, gap_bars: u8 }`: render side drains
  to the latest message per block (same drain-latest discipline as the
  tempo channel) and applies `set_gap_cycle`. Existing constructors and
  the tempo channel are untouched — S1/S2 behavior is byte-identical
  when no gaps are configured.

## 5. Acceptance criteria (numbered, testable)
1. `with_gaps(2, 1)` in 4/4: bars 1–2 click audibly, bar 3 emits pure
   silence (every sample 0.0), bar 4 clicks again — the cycle repeats.
2. During a silent bar the grid holds: `current_beat()` cycles 0..3 and
   the first audible downbeat after the gap fires exactly one beat
   period after the gap's last (silent) beat — no phase jump anywhere.
3. `with_count_in(1).with_gaps(1, 1)`: the count-in bar is audible
   (all-accent), the first LIVE bar is audible, the second live bar is
   silent — count-in neither gaps nor advances the cycle.
4. `with_gaps(n, 0)` produces a sample stream byte-identical to no gaps.
5. With a fire channel attached, silent beats push NO `ClickFire`; the
   reported indices are exactly the audible beats' sample indices (the
   timeline does not reset or drift across gaps).
6. `set_gap_cycle` mid-play re-maps immediately: with 1 live bar
   elapsed under (2, 2), switching to (1, 1) makes the NEXT bar silent
   (bar index 1 mod 2 ≥ 1) — pinned as an explicit audibility sequence.
7. A `TempoFedMetronome` with a gap channel applies the LATEST pushed
   `GapCycle` (two pushes in one block → the second wins), and the
   render path still never allocates (alloc pin extended).
8. `is_gap_bar()` is false during count-in and audible bars, true
   during silent bars.
9. The first audible downbeat after a gap uses the accent voice (the
   return is unmistakable), and `play_bars = 0` is clamped to 1 — a
   cycle can never be all-silence.

## 6. Edge cases & failure modes
- `gap_bars = 0` → cycle disabled entirely (AC4), not a zero-length gap.
- `play_bars = 0` → clamped to 1 (AC9); an all-silent click is not a
  metronome.
- Re-sizing to a cycle shorter than bars already elapsed: modulo
  arithmetic re-maps — no overflow, no panic (u64 bar counter).
- `update_config` with a narrower time signature mid-cycle: beat_index
  clamps to 0 (existing rule), which may start the next bar early by
  design; the bar counter advances once, never double-counts.
- A click already sounding when its bar would be re-mapped silent:
  the 30 ms envelope finishes (only future FIRES consult audibility) —
  no truncation pop.
- Cloned metronome: gap state copies (it is plain integers); the fire
  channel still does not clone (existing rule).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `output::tests::gap_bars_are_pure_silence_and_the_cycle_repeats` | per-bar audibility over 2 full cycles, silent bar all-zero |
| AC2 | `output::tests::gap_bars_keep_the_grid` | `current_beat()` sequence + exact re-entry fire index |
| AC3 | `output::tests::count_in_is_never_gapped_and_does_not_count` | accent bar, then audible, then silent |
| AC4 | `output::tests::zero_gap_bars_is_todays_behavior` | byte-identical streams |
| AC5 | `output::tests::fire_reports_skip_silent_beats` | reported indices == audible beats only |
| AC6 | `output::tests::set_gap_cycle_remaps_immediately` | pinned audibility sequence across the switch |
| AC7 | `pocket_alloc_test` (extended) + `output_engine` unit | zero allocs with gap channel live; drain-latest wins |
| AC8 | asserted inside AC1/AC3 tests | `is_gap_bar()` at each bar |
| AC9 | `output::tests::gap_return_downbeat_accents_and_play_bars_clamps` | accent voice on return; (0, n) behaves as (1, n) |

## 8. Architecture / approach
All in `crates/ears` (`output.rs`, `output_engine.rs`) — the same seam
S1/S2 used. Gap state is three integers on `Metronome` (cycle config +
a `u64` live-bar counter); the hot path gains integer compares only, no
allocation, no floats. The channel reuses the proven SPSC drain-latest
pattern from `pocket_tempo_channel`. Fully offline; no new dependencies;
no network.

## 9. Slice breakdown (ordered, each a shippable PR)
| # | Slice (goal) | Footprint (files/modules) | Depends on | Heavy build? |
|---|---|---|---|---|
| S4a | gap engine: bar-windowed silence + live re-size channel | `crates/ears/src/output.rs`, `crates/ears/src/output_engine.rs`, `crates/ears/tests/pocket_alloc_test.rs` | S1, S2 (merged) | no |
| S4b | commands + Gaps UI + graded re-entry line (routed-measurement, Handoff pattern) + adaptive policy | `commands.rs`, `PocketControl.tsx`, pocket store slice | S4a, #516 (store slice) | no |

## 10. Risks / open questions
- Whether re-entry grading (S4b) reads perception tempo (like Handoff)
  or the #445-gated onset stream directly is an S4b design point; the
  engine keeps both viable (grid honest, gate silent during gaps).
- Where the Gaps control lives in the one-dialog-three-tabs layout is
  a founder look-and-feel call at S4b time.

## 11. References
#421 (option 3 + slice list; S1 shipped PR #434, S2 shipped PR #441),
`docs/specs/421-pocket-s1.md`, `docs/specs/421-s2-follow-handoff.md`,
#445 (click gate / `ClickFire`), #417 rule 0 (surfaces never blink).
