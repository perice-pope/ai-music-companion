#!/usr/bin/env bash
# Files the tester's feedback as a GitHub issue on perice-pope/ai-music-companion.
# Usage: feedback.sh "<title>" <path-to-body-markdown-file>
#
# Auth resolution order:
#   1) Token file at $AMC_TOKEN_FILE (default ~/.config/amc/feedback_token)  -> REST API
#   2) gh CLI if authenticated                                              -> gh issue create
# Prints "ISSUE_URL=<url>" on success. Prints "NO_AUTH" (exit 3) if neither is available.
set -uo pipefail

REPO="perice-pope/ai-music-companion"
TOKEN_FILE="${AMC_TOKEN_FILE:-$HOME/.config/amc/feedback_token}"

title="${1:-}"
bodyfile="${2:-}"
[ -n "$title" ] && [ -n "$bodyfile" ] && [ -f "$bodyfile" ] || {
  echo "USAGE: feedback.sh \"<title>\" <bodyfile>" >&2; exit 2; }

body="$(cat "$bodyfile")"

# --- Path 1: token file -> REST API ---------------------------------------
if [ -s "$TOKEN_FILE" ]; then
  token="$(tr -d '[:space:]' < "$TOKEN_FILE")"
  json="$(python3 - "$title" "$body" <<'PY'
import json, sys
print(json.dumps({"title": sys.argv[1], "body": sys.argv[2]}))
PY
)"
  resp="$(curl -sS -w $'\n%{http_code}' -X POST \
    -H "Authorization: Bearer $token" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "https://api.github.com/repos/$REPO/issues" \
    -d "$json")"
  code="$(printf '%s' "$resp" | tail -n1)"
  payload="$(printf '%s' "$resp" | sed '$d')"
  if [ "$code" = "201" ]; then
    printf '%s' "$payload" | python3 -c "import sys,json;print('ISSUE_URL='+json.load(sys.stdin)['html_url'])"
    exit 0
  fi
  echo "ERROR_$code: feedback token was rejected by GitHub." >&2
  printf '%s\n' "$payload" >&2
  exit 1
fi

# --- Path 2: gh CLI fallback (for the owner's own machine) -----------------
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  url="$(gh issue create --repo "$REPO" --title "$title" --body-file "$bodyfile" 2>/dev/null)"
  if [ -n "$url" ]; then echo "ISSUE_URL=$url"; exit 0; fi
  echo "ERROR: gh issue create failed." >&2; exit 1
fi

echo "NO_AUTH"
exit 3
