# Spec: Local stem separation — upload a full mix, pick the part you'll practice (#328)

> Founder decision 2026-07-08: **local Demucs-class separation, fully on-device.**
> No cloud path in v1. See decisions-log entry of the same date.

## 1. Summary
When an imported audio file is a full mix, the app separates it into stems on-device
(Demucs-class quality), asks the player which part they want to work on, and transcribes
that stem — instead of silently producing a garbage whole-mix transcription.

## 2. Problem / why
VA test #324: a clean 8-note sample transcribed 4/8 notes with no warning; a real song
would be noise presented as sheet music. The silent fail is the product bug (founder,
#324 review). Today `import_audio` transcribes whatever it's given as if it were a
monophonic line.

## 3. Non-goals
- No cloud separation in v1 (founder decision — local only; a cloud tier can be a later,
  separately-consented story under offline-first rules).
- No per-instrument fine separation beyond the model's native stems
  (vocals / drums / bass / other for htdemucs).
- No change to MusicXML/MIDI score import.
- Not a mixing/DAW feature: stems exist to be *practiced against*, one at a time.

## 4. Contract / interface
- New crate `crates/stems`: `separate(path, model) -> Result<Vec<Stem>, StemError>`,
  `Stem { kind: StemKind, wav_path: PathBuf }`. ONNX inference via `ort` `load-dynamic`,
  exactly like `crates/transcribe` (same vendored ONNX Runtime — no new native dep class).
- Model file: htdemucs exported to ONNX. **Open technical risk:** htdemucs's
  transformer + iSTFT export is known-painful. Slice 2 begins with a spike that
  benchmarks (a) a community htdemucs ONNX export against (b) Open-Unmix (umxl — clean
  ONNX story) on 3 real songs; quality bar is "the chosen stem is practice-usable", not
  archival quality. If htdemucs-ONNX fails in `ort`, Open-Unmix ships v1 and the
  decision-log entry gets amended — the *decision* (local, on-device) stands either way.
- Model weights are large (100–300 MB class): NOT bundled in the installer. Downloaded
  on first use behind an explicit prompt (disclosed outbound call → network allowlist +
  offline-first table + ConnectionsPrivacy row: "downloads the separation model from
  GitHub releases, once"). Cached in the app data dir; everything after the download is
  offline.
- IPC: `import_audio` gains a polyphony verdict; new commands
  `separate_stems(path) -> [StemDto]`, `import_stem(stem_id)`. UI: stem picker sheet
  with per-stem preview play.

## 5. Acceptance criteria (numbered, testable)
1. **The silent fail dies first (slice 1, no separator needed):** importing a
   polyphonic file surfaces "this sounds like a full mix — the notes may be
   approximate" on the result, and the transcription result carries
   `caught N notes, M uncertain` honesty counts. A clean monophonic file shows no
   warning. (Detection heuristic: existing transcription confidence + simultaneous-note
   density; threshold pinned by tests on fixture audio.)
2. With the model present, `separate(mix.wav)` produces the model's stems as playable
   WAVs; a synthetic two-source fixture (sine melody + noise bed) separates such that
   the melody stem transcribes ≥ the whole-mix note count.
3. The stem picker lists stems with preview; choosing one runs the existing
   transcription path on that stem alone and the score header names it
   ("Vocals — from song.wav").
4. First-use model download is prompted, disclosed, resumable-or-restartable, and
   verified by checksum; declining leaves slice-1 behavior (honest whole-mix note).
   Airplane-mode honors `NetworkPolicy` — no call fires.
5. Separation failure/timeout degrades to slice-1 behavior with a calm message — never
   a crash, never a silent bad score (ties to #267's `guard_transcription`).
6. `scripts/check_network_disclosure.sh` stays green (new call site enumerated).

## 6. Edge cases & failure modes
Already-monophonic input (skip the picker, no warning) · very long files (cap +
progress, off the main thread per #323) · model file corrupted (checksum → re-download
prompt) · disk-full during download · cancel mid-separation · unsupported sample rates
(resample first, as transcribe does).

## 7. Test plan
Unit: polyphony heuristic thresholds on fixtures (clean mono, dense mix, borderline).
Integration: synthetic two-source separation (AC2) behind a feature gate that skips
when the model file is absent (CI runs without the 300 MB weight; a nightly/manual job
can run with it). Frontend: picker rendering, decline path, honesty note. Manual:
one real song end-to-end on the founder's machine before enabling for the VA.

## 8. Slices
1. **Polyphony honesty gate** (#331, auto-fixable): detection + honest copy + counts.
2. **Spike + engine choice**: htdemucs-ONNX vs Open-Unmix bench in `ort`; commit the
   verdict to the decisions log.
3. **`crates/stems` + stem picker UI** behind a feature flag, model download flow.
4. **Polish**: progress, cancellation, VA playbook step.
