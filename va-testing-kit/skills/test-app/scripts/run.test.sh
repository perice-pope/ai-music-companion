#!/usr/bin/env bash
# Behavior tests for run.sh's stranger-on-the-port guard (#312).
# Needs only bash + python3 + curl; run directly: bash run.test.sh
#
# kill_ours is stubbed per-case: the real one pkills vite/tauri machine-wide,
# which must never happen on a dev box running the tests. Its behavior is the
# pre-existing cmd_stop path; what's under test here is the guard's DECISIONS.
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

# --- load run.sh's functions without dispatching -----------------------------
export AMC_SOURCE_ONLY=1
AMC_HOME="$TMP/home" AMC_URL="http://127.0.0.1:$EMPTY_PORT" . "$HERE/run.sh"
unset AMC_SOURCE_ONLY

# 1. The real app page is recognized as ours.
URL="http://127.0.0.1:$OURS_PORT"
if serves_our_app; then pass "serves_our_app accepts the shipped index.html"
else fail "serves_our_app rejected the app's own page — every launch would abort"; fi

# 2. A stranger's login page is NOT mistaken for the app.
URL="http://127.0.0.1:$FOREIGN_PORT"
if serves_our_app; then fail "serves_our_app accepted a foreign login page — the #312 blocker would recur"
else pass "serves_our_app rejects a foreign login page"; fi

# 3. A silent port neither responds nor blocks the launch.
URL="http://127.0.0.1:$EMPTY_PORT"
if url_responds; then fail "url_responds true on a closed port"
else pass "url_responds false on a closed port"; fi
KILLS=0; kill_ours(){ KILLS=$((KILLS+1)); }
if preflight_port && [ "$KILLS" -eq 0 ]; then pass "preflight_port lets a free port through untouched"
else fail "preflight_port blocked or killed on a free port — would break every clean first run"; fi

# 4. Our own already-running app passes pre-flight without being killed.
URL="http://127.0.0.1:$OURS_PORT"
KILLS=0
if preflight_port && [ "$KILLS" -eq 0 ]; then pass "preflight_port leaves our own running app alone"
else fail "preflight_port killed or blocked our own running app"; fi

# 5. A stranger that survives cleanup aborts with exit 13 and a message the
#    tester can act on ("login", "port 1420").
URL="http://127.0.0.1:$FOREIGN_PORT"
OUT="$( (kill_ours(){ :; }; preflight_port) 2>&1 )"; RC=$?
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

# 6. If clearing our leftovers frees the port, pre-flight recovers and allows
#    the launch (the self-healing path — a stale vite of ours, not a stranger).
URL="http://127.0.0.1:$FOREIGN_PORT"
kill_ours(){ kill "$FOREIGN_PID" 2>/dev/null; wait "$FOREIGN_PID" 2>/dev/null; }
if preflight_port; then pass "preflight_port recovers when cleanup frees the port"
else fail "preflight_port still aborted after cleanup freed the port"; fi
FOREIGN_PID=""

echo
if [ "$FAILS" -gt 0 ]; then echo "$FAILS test(s) failed" >&2; exit 1; fi
echo "all tests passed"
