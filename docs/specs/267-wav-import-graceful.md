# Spec: .wav import never crashes; provision a real macOS ONNX (#267)

> Found in VA desktop test #265 (audio upload crashed the app twice; "audio engine 404 during setup").

## 1. Summary
A `.wav`/`.mp3` import must degrade to a calm error instead of crashing when the audio engine is
missing or fails, and the tester kit must fetch a macOS ONNX Runtime that actually exists.

## 2. Problem / why
1. **Kit fetches a non-existent asset.** `ensure_audio_engine` in the kit's `run.sh` and
   `scripts/fetch-onnxruntime.sh` request `onnxruntime-osx-universal2-1.24.2.tgz`, which **404s** —
   1.24.2 ships macOS only as `onnxruntime-osx-arm64-1.24.2.tgz` (verified 200), no `universal2`/`x64`.
2. **The app crashes instead of degrading.** `import_audio` maps `TranscribeError` to a string
   (graceful), but a **native panic** in the transcription path (ONNX Runtime load, or a `symphonia`
   decode `unwrap`) unwinds past the synchronous `import_audio_file` command and takes the app down
   (crashed twice, manual relaunch). Panic profile is the default **unwind**, so a Rust panic is
   catchable.

## 3. Non-goals
- Not making `.wav` transcription *work* on Intel (x64) Macs — 1.24.2 has no macOS-x64 build; those
  degrade gracefully (AC1). arm64 (the tester's machine) is provisioned and works.
- No change to score (MusicXML/MIDI) import — it needs no ONNX.
- Not switching `ort` to `download-binaries` (the crate deliberately uses `load-dynamic`).

## 4. Contract / interface
- New app-boundary helper (commands.rs): `guard_transcription<T>(f) -> Result<T, String>` runs the
  native transcription behind `catch_unwind`; a panic becomes a fixed, friendly message, a normal
  `Err(String)` passes through. `import_audio` calls transcription through it.
- Kit `run.sh` `ensure_audio_engine` and `scripts/fetch-onnxruntime.sh` select the ONNX package by
  arch: `arm64 → onnxruntime-osx-arm64-<ver>`, `x86_64 → onnxruntime-osx-x64-<ver>` (best effort).

## 5. Acceptance criteria (numbered, testable)
1. `guard_transcription` returns the friendly error string (never re-panics) when its closure
   **panics**; passes an `Ok` value through unchanged; and passes an inner `Err(String)` through
   unchanged.
2. `import_audio` routes transcription through `guard_transcription` — a transcription panic yields
   `Err(_)` from the command, not a process crash.
3. Score (MusicXML/MIDI) import is unchanged (regression guard).
4. The kit + fetch script request an **arm64** macOS asset on arm64 (no `universal2`), i.e. the URL
   built for `uname -m == arm64` is `…/onnxruntime-osx-arm64-<ver>.tgz`.

## 6. Edge cases & failure modes
- ONNX dylib missing entirely → `Session::builder()` returns `Err` (already graceful; unchanged).
- ONNX dylib present but panics on load → `catch_unwind` converts it (AC1/AC2).
- A hard C-level `abort()` (not a Rust panic) is not catchable — mitigated by provisioning the correct
  runtime (AC4). Documented as a residual limitation.
- Intel Mac (no x64 build) → download fails → best-effort skip → import degrades calmly (AC1).

## 7. Test plan
| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `commands::tests::guard_transcription_converts_panic` | panic → friendly Err; Ok/Err passthrough |
| AC2 | `commands::tests::import_audio_uses_the_guard` (or wiring assertion) | import path returns Err, not panic |
| AC3 | existing MusicXML/MIDI import tests | unchanged |
| AC4 | manual: `curl -I` the built arm64 URL = 200 (checked); shell arch branch reviewed | correct asset |

## 8. Architecture / approach
Panic isolation lives at the **app boundary** (commands.rs), not in the `transcribe` library (which
correctly returns `Result`). `catch_unwind` + `AssertUnwindSafe` (the closure moves `bytes` and a
`&str`). The kit/fetch-script change is a pure arch-aware URL fix. No new deps, no network policy
change (transcription is on-device; ONNX is a local dylib).

## 9. Slice breakdown
Single slice: guard + wiring + tests (app), arch-aware fetch (kit + script).

## 10. Risks / open questions
- Residual: a native `abort()` from ONNX can't be caught; correct provisioning is the real defense.
- x64 macOS transcription remains unavailable until a compatible build exists — acceptable (degrades).

## 11. References
- #265 (VA test), `crates/transcribe/src/{lib,inference,error}.rs`, `apps/desktop/src-tauri/src/commands.rs`
  (`import_audio`, `import_audio_file`), `va-testing-kit/skills/test-app/scripts/run.sh`,
  `scripts/fetch-onnxruntime.sh`.
