# Spec: My Patterns — the practice-data bank entry (#419 S3, E2)

## 1. Summary
The bank's last resting entry goes live: "My patterns" lists cells
derived from the machine's own practice data — the licks you lifted,
the openers you began, the drills you rowed — and one tap drops a
pattern back into the builder as a Notes item. The founder's flagship
idea: the app remembers what YOUR hands actually played.

## 2. Contract
- `my_patterns` command → `Vec<MyPatternDto { label, offsets: Vec<i8>,
  times_practiced, last_tonic }>`: reads the exercise log (rows carry
  the full VariationSpec as spec_json — replayable by design), keeps
  rows whose spec has a CELL, dedups by cell (identical offsets = one
  pattern, practiced N times), orders most-recent-first, caps at 6.
  Unparseable spec_json rows are skipped calmly.
- Labels stay honest and human: "your 5-note cell · 3×, last in A"
  (count from dedup, key from the latest row's tonic).
- Panel: the "My patterns" section replaces the last COMING_SOON chip —
  pattern chips add `{type:"notes", offsets}` (the S1 wire, unchanged);
  EMPTY state: "play and lift a few things first — your patterns
  appear here" (honest, not hidden).
- Refresh: fetched when the Openers panel opens (not live-updating —
  a pattern earned mid-session appears next open).

## 3. ACs
1. Log rows with cells → deduped, counted, recent-first, capped at 6;
   rows without cells and unparseable spec_json skipped calmly.
2. Tapping a pattern adds the exact Notes{offsets} item and previews
   (the S1 purity/wire pins keep passing).
3. Empty log → the honest empty state; no chips, no crash.
4. Labels carry count and last key, derived from log data only.
5. The command never errors the panel: store failures → empty list.

## 4. Test map
| AC | Test |
|---|---|
| 1,4,5 | commands my_patterns_impl unit (seeded log: dedup, order, cap, REAL garbage rows incl. score-practice-shaped, single-note skip; the unreadable-store branch is code-covered with a warn — not induceable on the in-memory test store, stated honestly) |
| 2 | OpenersPanel test: pattern chip tap → preview_opener with Notes offsets |
| 3 | OpenersPanel test: empty → the honest line |
