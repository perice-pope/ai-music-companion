# Bundled OMR engine (oemer sidecar)

This directory is the **seam** where the on-device Optical Music Recognition
engine is dropped so `cargo tauri build` bundles it into the packaged desktop
app. At run time the app resolves it here and sets `OMR_ENGINE_PATH` (see
`src/runtime.rs`), which the `omr` crate's `SidecarOmrEngine` invokes to turn a
sheet-music **PDF → MusicXML** — fully offline, no Python/JRE in our address
space, no network.

The engine is a **frozen `oemer` build** (its ONNX models bundled inside the
self-contained executable). It is **not committed** — it is large and
platform-specific. Populate this directory before building an installer:

```bash
# from the repo root — fetches the engine for the host platform
OMR_ENGINE_URL=<artifact-url> ./scripts/fetch-omr-engine.sh
```

Expected file (per platform), placed directly in this directory:

| Platform | File |
|----------|------|
| Linux / macOS | `amc-omr-engine` |
| Windows | `amc-omr-engine.exe` |

**Sidecar contract:** the binary reads PDF bytes on **stdin** and writes
MusicXML to **stdout**; a non-zero exit means failure, with the reason on
stderr. This is the contract `crates/omr/src/sidecar.rs` depends on.

> Status (Phase 1): this is the **sidecar spike** committed in
> `docs/architecture/score-import-and-transcription.md` — an end-to-end
> "PDF → notes" path behind the `AMC_ENABLE_PDF_OMR` beta flag. The frozen-oemer
> artifact is produced by the (separate) engine-packaging pipeline; until that
> artifact is wired into CI, this directory is empty in dev builds and PDF
> import reports it's unavailable rather than pretending. A later phase decides
> whether to replace the sidecar with an in-process Rust port of oemer's
> post-processing (the engine-integration fork in the design doc).
