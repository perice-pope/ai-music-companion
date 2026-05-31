# Bundled ONNX Runtime (native library)

This directory is the **seam** where the native ONNX Runtime shared library is
dropped so `cargo tauri build` bundles it into the packaged desktop app. At run
time the app resolves it here and sets `ORT_DYLIB_PATH` (see
`src/runtime.rs`), which the `transcribe` crate's `ort` `load-dynamic` backend
reads to power audio→MIDI transcription.

The library is **not committed** (it is large and platform-specific). Populate
this directory before building an installer:

```bash
# from the repo root — downloads the runtime for the host platform
./scripts/fetch-onnxruntime.sh
```

Expected file (per platform), placed directly in this directory:

| Platform | File |
|----------|------|
| Linux    | `libonnxruntime.so` |
| macOS    | `libonnxruntime.dylib` |
| Windows  | `onnxruntime.dll` |

The version must match the C API that `ort` (`=2.0.0-rc.10`) targets — ONNX
Runtime **1.24.x**.

> Status: the desktop installer pipeline (`cargo tauri build`) does not exist
> yet, so nothing populates this directory in CI today. In dev builds the app
> falls back to a developer-set `ORT_DYLIB_PATH` or the system loader.
