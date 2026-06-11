#!/usr/bin/env bash
#
# Fetch the on-device OMR (oemer) sidecar engine into the Tauri resource dir so
# `cargo tauri build` bundles it for PDF → sheet-music import. Run before
# building a desktop installer.
#
# Unlike the ONNX Runtime (an official Microsoft release with a stable URL — see
# fetch-onnxruntime.sh), the OMR engine is a *frozen oemer build* produced by
# our own engine-packaging pipeline: a self-contained executable with its ONNX
# models embedded. There is therefore no canonical public URL to hard-code, so
# the artifact location is passed in explicitly:
#
#   OMR_ENGINE_URL=https://.../amc-omr-engine-<os>-<arch>.tgz ./scripts/fetch-omr-engine.sh
#
# The downloaded archive must contain the platform executable named per the
# table in apps/desktop/src-tauri/resources/omr/README.md. The engine works
# fully offline at run time; the network is touched only here, at build time.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${REPO_ROOT}/apps/desktop/src-tauri/resources/omr"
mkdir -p "$DEST"

uname_s="$(uname -s)"
case "$uname_s" in
  Linux|Darwin) engine="amc-omr-engine" ;;
  *)
    echo "Unsupported OS: $uname_s." >&2
    echo "On Windows, fetch amc-omr-engine.exe from your OMR_ENGINE_URL and" >&2
    echo "copy it into: $DEST" >&2
    exit 1 ;;
esac

if [ -z "${OMR_ENGINE_URL:-}" ]; then
  echo "OMR_ENGINE_URL is not set." >&2
  echo "Point it at the frozen-oemer artifact for this platform, e.g.:" >&2
  echo "  OMR_ENGINE_URL=https://.../amc-omr-engine-\$(uname -s)-\$(uname -m).tgz \\" >&2
  echo "    ./scripts/fetch-omr-engine.sh" >&2
  echo "See apps/desktop/src-tauri/resources/omr/README.md for the contract." >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $OMR_ENGINE_URL"
curl -fsSL -o "$tmp/omr.tgz" "$OMR_ENGINE_URL"
tar xzf "$tmp/omr.tgz" -C "$tmp"

# Locate the engine executable inside the archive (allow a top-level dir) and
# place it directly in the resource dir under the expected name.
found="$(find "$tmp" -type f -name "$engine" -print -quit)"
if [ -z "$found" ]; then
  echo "Archive did not contain an executable named '$engine'." >&2
  exit 1
fi
cp "$found" "$DEST/$engine"
chmod +x "$DEST/$engine"
echo "Placed $engine in $DEST"
