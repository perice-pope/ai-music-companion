# Spec: time-gated click rejection + headphone blurb (#445 pt 2)

## 1. Summary
With speakers + built-in mic, the Pocket's own metronome click leaks into
the mic and reads as onsets — "it hears the click and gets confused about
what's me and what's the click." We *synthesize* the click, so we know
exactly when each one fires. This slice reports every click fire from the
render path over a lock-free SPSC channel, bridges it to wall time, and
gates onset flags in the audio worker that coincide with a recent click.
Plus one quiet UI line: headphones keep the click out of the mic entirely.

## 2. Design (founder-approved)
- **Fire reporting (ears)**: `ClickFire { sample_index, is_accent }` +
  `click_fire_channel(capacity)` in `output_engine.rs` (same ringbuf
  HeapRb SPSC pattern as `pocket_tempo_channel`). `Metronome` gains
  `samples_emitted: u64` (incremented every sample, NEVER reset by
  `update_config`/count-in) and an optional producer installed via
  `with_fire_channel(tx)`. In the fire branch of `next_sample()` it
  `try_push`es — lock-free, alloc-free, render-safe. Because the channel
  lives inside `Metronome`, BOTH anchor mode (bare `Metronome` as
  `RenderSource`) and follow/handoff (`TempoFedMetronome` wrapping the
  same `Metronome`) report.
- **Clock bridge (commands)**: output sample indices and input event
  timestamps have different epochs. `start_pocket` records
  `epoch: Instant` when the output starts and installs
  `ClickGate { fires, epoch, output_sample_rate }` into a shared slot on
  `AppState` (`Arc<Mutex<Option<ClickGate>>>`); `teardown_pocket` clears
  it. The audio worker records its own `session_epoch: Instant` at loop
  start (≈ the instant `timestamp_secs == 0` was captured). A click's
  wall time = `epoch + sample_index / output_sample_rate`; an event's
  wall time = `session_epoch + timestamp_secs`. Both land in the same
  frame to within a few ms — far inside the gate window.
- **The gate (audio worker, PROCESSING thread — locking is fine here,
  never in a render/capture callback)**: each window the worker drains
  the fire consumer (bounded per window) into a small pre-allocated
  ring of recent click wall-times (no allocation in the loop), then, if
  the detected event `is_onset`, strips the flag when the onset falls in
  `[click − CLICK_GATE_PRE_MS, click + CLICK_GATE_POST_MS]` of any
  recent click. `CLICK_GATE_PRE_MS = 15`, `CLICK_GATE_POST_MS = 90`
  (click duration is 30 ms + envelope decay + device/acoustic latency
  slop; pre covers detector-window quantization jitter).
- **Blurb (face)**: one `text-[10px] text-gray-600` line under the
  Pocket's mode chips, always visible with the Pocket UI:
  "Tip: headphones keep the click out of the mic — on speakers I do my
  best to ignore my own click." (`data-testid="pocket-headphone-tip"`).
  Complements (never contradicts) the wired-speakers tip: headphones
  are strictly better for follow mode.

### Honesty properties
- The gate eats only onsets that AGREE with the click — a player right
  on the beat needs no correction anyway — and passes disagreement,
  which is exactly what Follow mode needs to hear. It can never invent
  an onset, only suppress one.
- Pitch is deliberately untouched (scope: tempo confusion; a click
  bleeding into pitch reads is out of scope for this slice — disclosed
  here).
- Fail-open: no Pocket / cleared slot / poisoned lock → nothing is
  gated; onsets flow exactly as today.
- Offline-first: pure in-process plumbing; no network, no disclosure
  needed. No new dependency (ringbuf already in use).

## 3. Contract
- `pub struct ClickFire { pub sample_index: u64, pub is_accent: bool }`;
  `pub fn click_fire_channel(capacity) -> (HeapProd<ClickFire>, HeapCons<ClickFire>)`.
- `Metronome::with_fire_channel(tx)` — builder, like `with_count_in`.
  Reports fire at the exact sample index the click starts (index 0 = the
  first rendered sample). A full ring drops the report (never blocks).
  A `clone()`d metronome does NOT inherit the channel (SPSC producers
  can't be shared); `samples_emitted` is monotonic for the life of the
  instance.
- `audio_pipeline::ClickGate` + `SharedClickGate`; the gate check is a
  pure function `onset_gated(event_wall_secs, &recent_clicks) -> bool`
  with an inclusive window on both edges.
- Gated events keep everything else (pitch, amplitude, confidence,
  timestamp) — only `is_onset` flips to `false`, BEFORE the event
  reaches emit/aggregator/perception, so every downstream consumer sees
  one consistent story.

## 4. ACs
1. A `Metronome` with a fire channel at a known BPM/sample-rate reports
   fires at the expected sample indices (± block math), with the accent
   flag correct — through the bare-`Metronome` path AND through
   `TempoFedMetronome` with a mid-stream retime (fires keep arriving at
   the retimed grid).
2. `samples_emitted` continuity: count-in and `update_config` never
   reset the fire timeline (indices stay monotonic and correct across
   both).
3. `onset_gated`: inside pre/post window → gated; outside → passes;
   empty click list → passes; boundary values (exactly −15 ms, exactly
   +90 ms) pinned inclusive.
4. Worker-loop integration: with a gate installed whose clicks coincide
   with the detected onsets, no emitted event carries `is_onset`; with
   clicks that DISAGREE (far from the onsets), the onsets pass; without
   a gate, behavior is unchanged.
5. Alloc law: a `Metronome` WITH a live fire channel renders with zero
   heap allocations (CountingAlloc pattern; existing
   `pocket_alloc_test.rs` stays green).
6. The headphone tip renders under the Pocket UI in both idle and
   playing states, without displacing the existing Pocket UI (all
   existing Pocket tests stay green).
7. Gate lifecycle: `start_pocket` installs the shared gate,
   `teardown_pocket` (stop, session end, band start) clears it.

## 5. Test map
| AC | Test |
|---|---|
| 1 | ears `output.rs`: `fire_channel_reports_click_sample_indices_and_accents`; `output_engine.rs`: `tempo_fed_metronome_reports_fires_through_retiming` |
| 2 | ears `output.rs`: `fire_timeline_survives_count_in_and_update_config` |
| 3 | `audio_pipeline.rs`: `onset_gated_window_boundaries_are_pinned` (+ empty/no-click cases) |
| 4 | `audio_pipeline.rs`: `click_coincident_onsets_are_gated_and_off_beat_onsets_pass` (drives the REAL worker loop with scripted PCM — the pipeline integration is cheap because the loop already has the `CaptureSource` seam, so it is NOT skipped) |
| 5 | `crates/ears/tests/pocket_alloc_test.rs` extended: fire channel attached, zero allocs, fires actually reported inside the measured window |
| 6 | `PocketControl.test.tsx`: tip present in idle + playing states alongside the existing elements |
| 7 | `commands.rs`: `pocket_click_gate_installed_on_start_and_cleared_on_teardown` |
