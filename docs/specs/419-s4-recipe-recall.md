# Spec: Recipe persistence + recall (#419 S4, E5)

## 1. Summary
Openers become rememberable. Two features: **saved recipes** (name the
current builder — items + direction — and get it back with a tap) and
**yesterday's opener** (replay the most recent begun opener EXACTLY —
from the STORED seed in the exercise log, never a recomputed hash).

## 2. The stored-seed law (the S1 review rule)
The per-recipe seed is a cell hash that is only promised stable
*within a session* ("between the preview and the Begin that follows
it" — commands.rs). Replaying yesterday's opener by re-hashing would
silently change the variations whenever the hash input or function
drifts across releases. So recall NEVER recomputes: it replays with
the `seed` column the log row already carries, plus the row's stored
cell, tonic, and direction.

Corollary: the log must know the direction. `exercise_log` gains a
nullable `direction` column (guarded ALTER for existing installs);
`begin_opener` writes it. Rows that predate the column — or non-opener
rows — simply don't offer recall (honest absence, not a guess).

## 3. Contract
- **New table `starter_recipes`**: `id` PK, `name` TEXT, `items_json`
  TEXT (serde `Vec<StarterItem>`), `direction` TEXT, `created_at`.
  List is most-recent-first. A row whose `items_json` no longer parses
  is skipped on list (the My Patterns garbage-tolerance rule), never an
  error.
- **Commands**: `save_opener_recipe(name, items, direction)` →
  RecipeDto; `list_opener_recipes()` → `Vec<RecipeDto>`;
  `delete_opener_recipe(id)`; `recall_last_opener()` →
  `Option<LastOpenerDto{label, cell, tonic, direction, seed}>` (newest
  `source="opener"` log row WITH direction present);
  `begin_opener_recall()` → ExploreDto — runs `start_explore_cell`
  with the STORED cell/tonic/direction/seed and commits (logs a fresh
  opener row, same discipline as `begin_opener`).
- **Panel**: a "Saved recipes" strip (name chips, tap → repopulate
  builder items + direction → existing preview flow; per-chip delete)
  and a "Yesterday's opener" chip (label from the log row; tap →
  `begin_opener_recall`). Honest empty states for both. Save = a name
  field + button, disabled while the builder is empty.
- Frontend stays semantic: items/direction round-trip as data; all
  seeds, hashing, and replay live in Rust.
- Offline-first: no network. Persistence is the existing SQLite store.

## 4. ACs
1. Save → list round-trips name, items, direction (most-recent-first);
   delete removes; corrupt `items_json` rows are skipped on list.
2. Tapping a saved recipe repopulates the builder (items + direction)
   and previews via the EXISTING preview path — no new generation
   logic frontend-side.
3. `recall_last_opener` returns the newest opener row's label + stored
   seed/cell/tonic/direction; returns None when no opener row carries
   a direction (pre-S4 rows, empty log, non-opener rows).
4. `begin_opener_recall` replays with the STORED seed — pinned by a
   test whose log row carries a seed that is NOT the cell hash, and
   whose replayed spec differs from the freshly-hashed one (the
   recompute mutant must die).
5. `begin_opener` now logs direction; the recall chip only exists
   because of it (new rows recallable, old rows honestly absent).
6. Panel: both sections render, tap flows work, empty states honest,
   save disabled with an empty builder.

## 5. Test map
| AC | Test |
|---|---|
| 1 | store: recipe save/list/delete round-trip + garbage row skipped |
| 2 | OpenersPanel: tap recipe → builder repopulated + preview invoked |
| 3 | commands: recall_last_opener stored-row happy + None cases |
| 4 | commands: replay uses STORED seed ≠ cell hash (spec differs from re-hash) |
| 5 | commands: begin_opener writes direction; recall sees it |
| 6 | OpenersPanel: sections, save flow, empty states |
