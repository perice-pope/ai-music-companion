# Spec: Local telemetry foundation — T1 of the teacher dashboard (#449)

> Slice T1 of `docs/architecture/teacher-dashboard-datamodel.md` §Sequencing.
> Backend-only. Everything here is local; **nothing in this slice syncs**.

## 1. Summary

Adds the local telemetry the teacher dashboard will later project up: the
`practice_events` tool-usage journal (§1a), the three session-integrity columns on
`sessions` (§1b), and `exercise_log.spec_hash` (§1c) — plus the emitters in the Tauri
command layer and the best-effort writer that can never break the practice loop.

## 2. Problem / why

The datamodel audit (same doc, "Tool usage: logged vs. not logged") found the material
trail excellent and the tool trail half missing: Pocket on/off/mode/tempo, band
on/off/key-pin, and narration usage leave no persisted record; played-vs-wall honesty
(#445-6b) is recomputed at narration time and never stored; retries of the same material
require parsing `spec_json` per query. T1 closes exactly that local gap.

## 3. Non-goals

- **No sync.** `practice_events` (and the new columns) leave the device **only** under
  T2's enrollment sync opt-in; T2 adds the ConnectionsPrivacy rows and the offline-first
  enumeration entries in the same PR that syncs them. Until then: zero outbound bytes.
- No frontend changes beyond additive optional `SessionSummaryDto` fields (History UI
  rides T4-era work).
- No cloud schema, no dashboard, no product analytics piggybacking on this journal
  ("no second telemetry pipe").
- No persistence of zero-phrase sessions: the empty-session path (recorder returns
  `SessionError::Empty` → calm empty-state recap, no row) is unchanged this slice. The
  F1 walk-away *metrics* are pinned by the pure `session_integrity` computation + store
  round-trip tests; making walk-away sessions persist a row is a T2+ decision to record
  in the datamodel doc, not a silent behavior change here.

## 4. Contract / interface

### 4a. `practice_events` (SCHEMA, verbatim from the datamodel doc §1a)

```sql
CREATE TABLE IF NOT EXISTS practice_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    at_secs REAL NOT NULL,
    kind TEXT NOT NULL,
    params_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_practice_events_session
    ON practice_events(session_id, at_secs);
```

Append-only; never read on the hot path. `at_secs` is seconds since
`sessions.started_at` — the recorder's `started_at()` is the same base
`end_practice_session_impl` already uses for `elapsed_secs`, and the audio worker's
phrase clock starts at pipeline spin-up a few ms after it, so range joins against
`session_phrases.start_secs/end_secs` are sound (the slop is orders of magnitude
inside any query window).

New `SessionStore` API (`crates/brain/src/store.rs`):
- `log_practice_event(&self, session_id: &str, at_secs: f64, kind: &str, params_json: &str) -> Result<(), StoreError>`
- `list_practice_events(&self, session_id: &str) -> Result<Vec<PracticeEventRow>, StoreError>` (ordered `at_secs, id`)
- `count_practice_events(&self) -> Result<usize, StoreError>`

Command-layer writer (`apps/desktop/src-tauri/src/commands.rs`):
- `log_practice_event_best_effort(state, kind, params)` — mirrors
  `log_exercise_best_effort`: **no active session → no row, calmly; store error → one
  `tracing::warn`, never an error to the caller.** Returns `()` by construction.
- Session identity + clock live in a new `AppState.telemetry:
  std::sync::Mutex<Option<SessionTelemetry>>` (session id, `started_at`, tempo
  coalescing state), set by `start_practice_session_impl` on success and cleared at the
  end of `end_practice_session_impl` on every path. A `std` mutex so sync commands
  (`set_pocket_tempo`) can emit; lock order is always telemetry → session_store.

### 4b. Event vocabulary (v1) and emitter placement

| kind | params_json | emitted at |
|---|---|---|
| `pocket_start` | `{"bpm", "mode": "anchor", "count_in"}` | `start_pocket`, after the click starts. `mode` is `"anchor"` by backend contract: every click starts as the strict Anchor; retimes arrive later. |
| `pocket_stop` | `{"bpm": last-known-effective}` | wherever a *running* click is torn down: `stop_pocket`, `start_pocket` (restart), `start_accompaniment` (band replaces click), session end. `teardown_pocket` now returns whether it stopped one. |
| `pocket_mode` | `{"mode": "anchor"\|"follow"\|"handoff"}` | new `set_pocket_mode` IPC command (validated). The mode lives in frontend Zustand today, so the backend cannot observe changes; this command is the seam. Wiring the one frontend call rides T2/T4 (no frontend changes this slice), so this kind produces no rows in production until then — recorded here on purpose. |
| `pocket_tempo` | `{"bpm"}` | `set_pocket_tempo` while a click is live — **coalesced**, see below. |
| `band_start` | `{"key_pinned": bool}` | `start_accompaniment` after the band starts. |
| `band_stop` | `{}` | wherever a running band is torn down (`stop_accompaniment`, `start_accompaniment` restart, `start_pocket`, session end); `teardown_accompaniment` returns whether it stopped one. |
| `band_key_pin` | `{"tonic", "minor"}` | `set_accompaniment_key`. (The doc's example says `mode: "dorian"`; the shipped command speaks `(tonic, minor)` — recorded honestly as-is; `params_json` is additive if the pin ever grows modes.) |
| `opener_begin` | `{"recipe": null}` | `opener_impl(commit=true)` (Begin) and `begin_opener_recall_impl`. The recipe *name* is not observable backend-side (the frontend loads a recipe into the builder, then Begins); `null` until that wire exists. |
| `score_open` | `{"score_id"}` | `get_score` after a successful load. |
| `narration_used` | `{"kind": "tip"}` | `get_coaching_tip` command when it returns `Some` — under the engine's silence-beats-a-lie contract, `Some` ⇔ a real LLM tip parsed (offline / rate-limited / failed / no key all return `None`). |
| `narration_used` | `{"kind": "recap"}` | `end_practice_session_impl` when the recap generator reports the LLM path actually produced the recap (new additive `RecapGenerator::recap_used_llm()`, default `false`; `CoachingEngine` tracks it with an `AtomicBool` set only when a network recap response parses — fallback, thin-session, and offline paths report `false`). |

**No session → no row** for every kind (the writer's contract). Band tempo retimes
(`set_band_tempo`) are deliberately not journaled in v1 — the vocabulary keys tempo to
the click; the band's presence is the `band_start`/`band_stop` span.

**Tempo coalescing rule:** one baseline is recorded by `pocket_start`; a `pocket_tempo`
row is appended only when **both** (a) the effective (clamped) tempo differs from the
last *logged* tempo by ≥ 5.0 BPM and (b) ≥ 5.0 s have passed since the last logged
tempo row. The last-known effective tempo is still tracked on every push (so
`pocket_stop` reports the true final tempo) — only the *journaling* is coalesced. A
follow-mode stream (~1 Hz, ±2 BPM wobble) therefore logs ~0 rows; a real ramp logs ≤ 1
row per 5 s.

### 4c. Sessions integrity columns (§1b)

Fresh DBs: columns in `SCHEMA`'s `CREATE TABLE sessions`. Existing DBs: migration v2
(`PRAGMA user_version` guard) via `add_column_if_missing` — the shipped idiom:

```sql
ALTER TABLE sessions ADD COLUMN played_secs REAL;
ALTER TABLE sessions ADD COLUMN note_count INTEGER;
ALTER TABLE sessions ADD COLUMN silence_ratio REAL;
```

Computed **once, in Rust, at session close** (`end_practice_session_impl`, right after
the row + phrases persist) by the pure
`brain::store::session_integrity(phrases, wall_secs) -> SessionIntegrity`:
- `played_secs` = Σ phrase `(end_time − start_time)` — the #445-6b/#451 played clock, persisted;
- `note_count` = Σ phrase `note_count` (voiced events detected);
- `silence_ratio` = `1 − played_secs / wall_secs`, clamped to `[0, 1]`;
  **`wall_secs ≤ 0` → `1.0`** (a zero-length wall has, vacuously, no played sound —
  documented so three dashboards can never re-derive it differently).
`wall_secs` is `(ended_at − started_at)` — the same derivation `SessionStore::save`
uses for `duration_secs`, so the ratio's numerator and denominator can never disagree
with the stored row. Stored via `record_session_integrity` (best-effort, like
`record_session_meta`). `SessionSummary`/`SessionSummaryDto` gain
`played_secs: Option<f64>`, `note_count: Option<u64>`, `silence_ratio: Option<f64>`
(`None` on pre-migration rows — honest absence, never fabricated zeros) so the
founder's own History page can show played-vs-wall honestly ("the same honesty resold
upward"); no History UI changes this slice.

### 4d. `exercise_log.spec_hash` (§1c)

```sql
ALTER TABLE exercise_log ADD COLUMN spec_hash TEXT;
```

FNV-1a 64 hex of the `spec_json` **bytes as-is** (the `score_content_hash` idiom,
same stability rationale). The doc says "tonic excluded" — `spec_json` (a serialized
`VariationSpec`) does not contain the tonic; tonic is its own column. So same cell,
different key ⇒ same `spec_json` ⇒ same `spec_hash`, different `tonic` — the RV
GROUP BY works with no exclusion step. Written on every `log_exercise`; existing rows
are **backfilled once** inside migration v2 (single pass over `spec_hash IS NULL`
rows — O(rows) FNV over strings already in memory, a few ms even for thousands of
rows, and the `user_version` guard means it runs exactly once per DB).

## 5. Acceptance criteria

1. A fresh DB and a legacy DB (pre-T1 shape) both end up with: `practice_events` +
   index, the three sessions columns, `spec_hash`, `user_version = 2`; running
   migrations again changes nothing (idempotent).
2. Legacy `exercise_log` rows gain a backfilled `spec_hash` equal to
   `exercise_spec_hash(spec_json)`; new `log_exercise` rows carry it on insert.
3. `log_practice_event_best_effort` with no active session writes zero rows and
   returns calmly; with an active session it writes a row whose `at_secs` is offset
   from the session's `started_at`; a store failure surfaces only as a warning
   (the brain-level writer's `Err` is provably swallowed by the command layer's `()`).
4. Emitters fire per the §4b table (pocket start/stop, band start/stop/key-pin,
   opener begin, score open, narration tip) — and **only** inside an active session.
5. A follow-mode tempo stream (many small pushes in quick succession) journals few
   rows: zero `pocket_tempo` rows until both the ≥5 BPM and ≥5 s gates pass; the
   final effective tempo still appears on `pocket_stop`.
6. F1 walk-away: `session_integrity([], 1800)` → `played_secs = 0`, `note_count = 0`,
   `silence_ratio = 1.0`, consistent with the #445-6b thin thresholds (< 3 phrases,
   < 20 s played). `wall = 0` → ratio `1.0`; played > wall clamps to `0.0`.
7. Integrity columns round-trip: after close, the session row holds the computed
   values and `SessionSummary`/`SessionSummaryDto` expose them; pre-migration rows
   read `None`.
8. F4 retry-farming: N logs of one spec at one tonic GROUP BY `spec_hash` to a single
   group of N. F5 key-camping: one spec across tonics shares one `spec_hash` with
   `tonic` spread visible in GROUP BY `spec_hash, tonic`.
9. F8: pocket events + phrases with near-zero `note_count` join on
   `(session_id, at_secs × start/end range)` to expose a tool-on-no-notes span.
10. `recap_used_llm` is `true` only when a network recap response actually parsed;
    offline policy, missing engine, and mock generators report `false` (so
    `narration_used {"kind":"recap"}` cannot lie).

## 6. Edge cases & failure modes

- No active session (library browsing, opener preview, post-end stragglers) → no rows.
- Store degraded to in-memory at startup → events write to the in-memory DB like
  everything else; loss on restart is the existing, disclosed degradation.
- Double start / re-entrant end: telemetry ctx is set only on successful start and
  cleared on every end path; `AlreadyActive` never clobbers a live ctx.
- Poisoned locks: `lock_or_recover`, same as every other std mutex here.
- NaN/out-of-range tempo: events record the **clamped** effective tempo
  (`clamp_pocket_params`), never the raw wire value — played == reported == journaled.
- Old DBs opened by `ScoreStore` first: `ScoreStore::migrate` runs `SCHEMA` only; the
  versioned blocks still run on `SessionStore::open` of the same file.

## 7. Test plan

| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `brain::store::tests::migration_v2_adds_telemetry_schema_to_legacy_db_once` | legacy DB gains table+columns, `user_version`=2, second migrate no-op |
| AC2 | `brain::store::tests::migration_backfills_spec_hash_for_legacy_rows`, `log_exercise_writes_spec_hash` | backfill = `exercise_spec_hash(spec_json)`; new rows hashed on insert |
| AC3 | `commands::tests::no_session_emits_no_practice_events`, `practice_event_rows_carry_session_offset_clock`, `brain::store::tests::log_practice_event_err_is_surfaced_to_the_swallowing_caller` | zero rows w/o session; `at_secs` ≥ 0 offset; brain `Err` exists for the `()` writer to swallow |
| AC4 | `commands::tests::{begin_opener_logs_opener_begin_only_in_session, get_score_logs_score_open_only_in_session, set_accompaniment_key_logs_band_key_pin, set_pocket_mode_validates_and_logs, coaching_tip_some_logs_narration_used}` | row kind+params per table; absent w/o session |
| AC5 | `commands::tests::{tempo_log_due_gates_on_delta_and_gap, pocket_tempo_stream_coalesces_to_few_rows}` | pure gate truth table; a 20-push stream logs 0 rows until gates pass, final bpm tracked |
| AC6 | `brain::store::tests::{session_integrity_walk_away_is_all_silence, session_integrity_clamps_and_zero_wall}` | F1 numbers; wall=0 → 1.0; clamp |
| AC7 | `brain::store::tests::record_session_integrity_round_trips`, `commands::tests::session_close_persists_integrity_columns` | row + summary DTO fields; `None` pre-migration |
| AC8 | `brain::store::tests::{retry_farming_groups_by_spec_hash, key_camping_shares_spec_hash_across_tonics}` | F4 single group of N; F5 one hash, tonic spread |
| AC9 | `brain::store::tests::pocket_on_span_with_no_notes_is_joinable` | range join finds the silent tool-on span |
| AC10 | `brain::coaching::tests::{recap_used_llm_true_only_after_parsed_network_recap, recap_used_llm_false_when_offline}` | flag truth |

## 8. Architecture / approach

Storage in `crates/brain/src/store.rs` (SCHEMA + versioned `user_version` migration +
`add_column_if_missing`, the shipped idioms). Emitters in
`apps/desktop/src-tauri/src/commands.rs` at the command layer — the same layer
`log_exercise_best_effort` lives on; **never** the audio thread. One clock: `at_secs`
offsets from the recorder's `started_at`. Offline-first: this slice adds **no network
surface**; `practice_events` and the new columns sync **nothing** until T2 lands the
enrollment opt-in + `ConnectionsPrivacy.tsx` + offline-first enumeration rows in the
same PR that syncs them (standing rule).

## 9. Slice breakdown

One slice (this PR): schema+store+emitters+tests are one vertical cut; T2 (projection),
T3 (cloud schema), T4 (dashboard) follow per the datamodel doc.

## 10. Risks / open questions

- `pocket_mode` rows await one frontend line (T2/T4) — recorded in §4b.
- Zero-phrase sessions still persist no row (F1 caught at the computation level);
  whether walk-away sessions should persist is a T2 decision for the datamodel doc.
- `opener_begin.recipe` is `null` until the recipe name crosses the IPC boundary.

## 11. References

- `docs/architecture/teacher-dashboard-datamodel.md` §§1a–1c, fudge table, Sequencing T1
- `crates/brain/src/store.rs` (SCHEMA, `add_column_if_missing`, `user_version` idiom,
  `score_content_hash`)
- `apps/desktop/src-tauri/src/commands.rs` (`log_exercise_best_effort`,
  `start_pocket`/`teardown_pocket`, `start_accompaniment`, `begin_opener`,
  `get_score`, `end_practice_session_impl`)
- #445-6b / #451 (played clock, thin thresholds), #385 (`content_hash` idiom)
