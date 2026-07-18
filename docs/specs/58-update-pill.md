# Spec: The update pill (#58, final slice)

## 1. Summary
A calm bottom-left pill — "Update to v2.X.Y" — appears when a newer signed
release exists, Claude-desktop style. One click downloads and installs;
the pill then asks for a quit-and-reopen. Gated behind a NEW opt-in
"Check for updates automatically" toggle (off by default) so the shipped
offline-first promise ("no update request on launch or in the background")
stays true for everyone who hasn't opted in.

## 2. Problem / why
The signing/manifest pipeline is proven live (v2.28.12+: latest.json +
per-platform .sig). But finding an update still requires opening
Connections & Privacy and clicking "Check for updates" — testers ship
feedback on stale builds. The missing half is the surface.

## 3. Non-goals
- No auto-download or auto-install — the pill always asks.
- No in-app relaunch (needs tauri-plugin-process; follow-up slice).
- No release-notes rendering (version string only in S1).
- No change to the manual check path.

## 4. Contract / interface
- `connectionsStore` gains `autoUpdateCheckEnabled` (+ setter), persisted
  under `ai-music-companion:auto-update-check-enabled`, default false —
  same loadFlag/saveFlag pattern as every networked opt-in.
- New `updateStore`: `phase: "idle"|"available"|"downloading"|"ready"|
  "error"`, `availableVersion: string|null`, `notice: string|null`;
  actions `checkForUpdate()` (plugin `check()`; sets available),
  `installUpdate()` (plugin `downloadAndInstall()`; available→downloading
  →ready), `dismiss()` (per-version, session-persistent via localStorage
  `ai-music-companion:dismissed-update`).
- New `UpdatePill.tsx`, mounted once in `App`, fixed bottom-left. ONE
  persistent element across phases (#417 rule 0): available (full
  opacity) → downloading (dimmed, "Updating…") → ready ("Quit and reopen
  to finish") → error (calm one-liner, dimmed). Dismiss "×" hides that
  version only; a newer version re-surfaces.
- `App` effect: on mount and every 4 h, IF `autoUpdateCheckEnabled`,
  `checkForUpdate()`; failures are silent (offline is normal, never an
  error surface).
- `ConnectionsPrivacy`: new ToggleRow wired to the flag; the App-updates
  info card copy updated to describe the opt-in; the disclosure doc's
  auto-update row updated the same way.

## 5. Acceptance criteria
1. With the toggle OFF (default), no updater call happens on mount or on
   the interval — the plugin mock is never invoked. (The shipped privacy
   promise, now pinned by test.)
2. With the toggle ON and a newer version available, the pill renders
   bottom-left with "Update to v{X}" within one check cycle.
3. Clicking the pill downloads+installs via the plugin and transitions
   the SAME element available→downloading→ready — no unmount/remount
   (rule 0), asserted via a stable testid surviving all phases.
4. Dismissing hides that version for the session; a different newer
   version shows the pill again.
5. A failed check or install shows a calm dimmed notice, never a crash,
   and never re-fires in a loop.
6. With no update available, nothing renders (no empty chrome).
7. The toggle persists across reloads; turning it off stops the interval.

## 6. Edge cases
- Offline with toggle on: check rejects → silent (AC5 covers install
  errors; check errors stay invisible — offline is normal).
- localStorage unavailable: toggle works for the session (loadFlag
  pattern), dismiss falls back to in-memory.
- Version equal/older: plugin returns null → idle, nothing renders.

## 7. Test plan
| AC | Test |
|---|---|
| 1 | updateStore/App test: toggle off → mocked check never called (mount + advanced timers) |
| 2 | UpdatePill test: available → pill text "Update to v9.9.9" |
| 3 | UpdatePill test: click → downloadAndInstall called; same testid element through phases; opacity classes transition |
| 4 | updateStore test: dismiss v9.9.9 → hidden; v9.9.10 available → shown |
| 5 | UpdatePill test: install rejects → calm notice, dimmed, no rethrow |
| 6 | UpdatePill test: idle → null render |
| 7 | connectionsStore test: flag persists via localStorage key |

## 8. Architecture
Frontend-only (plugin already registered both sides; endpoints/pubkey
live in tauri.conf.json). No new Rust. No new dependencies. The check
call is the only network I/O and rides the documented GitHub endpoint,
now opt-in-gated in first-party code as well as user action.
