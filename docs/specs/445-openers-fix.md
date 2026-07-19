# Spec: Openers — the reachable panel + the indigo restyle (#445 pt 6a + palette follow-up)

## 1. Summary

Two founder reports on the same surface (`OpenersPanel.tsx`, mounted in
`PracticeSession`'s free-play branch as `{listenToRoom ? null : <OpenersPanel />}`):

1. **The dead zone.** "the openers green things is cool but I cant use it..
   its not letting me actually click its just a static UI thats useless."
2. **The palette.** "why is it green.. thats not our UI color palette.. pick a
   sleeker color."

Part 1 is a real-browser layout defect (behavior). Part 2 is paint.

## 2. Root cause — the dead zone (measured, not guessed)

The panel's logic is sound: all 24 existing tests click bank buttons and Begin
in jsdom and pass. jsdom applies no layout, so it cannot see the actual defect,
which is **layout, not an overlay and not pointer-events**.

Reproduced against the production web bundle in headless Chromium at the real
Tauri window size (1200×800), driving selector → session → open Openers → tap
one note:

| element | rect (viewport px) | below the 800px fold? |
|---|---|---|
| pitch-display | top 86 | no |
| openers-panel | top 190, **bottom 1164** | — |
| opener-chips (the feedback for a tap) | top 889 | **yes** |
| opener-preview (live CellStaff) | top 921 | **yes** |
| opener-begin (the primary action) | top 1111 | **yes** |

`elementFromPoint` at every *visible* control returned the control itself
(`covered: false`) — so nothing overlays it and no `pointer-events` is disabled.
The mechanism is:

- The open panel renders an **unbounded-height card** (no `max-h`, no internal
  scroll) inside the free-play column, which is vertically centered
  (`justify-center`) and sits below the live `PitchDisplay`.
- Once **any** item is added, the added-item chips **and** the live CellStaff
  preview render at the very bottom of the card, pushing the card to ~1164px in
  an 800px viewport.
- So the **only visible feedback for a click** (chips + preview) and the
  **primary action** (Begin) both land **below the fold**. The page technically
  scrolls (≈474px of overflow), but macOS/Tauri overlay scrollbars are hidden
  until a scroll gesture, so there is **no scroll affordance**.
- Net effect exactly matches the report: the founder clicks the visible top rows,
  sees nothing change (feedback is off-screen), cannot find or reach Begin, and
  reads the surface as an unresponsive static wall.

This is a hit-area/reachability defect driven by an unbounded panel taller than
the viewport — not stacking or pointer-events.

## 3. The fix — bound the panel, scroll it internally, pin Begin

Frontend-layout-only. Same structure, same testids, same behavior:

- The open panel root becomes a bounded, internally scrollable card:
  `flex max-h-[70vh] flex-col overflow-y-auto`. The card never exceeds the
  viewport; its own bounded scroll region is a visible affordance (the scrollbar
  appears within the card on hover/scroll), and the bank content scrolls
  **inside** the card instead of pushing the page.
- The **Begin** button becomes a `sticky bottom-0` footer inside that scroll
  container, so it is **always visible and clickable** at the bottom edge of the
  card regardless of how tall the bank/preview grows or how short the window is.

Verified with the same probe after the fix (1200×800, one item added): panel
top 190 / **bottom 750** (fully on-screen), Begin top 697 / bottom 733,
`beginVisibleAndClickable: true`, `panelScrollable: true`, and a real
trial-click on Begin succeeds without any programmatic page scroll.

No `.rs` changes; no DTO changes; no store changes.

## 4. The palette contract — teal → indigo

The app's language is gray-900 base with **indigo** accents (ExplorePanel chips
`bg-indigo-600/90`; `PieceMatchChip` `border-indigo-700 bg-indigo-950/60
text-indigo-100`). The entire Openers surface — the invitation button and the
full builder — moves off teal into that family, sleeker/less saturated:

| role | class |
|---|---|
| panel root | `bg-indigo-950/30 border-indigo-900` |
| invitation button | `border-indigo-800 bg-indigo-950/40 text-indigo-200 hover:bg-indigo-900/50` |
| panel title / close | `text-indigo-200` / `text-indigo-400/70 hover:text-indigo-200` |
| section headers | `text-indigo-300/70` |
| item chips / note keys | `bg-indigo-800/60 text-indigo-100 hover:bg-indigo-700` |
| direction (active) | `bg-indigo-600 font-semibold text-white` |
| inputs | `border-indigo-800 bg-indigo-950/60 text-indigo-100 placeholder:text-gray-500` |
| empty-state text | `text-gray-500` |
| added-item chips | `bg-indigo-800 text-indigo-100 hover:bg-red-900/60` (red remove hover kept) |
| Save | `bg-indigo-600/80 hover:bg-indigo-500` |
| Begin | `bg-indigo-600 hover:bg-indigo-500` |

Scope guard: only the Openers surface changes. The lone remaining `teal-` in the
frontend is `CoachingTipPanel`'s "technique" category badge — a non-openers
surface, deliberately untouched.

## 5. Acceptance criteria

1. **Reachability contract (dead-zone fix).** The open panel root declares a
   bounded height (`max-h-[…]`) and `overflow-y-auto`; the Begin button declares
   `sticky bottom-0`. Removing any of these — i.e. reverting to the unbounded
   wall or un-pinning Begin — fails the test (that revert is exactly what
   recreates the founder's dead zone). Structural pin because jsdom cannot
   measure layout; the layout itself is verified by the headless-browser probe
   in §2/§3.
2. **Palette contract.** No `teal-` class appears in the rendered output of
   either the closed invitation or the open panel.
3. **Behavior unchanged.** All 24 existing OpenersPanel tests stay green with
   unchanged semantics (taps compose items, preview is pure, Begin commits and
   hands off, refusals surface calmly).

## 6. Test map

| AC | Test |
|---|---|
| 1 | open panel root has `overflow-y-auto` + a `max-h-` class; Begin has `sticky` + `bottom-0` |
| 2 | rendered closed invitation and open panel contain no `teal-` substring |
| 3 | the existing 24-test suite, untouched |
