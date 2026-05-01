# Story — Score Mode (MusicXML import + OSMD rendering + cursor follow): Design Proposal

**Status:** Approved — implementation-ready
**Author:** Design proposal generated for review
**Target story:** *No GitHub issue yet — create one before kicking off PR 0. Suggested title: "Phase 1: Score Mode — MusicXML import, OSMD render, cursor follow." Suggested labels: `story`, `phase-1`.*

**Revision notes (2026-05-01):**
- ✅ **Library kept** (§4, PR 0). One-shot uploads rejected — practising the same piece across multiple sessions is the central use case.
- ✅ **MIDI is a first-class import format** (§3, §6). The Rust parser already supports it; the MIDI→MusicXML emitter we'd build anyway is reused later by Phase 2's audio-to-MIDI pipeline.
- ✅ **Cursor strategy: phrase-granularity in PR 2, per-event smoothing in PR 3** (§2 cursor delivery, §7 PR slicing). Keeps PRs under 600 lines and lets us validate the cursor-on-phrase feel before investing in higher-frequency IPC.

**Dependencies landed:**
- Free Play loop (`story-14`) — `practiceStore`, `PracticeShell` screen router, `start_practice_session`, `end_practice_session`, `switch_instrument`, recap.
- Score parsing (`crates/brain/src/score/`) — `parse_musicxml_str`, `parse_musicxml_str_part`, `list_parts`, MIDI parser. Native Rust, no Python/PyO3 (architecture v2 line 141 punts `partitura` to Phase 2 — we already cleared that).
- Score follower (`crates/brain/src/follower.rs`) — Online DTW, ~3ms median alignment per architecture v2 latency budget. Optimised in v1.17.3.
- Score follower wiring into `PhraseAggregator::set_score_follower` (PR #85, v1.8.0).
- `PhraseSummary.score_position: Option<ScorePosition>` (PR #94).

---

## 1. Product framing

### What Score Mode is (user's POV)

A musician opens Musa, picks an instrument from the selector, and now sees a second option next to **Start Practice**: **Practice with score**. They click it. A small panel slides in:

- Drag a `.musicxml`, `.mxl`, `.xml`, or `.mid` file onto the panel, **or**
- Click "Choose file…" to pick one, **or**
- Click any score they've already loaded from the small library on the right.

Once they pick a score, the screen swaps to a session view with the sheet music rendered across the top — same Tauri webview, same window. They click **Start**. As they play, a thin highlight cursor moves through the notes in time with what the mic hears. The pitch trace they used in Free Play is still there in a corner. Phrase tips still appear in the side panel; recaps still arrive at the end. The difference is that the LLM now knows *what they were trying to play*, so the recap can say "your second phrase, measure 5, lost direction at the apex" instead of "your second phrase had a flat shape."

When the session ends, the score sticks around in the library — they don't have to re-upload it tomorrow.

### Why Score Mode now

- Phase 1's roadmap (`architecture-v2.md` §9) explicitly lists "Free Play mode (no score) **and** Score Mode (MusicXML import)" as joint Phase 1 deliverables. Free Play is shipped and demonstrably working as of v1.20.0; Score Mode is the last major gap before Phase 1 closeout.
- It is **the moment our differentiator becomes legible**. Without a score, the LLM coach is drawing inferences from pitch and dynamics alone. With a score, it can refer to specific measures, expression markings, and composer intent. The recap goes from impressive to teacher-quality.
- It **unlocks downstream features cheaply**. Phase 2's OMR import (photo of sheet music → MusicXML) and YouTube import (audio → MIDI → MusicXML) both terminate in this loader. Building Score Mode now is also building 80% of Phase 2's import wizard plumbing.
- The Rust parsing + score-follower stack already exists. The unbuilt slice is **mostly the UI** — file selection, OSMD rendering, cursor sync — plus a small library shell.

### Three UX decisions that reinforce "coach, don't judge"

1. **The cursor highlights, it does not grade.** A single thin coloured stripe (single hue, neutral) shows where the player is. We do **not** colour notes green/yellow/red as the cursor passes them. The same wedge from Free Play applies: any per-note paint is the start of note-grading.
2. **Tempo follows the player, not the score.** The score-follower is online DTW with ±20% tempo tolerance. We do not show "you're 12 bpm under tempo" anywhere in the UI. If the musician slows down, the cursor slows down with them. Tempo coaching, if it ever comes, is recap-only — never live.
3. **Loading a score is not gated by parse perfection.** If MusicXML import fails or the file has weird encoding, the user sees a **calm fallback**: "We couldn't read this score. You can still practice without it." Free Play remains a single click away. We never strand a user inside a broken score upload.

Things we are rejecting in Score Mode:

- Per-note red/green pass-fail.
- A "play to the metronome" mode (out of scope; Phase 2 with backing tracks).
- "Wait for the right note before advancing" — the follower is forgiving by design; rigid waiting infantilises the player.
- Fancy notation editing in-app.
- A "performance score" at the end ("87% accuracy"). The recap reads like a teacher's note, same as Free Play.

---

## 2. Frontend architecture

### Routing changes

`PracticeShell` already routes on a `screen` enum. We add two screens:

```typescript
type AppScreen =
  | "selector"
  | "score-picker"   // NEW — pick or upload a score before starting
  | "session"
  | "recap"
  | "history";
```

The instrument selector grows a small **second CTA**: `[Start Practice]` (free play, existing) + `[Practice with score]` (new — routes to `score-picker`). Selecting an instrument enables both. From `score-picker` the user can either back out or proceed; both `Practice with score` (with a score loaded) and the existing `Start Practice` (no score) drop into the same `session` screen — the screen knows whether a score is loaded by reading `practiceStore.activeScore`.

### Component tree (delta over story-14)

```
<PracticeShell>
  ├── <InstrumentSelector>       // EXISTS — gains second CTA
  ├── <ScorePicker>              // NEW
  │    ├── <ScoreDropZone>       // NEW — file drag/drop + Browse button
  │    └── <ScoreLibrary>        // NEW — list of previously-loaded scores
  ├── <PracticeSession>          // EXISTS — gains optional <ScoreView> region
  │    ├── <ScoreView>           // NEW — OSMD canvas + cursor; only when activeScore != null
  │    ├── <PitchDisplay>        // EXISTS, unchanged
  │    ├── <SessionTimer>        // EXISTS
  │    ├── <CoachingTipPanel>    // EXISTS
  │    └── <EndSessionButton>    // EXISTS
  ├── <SessionRecap>             // EXISTS — gains "measure N" anchors when score was loaded
  └── <PracticeHistory>          // EXISTS
```

Three new components: `ScorePicker`, `ScoreDropZone`, `ScoreLibrary`, `ScoreView`. The first three are pure-React form/list UI; the fourth wraps OSMD.

### OSMD integration

OSMD (`opensheetmusicdisplay` on npm) is the chosen renderer per architecture v2 §7. It's a pure-JS library that takes a MusicXML string and renders SVG. We do **not** render in Rust.

```typescript
// apps/desktop/src/components/ScoreView.tsx (sketch)
import { useEffect, useRef } from "react";
import { OpenSheetMusicDisplay } from "opensheetmusicdisplay";

interface Props {
  musicXml: string;          // raw XML string from the loaded score
  cursorPosition: ScorePosition | null;  // measure_number + beat, fed by store
}
```

- **Render** with `osmd.load(musicXml)` then `osmd.render()` once on mount.
- **Cursor**: OSMD ships an `osmd.cursor` API — `cursor.show()`, `cursor.next()`, `cursor.update()`, plus an iterator that maps measure+beat to a note index. We map `ScorePosition { measure_number, beat }` → cursor iteration. The cursor moves on each new `phrase-detected` event (cheap — phrase boundaries are seconds apart, not frames). For sub-phrase cursor smoothness we may also tap `audio-event` and call `osmd.cursor.update()` lazily.
- **Sizing**: OSMD reflows on container resize. We wrap it in a fixed-height scrollable region so the cursor's "follow into view" behaviour works.
- **Worker**: OSMD runs synchronously on the main thread. A 30-bar piano score parses in <100ms. We don't need a worker. If we hit a 200-bar orchestral score later, we revisit.

Bundle cost: OSMD is ~600KB minified, 130KB gzipped. Acceptable for a desktop app. We code-split it: import `OpenSheetMusicDisplay` lazily so the bundle for the first run (free play) doesn't pull it in.

### Zustand store extension (`practiceStore`)

```typescript
export interface ActiveScore {
  id: string;                  // ScoreId from Rust (UUID)
  title: string;
  composer: string | null;
  durationMeasures: number;
  partIndex: number;           // 0 for single-part scores
  musicXml: string;            // raw XML for OSMD
  // The structured ScoreModel is held in Rust; we only need title+xml on TS
}

export interface ScoreLibraryEntry {
  id: string;
  title: string;
  composer: string | null;
  added_at: number;            // ms since epoch
  last_practiced_at: number | null;
  source_filename: string;     // for display ("haydn-trumpet.musicxml")
}

interface PracticeStateAdditions {
  activeScore: ActiveScore | null;          // set on score load, cleared on session end
  scoreLibrary: ScoreLibraryEntry[];        // populated by listScores()
  cursorPosition: ScorePosition | null;     // driven by phrase-detected/score-position-updated events

  loadScoreFromFile: (path: string) => Promise<void>;
  loadScoreFromId: (id: string) => Promise<void>;
  refreshScoreLibrary: () => Promise<void>;
  deleteScore: (id: string) => Promise<void>;
  clearActiveScore: () => void;             // when leaving score-picker without picking
}
```

`activeScore` and `cursorPosition` are session-scoped. Library state is global. We **do not** persist `activeScore` across page reloads — restarts always begin at `selector`.

### `start_practice_session` extension

`startSession` gains an optional second argument: `scoreId?: string`. When provided:

- The Rust side loads the parsed `ScoreModel` from the library, instantiates a `ScoreFollower`, and calls `PhraseAggregator::set_score_follower(...)` (already exists from PR #85).
- Frontend leaves `activeScore` set; cursor state begins ticking.

When omitted: identical to today's free-play path.

### Cursor delivery

Two options, in order of preference:

1. **Cursor derived from `phrase-detected` events.** When a phrase closes, `PhraseSummary.score_position` (added in PR #94) carries the measure+beat. The frontend updates the cursor on each phrase. **Cost: zero new IPC.** Granularity: one cursor jump per phrase (~few seconds). Smooth enough for "where am I" feedback during practice; not ideal for live note-by-note tracking.
2. **Per-event cursor stream.** A new `score-position-updated` event emits the follower's current position alongside (or instead of, sampled) `audio-event`. Frontend updates OSMD cursor on each. **Cost: new event, more IPC traffic, but cheap (small payload, ~50 events/sec already proven scalable in our IPC layer per the audio-event throughput we measured).**

**Recommendation: ship (1) in PR 1, add (2) in PR 2 if user feedback wants tighter tracking.** Phrase-granularity cursor is enough to make Score Mode feel real; per-event tracking is polish.

### Library UI

`<ScoreLibrary>` is a 2-column grid of cards (title, composer, last-practiced ago) with delete affordance. Empty state: "Drop your first score to get started." Sorted by `last_practiced_at desc`, with un-practiced scores at the top.

We do **not** add tagging, folders, search, or favourites. If the library grows past ~20 scores in dogfood, we revisit.

---

## 3. Backend wiring (Rust side)

### New Tauri commands

```rust
/// Validate, parse, persist, and return a library entry for a score file.
/// `path` is the OS file path the user picked or dropped.
/// On success the file is copied into the app's score directory; the
/// originating path is no longer relied on.
#[tauri::command]
async fn import_score(
    path: String,
    state: State<'_, AppState>,
) -> Result<ScoreLibraryEntry, String>;

/// List every score in the library, ordered by `last_practiced_at desc`
/// then `added_at desc`.
#[tauri::command]
async fn list_scores(state: State<'_, AppState>) -> Result<Vec<ScoreLibraryEntry>, String>;

/// Load a score by id and return the structured model + raw MusicXML
/// the frontend feeds to OSMD.
#[tauri::command]
async fn get_score(
    id: String,
    state: State<'_, AppState>,
) -> Result<LoadedScore, String>;

/// Permanently remove a score (and its file copy) from the library.
#[tauri::command]
async fn delete_score(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String>;
```

`LoadedScore` is the IPC DTO:

```rust
#[derive(Serialize, Deserialize)]
pub struct LoadedScore {
    pub entry: ScoreLibraryEntry,
    pub model: ScoreModel,        // already serializable
    pub music_xml: String,        // raw XML for OSMD
}
```

`start_practice_session` gains an optional `score_id: Option<String>` parameter. When `Some`:

1. Look up the score in the library.
2. Build a `ScoreFollower::new(score_model)`.
3. After `PhraseAggregator` is constructed, call `set_score_follower(...)`.
4. Update the score's `last_practiced_at`.

Multi-part scores: `import_score` accepts a `part_index: Option<usize>` (default 0). For now the score-picker UI shows a part-list selector when `list_parts()` returns >1. Power users can switch parts later by re-importing — we don't ship per-segment part switching in this story.

### Score storage on disk

Scores live under `dirs::data_dir() / "ai-music-companion" / "scores" /`:

```
<id>.xml          ← canonical MusicXML (we re-emit if input was .mxl)
<id>.meta.json    ← title, composer, source_filename, added_at, last_practiced_at, part_index
```

For `.mxl` (compressed MusicXML — a zip): unzip in-memory, extract the `META-INF/container.xml`-pointed XML, store the uncompressed XML on disk. `roxmltree` already handles uncompressed XML; the unzip step uses `zip` crate (~100 lines of glue).

For `.mid`: parse with the existing `crates/brain/src/score/midi.rs`, then **emit MusicXML** before storing. (Rationale: the rest of the stack — OSMD, follower — speaks MusicXML / `ScoreModel`. MIDI is an input format, not a storage format.) The Rust side already builds a `ScoreModel` from MIDI; we add a small `score_model_to_musicxml(&ScoreModel) -> String` exporter. That exporter is also Phase 2 leverage (yt-dlp → basic-pitch → MIDI → MusicXML pipeline ends in the same emitter).

**Index**: a single SQLite table `scores` (id, title, composer, source_filename, added_at, last_practiced_at, part_index). Same db file as `sessions.db` already in use — one extra migration. No filesystem scan on every list — the index is the source of truth for the library list view.

### New Tauri events

| Event | Payload | When | Consumer |
|---|---|---|---|
| `score-position-updated` | `ScorePosition` | (Phase 2 of this story) per follower alignment, throttled to 10Hz | `practiceStore.cursorPosition` |
| *(none in PR 1)* | — | — | Cursor derives from `phrase-detected.score_position` |

We do **not** add `score-loaded` or `score-imported` events. Imports return synchronously on the command response; library refresh is explicit (`list_scores` after a mutation).

### State machine

The `Idle → Starting → Listening → Ending` machine from story-14 is unchanged. Score loading happens **before** `Starting`. From the AppState perspective, a session with a score and a session without a score are identical except that the `ActiveSession` carries an `Option<Arc<ScoreModel>>` and the aggregator is set up with a follower.

---

## 4. Data model

### `ScoreLibraryEntry` (IPC + DB row)

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct ScoreLibraryEntry {
    pub id: ScoreId,                       // uuid_newtype, like SessionId
    pub title: String,
    pub composer: Option<String>,
    pub source_filename: String,           // e.g. "haydn-trumpet.musicxml"
    pub added_at: DateTime<Utc>,
    pub last_practiced_at: Option<DateTime<Utc>>,
    pub part_index: usize,                 // 0 for single-part scores
    pub duration_measures: usize,          // for "this score is ~30 bars" UI hint
}
```

### Sessions ↔ scores

Existing `sessions` table grows one nullable column: `score_id: Option<ScoreId>`. The `StoredSession` IPC DTO grows a matching field. `SessionRecap.measure_anchors: Vec<MeasureAnchor>` is a future extension (Phase 2 / coaching follow-up); not in this story.

### Migration

One additive migration:

```sql
CREATE TABLE IF NOT EXISTS scores (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    composer TEXT,
    source_filename TEXT NOT NULL,
    added_at TEXT NOT NULL,
    last_practiced_at TEXT,
    part_index INTEGER NOT NULL DEFAULT 0,
    duration_measures INTEGER NOT NULL DEFAULT 0
);

ALTER TABLE sessions ADD COLUMN score_id TEXT REFERENCES scores(id) ON DELETE SET NULL;
```

`ON DELETE SET NULL` — deleting a score should not destroy the session history that referenced it. The recap's text already mentions the title; the link is a nice-to-have.

---

## 5. Testing strategy

### Rust side

| Test | What it covers |
|---|---|
| `commands::import_score_persists_musicxml` | Pass a fixture file path → score copied to data dir, library row inserted, returned entry has correct title/composer. |
| `commands::import_score_rejects_unsupported_format` | `.txt` or `.pdf` returns a clear error; nothing persisted. |
| `commands::import_score_handles_mxl_zip` | Compressed MusicXML round-trips; stored copy is uncompressed XML. |
| `commands::import_score_with_midi_emits_musicxml` | `.mid` input → MusicXML on disk; OSMD-loadable. |
| `commands::list_scores_orders_by_recency` | Three imports + one practiced → practiced one first, others by `added_at desc`. |
| `commands::get_score_returns_xml_and_model` | Round-trip the imported file; `LoadedScore.music_xml` parses, `LoadedScore.model.measures.len() > 0`. |
| `commands::delete_score_clears_file_and_index` | After delete, file gone, library row gone, sessions that referenced it survive with `score_id: None`. |
| `commands::start_session_with_score_attaches_follower` | `start_practice_session(score_id: Some(...))` → the active session's `PhraseAggregator` has a follower set; phrase summaries carry `score_position`. |
| `score_to_musicxml::midi_roundtrip` | Parse a small MIDI fixture, emit MusicXML, reparse → equivalent `ScoreModel`. |

These extend `apps/desktop/src-tauri/src/commands.rs` and (for the MIDI emitter) `crates/brain/src/score/musicxml.rs`. Existing parser/follower tests in `crates/brain/src/score/` and `follower.rs` are not duplicated.

### Frontend side (Vitest + RTL)

| Test | What it covers |
|---|---|
| `ScorePicker.test` — drop zone accepts `.musicxml` files | Mock invoke for `import_score`, drop a fake file, assert resulting `activeScore` set in store. |
| `ScorePicker.test` — drop zone rejects `.pdf` with calm message | No invoke fired, error copy visible. |
| `ScoreLibrary.test` — renders entries from store | Seed `scoreLibrary`, assert each card. |
| `ScoreLibrary.test` — delete prompts confirmation | Click delete, see confirm, confirm, mock invoke fires, store updates. |
| `ScoreView.test` — renders on mount | Mock `OpenSheetMusicDisplay` constructor, pass musicXml prop, assert `.load(xml).render()` called. |
| `ScoreView.test` — cursor advances on phrase-detected score_position | Mount with cursor at measure 1, push phrase with `score_position.measure_number = 3`, assert OSMD cursor at iteration matching measure 3. |
| `practiceStore.test` — `loadScoreFromFile` happy path | invoke succeeds → `activeScore` set, `screen` advances to session-ready. |
| `practiceStore.test` — `clearActiveScore` from score-picker | Backing out leaves no residual state. |
| `PracticeShell.test` — routes to `score-picker` from selector CTA | Click "Practice with score" → screen flips. |
| `SessionRecap.test` — score-anchored phrasing when activeScore was set | Recap text containing measure references survives serialization. |

### Integration / smoke

We extend the synthetic-`AudioSource` integration test from story-14 (§5) to include a score-loaded variant: a fixture MusicXML + a synthetic event stream → assert that emitted `PhraseSummary`s carry non-`None` `score_position` whose `measure_number` advances monotonically.

OSMD itself is a third-party render library — we trust it. We do not run a screenshot diff on its output.

### AC → test mapping

| AC | Test(s) |
|---|---|
| User can import a `.musicxml` / `.xml` / `.mxl` / `.mid` file | `commands::import_score_*` (4 tests) |
| Imported scores show up in the library | `commands::list_scores_orders_by_recency` + `ScoreLibrary.test` |
| Selecting a score and starting a session feeds the follower | `commands::start_session_with_score_attaches_follower` |
| Cursor follows the player's position through the score | `ScoreView.test` cursor advance |
| Score Mode session produces a recap that references measures | `SessionRecap.test` measure phrasing + the wired LLM prompt change |
| Deleting a score removes it but preserves session history | `commands::delete_score_clears_file_and_index` |
| `clippy --deny warnings` clean | CI |
| `pnpm lint` + `pnpm test` pass | CI |

---

## 6. Cut lines — what we are NOT doing in this story

- **OMR import (photo / PDF)** — Phase 2. Audiveris dependency, separate story.
- **YouTube / audio-to-MIDI import** — Phase 2.
- **MusicXML editing** — out forever; we are not a notation editor.
- **Multi-instrument score handling (orchestral parts switching mid-session)** — out. Pick one part on import.
- **Tempo display / metronome / "you're at 92 bpm"** — out. Tempo is implicit in the cursor.
- **Repeats / DS al Coda / 1st-2nd endings** — the parser flattens to linear measures (verify with the parser; if it preserves repeat marks, we unfold them at import time). We do NOT attempt to interpret performance directions live.
- **Click-to-jump-to-measure** in the score view. Out. The follower aligns automatically; manual seeking is a future feature.
- **Score-loaded LLM coaching prompt rewrites** — minimal in this story. The `SessionContext` gains a `score_title` field and the LLM is told "the user is playing X." Recap-time prompts gain access to `phrase_summaries[i].score_position.measure_number` so the LLM can reference measures naturally. Anything more (passage-level coaching, expression-marking-aware feedback) is a coaching-engine follow-up, not a Score Mode story.
- **Backing tracks / play-along audio** — Phase 2 (Demucs).
- **Saving annotations on a score** ("I've been struggling with this passage") — Phase 2/3.

---

## 7. PR slicing

Target: 4 PRs, each <600 lines ideally, <800 max, each testable and mergeable alone.

### PR 0 — Score library backend (~600 lines)

**Ships:**
- `ScoreId` newtype + `ScoreLibraryEntry`.
- SQLite migration for the `scores` table + `sessions.score_id` column.
- `crates/brain` exporter: `score_model_to_musicxml(&ScoreModel) -> String` (used by MIDI import and as the "always-store-as-XML" canonicaliser).
- New Tauri commands: `import_score`, `list_scores`, `get_score`, `delete_score`. All four behind their tests.
- `.mxl` zip handling.
- No UI yet.

**Merge criterion:** All commands tests green. Migration idempotency test green. Existing session/store tests still pass (additive migration).

**Why first:** Frontend can't render anything without the data layer. PR 1 imports against a live backend; mocks become unnecessary.

### PR 1 — Score Picker UI + library list + load-into-session (~650 lines)

**Ships:**
- New screen `score-picker` in `PracticeShell`.
- `<ScorePicker>`, `<ScoreDropZone>`, `<ScoreLibrary>` components.
- `practiceStore` extensions: `activeScore`, `scoreLibrary`, the four actions.
- `start_practice_session` accepts optional `score_id`.
- The "Practice with score" CTA on the instrument selector.
- Vitest tests for components + store.
- Rust test: `start_session_with_score_attaches_follower`.

**Behaviour:** You can import a score, see it in the library, pick it, start a session. The session view still doesn't show the score (no OSMD yet). The follower is attached and `phrase-detected` events carry `score_position`. The recap will already reference measures because `PhraseSummary.score_position` reaches the LLM via the recap prompt.

**Merge criterion:** ACs around "import + library + load + start" green. Free Play (no score) path still works unchanged — explicit regression test.

### PR 2 — `<ScoreView>` + OSMD render + phrase-granularity cursor (~500 lines)

**Ships:**
- Lazy import of `opensheetmusicdisplay`.
- `<ScoreView>` component: mount, `osmd.load(xml).render()`, sized container, scroll-into-view on cursor change.
- Cursor wiring from `practiceStore.cursorPosition` (set by `phrase-detected` payload's `score_position`).
- `<PracticeSession>` shows `<ScoreView>` above `<PitchDisplay>` when `activeScore != null`.
- Vitest tests (mock OSMD constructor).

**Merge criterion:** Score visibly renders on the session screen. Cursor jumps to the correct measure on each phrase boundary in a manual smoke test. Component tests green.

### PR 3 — Per-event cursor smoothing + recap measure references (~350 lines)

**Ships:**
- New event `score-position-updated` emitted at ~10Hz from the audio pipeline (downsampled).
- Frontend subscribes; `practiceStore.cursorPosition` updates from this event when a session has a score.
- LLM coaching/recap prompts updated: `SessionContext.score_title` field, recap prompt template gets a `phrases_with_positions` block so the LLM can quote measure numbers naturally.
- A short integration test: synthetic score session → recap text contains "measure" or a measure number.

**Merge criterion:** Cursor advances smoothly within a measure (visual smoke test). Recap references measures when score is loaded. CI green.

---

## 8. Open questions for the founder

### Resolved (2026-05-01)

- ~~**Library vs. one-shot uploads.**~~ → **Library.** Practising the same piece across multiple sessions is the central use case. Carries the SQLite migration + file-copy cost in PR 0.
- ~~**MIDI as a first-class import format.**~~ → **Yes.** Existing Rust parser plus the `score_model_to_musicxml` exporter we need anyway. Also unblocks Phase 2's audio-to-MIDI pipeline ending in the same loader.
- ~~**Cursor delivery strategy.**~~ → **Phrase-granularity in PR 2, per-event smoothing in PR 3.** Keeps each PR under 600 lines and lets us validate the cursor-on-phrase feel before investing in 10Hz IPC.

### Still open

1. **Multi-part orchestral scores: pick-on-import, or pick-per-session?** *Recommendation: pick on import (keep state simple). Power users with multi-part needs can re-import.*
2. **Cursor styling — single fixed colour, or grow to a small "moving spotlight" effect later?** I lean fixed colour for v1, with the cursor implemented as a CSS-styleable element so a designer can iterate without code changes.
3. **What happens if the user starts a session with a score loaded but plays something completely different?** The follower will likely lose alignment. Options: (a) ignore — let the cursor freeze where it last had high confidence, (b) detect divergence, fall back to free-play behaviour silently. *Recommendation: (a) for v1; we'll learn from real practice sessions whether (b) is needed.*
4. **Should the recap LLM prompt explicitly mention the score title?** "You played 'Haydn Trumpet Concerto, mvt 1' today." This personalises the recap and hooks into emotional buy-in. *Recommendation: yes — it's a one-line prompt change.*
5. **Empty-state library: do we ship with a small built-in fixture (e.g. a single Bach minuet) so a brand-new user has something to practise with?** Could double as a demo/onboarding step. *Recommendation: ship one fixture, make it skippable, ensure it can be deleted.*

---

**End of design doc.**
