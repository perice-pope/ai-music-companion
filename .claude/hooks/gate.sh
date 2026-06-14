#!/usr/bin/env bash
# Stop hook: a FAST format gate that blocks finishing on unformatted code.
# Formatting is the cheapest gate (no compile) and the most common CI failure.
# Heavier gates (clippy, tests, build) are too slow for a stop hook — they run
# in the /feature pipeline and in CI, not here.
#
# Exit 0 = allow stop. Exit 2 = block stop; stderr is shown back to the model.
set -uo pipefail

# Only run at the repo root (where the workspace Cargo.toml lives).
[ -f Cargo.toml ] || exit 0

problems=""

# Only Rust fmt is checked here: it IS a CI gate (cargo fmt --check) and is the
# most common failure, and it's fast (no compile). Frontend formatting is left
# to the PostToolUse auto-formatter + the project's own eslint CI gate, so this
# hook never blocks on something CI doesn't enforce.
if command -v cargo >/dev/null 2>&1; then
  cargo fmt --all -- --check >/dev/null 2>&1 \
    || problems="${problems}\n- Rust (workspace) formatting — run: cargo fmt --all"
  if [ -f apps/desktop/src-tauri/Cargo.toml ]; then
    cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check >/dev/null 2>&1 \
      || problems="${problems}\n- Rust (desktop) formatting — run: cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all"
  fi
fi

if [ -n "$problems" ]; then
  printf 'Format gate failed — fix before finishing:%b\n' "$problems" >&2
  exit 2
fi
exit 0
