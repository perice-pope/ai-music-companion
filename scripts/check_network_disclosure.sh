#!/usr/bin/env bash
#
# check_network_disclosure.sh — enforce the network-transparency guarantee.
#
# Scans the first-party Rust sources for outbound-network call sites and FAILS
# if any live in a file that is not in the disclosure registry
# (`docs/architecture/network-call-sites.allowlist`). The registry, in turn, is
# anchored to the enumeration table in
# `docs/architecture/offline-first-and-network-transparency.md`.
#
# The point: a new outbound HTTP/socket call cannot land silently. If it isn't
# disclosed (registry + doc + Connections & Privacy), this check is red.
#
# Design notes (low false positives):
#   * Only first-party source is scanned (crates/*/src, apps/desktop/src-tauri/src).
#     Vendored deps, target/, and node_modules are never in scope.
#   * Comment-only lines are stripped before matching, so prose that *mentions*
#     `HttpClient` (e.g. "no HttpClient in sight") never trips the scanner.
#   * Indicators are the things that actually open a socket: the real HTTP client
#     crates (reqwest / ureq / hyper / isahc / surf / curl) and raw socket
#     egress (TcpStream::connect, UdpSocket / std::net connect). The in-house
#     `HttpClient` *trait* abstraction is matched via its concrete client crate,
#     not its name, so mock/test impls of the trait don't register.
#
# Usage:
#   scripts/check_network_disclosure.sh           # scan + verify
#   CHECK_NET_VERBOSE=1 scripts/check_network_disclosure.sh   # show every hit
#
# Exit codes: 0 = all call sites disclosed; 1 = an undisclosed call site exists
# (or the doc/registry consistency check failed).

set -euo pipefail

# Resolve repo root from this script's location so it runs from anywhere / CI.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

ALLOWLIST="docs/architecture/network-call-sites.allowlist"
DOC="docs/architecture/offline-first-and-network-transparency.md"

# First-party source roots to scan. Add new first-party Rust source roots here.
SCAN_ROOTS=("crates" "apps/desktop/src-tauri/src")

# Outbound-network indicators. Each is a real socket-opening construct; the
# in-house HttpClient trait is represented by its concrete client crate
# (reqwest), so abstract trait references and test mocks do not match.
#
# ERE, applied to comment-stripped lines.
INDICATORS='(^|[^[:alnum:]_])(reqwest|ureq|isahc|surf)::'
INDICATORS+='|(^|[^[:alnum:]_])hyper::(Client|client::)'
INDICATORS+='|extern crate (reqwest|ureq|hyper|isahc|surf|curl)'
INDICATORS+='|use (reqwest|ureq|hyper|isahc|surf|curl)([^[:alnum:]_]|$)'
INDICATORS+='|TcpStream::connect|UdpSocket::bind|std::net::(TcpStream|UdpSocket)'

# Tooling check.
if ! command -v rg >/dev/null 2>&1; then
  echo "ERROR: ripgrep (rg) is required by ${0##*/}." >&2
  exit 1
fi

if [[ ! -f "${ALLOWLIST}" ]]; then
  echo "ERROR: disclosure registry not found at ${ALLOWLIST}" >&2
  exit 1
fi

# ── Load the allowlist into a set ────────────────────────────────────────────
declare -A ALLOWED=()
while IFS= read -r line; do
  # Strip inline comments and surrounding whitespace; skip blanks/comments.
  line="${line%%#*}"
  line="$(printf '%s' "${line}" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  [[ -z "${line}" ]] && continue
  ALLOWED["${line}"]=1
done < "${ALLOWLIST}"

# ── Scan ─────────────────────────────────────────────────────────────────────
# `rg` with:
#   -t rust       : only Rust files (skips JSON/asset noise)
#   --no-heading  : file:line:text on every match line
#   We strip `//`-comment-only lines via a pre-filter so mentions in prose
#   never count. A line is considered code if, after removing a leading `//`
#   comment, the indicator still matches.
declare -A HITS=()          # file -> first matching "line: text"
undisclosed=0

# Collect candidate matches. `|| true` so "no matches" (rg exit 1) isn't fatal.
matches="$(rg --no-heading --line-number -t rust -e "${INDICATORS}" "${SCAN_ROOTS[@]}" 2>/dev/null || true)"

if [[ -n "${matches}" ]]; then
  while IFS= read -r match; do
    [[ -z "${match}" ]] && continue
    file="${match%%:*}"
    rest="${match#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"

    # Strip a leading line comment: if the code *before* any `//` no longer
    # contains an indicator, this was a comment-only mention — ignore it.
    code_part="${text%%//*}"
    if ! printf '%s' "${code_part}" | grep -Eq "${INDICATORS}"; then
      continue
    fi

    if [[ -n "${CHECK_NET_VERBOSE:-}" ]]; then
      echo "hit: ${file}:${lineno}: ${text}"
    fi

    # Record the first hit per file for the report.
    if [[ -z "${HITS[${file}]:-}" ]]; then
      HITS["${file}"]="${lineno}: ${text}"
    fi
  done <<< "${matches}"
fi

# ── Verify every hit is disclosed ────────────────────────────────────────────
for file in "${!HITS[@]}"; do
  if [[ -z "${ALLOWED[${file}]:-}" ]]; then
    if [[ "${undisclosed}" -eq 0 ]]; then
      echo "❌ Undisclosed outbound-network call site(s) found." >&2
      echo "   Every such file must be in ${ALLOWLIST}" >&2
      echo "   AND in the enumeration table in ${DOC}." >&2
      echo >&2
    fi
    undisclosed=1
    echo "   • ${file}  (${HITS[${file}]})" >&2
  fi
done

# ── Verify the registry has no stale entries (file gone or no longer a call site)
stale=0
for allowed in "${!ALLOWED[@]}"; do
  if [[ ! -f "${allowed}" ]]; then
    echo "⚠️  Registry entry points at a missing file: ${allowed}" >&2
    stale=1
    continue
  fi
  if [[ -z "${HITS[${allowed}]:-}" ]]; then
    echo "⚠️  Registry entry no longer contains a call site (remove it?): ${allowed}" >&2
    stale=1
  fi
done

# ── Verify each disclosed file is also named in the doc (defense in depth) ────
doc_gap=0
if [[ -f "${DOC}" ]]; then
  for allowed in "${!ALLOWED[@]}"; do
    if ! grep -Fq "${allowed}" "${DOC}"; then
      echo "❌ Disclosed call site is not referenced in the doc table: ${allowed}" >&2
      echo "   Add it to the enumeration table in ${DOC}." >&2
      doc_gap=1
    fi
  done
else
  echo "ERROR: offline-first doc not found at ${DOC}" >&2
  exit 1
fi

if [[ "${undisclosed}" -ne 0 || "${doc_gap}" -ne 0 ]]; then
  echo >&2
  echo "Network-disclosure check FAILED." >&2
  echo "See ${ALLOWLIST} for how to disclose a new call site." >&2
  exit 1
fi

if [[ "${stale}" -ne 0 ]]; then
  echo >&2
  echo "Network-disclosure check FAILED (stale registry entries)." >&2
  exit 1
fi

n="${#HITS[@]}"
echo "✅ Network-disclosure check passed: ${n} disclosed call site(s), no undisclosed egress."
