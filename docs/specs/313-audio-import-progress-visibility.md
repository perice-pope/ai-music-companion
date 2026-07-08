# Spec: audio-import progress must actually show (#313)

> Found in VA desktop test #313: ".wav upload produced sheet music, but the 'Listening for
> notes… / Building the score…' loading message did not appear — end result worked, but
> in-progress feedback is missing."

## 1. Summary
The `import-progress` events an audio (or PDF-OMR) import emits must reach the UI **while the
import runs**, so the tester-facing "Listening for notes… / Building the score…" indicator
paints live instead of never appearing.

## 2. Problem / why
`import_audio_file` and `recognize_pdf_score` are **synchronous** Tauri commands. Tauri runs
sync commands on the main thread, which is also the webview's event loop — so for the whole
transcription (seconds of ONNX inference) the UI cannot paint and emitted events cannot be
delivered. All four progress events plus the command result arrive in one burst at the end,
at which point `ScoreDropZone.tsx` has already reached its `finally` and cleared the progress
state. The frontend is correct; the backend starves it.

## 3. Non-goals
- No new progress stages or percentages — same four beats, same payload shape.
- No change to fast imports (MIDI/MusicXML/part listing) — they emit no progress and finish
  in milliseconds; main-thread execution is fine there.
- No streaming per-note progress from inside the transcriber (a later slice if ever needed).

## 4. Contract / interface
- `import_audio_file` and `recognize_pdf_score` become `async` commands that run the heavy
  work via `tauri::async_runtime::spawn_blocking`, leaving the main thread free to deliver
  `import-progress` events and paint.
- IPC surface unchanged: same command names, same frontend-supplied args (`sourceFilename`,
  `bytes`), same return DTOs, same `import-progress` payload `{ stage, pct }`. No TS changes.
- A panic on the blocking thread (beyond the existing #267 transcription guard) surfaces as
  the calm engine-unavailable / reader-stopped message — never a crash, never a hung promise.

## 5. Acceptance criteria (numbered, testable)
1. A successful audio import emits `decoding`(15) → `transcribing`(45) → `converting`(85) →
   `done`(100), in that order, and the import work runs on a **different thread** than the
   command's dispatching thread.
2. An undecodable audio file makes the command return a calm `Err` and emits **no**
   `converting`/`done` stage after the failure.
3. A panic anywhere in the off-thread audio import degrades to the calm engine-unavailable
   message (extends #267's guard to the whole blocking section).
4. A successful PDF recognition emits `rasterizing`(20) → `reading-notes`(55) → `done`(100),
   in that order, with the OMR run on a different thread than the command's dispatching thread.
5. A panic in the blocking OMR section degrades to the calm reader-stopped message and never
   claims `done`.
6. `recognize_pdf_score`'s gates are unchanged: beta-off refuses with the gate message before
   any progress event; the engine-path message and result DTO are identical.

## 6. Edge cases & failure modes
- Panic outside `guard_transcription` (e.g. MIDI→MusicXML conversion) → `spawn_blocking`
  join error → calm message (AC3), not an unresolved IPC promise.
- OMR gate off / engine path missing → early return before any progress event (unchanged).
- Concurrent imports: each command call spawns its own blocking task; the store's mutex
  serializes writes as today.

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `commands::tests::import_audio_command_reports_progress_and_runs_off_the_dispatching_thread` | exact stage/pct sequence; import `ThreadId` ≠ caller `ThreadId` |
| AC2 | `commands::tests::import_audio_file_fails_calmly_without_claiming_completion` | real command + garbage bytes → `Err`, stages end at `transcribing` |
| AC3 | `commands::tests::import_audio_command_converts_a_blocking_panic_to_the_calm_error` | seam panics → engine-unavailable `Err`, no crash |
| AC4 | `commands::tests::recognize_pdf_command_reports_progress_and_runs_off_the_dispatching_thread` | exact stage/pct sequence; OMR `ThreadId` ≠ caller `ThreadId` |
| AC5 | `commands::tests::recognize_pdf_command_converts_a_blocking_panic_to_the_calm_error` | seam panics → reader-stopped `Err`, no `done` beat |
| AC6 | `commands::tests::recognize_pdf_command_stays_gated_off_with_no_progress_events` + existing `recognize_pdf_*` state tests | gate refuses before any event; messages pinned |

A revert of either command to a synchronous signature is additionally pinned at **compile
time**: the AC2/AC6 tests `.await` the real commands.

## 8. Architecture / approach
The fix lives at the command boundary in `apps/desktop/src-tauri/src/commands.rs`, mirroring
how the #267 panic guard was placed: `AppState` logic untouched, the `#[tauri::command]`
wrappers gain an async shell + `spawn_blocking` body that reaches state through a cloned
`AppHandle`. A `import_audio_file_with` seam (same pattern as `import_audio_with`) lets tests
inject the import step to observe threading without ONNX. Offline-first: no network involved.
Real-time: nowhere near the audio thread.

## 9. Slice breakdown
Single slice: async shells + seam + three command-layer tests.

## 10. Risks / open questions
- `spawn_blocking` occupies a blocking-pool thread for the import's duration — the pool is
  sized for exactly this; the async workers and main thread stay free.

## 11. References
- #313 (VA test finding), #267 (panic guard this extends), `apps/desktop/src-tauri/src/commands.rs`
  (`import_audio_file`, `recognize_pdf_score`, `emit_import_progress`),
  `apps/desktop/src/components/ScoreDropZone.tsx` (the already-correct consumer).
