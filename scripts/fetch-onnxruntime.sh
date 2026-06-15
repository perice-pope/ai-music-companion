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
    esac
    is_zip=false
    ;;
  Darwin)
    pkg="onnxruntime-osx-universal2-${ORT_VERSION}"; lib="libonnxruntime.dylib"
    is_zip=false
    ;;
  MINGW*|MSYS*|CYGWIN*)
    pkg="onnxruntime-win-x64-${ORT_VERSION}"; lib="onnxruntime.dll"
    is_zip=true
    ;;
  *)
    echo "Unsupported OS: $uname_s." >&2
    exit 1 ;;
esac

if [ "$is_zip" = true ]; then
  archive_ext="zip"
  url="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/${pkg}.zip"
else
  archive_ext="tgz"
  url="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/${pkg}.tgz"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url"
curl -fsSL -o "$tmp/ort.$archive_ext" "$url"

if [ "$is_zip" = true ]; then
  unzip -q "$tmp/ort.$archive_ext" -d "$tmp"
else
  tar xzf "$tmp/ort.$archive_ext" -C "$tmp"
fi

# Copy the library file(s). For Unix, copy all versions (symlinks).
# For Windows, copy just the single .dll.
if [ "$is_zip" = true ]; then
  cp "$tmp/${pkg}/lib/${lib}" "$DEST/"
else
  # Unix: copy libonnxruntime.so, libonnxruntime.so.1, libonnxruntime.so.1.24.2, etc.
  cp "$tmp/${pkg}/lib/${lib}"* "$DEST/" 2>/dev/null || cp "$tmp/${pkg}/lib/${lib}" "$DEST/"
fi
echo "Placed ${lib} (ONNX Runtime ${ORT_VERSION}) in $DEST"
