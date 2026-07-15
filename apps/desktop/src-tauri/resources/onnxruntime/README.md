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

Expected layout (per platform):

| Platform | File |
|----------|------|
| Linux    | `libonnxruntime.so` |
| macOS (universal installer) | `aarch64/libonnxruntime.dylib` **and** `x86_64/libonnxruntime.dylib` (fetch with `--macos-universal`) |
| macOS (single-arch dev)     | `libonnxruntime.dylib` |
| Windows  | `onnxruntime.dll` |

`src/runtime.rs` resolves `<arch>/<lib>` first (universal layout), then the
flat file — a foreign arch's dylib never satisfies resolution.

Versions: `ort` (`=2.0.0-rc.10`) targets the ONNX Runtime **1.24.x** C API,
but upstream ships no osx-x64 build for 1.24.x — Intel macOS rides **1.22.0**
(`ORT_VERSION_MACOS_X64` in the script; re-verify with the real-inference
integration tests if you bump either version).

> Status (#383, PR #395): the release workflow populates this directory on
> every platform before `tauri build`, and a red pre-build assert fails the
> installer job if the expected library is missing. In dev builds the app
> falls back to a developer-set `ORT_DYLIB_PATH` or the system loader.
