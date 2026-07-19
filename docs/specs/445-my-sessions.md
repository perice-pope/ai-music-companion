# Spec: My Sessions — the history that already existed, made reachable and real (#445 pt 8, F3)

## 1. What's actually missing (root-caused)
The History page, session list, stats, get_session_history and
get_session_detail commands ALL exist — but (a) the only path to the
page is a back-link inside Connections & Privacy (the founder has never
seen it), (b) tapping a session card fetches the detail into
`selectedSessionDetail` and NOTHING RENDERS IT, and (c) History has no
way back to the selector. F3 is wiring, not architecture.

## 2. Contract
- **Entry**: the selector screen gains "My sessions" (testid
  open-history) → `goToHistory()`.
- **Exit**: History gains "← Back to practice" → `returnToSelector()`.
- **Detail**: tapping a card opens the PAST RECAP — a read-only view of
  the stored `StoredSessionDto` (date + duration + instrument header,
  overall_assessment, strengths / areas_to_improve /
  next_session_suggestions lists, phrase count; score_summary block
  when present — render ONLY what the stored recap carries, invent
  nothing). "← All sessions" (and only that) returns to the list;
  `clearSelectedSession()` added to historyStore.
- Detail replaces the list while open (focused reading, founder's
  no-scroll taste); the list state (filters, scroll) is otherwise
  untouched.
- Read-only over already-persisted data; no new commands, no schema
  changes, no network.

## 3. ACs
1. Selector shows "My sessions"; tapping lands on History.
2. History's back button returns to the selector.
3. Tapping a session card renders the stored recap detail (assessment,
   lists, phrase count, duration); "← All sessions" returns to the
   list with the detail cleared.
4. A recap with score_summary shows the score block; one without shows
   no score block (honest absence).
5. A get_session_detail failure surfaces the store's calm error, list
   intact.

## 4. Test map
| AC | Test |
|---|---|
| 1 | PracticeShell: My sessions → screen history |
| 2 | History: back → selector |
| 3 | History: card tap → detail fields; back → list |
| 4 | detail with/without score_summary |
| 5 | History: detail fetch rejects → error shown, list remains |
