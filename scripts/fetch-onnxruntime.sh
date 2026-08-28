#!/usr/bin/env bash
#
# Fetch the native ONNX Runtime shared library into the Tauri resource dir so
# `cargo tauri build` bundles it for audio transcription. Run before building a
# desktop installer. (#383: installers shipped WITHOUT the runtime — the
# resource dir was never populated in CI — so .wav import and every
# poly-engine feature silently died in packaged builds while CI stayed green.)
#
# Layout:
#   resources/onnxruntime/<lib>              — single-arch platforms
#   resources/onnxruntime/<arch>/<lib>       — macOS universal (both arches;
#                                              pass --macos-universal)
#
# Versions: `ort` (=2.0.0-rc.10) targets the ONNX Runtime 1.24.x C API, but
# upstream ships no osx-x64 build for 1.24.x — Intel macOS uses 1.22.0, which
# the same crate loads cleanly (proven by the local x86_64 dev setup).
set -euo pipefail

# Accumulate temp dirs and clean them even when `set -e` aborts mid-fetch
# (a function-local RETURN trap never fires on an -e exit — review r1 on
# PR #395 confirmed the leak empirically).
TMP_DIRS=()
cleanup_tmp() { for d in "${TMP_DIRS[@]:-}"; do [ -n "$d" ] && rm -rf "$d"; done; }
trap cleanup_tmp EXIT

ORT_VERSION="${ORT_VERSION:-1.24.2}"
ORT_VERSION_MACOS_X64="${ORT_VERSION_MACOS_X64:-1.22.0}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${REPO_ROOT}/apps/desktop/src-tauri/resources/onnxruntime"
mkdir -p "$DEST"

# Download $1 (package name, no extension) and copy its shared lib ($2) into
# dir $3. Handles both .tgz (unix) and .zip (windows) upstream archive shapes.
fetch_into() {
  local pkg="$1" lib="$2" dest="$3" ext url tmp
  case "$pkg" in
    *win*) ext="zip" ;;
    *) ext="tgz" ;;
  esac
  url="https://github.com/microsoft/onnxruntime/releases/download/v${pkg##*-}/${pkg}.${ext}"
  tmp="$(mktemp -d)"
  TMP_DIRS+=("$tmp")
  echo "fetching ${pkg}.${ext} → ${dest}/${lib}"
  # --retry-all-errors: curl's default retry set excludes connection resets
  # (exit 35), the flake that turns CI red on a single dropped transfer.
  curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors -o "$tmp/ort.$ext" "$url"
  if [ "$ext" = "zip" ]; then
    python3 -m zipfile -e "$tmp/ort.$ext" "$tmp/" 2>/dev/null \
      || python -m zipfile -e "$tmp/ort.$ext" "$tmp/"
  else
    tar xzf "$tmp/ort.$ext" -C "$tmp"
  fi
  mkdir -p "$dest"
  # The lib lives under <pkg>/lib/; versioned .so/.dylib names get symlinked
  # upstream — copy the real file under the canonical name the app resolves.
  local found stem ext
  stem="${lib%%.*}"       # libonnxruntime / onnxruntime
  ext="${lib##*.}"        # dylib / so / dll
  # Upstream naming varies: exact (onnxruntime.dll), trailing-versioned
  # (libonnxruntime.so.1.24.2), or middle-versioned
  # (libonnxruntime.1.22.0.dylib). Match all three; the underscore in
  # provider libs (libonnxruntime_providers_*) matches none of them.
  found="$(find "$tmp/$pkg/lib" -type f \( -name "$lib" -o -name "${lib}.*" -o -name "${stem}.*.${ext}" \) | sort | head -1)"
  [ -n "$found" ] || { echo "no ${lib} in ${pkg}" >&2; exit 1; }
  cp "$found" "$dest/$lib"
  rm -rf "$tmp"
}

if [ "${1:-}" = "--macos-universal" ]; then
  # Both arches for a universal .app: runtime.rs picks by std::env::consts::ARCH.
  fetch_into "onnxruntime-osx-arm64-${ORT_VERSION}" "libonnxruntime.dylib" "$DEST/aarch64"
  fetch_into "onnxruntime-osx-x86_64-${ORT_VERSION_MACOS_X64}" "libonnxruntime.dylib" "$DEST/x86_64"
  exit 0
fi

uname_s="$(uname -s)"
uname_m="$(uname -m)"
case "$uname_s" in
  Linux)
    case "$uname_m" in
      x86_64)        fetch_into "onnxruntime-linux-x64-${ORT_VERSION}" "libonnxruntime.so" "$DEST" ;;
      aarch64|arm64) fetch_into "onnxruntime-linux-aarch64-${ORT_VERSION}" "libonnxruntime.so" "$DEST" ;;
      *) echo "unsupported arch: $uname_m" >&2; exit 1 ;;
    esac ;;
  Darwin)
    # Single-arch dev fetch; installer builds use --macos-universal.
    case "$uname_m" in
      arm64)  fetch_into "onnxruntime-osx-arm64-${ORT_VERSION}" "libonnxruntime.dylib" "$DEST" ;;
      x86_64) fetch_into "onnxruntime-osx-x86_64-${ORT_VERSION_MACOS_X64}" "libonnxruntime.dylib" "$DEST" ;;
      *) echo "unsupported macOS arch: $uname_m" >&2; exit 1 ;;
    esac ;;
  MINGW*|MSYS*|CYGWIN*)
    fetch_into "onnxruntime-win-x64-${ORT_VERSION}" "onnxruntime.dll" "$DEST" ;;
  *)
    echo "Unsupported OS: $uname_s." >&2
    exit 1 ;;
esac
