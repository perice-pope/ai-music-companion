#!/usr/bin/env bash
# Behavior tests for run.sh's stranger-on-the-port guard (#312).
# Needs only bash + python3 + curl; run directly: bash run.test.sh
#
# Every guard call runs inside a subshell with kill_ours (and, for the
# command-level cases, the machine-touching setup steps) stubbed: the real
# kill_ours pkills our vite/tauri, which must never happen on a dev box
# running the tests. The stubs replace only pre-existing plumbing — the
# guard's decisions, termination, and wiring are what's under test.
#
# shellcheck disable=SC2034  # URL is read by the functions sourced from run.sh
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
TMP="$(mktemp -d)"
OURS_PID=""; FOREIGN_PID=""
cleanup(){ [ -n "$OURS_PID" ] && kill "$OURS_PID" 2>/dev/null; [ -n "$FOREIGN_PID" ] && kill "$FOREIGN_PID" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT

FAILS=0
fail(){ echo "not ok - $*" >&2; FAILS=$((FAILS+1)); }
pass(){ echo "ok - $*"; }

# --- fixtures: the real app page vs. a stranger's login page ----------------
mkdir -p "$TMP/ours" "$TMP/foreign" "$TMP/home"
cp "$HERE/../../../../apps/desktop/index.html" "$TMP/ours/index.html"
cat > "$TMP/foreign/index.html" <<'HTML'
<!doctype html><html><head><title>Acme Device Manager</title></head>
<body><h1>Sign in</h1><form><input type="email"><input type="password">
<button>Log in</button></form></body></html>
HTML

pick_port(){ python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }
OURS_PORT="$(pick_port)"; FOREIGN_PORT="$(pick_port)"; EMPTY_PORT="$(pick_port)"

( cd "$TMP/ours"    && exec python3 -m http.server "$OURS_PORT"    --bind 127.0.0.1 >/dev/null 2>&1 ) & OURS_PID=$!
( cd "$TMP/foreign" && exec python3 -m http.server "$FOREIGN_PORT" --bind 127.0.0.1 >/dev/null 2>&1 ) & FOREIGN_PID=$!
for _ in $(seq 1 50); do
  curl -sf -o /dev/null "http://127.0.0.1:$OURS_PORT" && curl -sf -o /dev/null "http://127.0.0.1:$FOREIGN_PORT" && break
  sleep 0.2
done

OURS_URL="http://127.0.0.1:$OURS_PORT"
FOREIGN_URL="http://127.0.0.1:$FOREIGN_PORT"
EMPTY_URL="http://127.0.0.1:$EMPTY_PORT"

# --- load run.sh's functions without dispatching -----------------------------
export AMC_SOURCE_ONLY=1
AMC_HOME="$TMP/home" AMC_URL="$EMPTY_URL" . "$HERE/run.sh"
unset AMC_SOURCE_ONLY

# Stubs for the machine-touching plumbing around the guard. Used by the
# command-level cases; each runs in its own subshell so nothing leaks.
stub_env(){
  kill_ours(){ :; }
  ensure_path(){ :; }; require_base(){ :; }
  update_repo(){ :; }; install_deps(){ :; }
  ensure_clt(){ :; }; ensure_rust(){ :; }; ensure_tauri_cli(){ :; }
  ensure_audio_engine(){ :; }
  wait_for_url(){ return 0; }
}

# 1. The real app page is recognized as ours.
URL="$OURS_URL"
if serves_our_app; then pass "serves_our_app accepts the shipped index.html"
else fail "serves_our_app rejected the app's own page — every launch would abort"; fi

# 2. A stranger's login page is NOT mistaken for the app.
URL="$FOREIGN_URL"
if serves_our_app; then fail "serves_our_app accepted a foreign login page — the #312 blocker would recur"
else pass "serves_our_app rejects a foreign login page"; fi

# 3. A silent port neither responds nor blocks the launch.
URL="$EMPTY_URL"
if url_responds; then fail "url_responds true on a closed port"
else pass "url_responds false on a closed port"; fi
OUT="$( (kill_ours(){ echo KILLED; }; preflight_port && echo PASSED) 2>&1 )"
case "$OUT" in
  *KILLED*) fail "preflight_port killed on a free port — would break every clean first run" ;;
  *PASSED*) pass "preflight_port lets a free port through untouched" ;;
  *) fail "preflight_port blocked a free port (output: $OUT)" ;;
esac

# 4. Our own already-running app passes pre-flight without being killed.
URL="$OURS_URL"
OUT="$( (kill_ours(){ echo KILLED; }; preflight_port && echo PASSED) 2>&1 )"
case "$OUT" in
  *KILLED*) fail "preflight_port killed our own running app" ;;
  *PASSED*) pass "preflight_port leaves our own running app alone" ;;
  *) fail "preflight_port blocked our own running app (output: $OUT)" ;;
esac

# 5. A stranger that survives cleanup TERMINATES the script (exit 13, not a
#    discarded return — run.sh has no set -e, so a plain return would let the
#    launch continue) with a message the tester can act on.
URL="$FOREIGN_URL"
OUT="$( (kill_ours(){ :; }; preflight_port; echo LEAKED) 2>&1 )"; RC=$?
case "$OUT" in
  *LEAKED*) fail "preflight_port returned instead of exiting — the launch would proceed into the stranger" ;;
  *) pass "preflight_port terminates the run on a persistent stranger" ;;
esac
if [ "$RC" -eq 13 ]; then pass "preflight_port exits 13 on a persistent stranger"
else fail "preflight_port exit code was $RC (want 13) on a persistent stranger"; fi
case "$OUT" in
  *login*) pass "stranger message explains the surprise login screen" ;;
  *) fail "stranger message doesn't mention the login screen: $OUT" ;;
esac
case "$OUT" in
  *"port 1420 is taken"*) pass "stranger message gives the manager-ready phrase" ;;
  *) fail "stranger message lacks the 'port 1420 is taken' phrase: $OUT" ;;
esac

# 6. WIRING: web mode aborts with 13 and never reaches the browser when a
#    stranger owns the port.
OUT="$( (stub_env; URL="$FOREIGN_URL"; WEB_LOG=/dev/null; open_browser(){ echo BROWSER_OPENED; }; cmd_start_web) 2>&1 )"; RC=$?
if [ "$RC" -eq 13 ]; then pass "cmd_start_web aborts 13 when a stranger owns the port"
else fail "cmd_start_web exit code was $RC (want 13) with a stranger on the port"; fi
case "$OUT" in
  *BROWSER_OPENED*) fail "cmd_start_web opened the browser onto a stranger's page" ;;
  *) pass "cmd_start_web never opens the browser onto a stranger's page" ;;
esac

# 7. WIRING: web mode still reuses our own healthy server (happy path intact).
echo $$ > "$TMP/home/.web.pid"
OUT="$( (stub_env; URL="$OURS_URL"; open_browser(){ echo BROWSER_OPENED; }; cmd_start_web) 2>&1 )"; RC=$?
if [ "$RC" -eq 0 ] && case "$OUT" in *"already running"*BROWSER_OPENED*) true ;; *) false ;; esac; then
  pass "cmd_start_web reuses our own running app and opens the browser"
else fail "cmd_start_web broke the reuse happy path (rc=$RC, output: $OUT)"; fi
rm -f "$TMP/home/.web.pid"

# 8. WIRING: desktop mode pre-flights before spending 10-30 min building.
OUT="$( (stub_env; URL="$FOREIGN_URL"; cmd_start_desktop) 2>&1 )"; RC=$?
if [ "$RC" -eq 13 ]; then pass "cmd_start_desktop aborts 13 when a stranger owns the port"
else fail "cmd_start_desktop exit code was $RC (want 13) with a stranger on the port"; fi
case "$OUT" in
  *"Building and launching"*) fail "cmd_start_desktop started a build despite the stranger" ;;
  *) pass "cmd_start_desktop refuses before building" ;;
esac

# 9. WIRING: a stranger stealing the port MID-build (vite strict-port death)
#    is explained, not dumped as raw log jargon.
printf 'error when starting dev server:\nError: Port 1420 is already in use\n' > "$TMP/home/.desktop.log"
OUT="$( (stub_env; URL="$EMPTY_URL"; cmd_start_desktop) 2>&1 )"; RC=$?
if [ "$RC" -eq 13 ]; then pass "mid-build port theft aborts 13 with the stranger explanation"
else fail "mid-build port theft gave rc=$RC (want 13): $OUT"; fi
case "$OUT" in
  *login*) pass "mid-build port theft message explains the login-screen symptom" ;;
  *) fail "mid-build port theft message lacks the explanation: $OUT" ;;
esac
rm -f "$TMP/home/.desktop.log" "$TMP/home/.desktop.pid"

# 10. If clearing our leftovers frees the port, pre-flight recovers and allows
#     the launch (the self-healing path — a stale vite of ours, not a stranger).
URL="$FOREIGN_URL"
OUT="$( (kill_ours(){ kill "$FOREIGN_PID" 2>/dev/null; wait "$FOREIGN_PID" 2>/dev/null; }; preflight_port && echo PASSED) 2>&1 )"
case "$OUT" in
  *PASSED*) pass "preflight_port recovers when cleanup frees the port" ;;
  *) fail "preflight_port still aborted after cleanup freed the port: $OUT" ;;
esac
FOREIGN_PID=""

echo
if [ "$FAILS" -gt 0 ]; then echo "$FAILS test(s) failed" >&2; exit 1; fi
echo "all tests passed"
