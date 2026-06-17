# Spec: Follow-me accompaniment — a band that locks to YOU in real time (#212)

## 1. Summary
One tap (**"Play with me"**) starts a backing track — drums/bass/pad — in the player's
**detected key**, locked to their **live tempo** and **swing**, and following them as they
change. The core experience is **fully offline** on a local synth. A later opt-in online slice
adds a Suno-class generative "bed." This epic also delivers the **Audio Output & Sound Engine**,
the shared playback foundation that #213 (corrected playback) and #215 (voice coach) also need.

## 2. Problem / why
The app is **capture-only** today: the only device streams are `build_input_stream`
(`crates/ears/src/capture.rs:122-144`); there is **no cpal output stream anywhere**. We already
perceive key/mode (`crates/theory::KeyTracker`), tempo & swing (`crates/groove`), and onset
timing live on the processing thread — but we can make no sound. Practicing with a band feel is
the single biggest engagement multiplier, and "follows you" is something competitors can't fake
without a perception layer like ours. The pieces exist in isolation; nothing connects perception
→ sound → speaker.

Evidence of the seam: `crates/groove` deliberately does **not** implement "lay-back vs a beat
grid" (`crates/groove/src/lib.rs:33-41`) because it needs an external beat reference — exactly
the live clock an accompanist provides. And `crates/ears/src/output.rs` already has no-alloc
generators (`Metronome`, `TuningDrone`, `OutputMixer`) that are wired to **nothing**.

## 3. Non-goals
- **Not** a DAW: no multitrack editing, mixing UI, or arbitrary instrument selection.
- **Not** transcription/chord-charting of a specific song (that's #214). The accompaniment
  follows key/tempo/swing, not a chord progression read off a score.
- **Not** MIDI-out or external-device routing.
- The **Suno-class cloud bed ships last and is deferred** until an API key + cost decision is
  made (per `docs/ROADMAP.md:36-43`). The offline synth path is fully usable without it.
- No new always-on network calls. The offline path is structurally incapable of egress.

## 4. Contract / interface

### 4.1 Audio output engine (Slice 1) — `crates/ears/src/output_engine.rs` (new) + `output.rs`
A playback counterpart to `AudioCapture`. Owns a cpal **output** stream on its own thread,
fed by a lock-free SPSC ring buffer (mirror `capture.rs` `HeapRb`/`split()` pattern). The audio
callback **only** pops samples — no allocation, no locks.

```rust
// crates/ears (device-free core, unit-testable without a speaker)
pub struct AudioOutput { /* holds _stream, sample_rate, channels, producer handle */ }
impl AudioOutput {
    /// Opens the default output device, spawns the callback that drains `consumer`.
    pub fn new() -> Result<(Self, OutputProducer), OutputError>;
    pub fn sample_rate(&self) -> u32;
}
/// SPSC producer the processing/render thread pushes interleaved f32 frames into.
pub struct OutputProducer { /* HeapProd<f32> */ }
impl OutputProducer {
    /// Non-blocking; returns frames actually written (callback never blocks). No alloc.
    pub fn push_samples(&mut self, mono_or_interleaved: &[f32]) -> usize;
}
```
- Reuse `create_sample_buffer`-style split so the ring buffer is testable without a device
  (`capture.rs:201`).
- A `RenderSource` trait (or reuse `OutputMixer`) supplies samples: `fn render(&mut self, out: &mut [f32])`.

### 4.2 Continuous live clock (Slice 2) — `crates/groove` (new `clock` module)
Extend the per-phrase analysis into a **continuous** estimator on the processing thread.
```rust
pub struct LiveClock { /* rolling onset history, no realtime-thread use */ }
pub struct ClockState {
    pub tempo_bpm: Option<f32>,     // live median-IOI tempo, smoothed
    pub swing_ratio: Option<f32>,   // live long/short IOI ratio
    pub beat_phase: f32,            // 0.0..1.0 position within the current beat
    pub confidence: f32,            // 0.0..1.0 — gates whether the band locks
}
impl LiveClock {
    pub fn observe_onset(&mut self, t_secs: f64);
    pub fn tick(&mut self, now_secs: f64) -> ClockState; // advance phase to `now`
    pub fn reset(&mut self);
}
```
Pure logic — deterministic given a timestamped onset stream. Reuses `groove::analyze` IOI math.

### 4.3 Local synth voices (Slice 3) — `crates/ears` synth module(s)
No-alloc `RenderSource` voices that take `(key: KeyEstimate-ish, ClockState)` and render
drums/bass/pad into the mixer. Pre-allocate all buffers/wavetables at construction.
```rust
pub struct AccompanimentSynth { /* drum voice, bass voice, pad voice, all pre-allocated */ }
impl AccompanimentSynth {
    pub fn new(sample_rate: u32) -> Self;
    pub fn set_key(&mut self, tonic: u8, mode: theory::Mode);
    pub fn set_clock(&mut self, clock: ClockState);
    pub fn render(&mut self, out: &mut [f32]); // RenderSource; no alloc, no locks
}
```

### 4.4 "Play with me" wiring (Slice 4) — Tauri command + event + UI
- New `AppState` field `audio_output: Mutex<Option<AudioOutput handle>>` alongside
  `audio_pipeline` (`commands.rs:566`).
- Commands (registered in `main.rs:106`): `start_accompaniment` / `stop_accompaniment`.
- The processing thread (`audio_pipeline.rs` worker) feeds onsets to `LiveClock`, pushes the
  current `ClockState` + key into the synth, and the synth renders into the output ring buffer.
- New Tauri event `accompaniment-status` (`{ playing, tempo_bpm, key_name }`) so the UI can
  show "🎵 Band locked — G Mixolydian · 92 BPM". FE button in the practice view.

### 4.5 Opt-in cloud bed (Slice 5, deferred) — `NetworkPolicy`-gated
Mirror the coaching template exactly: a `NetworkPolicy` (`coaching.rs:54`) honored **in Rust**,
off by default; pre-generate a loop ahead for current key+tempo; the local engine time-aligns it.
Disclosed via a `ToggleRow` in `ConnectionsPrivacy.tsx`, added to
`docs/architecture/network-call-sites.allowlist` and the offline-first doc.

## 5. Acceptance criteria (numbered, testable)
1. **Output engine plays.** Given the audio output engine, samples pushed to its producer are
   drained by the callback in order; the ring buffer never blocks and the callback performs **zero
   heap allocation** (asserted by the no-alloc test harness, mirroring
   `tests/audio_thread_output_test.rs`).
2. **Producer is non-blocking & lossless-until-full.** Pushing more than the buffer capacity
   returns the count actually written (drops excess) and never panics or blocks.
3. **Live clock locks tempo.** Given a synthetic onset stream at a fixed BPM, `LiveClock` reports
   `tempo_bpm` within ±3% after ≥4 onsets, and `confidence` rises above the lock threshold.
4. **Live clock follows tempo change.** Given onsets that step from 90→120 BPM, `tempo_bpm`
   converges toward 120 within a bounded number of onsets (it tracks, not freezes).
5. **Live clock detects swing.** A long/short alternating IOI stream yields `swing_ratio` > 1.4;
   an even stream yields `swing_ratio` ≈ 1.0 (or `None` when ambiguous).
6. **Beat phase advances monotonically** between onsets and wraps at 1.0 → 0.0 at the beat
   boundary implied by `tempo_bpm`.
7. **Synth renders in key.** Given key = G Mixolydian and a locked clock, the bass voice's
   rendered fundamental matches a pitch class in the G-Mixolydian scale (FFT-of-output assertion);
   output is non-silent only when the clock is locked.
8. **Synth is real-time safe.** `AccompanimentSynth::render` performs zero allocation per the
   no-alloc harness, and renders a full callback buffer within the per-buffer time budget.
9. **"Play with me" starts offline.** With no network, `start_accompaniment` produces audible
   backing in the detected key that tracks tempo and swing changes; `stop_accompaniment` silences
   it and releases the output device. (Behavior tested at the command/handle layer; audibility is
   the manual-verify step.)
10. **`accompaniment-status` reflects state.** Starting emits `{ playing: true }`;
    stopping emits `{ playing: false }`; the UI toggle + chip reflect it.
    **Deferred (follow-up slice S4c):** the live `tempo_bpm` / `key_name` in the payload and the
    richer "🎵 Band locked — G Mixolydian · 92 BPM" chip. That needs the pipeline worker to emit
    throttled status updates carrying the current `ClockState` + key — a backend change beyond the
    play/stop wiring shipped in 4a/4b. The play/stop state round-trip (this AC's core) is done.
11. **Offline-first holds.** With the cloud bed toggle **off** (default), no outbound network call
    is reachable from the accompaniment path; `check_network_disclosure.sh` passes. (Slice 5 adds
    the gated online path + disclosure; until then the path has no egress at all.)
12. **Latency budget respected.** The added analysis (LiveClock tick) keeps the
    `samples → AudioEvent` bench under the 25 ms gate; output render stays within its callback
    buffer period.

## 6. Edge cases & failure modes
- **No output device / device error:** `AudioOutput::new` returns `Err` (mirrors capture error
  channel); `start_accompaniment` surfaces it; capture/analysis keep working. No panic.
- **Silence / no onsets:** `LiveClock` reports `tempo_bpm: None`, `confidence: 0` → band stays
  silent (does not invent a tempo). AC7's "non-silent only when locked."
- **Erratic/insufficient onsets (< min):** `swing_ratio: None`, low confidence → no lock.
- **Key flips mid-session:** synth re-pitches at the next phrase/clock update without clicks
  (apply key change on a beat boundary, ramp gain).
- **Tempo doubling/halving ambiguity:** clamp tempo to a sane band (e.g. 40–220 BPM) and prefer
  continuity (smallest change from the current estimate).
- **Output sample-rate ≠ input:** engine uses the **output** device's negotiated rate; synth is
  constructed with that rate. No assumption that in == out.
- **Buffer underrun:** callback outputs silence for missing frames rather than blocking/repeating.
- **Cloud (Slice 5) while offline:** structurally impossible — `Offline` policy short-circuits
  before any HTTP client is constructed (same guarantee as `coaching.rs`).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `ears::output_engine` no-alloc test (mirror `audio_thread_output_test.rs`) | callback drains in order, zero alloc |
| AC2 | `ears::output_engine::tests::push_over_capacity` | returns written count, no block/panic |
| AC3 | `groove::clock::tests::locks_fixed_tempo` | tempo within ±3%, confidence > threshold |
| AC4 | `groove::clock::tests::follows_tempo_step` | converges 90→120 in bounded onsets |
| AC5 | `groove::clock::tests::swing_ratio_{swung,even}` | >1.4 swung; ≈1.0 / None even |
| AC6 | `groove::clock::tests::beat_phase_wraps` | monotonic advance, wrap at 1.0 |
| AC7 | `ears::synth::tests::bass_in_key` | FFT fundamental ∈ scale; silent when unlocked |
| AC8 | `ears::synth` no-alloc + timing test | zero alloc, render within budget |
| AC9 | `commands::tests::accompaniment_start_stop` (handle layer) | start installs output + synth; stop releases | + **manual-verify** audibility |
| AC10 | `commands::tests::accompaniment_status_payload` | payload fields on start/stop |
| AC11 | `scripts/check_network_disclosure.sh` in `just ci` | no undisclosed egress; offline path has none |
| AC12 | `crates/ears/benches/latency.rs` gate + render-timing test | mean < 25 ms; render < buffer period |

Manual-verify (per slice, full checklist in §9): play into the mic and **hear** a metronome
(S1), then a band that locks to your key/tempo/swing and follows when you speed up or swing (S4).

## 8. Architecture / approach
- **Mirror the input side.** The output engine reuses the exact `ringbuf` SPSC +
  cpal-stream-built-inside-the-worker-thread pattern from `capture.rs` / `audio_pipeline.rs`
  (`cpal::Stream` is `!Send` on macOS). The callback only pops; all growth happens on the render
  thread with pre-allocated scratch.
- **Reuse `output.rs`.** `OutputMixer`/`Metronome` already exist and are no-alloc with tests;
  Slice 1 wires them to a real device, Slice 3 adds accompaniment voices as new `RenderSource`s.
- **Live clock on the processing thread**, not the realtime callback — same placement as the
  existing `KeyTracker`/groove analysis (`audio_pipeline.rs` worker). It consumes the onset
  timestamps already produced per `AudioEvent` (`is_onset`).
- **Offline-first.** Slices 1–4 add **no** network code; the synth path cannot egress. Slice 5
  adds the only outbound call, gated by a `NetworkPolicy`-style enum honored below IPC
  (`coaching.rs:54` template), off by default, disclosed in `ConnectionsPrivacy.tsx` + the
  allowlist + offline-first doc, and **structurally incapable of firing when offline**.
- **Hard rules:** no allocation in either audio callback; any `unsafe` carries `// SAFETY:`;
  all music logic stays in the Rust core (the FE only sends start/stop + renders status).

## 9. Slice breakdown (ordered, each a shippable PR)

> Dependency graph: **S1 ⟂ S2** (independent — fleet candidate) → **S3** (needs S1) →
> **S4** (needs S1+S2+S3) → **S5** (needs S4, deferred on API key).

### Slice 1 — Audio output engine (shared foundation) · ~350 lines
- **Scope:** cpal output stream on its own thread + SPSC ring buffer + `AudioOutput` /
  `OutputProducer`; wire `OutputMixer`/`Metronome` to it; `start_metronome`/`stop_metronome`
  Tauri command as a thin, hearable proof. **Satisfies:** AC1, AC2, (foundation for 9, 12).
- **Deps:** none. **Independent** of S2.
- **Manual-verify:** start the app → trigger the metronome → you hear a click; stop → silence.

### Slice 2 — Continuous live clock (tempo/beat/swing) · ~300 lines
- **Scope:** `groove::clock::LiveClock` + `ClockState`; deterministic from a timestamped onset
  stream; smoothing, lock confidence, beat-phase, tempo clamp. **Satisfies:** AC3, AC4, AC5, AC6, AC12.
- **Deps:** none. **Independent** of S1 (pure analysis; can build in parallel).
- **Manual-verify:** N/A (unit-tested); covered live in S4.

### Slice 3 — Local synth accompaniment voices · ~380 lines
- **Scope:** `AccompanimentSynth` (drums + bass + pad) as no-alloc `RenderSource`s driven by
  key + `ClockState`. **Satisfies:** AC7, AC8.
- **Deps:** **S1** (renders into the output engine). Builds against S2's `ClockState` type.
- **Risk:** if it exceeds ~400 lines, split into S3a (drums+bass) → S3b (pad). Flagged in §10.
- **Manual-verify:** a debug command feeds a fixed key+tempo → you hear a groove in that key.

### Slice 4 — "Play with me" live follow (the flagship) · ~300 lines
- **Scope:** wire onsets → `LiveClock` → `AccompanimentSynth` → output engine in the pipeline
  worker; `start_accompaniment`/`stop_accompaniment` commands; `accompaniment-status` event;
  FE "Play with me" button + status chip. **Satisfies:** AC9, AC10, AC11 (offline-only state).
- **Deps:** **S1 + S2 + S3.** **Sequential.**
- **Manual-verify:** play into the mic → band starts in your key, locks to your pulse; speed up →
  it follows; swing → it swings; press stop → silence.

### Slice 5 — Opt-in Suno-class cloud bed (deferred) · ~350 lines
- **Scope:** `NetworkPolicy`-gated pre-generation of a backing bed; local engine time-aligns;
  `ConnectionsPrivacy` toggle (off by default) + allowlist + offline-first doc entry.
  **Satisfies:** AC11 (online path + disclosure), the "Suno-class quality" acceptance line.
- **Deps:** **S4**, **plus an API key + cost decision** (`docs/ROADMAP.md:36-43`). **Sequential, deferred.**
- **Manual-verify:** toggle on → richer bed; toggle off / offline → impossible to fire.

## 10. Risks / open questions
- **Synth quality is subjective** — "feels good" is a manual-verify only the user can judge
  (`docs/ROADMAP.md:42-43`). First pass aims for *musically correct and pleasant*, not produced.
- **Slice 3 size** may exceed 400 lines → pre-planned split S3a/S3b.
- **macOS output device quirks** (sample rate, channel count) — handled by using the output
  device's negotiated config, not the input's.
- **Tempo octave errors** (double/half) — mitigated by clamp + continuity preference; may need
  tuning during manual-verify.
- **Sample assets:** **DECIDED — synthesized voices** (oscillators + envelopes, zero asset/
  licensing burden, fully offline, ships now). Upgrade to sampled/SoundFont later if the sound
  warrants it.
- **Slice 5** is blocked on the API-key/cost decision and which provider (Suno vs alternative).

## 11. References
- Issue #212; `docs/ROADMAP.md:18-49` (Wave 1 foundation; the 3 human decisions).
- Existing generators: `crates/ears/src/output.rs`; its test `crates/ears/tests/audio_thread_output_test.rs`.
- Input pattern to mirror: `crates/ears/src/capture.rs` (ring buffer `:104-105`, `:201`),
  `apps/desktop/src-tauri/src/audio_pipeline.rs` (worker/thread/handle).
- Analysis sources: `crates/groove/src/analyze.rs` (tempo/swing; beat-grid seam `lib.rs:33-41`),
  `crates/theory` (key), `crates/brain/src/follower.rs` (`ScorePosition`).
- Network gating template: `crates/brain/src/coaching.rs:54` (`NetworkPolicy`),
  `docs/architecture/network-call-sites.allowlist`,
  `apps/desktop/src/components/ConnectionsPrivacy.tsx`,
  `docs/architecture/offline-first-and-network-transparency.md`.
- IPC: registration `apps/desktop/src-tauri/src/main.rs:106`; emit helpers `commands.rs:1246-1275`;
  FE subscriptions `apps/desktop/src/App.tsx:55-108`.
</content>
</invoke>
