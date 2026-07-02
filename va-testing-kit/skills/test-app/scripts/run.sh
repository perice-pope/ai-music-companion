#!/usr/bin/env bash
# AI Music Companion runner for non-technical testers.
# Usage:
#   run.sh start [web|desktop]   (default: web)
#   run.sh stop
#   run.sh status
#   run.sh info
set -uo pipefail

REPO_URL="https://github.com/perice-pope/ai-music-companion.git"
AMC_HOME="${AMC_HOME:-$HOME/amc}"
REPO_DIR="$AMC_HOME/ai-music-companion"
APP_DIR="$REPO_DIR/apps/desktop"
URL="http://localhost:1420"

WEB_PIDFILE="$AMC_HOME/.web.pid";      WEB_LOG="$AMC_HOME/.web.log"
DESK_PIDFILE="$AMC_HOME/.desktop.pid"; DESK_LOG="$AMC_HOME/.desktop.log"
DESK_BIN="target/debug/ai-music-companion"

log(){ printf '\033[36m> %s\033[0m\n' "$*"; }
ok(){  printf '\033[32m+ %s\033[0m\n' "$*"; }
err(){ printf '\033[31mx %s\033[0m\n' "$*" >&2; }

ensure_path(){
  [ -x /opt/homebrew/bin/brew ] && eval "$(/opt/homebrew/bin/brew shellenv)" 2>/dev/null || true
  [ -x /usr/local/bin/brew ]    && eval "$(/usr/local/bin/brew shellenv)"    2>/dev/null || true
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" 2>/dev/null || true
  command -v corepack >/dev/null 2>&1 && corepack enable >/dev/null 2>&1 || true
}

require_base(){
  local missing=0
  for t in git node; do
    command -v "$t" >/dev/null 2>&1 || { err "Missing: $t"; missing=1; }
  done
  if [ "$missing" = 1 ]; then
    err "Setup is incomplete. Please re-run the one-time installer your manager sent you."
    exit 4
  fi
}

update_repo(){
  # The bootstrap shim (amc.sh) already pulled the latest repo before exec'ing
  # this script, so skip the redundant reset when launched that way.
  [ "${AMC_BOOTSTRAPPED:-}" = "1" ] && return 0
  mkdir -p "$AMC_HOME"
  if [ ! -d "$REPO_DIR/.git" ]; then
    log "Downloading the app for the first time..."
    git clone --depth 1 "$REPO_URL" "$REPO_DIR" >/dev/null 2>&1 \
      || { err "Could not download the app. Check your internet connection."; exit 5; }
  else
    log "Getting the latest version of the app..."
    git -C "$REPO_DIR" fetch --depth 1 origin main >/dev/null 2>&1 || true
    git -C "$REPO_DIR" reset --hard origin/main >/dev/null 2>&1 || true
    git -C "$REPO_DIR" clean -fd >/dev/null 2>&1 || true
  fi
}

install_deps(){
  log "Preparing the app (this can take a minute the first time)..."
  ( cd "$APP_DIR" && pnpm install --silent ) \
    || { err "Could not prepare the app dependencies."; exit 6; }
}

# ---------- desktop-only prerequisites (installed on demand) ----------------
ensure_clt(){
  if ! xcode-select -p >/dev/null 2>&1; then
    log "Opening the Apple developer tools installer — please click Install in the popup, let it finish, then run /test-app desktop again."
    xcode-select --install >/dev/null 2>&1 || true
    err "Waiting on Apple developer tools. Re-run after the popup finishes."
    exit 8
  fi
}
ensure_rust(){
  ensure_path
  if ! command -v cargo >/dev/null 2>&1; then
    log "Installing Rust (one time, a few minutes)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
      || { err "Could not install Rust."; exit 9; }
    . "$HOME/.cargo/env"
  fi
}
ensure_tauri_cli(){
  ensure_rust
  if ! cargo tauri --version >/dev/null 2>&1; then
    log "Installing the desktop builder (compiles for several minutes, one time)..."
    cargo install tauri-cli --version "^2.0.0" --locked \
      || { err "Could not install the desktop builder."; exit 10; }
  fi
}
# ONNX Runtime for audio-file transcription. Stored outside the repo (survives
# the repo reset) and exposed via ORT_DYLIB_PATH, which the app honors as a
# fallback loader path. Best effort: if it fails, audio-file import is simply
# unavailable; everything else (incl. MusicXML/MIDI import) still works.
ensure_audio_engine(){
  local dir="$AMC_HOME/onnxruntime" lib
  lib="$dir/libonnxruntime.dylib"
  if [ ! -f "$lib" ]; then
    log "Fetching the audio engine (one time, ~50 MB)..."
    # 1.24.2 ships macOS as arch-specific builds (osx-arm64 / osx-x64), NOT
    # osx-universal2 (that name 404s — #267). Pick by CPU.
    local ver="1.24.2" arch pkg tmp
    case "$(uname -m)" in
      arm64) arch="arm64" ;;
      x86_64) arch="x64" ;;
      *) arch="arm64" ;;
    esac
    pkg="onnxruntime-osx-${arch}-${ver}"
    tmp="$(mktemp -d)"
    mkdir -p "$dir"
    if curl -fsSL -o "$tmp/ort.tgz" \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ver}/${pkg}.tgz" \
       && tar xzf "$tmp/ort.tgz" -C "$tmp"; then
      cp "$tmp/${pkg}/lib/libonnxruntime.dylib"* "$dir/" 2>/dev/null \
        || cp "$tmp/${pkg}/lib/libonnxruntime.dylib" "$dir/" 2>/dev/null || true
    fi
    rm -rf "$tmp"
  fi
  [ -f "$lib" ] && export ORT_DYLIB_PATH="$lib" || log "(Audio-file import may be unavailable — that's OK.)"
}

web_alive(){  [ -f "$WEB_PIDFILE" ]  && kill -0 "$(cat "$WEB_PIDFILE")"  2>/dev/null; }
desk_alive(){ [ -f "$DESK_PIDFILE" ] && kill -0 "$(cat "$DESK_PIDFILE")" 2>/dev/null; }

wait_for_url(){
  local i=0
  while [ $i -lt 90 ]; do
    curl -sf -o /dev/null "$URL" && return 0
    sleep 1; i=$((i+1))
  done
  return 1
}

open_browser(){ open -a "Google Chrome" "$URL" 2>/dev/null || open "$URL" 2>/dev/null || true; }
commit(){ git -C "$REPO_DIR" rev-parse --short HEAD 2>/dev/null || echo unknown; }

# ---------- web mode --------------------------------------------------------
cmd_start_web(){
  ensure_path; require_base
  update_repo; install_deps
  if web_alive && curl -sf -o /dev/null "$URL"; then
    ok "App is already running."
  else
    [ -f "$WEB_PIDFILE" ] && kill "$(cat "$WEB_PIDFILE")" 2>/dev/null || true
    log "Starting the app..."
    ( cd "$APP_DIR" && nohup pnpm dev >"$WEB_LOG" 2>&1 & echo $! >"$WEB_PIDFILE" )
    if ! wait_for_url; then
      err "The app did not start in time. Last log lines:"; tail -n 20 "$WEB_LOG" >&2 || true; exit 7
    fi
  fi
  open_browser
  ok "App is open in Chrome at $URL"
  echo "MODE=web"; echo "COMMIT=$(commit)"; echo "URL=$URL"
}

# ---------- desktop mode ----------------------------------------------------
cmd_start_desktop(){
  ensure_path; require_base
  ensure_clt; ensure_rust; ensure_tauri_cli
  update_repo; install_deps
  ensure_audio_engine   # ONNX runtime for audio-file transcription (best effort)

  # Optional: real AI coaching tips + recap narration need an LLM key. The
  # on-device analysis (pitch, tone, intonation, groove) works without it.
  local LLM_KEY_FILE="${AMC_LLM_KEY_FILE:-$HOME/.config/amc/llm_key}"
  if [ -s "$LLM_KEY_FILE" ]; then
    export MUSIC_COMPANION_LLM_API_KEY="$(tr -d '[:space:]' < "$LLM_KEY_FILE")"
  fi

  if desk_alive && pgrep -f "$DESK_BIN" >/dev/null 2>&1; then
    ok "Desktop app is already running."
    echo "MODE=desktop"; echo "COMMIT=$(commit)"; return 0
  fi
  [ -f "$DESK_PIDFILE" ] && kill "$(cat "$DESK_PIDFILE")" 2>/dev/null || true

  log "Building and launching the desktop app."
  log "The FIRST build can take 10-30 minutes while it compiles. The app window opens when ready. Please leave it running."
  ( cd "$APP_DIR" && nohup cargo tauri dev >"$DESK_LOG" 2>&1 & echo $! >"$DESK_PIDFILE" )

  local i=0 max=120   # 120 * 20s = 40 min ceiling
  while [ $i -lt $max ]; do
    # success: cargo launched the built binary, or the process is up
    if grep -q 'Running `target' "$DESK_LOG" 2>/dev/null || pgrep -f "$DESK_BIN" >/dev/null 2>&1; then
      ok "Desktop app window should now be open (look for 'AI Music Companion')."
      echo "MODE=desktop"; echo "COMMIT=$(commit)"; return 0
    fi
    # hard failure: compile error
    if grep -qiE 'error: could not compile|error\[E[0-9]+\]|^error:' "$DESK_LOG" 2>/dev/null; then
      err "The desktop app failed to build. Last log lines:"; tail -n 25 "$DESK_LOG" >&2 || true
      kill "$(cat "$DESK_PIDFILE" 2>/dev/null)" 2>/dev/null || true; rm -f "$DESK_PIDFILE"
      exit 11
    fi
    # process died without a clear error
    desk_alive || { err "Desktop build stopped unexpectedly:"; tail -n 25 "$DESK_LOG" >&2 || true; exit 12; }
    log "Still compiling... ($((i*20/60)) min elapsed). This is normal on the first run."
    sleep 20; i=$((i+1))
  done
  err "Desktop build is taking longer than expected; it may still finish. Check for the app window."
  echo "MODE=desktop"; echo "COMMIT=$(commit)"
}

cmd_start(){
  case "${1:-web}" in
    web)     cmd_start_web ;;
    desktop) cmd_start_desktop ;;
    *) err "Unknown mode: $1 (use web|desktop)"; exit 2 ;;
  esac
}

cmd_stop(){
  for pf in "$WEB_PIDFILE" "$DESK_PIDFILE"; do
    [ -f "$pf" ] && { kill "$(cat "$pf")" 2>/dev/null || true; rm -f "$pf"; }
  done
  pkill -f "cargo tauri dev" 2>/dev/null || true
  pkill -f "$DESK_BIN" 2>/dev/null || true
  pkill -f "vite" 2>/dev/null || true
  ok "App stopped."
}

cmd_status(){
  if web_alive && curl -sf -o /dev/null "$URL"; then ok "Web running at $URL"; fi
  if desk_alive && pgrep -f "$DESK_BIN" >/dev/null 2>&1; then ok "Desktop running"; fi
  web_alive || desk_alive || log "Not running."
}

cmd_info(){
  ensure_path
  if [ -d "$REPO_DIR/.git" ]; then
    echo "COMMIT=$(commit)"; echo "BRANCH=$(git -C "$REPO_DIR" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
  else echo "COMMIT=none"; fi
}

case "${1:-start}" in
  start)  shift; cmd_start "${1:-web}" ;;
  desktop) cmd_start_desktop ;;
  web)     cmd_start_web ;;
  stop)   cmd_stop ;;
  status) cmd_status ;;
  info)   cmd_info ;;
  *) err "Unknown command: $1"; exit 2 ;;
esac
