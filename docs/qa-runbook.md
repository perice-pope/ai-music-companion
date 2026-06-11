# Manual Hardware-QA Runbook

**Audience:** anyone signing off a release build of AI Music Companion.
**When to run:** before tagging a release, after any change to audio capture,
IPC, the Tauri shell, or `ScoreView` rendering, and whenever the automated
latency benches change.

## Why this document exists

The automated latency gate
(`.github/workflows/latency-bench.yml` + the Criterion benches in
`crates/ears/benches/latency.rs` and `crates/brain/benches/`) measures only the
**Rust analysis + alignment path**:

| Stage (architecture-v2 §10) | Budget | Covered by automation? |
|---|---|---|
| Audio capture (cpal buffer) | ~5 ms | **No** — no audio device in CI |
| Pitch detection | ~6 ms | Yes — `ears` `latency_per_event` bench |
| Score alignment | ~3 ms | Yes — `brain` `score_align_path` bench |
| IPC + render | ~5 ms | **No** — no Tauri runtime / GPU / display in CI |
| (composed local chain) | ~9 ms | Yes — `brain` `integrated_analysis_path` bench |
| **Total mic-to-screen** | **<25 ms** | **Partially** — see below |

A green CI run means *the analysis and alignment math is within budget*. It does
**not** prove the end-to-end mic-to-screen latency, because three stages can only
be measured on real hardware:

1. **Real-mic capture** — needs a physical microphone + OS audio stack (cpal).
2. **Tauri IPC serialization** — needs the running Tauri runtime (Rust ⇄ JS bridge).
3. **GPU/SVG render** — needs a real display and compositor (OSMD/VexFlow paint).

This runbook covers exactly those three. Each check ties back to the §10 budget
stage it validates, gives a concrete measurement method, and has explicit
pass/fail criteria.

---

## Pre-flight

- [ ] Build a **release** bundle for the target OS (debug builds add latency and
      will give misleading numbers):
  - macOS / Linux: `just` build of the Tauri app in release mode, or
    `cargo tauri build` from `apps/desktop`.
  - Windows: same, from a Windows host (cross-compiled bundles are not valid
    for latency measurement).
- [ ] Use a **wired** audio interface or the built-in mic, not Bluetooth.
      Bluetooth audio adds 100–300 ms of its own and invalidates the test.
- [ ] Quiet room, instrument or voice ready, tuner handy.
- [ ] Record the machine: OS + version, CPU, audio device, buffer size if
      configurable. Latency is hardware-dependent; numbers are only comparable
      against the same class of machine.

---

## Check 1 — Real-mic end-to-end latency "feel" + loopback measurement

**Validates:** the *whole* §10 budget end-to-end (capture ~5 ms + pitch ~6 ms +
alignment ~3 ms + IPC+render ~5 ms = **<25 ms** mic-to-screen), with emphasis on
the capture and IPC+render stages the automation cannot see.

### 1a. Subjective "feel" pass

1. Launch the release app, select your instrument profile, start a free-play
   session (no score needed).
2. Play short, sharp, staccato notes (tongued brass/wind, plucked string, or
   crisp vocal "ta").
3. Watch the live pitch display / meter.

**Pass:** the on-screen response feels *immediate* — there is no perceptible
lag between the attack you hear acoustically and the screen reacting. A trained
musician should not be able to "race" the display.
**Fail:** you can perceive a gap, the display visibly trails your playing, or
fast passages smear / drop updates.

> "Feel" is subjective by design — humans reliably notice >~30–40 ms of
> audio-visual lag, so this catches gross budget blowouts the benches can't.
> For a number, do the loopback test below.

### 1b. Loopback / round-trip measurement (objective)

This measures true acoustic-in → photons-out latency, the only honest end-to-end
number. Two equivalent methods — use whichever you can set up:

**Method A — high-speed camera (preferred, no extra software):**

1. Point a phone camera in **slow-motion / high-fps mode** (240 fps if
   available) at both the instrument/your mouth **and** the screen in one frame.
2. Produce a sharp transient (clap, tongue stop, pluck).
3. Step through the recording frame by frame. Count frames between the
   **physical attack** (lips/string move, or the clap contacts) and the
   **first on-screen reaction** (meter jump / pitch appears).
4. Latency ≈ `frames × (1000 / fps)` ms. At 240 fps each frame is ~4.17 ms.
5. Repeat 5×; report the **median** and the **max**.

**Method B — audio loopback comparison (if you have a 2-in interface):**

1. Split the mic signal: one path to the app, one recorded raw alongside a
   screen-capture of the app at a known, high capture rate.
2. Align the raw-audio onset against the frame where the UI reacts in the
   screen capture; the offset is the mic-to-screen latency.

**Pass:** median mic-to-screen ≤ **25 ms**, max ≤ ~35 ms (allowing one frame of
display jitter).
**Fail:** median > 25 ms. If it fails, compare against the CI bench numbers: if
the Rust analysis path is in budget but the round-trip is not, the regression is
in **capture or IPC+render** (the hardware-only stages) — investigate cpal buffer
size and the Tauri event/serialization path, not the DSP.

Record: `Check 1b — median ___ ms, max ___ ms, method ___, device ___`.

---

## Check 2 — Native Tauri shell smoke test (per OS)

**Validates:** the Tauri runtime and IPC bridge actually start and round-trip on
each target OS — the runtime half of the §10 *IPC + render* stage, which CI
(headless, no Tauri runtime) cannot exercise.

Run the **full list on each of macOS, Windows, and Linux** with a release bundle.
Mark each OS pass/fail independently — a regression is often OS-specific.

For each OS:

- [ ] **Launch:** the bundled app starts from a clean install (not `dev`),
      window appears, no console/devtools errors on startup.
- [ ] **Mic permission:** the OS mic-permission prompt appears on first capture
      (macOS especially); granting it starts live audio. Denying it shows a
      graceful error, not a crash.
- [ ] **IPC round-trip:** starting a session streams live `AudioEvent`s to the UI
      (pitch display updates) — proves the Rust→JS event channel works on this OS.
- [ ] **Score load:** drag-and-drop / open a MusicXML file; it renders in
      `ScoreView` without error.
- [ ] **Recap path:** end a session; a recap is produced (or a clean
      offline/degraded message if no network) — proves the command IPC path.
- [ ] **Clean shutdown:** closing the window terminates the process; no orphaned
      audio thread, no zombie process in the activity/task monitor.

**Pass (per OS):** every box checked, no crash, no hang, no orphaned process.
**Fail (per OS):** any crash on launch, missing mic prompt, dead pitch display
(IPC broken), render error, or leaked process after close.

Record per OS: `Check 2 — macOS ☐  Windows ☐  Linux ☐ (version, build id)`.

---

## Check 3 — OSMD cursor-follow at 60fps under load on real hardware

**Validates:** the *render* half of the §10 *IPC + render* stage — that the OSMD
cursor visually tracks the player at 60fps on a real GPU. The Vitest perf guard
(`apps/desktop/src/components/ScoreView.perf.test.tsx`) only proves the
cursor-seek **algorithm** stays linear (no quadratic regression) under jsdom; it
**cannot** observe real frame rate because jsdom has no compositor or frame clock.
This check supplies the part the unit test honestly can't.

### Setup

1. Load a **long, dense score** (≥ 100 measures, ideally a full movement with
   fast notes — a Bach partita, a busy etude). The longer and denser, the more
   cursor advances per second.
2. Open the browser/devtools **Performance/FPS meter** for the app's webview
   (or use the OS's built-in frame counter / an external 120fps+ camera).

### Procedure

1. Play (or play back a recording into) the score so the follower drives the
   cursor continuously through the dense passages.
2. While the cursor follows, **add load**: resize the window, scroll, and trigger
   a phrase boundary (which fires a recap/tip) so render competes with other work.
3. Watch the cursor and the FPS meter through at least one full page-turn and one
   re-alignment / repeat (cursor jumps backward → resets → walks forward).

**Pass:**
- Sustained **≥ 55 fps** (target 60) while the cursor is actively following.
- The cursor stays glued to the player's position — no visible stutter, no
  "catch-up jump" lag of more than ~1 measure.
- A backward jump (repeat / re-alignment) re-seeks smoothly without a long freeze.

**Fail:**
- FPS drops below ~50 sustained, or visible stutter / dropped frames during
  normal following.
- The cursor lags more than ~1 measure behind, or freezes on a backward seek
  (a sign the linear-walk guard from the unit test regressed into something
  worse on real data).

> Cross-check: if this fails but `ScoreView.perf.test.tsx` still passes, the
> regression is in **real rendering / GPU paint**, not the seek algorithm —
> profile OSMD's `render`/cursor SVG updates, not `moveCursorToMeasure`.

Record: `Check 3 — sustained fps ___, score ___ measures, GPU ___`.

---

## Sign-off

| Check | Validates (§10 stage) | Result | Number / notes |
|---|---|---|---|
| 1a feel | full mic-to-screen | ☐ pass ☐ fail | |
| 1b loopback | full mic-to-screen (capture + IPC+render) | ☐ pass ☐ fail | median ___ ms |
| 2 macOS shell | IPC + render (runtime) | ☐ pass ☐ fail | |
| 2 Windows shell | IPC + render (runtime) | ☐ pass ☐ fail | |
| 2 Linux shell | IPC + render (runtime) | ☐ pass ☐ fail | |
| 3 cursor 60fps | IPC + render (render) | ☐ pass ☐ fail | ___ fps |

**Release is GO only when every applicable row passes.** A failure in Check 1b
with a passing CI bench means the regression lives in the hardware-only stages
(capture / IPC / render) — the runbook is doing its job by catching what the
automation structurally cannot.
