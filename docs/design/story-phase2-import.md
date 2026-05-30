# Story — Phase 2 Smart Import (MIDI + audio → MusicXML): Design Proposal

**Status:** Draft — pending founder review
**Author:** Design proposal generated for review
**Target story:** *No GitHub issue yet — create one before kicking off the import work. Suggested title: "Phase 2: Smart Import — MIDI + audio file import." Suggested labels: `story`, `phase-2`.*

**Dependencies landed:**
- Score Mode (story-score-mode) — score library backend (`import_score`, `get_score`, `list_scores`, `delete_score`), `ScoreStore`, OSMD rendering, cursor follow. Every import path terminates here.
- Score parsing (`crates/brain/src/score/`) — `parse_musicxml_str(_part)`, `list_parts`, **MIDI parser** (`midi::parse_midi_bytes`).
- **MusicXML emitter** (`crates/brain/src/score/emit.rs`, `score_model_to_musicxml`) — PR #121. The canonical "always store as MusicXML" serialiser that every import path feeds.
- Bundled-resource resolution (#120) — the pattern for shipping non-code assets (profiles today; an ONNX model tomorrow).

---

## 1. Product framing

### What Smart Import is (user's POV)

Today a user can only practise with a score if they already have a `.musicxml` / `.xml` / `.mid` file *and* the app exposes a way to load it. Smart Import widens the front door: a musician drops in **a MIDI file** or **an audio recording** of a piece, the app converts it into a score, and it lands in the same library Score Mode already reads from. From there it behaves exactly like any other score — render, cursor-follow, measure-aware coaching.

The two sources in scope:

1. **MIDI file (`.mid`, `.midi`)** — already parseable; what's missing is a user-facing *import* path that converts it to stored MusicXML. Cheap, deterministic, no ML. Ships first.
2. **Audio recording (`.wav`, `.mp3`, `.m4a`, `.flac`)** — the headline feature. The musician records (or already has) a monophonic recording of the line they want to practise; we run **basic-pitch** (Spotify's audio-to-MIDI model) to transcribe it, then reuse the MIDI → `ScoreModel` → MusicXML path. This is the "drop a recording and practise it" moment.

Both flow into the existing score library. Import is asynchronous: the user picks a file, sees a progress indicator, and the score appears when ready. Failures surface calmly with a suggestion ("This recording sounds polyphonic — basic-pitch works best on a single instrument line").

### Why now

- **Phase 1 is closed.** Score Mode is shipped and the import *terminus* (library + emitter) exists. Smart Import is the natural next layer.
- **It is mostly wiring around tools that already exist.** The MIDI parser and MusicXML emitter are done; basic-pitch is a mature, offline, Apache-2.0 model. The unbuilt slice is the import command surface, the basic-pitch sidecar, and a small import UI.
- **It establishes the ML-sidecar pattern** the rest of Phase 2 (the tone-quality model) will reuse — how we ship an ONNX model as a bundled resource and run inference off the real-time path.

### Three UX decisions that reinforce "coach, don't judge"

1. **Import quality is surfaced, never hidden.** Audio transcription is approximate. If basic-pitch returns low-confidence or obviously-polyphonic output, we say so plainly and let the user proceed or re-record — we never pretend a rough transcription is gospel.
2. **Imported scores are editable-by-reimport, not locked.** A bad import is a dropped file away from being replaced. We don't build a notation editor (out of scope forever) — but re-importing is always one click.
3. **Nothing blocks Free Play.** If an import fails entirely, the user is one click from practising without a score, exactly as in Score Mode.

### On YouTube import — recommend **cut from Phase 2** (revisit later, behind a flag, with counsel)

The architecture doc (§3) lists YouTube import (yt-dlp → basic-pitch → MusicXML) as a Phase 2 candidate. **I recommend we do not build it in this story.** Reasons:

- **Legal / ToS risk.** Extracting audio from YouTube violates YouTube's Terms of Service. "Personal practice use only" is a real defense for an *individual*, but shipping a feature whose primary purpose is downloading YouTube audio puts that liability on us as the distributor. This is a "consult counsel before building" item, not an engineering one.
- **Quality mismatch.** YouTube audio is almost always polyphonic (full mix, accompaniment, multiple instruments). basic-pitch's sweet spot is *monophonic* single-instrument input. The output on a typical YouTube track would be poor — the feature would mostly disappoint.
- **It's strictly downstream of audio import.** YouTube import = "yt-dlp fetches audio" + (the audio import path this story builds). If we ever do it, it's a thin front-end on top of work this story already delivers. Nothing is lost by deferring.

If the founder still wants it, the cleanest later shape is an **explicit, opt-in** "paste a link" affordance with a clear notice, gated behind the audio importer landing first — and a legal sign-off. Flagged as **Open Question 1** below.

Things we are rejecting in this story (same spirit as Score Mode):
- Polyphonic / multi-instrument transcription (basic-pitch is monophonic-first; we state the limitation).
- OMR (photo/PDF of sheet music via Audiveris) — separate, heavier story (JVM sidecar, AGPL). Phase 2 but its own design doc.
- In-app notation editing.
- A "transcription accuracy %" score.

---

## 2. The import pipeline (shared shape)

Every source normalises to the same spine:

```
source bytes ──► ScoreModel ──► score_model_to_musicxml ──► ScoreStore.import ──► library
                    ▲
   ┌────────────────┼─────────────────┐
   │                │                  │
 .mid          .wav/.mp3/…        (.xml direct — already supported)
 midi::parse   basic-pitch (sidecar)
 _midi_bytes   → MIDI bytes → midi::parse_midi_bytes
```

The key insight: **audio import is just "produce MIDI bytes, then run the MIDI path."** basic-pitch emits a standard MIDI file; once we have those bytes we reuse `midi::parse_midi_bytes` → `ScoreModel` → emitter, exactly like a dropped `.mid`. So the MIDI import path (PR 1) is a hard dependency of audio import (PR 2), and audio adds *only* the transcription front-end.

---

## 3. Backend wiring (Rust side)

### PR 1 — MIDI file import

`crates/brain` already has `midi::parse_midi_bytes(&[u8]) -> Result<ScoreModel, ScoreError>` and now the emitter. The remaining work is a Tauri command.

```rust
/// Import a MIDI file: parse → ScoreModel → MusicXML → library.
/// Returns the new library entry. `bytes` is the raw file content read
/// on the frontend (Tauri fs scope) or via a path the command opens.
#[tauri::command]
async fn import_midi_file(
    state: State<'_, AppState>,
    source_filename: String,
    bytes: Vec<u8>,
) -> Result<ScoreLibraryEntryDto, String>;
```

Internally: `parse_midi_bytes` → `score_model_to_musicxml` → `ScoreStore.import(title, composer, source_filename, music_xml, part_index=0, duration_measures)`. Title falls back to the filename stem when the MIDI has no track name. This is pure Rust, fully testable, no sidecar.

### PR 2 — Audio file import (basic-pitch sidecar)

**Tool:** [basic-pitch](https://github.com/spotify/basic-pitch) (Spotify, Apache-2.0). It ships as (a) a Python package and (b) a published **ONNX model** (`nmp.onnx`, ~17 MB). Two integration options:

| Option | How | Trade-off |
|---|---|---|
| **A. ONNX in-process** (recommended) | Bundle `nmp.onnx` as a Tauri resource (the #120 pattern); run inference in Rust via the `ort` crate (ONNX Runtime). Audio decode via `symphonia` (MIT, pure-Rust) → resample to 22.05 kHz mono → model → note events → MIDI. | No Python runtime to ship. One new heavy-ish dep (`ort`) + the model file. The note-event post-processing (basic-pitch's "note creation" step) must be ported to Rust — non-trivial but bounded. |
| **B. Python sidecar** | Ship basic-pitch as a bundled Python (PyInstaller) sidecar invoked via `tauri-plugin-shell`. | Reuses basic-pitch's exact post-processing. But shipping a Python runtime per-platform is a packaging headache and bloats the installer ~100 MB+. |

**Recommendation: Option A (ONNX + `ort`)**, because it keeps the app a single self-contained binary + a 17 MB model resource, matches how the tone model (also Phase 2) will ship, and avoids a per-platform Python bundle. The cost is porting basic-pitch's frame→note post-processing; we scope that explicitly as the bulk of PR 2.

Command surface:

```rust
/// Import an audio recording: decode → basic-pitch → MIDI → ScoreModel
/// → MusicXML → library. Long-running; emits `import-progress` events.
#[tauri::command]
async fn import_audio_file(
    state: State<'_, AppState>,
    source_filename: String,
    bytes: Vec<u8>,
) -> Result<ScoreLibraryEntryDto, String>;
```

New crate boundary: put transcription in a focused module — `crates/ears/src/transcribe.rs` (it's audio-domain, like pitch detection) or a new `crates/transcribe`. Decision in **Open Question 2**. It exposes `audio_to_midi(samples, sample_rate) -> Vec<u8>` (MIDI bytes), which the Tauri command feeds to the existing MIDI path.

**Progress + cancellation:** transcription of a 3-minute file is seconds, not milliseconds. The command runs on a worker (not the audio thread) and emits an `import-progress` event (`{ stage: "decoding" | "transcribing" | "converting", pct }`) so the UI shows a live indicator.

**Quality signalling:** basic-pitch returns per-note confidence. We compute a simple aggregate (mean confidence, polyphony heuristic = fraction of overlapping note-ons) and return it on the DTO so the UI can warn ("this sounds polyphonic / low-confidence") without inventing a fake accuracy score.

### Errors

All import errors are calm and actionable, surfaced via the command `Result::Err`:
- Unsupported/corrupt file → "We couldn't read this file."
- Empty transcription (silence / no detectable pitch) → "We didn't hear a clear melody — try a closer, single-instrument recording."
- Polyphonic warning is a *success with a note*, not an error.

---

## 4. Frontend architecture

Score Mode already has `ScorePicker` / `ScoreDropZone` / `ScoreLibrary`. Smart Import extends the drop zone, it does not add a screen.

- **`ScoreDropZone`** accepts the new extensions (`.mid`, `.midi`, `.wav`, `.mp3`, `.m4a`, `.flac`) and routes by type: MusicXML → existing `import_score`; MIDI → `import_midi_file`; audio → `import_audio_file`.
- **Import progress**: a small inline progress state on the drop zone, driven by the `import-progress` event (audio only). MIDI/MusicXML are instant.
- **Quality banner**: after an audio import, if the confidence/polyphony heuristic trips, show a dismissible "this transcription may be approximate" note above the freshly-added library card.
- **Store**: `practiceStore` gains `importAudioFromFile` / `importMidiFromFile` actions mirroring the existing `loadScoreFromFile`; library refresh is the existing path.

No business logic moves to the frontend — type-routing + progress display only. Transcription lives in Rust.

---

## 5. Testing strategy

### Rust
| Test | Covers |
|---|---|
| `import_midi_file` happy path | A fixture `.mid` → library row inserted, stored `music_xml` re-parses, measures > 0. |
| `import_midi_file` titles from filename when MIDI unnamed | Title fallback. |
| `import_midi_file` rejects corrupt bytes | Clear error, nothing persisted. |
| `transcribe::audio_to_midi` on a synthesised sine sweep | Known monophonic input (e.g. a generated C-major scale of sine tones) → MIDI whose note numbers match within tolerance. Deterministic fixture, no network. |
| `transcribe` empty/silence input | Returns empty/error cleanly, not a panic. |
| `import_audio_file` end-to-end on a tiny fixture wav | Decode → transcribe → MIDI → MusicXML → library row. |
| emitter round-trip (done, PR #121) | Already covers MIDI→model→xml fidelity. |

The synthesised-sine fixture is the crux: it lets us test transcription deterministically in CI without shipping copyrighted audio or a flaky real recording.

### Frontend (Vitest)
| Test | Covers |
|---|---|
| `ScoreDropZone` routes `.mid` → `import_midi_file` | Mock invoke, assert correct command. |
| `ScoreDropZone` routes `.wav` → `import_audio_file` | Same. |
| `ScoreDropZone` shows progress on `import-progress` | Synthetic events advance the indicator. |
| quality banner renders when DTO flags low confidence | Banner visibility. |
| unsupported extension → calm error, no invoke | Guard. |

### CI footprint
`ort` + the ONNX model add build weight to the **Tauri Rust checks** job. The model is a bundled resource (not compiled), and `ort` can use a downloaded/bundled ONNX Runtime — confirm the CI image can build it (**Open Question 3**). The `transcribe` unit tests must run without the full Tauri stack (keep transcription in a workspace crate so the cheap `Rust checks` job runs them).

---

## 6. Cut lines — NOT in this story

- **YouTube / URL import** — deferred (see §1; legal + quality + strictly-downstream).
- **OMR (photo/PDF → Audiveris)** — separate Phase 2 story (JVM sidecar, AGPL-3.0).
- **Polyphonic transcription / multi-track** — single melodic line only.
- **Tone-quality model** — separate Phase 2 story (reuses the ONNX-sidecar pattern this story establishes).
- **Demucs backing-track separation** — Phase 2, separate.
- **In-app notation editing.**
- **Tempo/quantization editing of imports** — we take basic-pitch's note grid as-is for v1; a "clean up rhythm" pass is a future enhancement.

---

## 7. PR slicing

Target: each PR <600 lines ideally, testable and mergeable alone.

### PR 0 — MusicXML emitter ✅ (PR #121, in review)
The serialiser every path feeds. Already covered by round-trip tests.

### PR 1 — MIDI file import (~300 lines)
- `import_midi_file` Tauri command (parse → emitter → store).
- `ScoreDropZone` accepts `.mid`/`.midi` and routes to it.
- Tests: command happy/corrupt/title-fallback; drop-zone routing.
- **Merge criterion:** drop a `.mid`, it appears in the library and renders in Score Mode. Pure Rust, no new deps — lowest risk, ships the import UX skeleton.

### PR 2 — Audio transcription core (~500 lines)
- `transcribe` module: `symphonia` decode + resample, `ort` + bundled `nmp.onnx`, ported note-creation post-processing → MIDI bytes.
- Bundle the model as a Tauri resource (reuse #120 resolution).
- Tests: sine-sweep → expected MIDI; silence handling. **No Tauri command yet** — pure transcription, unit-tested in isolation.
- **Merge criterion:** `audio_to_midi` transcribes the synthesised fixture within tolerance; CI builds `ort`.

### PR 3 — Audio import wiring + progress UI (~400 lines)
- `import_audio_file` command (decode → `audio_to_midi` → MIDI path → store), `import-progress` events, quality heuristic on the DTO.
- `ScoreDropZone` accepts audio extensions + progress indicator + quality banner.
- Tests: end-to-end command on a tiny wav; drop-zone routing; progress + banner UI.
- **Merge criterion:** drop a monophonic recording → score in the library; polyphonic input warns calmly.

---

## 8. Open questions for the founder

**Decisions made (founder, 2026-05-30):**

1. ~~**YouTube import — confirm cut?**~~ ✅ **CUT from Phase 2.** Not built in this story; revisit later only with legal sign-off and as an explicit opt-in.
3. ~~**basic-pitch integration — ONNX vs Python sidecar?**~~ ✅ **ONNX-in-process** (Option A). Single self-contained binary + bundled model; we take on the `ort` dependency and port basic-pitch's note-creation post-processing to Rust.
4. ~~**Model distribution — bundle vs fetch?**~~ ✅ **Bundle** `nmp.onnx` (~17 MB) as a Tauri resource (the #120 pattern), keeping the core loop fully offline.

**Still open (do not block PR 1):**

2. **Transcription crate placement** — `crates/ears/src/transcribe.rs` (audio-domain, near pitch detection) vs a dedicated `crates/transcribe`. I lean a dedicated crate so the heavy `ort` dependency doesn't bloat `ears`' real-time build. *Needed before PR 2.*
5. **Audio formats** — `.wav` + `.mp3` cover ~all cases via `symphonia`. Worth adding `.m4a`/`.flac` (symphonia supports them) or keep the surface small for v1? *Needed before PR 3.*

---

**End of design doc.**
