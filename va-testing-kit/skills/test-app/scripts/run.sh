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
# Overridable so run.test.sh can point the guards at fixture servers.
URL="${AMC_URL:-http://localhost:1420}"

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

url_responds(){ curl -sf -o /dev/null --max-time 5 "$URL"; }

# The app has NO login and no required account (offline-first promise). If the
# page at $URL isn't recognizably ours, some OTHER program on this Mac owns the
# app's address — vite fails on its strict port while the app window (or Chrome)
# happily renders the stranger, which the tester saw as a surprise "login
# screen" (#312). The marker is the <title> shipped in apps/desktop/index.html.
# Buffered (no pipe): under pipefail, grep -q's early exit would fail curl and
# report the app's own page as foreign.
serves_our_app(){
  local page
  page="$(curl -sf --max-time 5 "$URL" 2>/dev/null)" || return 1
  case "$page" in *"<title>AI Music Companion</title>"*) return 0 ;; *) return 1 ;; esac
}

kill_ours(){
  for pf in "$WEB_PIDFILE" "$DESK_PIDFILE"; do
    [ -f "$pf" ] && { kill "$(cat "$pf")" 2>/dev/null || true; rm -f "$pf"; }
  done
  pkill -f "cargo tauri dev" 2>/dev/null || true
  pkill -f "$DESK_BIN" 2>/dev/null || true
  # Path-scoped: only OUR checkout's vite, never another project's dev server.
  pkill -f "$APP_DIR.*vite" 2>/dev/null || true
}

stranger_msg(){
  err "Another program on this Mac is using the app's address (port 1420), so the"
  err "window would show THAT program instead of the app — for example a surprise"
  err "login screen. The app itself never asks for a login. Please restart the Mac"
  err "and run the test again; if this message comes back, tell your manager"
  err "\"port 1420 is taken\" and send a screenshot."
}

# Refuse to launch into a port owned by a stranger. Our own leftovers are
# cleared silently first; anything that still answers afterwards isn't ours.
preflight_port(){
  url_responds || return 0
  serves_our_app && return 0
  log "Something is already using the app's address — clearing our old runs..."
  kill_ours
  sleep 2
  url_responds || return 0
  serves_our_app && return 0
  stranger_msg
  exit 13
}

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
  preflight_port
  if web_alive && serves_our_app; then
    ok "App is already running."
  else
    [ -f "$WEB_PIDFILE" ] && kill "$(cat "$WEB_PIDFILE")" 2>/dev/null || true
    log "Starting the app..."
    ( cd "$APP_DIR" && nohup pnpm dev >"$WEB_LOG" 2>&1 & echo $! >"$WEB_PIDFILE" )
    if ! wait_for_url; then
      err "The app did not start in time. Last log lines:"; tail -n 20 "$WEB_LOG" >&2 || true; exit 7
    fi
    if ! serves_our_app; then stranger_msg; exit 13; fi
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
  preflight_port

  log "Building and launching the desktop app."
  log "The FIRST build can take 10-30 minutes while it compiles. The app window opens when ready. Please leave it running."
  ( cd "$APP_DIR" && nohup cargo tauri dev >"$DESK_LOG" 2>&1 & echo $! >"$DESK_PIDFILE" )

  local i=0 max=120   # 120 * 20s = 40 min ceiling
  while [ $i -lt $max ]; do
    # success: cargo launched the built binary, or the process is up
    if grep -q 'Running `target' "$DESK_LOG" 2>/dev/null || pgrep -f "$DESK_BIN" >/dev/null 2>&1; then
      # If the app's address answers but not with OUR page, the open window is
      # rendering a stranger — close it rather than let the tester "log in".
      if url_responds && ! serves_our_app; then
        kill_ours; stranger_msg; exit 13
      fi
      ok "Desktop app window should now be open (look for 'AI Music Companion')."
      echo "MODE=desktop"; echo "COMMIT=$(commit)"; return 0
    fi
    # vite lost its strict port mid-run — a stranger grabbed the app's address.
    # Checked before the generic error grep, which its death line also matches.
    if grep -qi "Port 1420 is already in use" "$DESK_LOG" 2>/dev/null; then
      kill "$(cat "$DESK_PIDFILE" 2>/dev/null)" 2>/dev/null || true; rm -f "$DESK_PIDFILE"
      stranger_msg; exit 13
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
  kill_ours
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

# Sourced by run.test.sh to exercise the functions without dispatching.
if [ "${AMC_SOURCE_ONLY:-}" = "1" ]; then return 0 2>/dev/null || exit 0; fi

case "${1:-start}" in
  start)  shift; cmd_start "${1:-web}" ;;
  desktop) cmd_start_desktop ;;
  web)     cmd_start_web ;;
  stop)   cmd_stop ;;
  status) cmd_status ;;
  info)   cmd_info ;;
  *) err "Unknown command: $1"; exit 2 ;;
esac
