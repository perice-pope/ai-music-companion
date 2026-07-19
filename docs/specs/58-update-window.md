# Spec: The update window — Restart now + release notes (#58 polish, E1)

## 1. Summary
The pill grows into the Claude-desktop experience: clicking an available
update opens a compact in-place window with the version and its release
notes and an "Update now" button; once staged, the pill offers a real
"Restart now" that relaunches into the new build. Founder priority.

## 2. Contract
- New dependency (justified: the official updater pairing):
  `tauri-plugin-process` (Rust) + `@tauri-apps/plugin-process` (JS),
  registered in main.rs, `process:allow-restart` in capabilities.
- updateStore: `releaseNotes: string | null` captured from the update
  handle's `body` at check time (committed with the same accepted
  response); `restartNow()` → plugin `relaunch()`.
- UpdatePill: the available-phase pill CLICK toggles an expansion —
  ONE persistent element growing in place (rule 0), showing
  "v{X} — what's new" + the notes (plain text, scrollable, max-height)
  + "Update now" + "Not now". Downloading/ready keep today's shapes,
  except ready's label becomes a "Restart now" BUTTON invoking
  restartNow(); if relaunch fails (it shouldn't), the calm quit-and-
  reopen line returns.
- Empty/missing notes → the window shows "No notes for this release."
  (never an empty pane).

## 3. ACs
1. Check captures notes with the accepted response (stale-response
   discipline preserved — notes never pair with a different version's
   pill).
2. Pill click (available) expands in place: version + notes rendered;
   "Not now" collapses without dismissing the update; the dismiss ×
   still fully dismisses per-version.
3. "Update now" starts the install (same downloadAndInstall path,
   dimming preserved).
4. Ready phase: "Restart now" invokes the process plugin's relaunch;
   a relaunch failure falls back to the quit-and-reopen text calmly.
5. Missing notes → the honest placeholder.
6. Rule 0 throughout: expansion/collapse animate the SAME node
   (identity-pinned); nothing blinks.
7. The auto-check opt-in gating is untouched (AC1 privacy tests stay
   green as-is).

## 4. Test map
| AC | Test |
|---|---|
| 1 | updateStore: notes committed only with accepted response (seq race reuse) |
| 2 | UpdatePill: expand shows notes; Not-now collapses, update persists |
| 3 | UpdatePill: Update-now → downloadAndInstall |
| 4 | UpdatePill: Restart-now → relaunch mock; rejection → fallback text |
| 5 | UpdatePill: null body → placeholder |
| 6 | UpdatePill: node identity across expand/collapse/phases |
| 7 | existing App/pill suites unchanged |
