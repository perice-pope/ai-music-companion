#!/usr/bin/env bash
# PostToolUse(Edit|Write) hook: auto-format the file just touched.
# Best-effort and non-blocking — keeps formatting clean so it never fails CI.
# Reads the tool-call JSON on stdin; extracts the file path.
set -uo pipefail

input="$(cat)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null)"
[ -z "${file:-}" ] && exit 0
[ -f "$file" ] || exit 0

case "$file" in
  *.rs)
    command -v rustfmt >/dev/null 2>&1 && rustfmt --edition 2021 "$file" >/dev/null 2>&1 || true
    ;;
  *.ts|*.tsx|*.js|*.jsx|*.css|*.json|*.md)
    # Only format frontend files where prettier is configured.
    case "$file" in
      *apps/desktop/*)
        if [ -x apps/desktop/node_modules/.bin/prettier ]; then
          (cd apps/desktop && node_modules/.bin/prettier --write "$file" >/dev/null 2>&1) || true
        fi
        ;;
    esac
    ;;
esac
exit 0
