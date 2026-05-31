#!/usr/bin/env bash
#
# Fetch the native ONNX Runtime shared library into the Tauri resource dir so
# `cargo tauri build` bundles it for audio transcription. Run before building a
# desktop installer.
#
# The version must match the C API that `ort` (=2.0.0-rc.10) targets — ONNX
# Runtime 1.24.x. Override with ORT_VERSION=... if needed.
set -euo pipefail

ORT_VERSION="${ORT_VERSION:-1.24.2}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${REPO_ROOT}/apps/desktop/src-tauri/resources/onnxruntime"
mkdir -p "$DEST"

uname_s="$(uname -s)"
uname_m="$(uname -m)"
case "$uname_s" in
  Linux)
    case "$uname_m" in
      x86_64)        pkg="onnxruntime-linux-x64-${ORT_VERSION}";      lib="libonnxruntime.so" ;;
      aarch64|arm64) pkg="onnxruntime-linux-aarch64-${ORT_VERSION}";  lib="libonnxruntime.so" ;;
      *) echo "unsupported arch: $uname_m" >&2; exit 1 ;;
    esac ;;
  Darwin)
    pkg="onnxruntime-osx-universal2-${ORT_VERSION}"; lib="libonnxruntime.dylib" ;;
  *)
    echo "Unsupported OS: $uname_s." >&2
    echo "On Windows, download onnxruntime-win-x64-${ORT_VERSION}.zip from the" >&2
    echo "ONNX Runtime GitHub releases and copy onnxruntime.dll into:" >&2
    echo "  $DEST" >&2
    exit 1 ;;
esac

url="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/${pkg}.tgz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url"
curl -fsSL -o "$tmp/ort.tgz" "$url"
tar xzf "$tmp/ort.tgz" -C "$tmp"

# Copy the versioned and unversioned library files (e.g. libonnxruntime.so,
# libonnxruntime.so.1, libonnxruntime.so.1.24.2) so the loader resolves cleanly.
cp "$tmp/${pkg}/lib/${lib}"* "$DEST/" 2>/dev/null || cp "$tmp/${pkg}/lib/${lib}" "$DEST/"
echo "Placed ${lib} (ONNX Runtime ${ORT_VERSION}) in $DEST"
