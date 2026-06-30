#!/usr/bin/env bash
# Thin self-updating bootstrap for the AI Music Companion tester.
# This is the ONLY script installed on the tester's machine. It pulls the latest
# repo (app + testing kit) and hands off to the real runner that lives IN the
# repo, so updates to the app, the checklist, the scripts, or the samples all
# propagate automatically with no re-install.
#
# Usage: amc.sh start [web|desktop] | stop | status | info
set -uo pipefail

REPO_URL="https://github.com/perice-pope/ai-music-companion.git"
AMC_HOME="${AMC_HOME:-$HOME/amc}"
REPO_DIR="$AMC_HOME/ai-music-companion"
KIT_RUNNER="$REPO_DIR/va-testing-kit/skills/test-app/scripts/run.sh"

log(){ printf '\033[36m> %s\033[0m\n' "$*"; }
err(){ printf '\033[31mx %s\033[0m\n' "$*" >&2; }

command -v git >/dev/null 2>&1 || { err "Setup is incomplete — please re-run the installer your manager sent you."; exit 4; }

mkdir -p "$AMC_HOME"
if [ ! -d "$REPO_DIR/.git" ]; then
  log "Getting the app for the first time..."
  git clone --depth 1 "$REPO_URL" "$REPO_DIR" >/dev/null 2>&1 \
    || { err "Could not download the app. Check your internet connection."; exit 5; }
else
  log "Updating to the latest version..."
  git -C "$REPO_DIR" fetch --depth 1 origin main >/dev/null 2>&1 || true
  git -C "$REPO_DIR" reset --hard origin/main >/dev/null 2>&1 || true
  git -C "$REPO_DIR" clean -fd >/dev/null 2>&1 || true
fi

if [ ! -f "$KIT_RUNNER" ]; then
  err "The testing kit isn't in the app repo yet (va-testing-kit/). Ask your manager to push it."
  exit 6
fi

export AMC_BOOTSTRAPPED=1   # tell run.sh the repo is already up to date
exec bash "$KIT_RUNNER" "$@"
