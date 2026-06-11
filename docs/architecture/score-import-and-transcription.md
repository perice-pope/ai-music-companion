# Score Import & Audio Transcription — design decision

**Status:** Decision (Phase: import UX)
**Date:** 2026-06-11
**Companion docs:** [`architecture-v2.md`](./architecture-v2.md) §9 (MusicXML is the
canonical internal format), [`offline-first-and-network-transparency.md`](./offline-first-and-network-transparency.md)

> **TL;DR** — A musician shows up with one of three things: a **PDF of their
> piece**, a **recording**, or (rarely) a notation file. This doc commits to
> making all three land on screen as practiceable sheet music, and to always
> asking **"which part do you want to read and practice?"** Every step stays
> **on-device** by riding the ONNX Runtime we already vendor for transcription —
> **no JRE, no Python, no upload**.

---

## The problem we're fixing

The import UX was confusing, and the confusion was real, not cosmetic:

1. **PDF — the format everyone actually has — wasn't accepted at all.** "Just
   upload the PDF and play" is the natural flow, and it didn't exist.
2. **MusicXML was accepted by the picker but not wired** ("MusicXML import isn't
   wired up yet"), even though it's our canonical internal format.
3. **Audio import was ambiguous.** Drop an `.mp3` of a full song and it ran
   monophonic pitch detection over the *whole mix* — no way to say "transcribe
   the **bass**, not the vocal." The result on a real recording was an
   unreliable single-line blob, and nothing told the user which part they'd get.

The fix is one coherent idea: **get real note data on screen, and always let the
user choose which line is theirs.**

---

## What shipped now

- **MusicXML import is wired** end-to-end (`.musicxml` / `.xml`). Parsing stays
  in Rust (metadata + the selected part); the original file is stored unchanged
  so the follower re-selects the same part by index.
- **Part selection** ("which part do you want to read and practice?") — a
  multi-part score shows a picker; a single-part score imports straight away.
  This reuses the `part_index` seam already threaded through
  `ScoreStore` → `ScoreFollower::from_musicxml_str(xml, part_index)`.
- `.mxl` (zipped MusicXML) gets a clear "re-export uncompressed" hint rather than
  a cryptic decode error (deferring a zip dependency on purpose).

That closes (1)-as-much-as-notation-files-allow and (2). The rest of this doc is
the committed plan for (1) PDF and (3) audio-part-selection.

---

## Decision 1 — PDF → sheet music via on-device OMR

**Commit to PDF→OMR as the real "load your sheet music" feature.** OMR (Optical
Music Recognition) turns a page image into note data → MusicXML → our existing
render + follow path. A PDF is rasterized to page images first, then run through
OMR.

### Tool choice: **oemer** (ONNX), not Audiveris

| | **oemer** | Audiveris |
|---|---|---|
| Output | image → MusicXML (end-to-end) | PDF/image → MusicXML 4.0 |
| Runtime | **ONNX models** — runs through the `ort` crate we already ship | **Java** — would bundle a JRE + Tesseract (heavy) |
| License | permissive (verify at integration) | **AGPL — a blocker for a shipped product** |
| Fit | reuses our transcription infra; no new language runtime | new toolchain, large bundle |

The deciding factor is architectural: the `transcribe` crate already loads
ONNX Runtime via `ort` (`load-dynamic`, `ORT_DYLIB_PATH`). oemer's models are
ONNX, so OMR can run **in-process in Rust** with the runtime we already vendor —
no JRE, fully offline. Audiveris is more battle-tested but its AGPL license and
JVM dependency make it the wrong call for a bundled desktop product.

**Pipeline:** `PDF → rasterize pages → oemer (ONNX) → MusicXML → store → render`,
then the **same part picker** as MusicXML import.

**Honesty contract (per offline-first + "silence > lies"):** OMR is approximate.
Surface a calm "this was read from a scan — check it looks right" note, exactly
like the audio quality banner. Never present a guessed score as ground truth.

**Model delivery (decided, 2026-06-11):** the OMR (and later stem-separation)
models ship **bundled at build/package time**, fetched by CI the same way the
ONNX Runtime is today (`scripts/fetch-onnxruntime.sh`, gitignored — never
committed to keep the repo lean). This means **zero network at runtime** for the
user and nothing new to disclose in `ConnectionsPrivacy.tsx` or the
network-transparency doc. The rejected alternative — download-on-first-use —
would be a runtime outbound call that breaks offline-by-default. Trade-off
accepted: a larger installer (~100–300 MB) in exchange for a feature that works
on a plane.

**Open items before building:** confirm oemer model license; choose the PDF
rasterizer (a Rust `pdfium`/`pdf` binding vs. a small bundled helper);
**resolve the engine-integration fork** — port oemer's post-processing to Rust
(faithful to the ONNX-in-`ort` story, but a large port) vs. bundle a frozen
oemer as a Tauri **sidecar** binary (far less code, still offline since it's
bundled, but ships a Python-derived executable). Phase 1 will spike the sidecar
path first to get an end-to-end "PDF → notes" demo behind the beta flag, then
decide whether the Rust port is worth it.

---

## Decision 2 — Audio: separate stems, then ask which part

**basic-pitch stays** as the note-detector — it's Apache-2.0, ONNX, already
integrated, and *good at exactly one thing*: transcribing a **single melodic
line**. The fix for "which part?" is not a different transcriber; it's a
**source-separation stage in front of it**.

**Tool choice: Demucs (htdemucs v4) via ONNX.** Demucs splits a mix into
`vocals / bass / drums / other` and is state-of-the-art on MUSDB18. An ONNX
export runs through our existing `ort` runtime — same offline, no-Python story
as OMR.

**Pipeline:**
`audio → Demucs stem-split → user picks a stem (vocals / bass / other) → basic-pitch on that stem → MIDI → MusicXML → part picker (here, "part" = the chosen stem)`

- The **stem picker is the audio answer to "which part?"** — the same UX promise
  as the MusicXML part picker, one layer earlier.
- **Drums are offered but excluded from transcription** (unpitched) — or routed
  to a future rhythm view, not the pitch follower.
- Re-scope the current full-mix path honestly in the meantime: today's
  monophonic transcription is labelled for **a single instrument line**, and the
  existing polyphonic-quality banner already warns when a mix is dropped in.

**Open items:** Demucs ONNX export + model size (these models are large —
measure against the bundle budget; consider an opt-in download with disclosure,
which would be a *networked* fetch and must appear in
`offline-first-and-network-transparency.md` and `ConnectionsPrivacy.tsx`);
separation latency is fine off the realtime path (import-time, with progress).

---

## Why this is one decision, not three

All paths converge on the same two invariants:

1. **MusicXML is the only thing the renderer and follower ever see** — OMR and
   transcription are just different front-ends that produce it.
2. **The user always chooses their line** — a `part_index` for notation, a
   **stem** for audio. The picker UI is shared in spirit and the
   `from_musicxml_str(xml, part_index)` seam is shared in code.

And every front-end runs on the **ONNX Runtime we already ship**, so "load your
sheet music and play" never requires the internet, a JVM, or a Python runtime.

---

## Phasing

| Phase | Deliverable |
|---|---|
| **0 (done)** | MusicXML import + part picker; `.mxl` guidance; honest audio quality banner |
| **1** | PDF→OMR via oemer (ONNX): rasterize → MusicXML → existing part picker + "read from a scan" note |
| **2** | Audio stem separation (Demucs ONNX) + stem picker in front of basic-pitch |
| **later** | `.mxl` (zip) support; drum/rhythm view; OMR correction UI |

## Sources

- [oemer — end-to-end OMR (image → MusicXML)](https://github.com/BreezeWhite/oemer)
- [Audiveris OMR engine](https://github.com/Audiveris/audiveris)
- [Demucs — music source separation](https://github.com/facebookresearch/demucs)
- [Best OMR software overview, 2025](https://melogenai.com/tw/blogs/best-omr-software)
</content>
</invoke>
