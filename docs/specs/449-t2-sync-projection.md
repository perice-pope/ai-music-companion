# Spec: Teacher Dashboard — the sync projection, device → cloud (#449 T2)

## 1. Summary

The one-way projection that carries P1–P4 of
`docs/architecture/teacher-dashboard-datamodel.md` §2 from the device SQLite into
the T3 star schema (`supabase/migrations/0006` + grants in `0007`), gated behind a
NEW dashboard-sync opt-in (default OFF) on top of the existing cloud-sync opt-in,
with the ConnectionsPrivacy disclosure and the offline-first enumeration-table row
landing in the same PR (the standing rule).

## 2. Problem / why

T1 (local telemetry) and T3 (cloud schema + grants) are merged; nothing connects
them. The star schema fact tables receive zero rows today, so T4 (dashboard v1)
has nothing to render. Doc §2 defines exactly which rows go up (P1–P5) and the
privacy contract around them.

**P5 note (doc §2, re-read):** P5 is `learner_model.key_mastery` via the
**existing** learner-model push (`syncStore.syncLearnerModel`, migration 0005,
already disclosed, rides the taste-profile opt-in). The doc's §Sequencing entry
for T2 lists "projection P1–P4 + ConnectionsPrivacy rows + enumeration entries" —
P5 requires **no new work** in this slice and its existing behavior is untouched.

## 3. Non-goals

- No dashboard UI (T4), no enrollment/consent screen (the T-enrollment slice
  will prompt for `dashboardSyncEnabled` at classroom join; this slice ships
  the toggle + plumbing only).
- No change to the legacy `sessions` push (P1's "existing behavior") beyond
  running the star projection alongside it when the new opt-in is on.
- No `spec_json`, no `seed`, no `phrase_json`/onsets/pitch curves, no audio —
  ever, structurally (the payload types do not have the fields).
- No parser fix for #470 — its option (b) is taken: the `narration_used`
  flag's real semantics are DOCUMENTED at the projection site.
  (Superseded later by #470 option (a) — see
  `docs/specs/470-strict-recap-parse.md`; the flag now means the headline
  was LLM-authored.)
- No new outbound call sites in Rust (the projection reuses the FE Supabase
  client; `network-call-sites.allowlist` is unchanged).
- No cloud→device writes (one-way by construction, doc §2).

## 4. Contract / interface

### New IPC commands (read-only; Rust shapes the payloads — no business logic in FE)

```rust
// commands.rs
pub struct SessionFactDto {      // P1 — fact_session (0006 lines 452–472)
    id: String,                  // device_session_id (SessionId uuid string)
    started_at: DateTime<Utc>, ended_at: DateTime<Utc>,
    duration_secs: f64, phrase_count: usize,
    instrument: String,
    practice_mode: Option<String>, app_version: Option<String>,
    played_secs: Option<f64>, note_count: Option<u64>, silence_ratio: Option<f64>,
    fingerprint: Option<MusicalFingerprint>,
    score: Option<ScoreRefDto>,  // { score_id, title } → dim_material kind='score'
}
pub struct PhraseFactDto {       // P2 — fact_phrase (0006 lines 508–518): THIN
    phrase_index: usize, start_secs: f64, end_secs: f64,
    note_count: usize, stability: f64,
    tone: Option<ToneDescriptor>,          // flat descriptor only
    key_name: Option<String>,              // KeyEstimate::name(); None = gate failed
}
pub struct ToolEventFactDto {    // P4 — fact_tool_event (0006 lines 592–602)
    device_event_id: i64, at_secs: f64, kind: String, params_json: String,
}
pub struct SessionProjectionDto {
    session: SessionFactDto,
    phrases: Vec<PhraseFactDto>,
    events: Vec<ToolEventFactDto>,
}
#[tauri::command] get_session_projection(session_id) -> SessionProjectionDto;

// store.rs — P3 rows WITHOUT spec_json/seed (structural: the type has no fields)
pub struct ExerciseFactRow {     // fact_exercise (0006 lines 551–563)
    id: i64, logged_at: String, source: String, label: String,
    spec_hash: String, difficulty: u8, tonic: u8, accuracy: Option<f64>,
}
pub fn list_exercise_facts_after(&self, after_id: i64) -> Vec<ExerciseFactRow>;
#[tauri::command] list_exercise_facts(after_id: i64) -> Vec<ExerciseFactRow>;
```

### Cloud writes (column-for-column against 0006/0007; client role: authenticated)

| P | Table | Op | Conflict key | Columns written |
|---|---|---|---|---|
| P1 | `fact_session` | upsert (insert/update granted, 0007-audited via 0006 L479) | `student_id,device_session_id` (unique, 0006 L472) | student_id, device_session_id, started_at, ended_at, duration_secs, played_secs, note_count, silence_ratio, phrase_count, instrument, practice_mode, score_material_id, fingerprint, app_version |
| P2 | `fact_phrase` | upsert `ignoreDuplicates` (insert-only grant, 0006 L520) | PK `session_id,phrase_index` (0006 L517) | session_id (cloud uuid), phrase_index, start_secs, end_secs, note_count, stability, tone, key_name |
| P3 | `dim_material` | upsert `ignoreDuplicates` on `spec_hash` (0006 L434) then select ids; | `spec_hash` unique (0006 L421) | spec_hash, label, source, kind='cell' |
| P3 | `fact_exercise` | upsert `ignoreDuplicates` (insert-only grant, 0006 L567) | `student_id,device_log_id` (0006 L562) | student_id, device_log_id, logged_at, material_id, tonic, difficulty, accuracy (session_id omitted → NULL: linkage unknown locally) |
| P4 | `fact_tool_event` | upsert `ignoreDuplicates` (insert-only grant, 0006 L605) | `session_id,device_event_id` (0006 L601) | session_id (cloud uuid), student_id, device_event_id, at_secs, kind, params |
| P1 | `dim_material` (score row) | select by `score_id`+kind, insert if absent (no unique on score_id — 0006 L423) | — | score_id, label (title), source='score_practice', kind='score' |

The cloud `fact_session.session_id` uuid comes back from
`.upsert(...).select("session_id")` and keys the P2/P4 child rows.

### New opt-in

`connectionsStore.dashboardSyncEnabled` — default **false**, persisted key
`ai-music-companion:dashboard-sync-enabled`; turning `cloudSyncEnabled` off also
withdraws it (same dependency rule as teacher sharing).

### Gating matrix (the projection runs only in the last row)

| signed in | cloudSyncEnabled | dashboardSyncEnabled | legacy sessions push (P1 existing) | star projection (P1–P4) |
|---|---|---|---|---|
| no | — | — | no | no |
| yes | off | — | no | no |
| yes | on | off | yes (unchanged) | **no — nothing else leaves** |
| yes | on | on | yes (unchanged) | yes |

### Watermarks (incremental — never re-push everything)

- P1/P2/P4: per-user localStorage **set** of device session ids already
  projected (`ai-music-companion:dashboard-synced-sessions:${userId}`) — the
  exact `syncedKey` idiom the session push already uses. Sessions are
  immutable after close, so set-membership is the watermark; the cloud unique
  keys make a lost set harmless (idempotent re-push).
- P3: per-user numeric high-water mark of `exercise_log.id`
  (`ai-music-companion:dashboard-exercise-watermark:${userId}`), advanced only
  after a successful push; `list_exercise_facts(after_id)` filters in SQL.

## 5. Acceptance criteria (numbered, testable)

1. Signed out, or cloud sync off, or dashboard sync off ⇒ `syncDashboard` makes
   **zero** IPC reads and **zero** Supabase calls (each of the three gate
   combinations separately).
2. All-on ⇒ one un-projected session produces a `fact_session` upsert on
   `student_id,device_session_id` whose row carries exactly the 0006 columns,
   including `played_secs`/`note_count`/`silence_ratio` and `practice_mode`/
   `app_version`.
3. All-on ⇒ the same run pushes the session's thin `fact_phrase` rows and
   `fact_tool_event` rows keyed by the cloud session uuid, and exercise rows to
   `fact_exercise` with `material_id` resolved through a `dim_material`
   `spec_hash` upsert.
4. Structural privacy pin: the P3 payload types (`ExerciseFactRow` in Rust, the
   `fact_exercise`/`dim_material` Insert types + builder output in TS) have **no**
   `spec_json`/`seed` members — a type-level assertion fails compilation if
   either key appears, and a runtime test asserts the built rows lack the keys.
   Same pin for P2: no `phrase_json`, no `onsets`.
5. Idempotency: a second `syncDashboard` run after success pushes nothing (set
   + watermark honored); a re-push of the same rows (cleared set) targets the
   device-id conflict keys, so the cloud keeps single rows.
6. Watermark advance: after pushing exercises id ≤ N, the next run requests
   `after_id = N`; a failed push does **not** advance the watermark and does not
   mark the session synced (retry next run).
7. Calm failure: a Supabase error sets `dashboardStatus: "error"` with the
   message, leaves the legacy session-sync status untouched, and never throws.
8. Disclosure: ConnectionsPrivacy renders a dashboard-sync toggle, default OFF,
   disabled unless cloud sync is on, whose copy names what leaves (session
   facts, phrase timings, exercise labels, tool events), to whom (your teacher
   through an active classroom enrollment), and what never leaves (audio, the
   notes you played, exercise recipes/seeds). The every-toggle-off pin counts 5.
9. The #470 option-(b) note — `narration_used {"kind":"recap"}` means "an LLM
   response parsed", NOT "the shown text was LLM-authored" — is documented at
   the projection site (the `ToolEventFactDto` builder) and asserted present.
   (Superseded by #470 option (a): the note and its pin test now state the
   stricter semantics — see `docs/specs/470-strict-recap-parse.md`.)
10. `get_session_projection` returns thin phrases (start/end/note_count/
    stability/tone/key_name only) and events for exactly the requested session;
    `list_exercise_facts` respects `after_id` and skips NULL-`spec_hash` rows.

## 6. Edge cases & failure modes

- Pre-T1 sessions: integrity columns NULL → projected as NULL (honest absence;
  0006 allows NULL).
- Session with a score but no score_summary title → fall back to the local
  `scores.title` lookup; score deleted locally → `score_material_id` NULL.
- `dim_material` select-after-upsert returns no id for a hash (race/RLS) → skip
  those exercise rows this run (watermark does not advance past them).
- localStorage unavailable → sync still works; re-push is absorbed by the
  device-id conflict keys (same note as `saveSyncedIds`).
- Offline / Supabase down → error status, nothing marked synced, next trigger
  retries. Offline is normal life.
- Key estimate absent (gate failed) → `key_name` NULL, never a guess.
- Exercise rows logged mid-run: `after_id` filter means they surface next run —
  no loss, no dupes.

## 7. Test plan

| AC / edge | Test | Asserts |
|---|---|---|
| AC1 | `syncStore.test.ts` "dashboard sync gating matrix: …" (×3) | no invoke, no supabase calls per gate |
| AC2 | "projects fact_session column-for-column…" | row shape + onConflict key |
| AC3 | "pushes thin phrases, tool events and exercises…" | child rows keyed by cloud uuid; material_id resolution |
| AC4 | `type-level pin` in syncStore.ts + "payload builders never contain spec_json/seed…" | compile-time + runtime key absence |
| AC5 | "a second run pushes nothing (idempotent)…" | no upserts on run 2 |
| AC6 | "advances the exercise watermark only on success" | after_id value; failure freezes it |
| AC7 | "a supabase failure is calm…" | error status, legacy status untouched |
| AC8 | `ConnectionsPrivacy.test.tsx` new + updated pin tests | toggle OFF/disabled/copy; switch count 5 |
| AC9 | `syncStore.test.ts` narration-note test reads the source; Rust doc-comment on builder | note present at projection site |
| AC10 | `commands.rs` `get_session_projection_returns_thin_phrases_and_events`, `store.rs` `list_exercise_facts_after_*` | thin fields only; after_id filter |

## 8. Architecture / approach

FE `syncStore.syncDashboard` mirrors the existing `syncAll` discipline (batch,
upsert on device ids, localStorage memory, calm errors); Rust shapes every
payload via the new read-only DTO commands, so the FE cannot even see
`spec_json`/`seed`/`phrase_json` (structural privacy). Runs only from the
existing sync trigger (AccountPanel effect — post-session-close by construction;
never the audio path). Disclosure: new ConnectionsPrivacy toggle row + a new row
in the offline-first enumeration table, same PR. No new Rust network call sites.

## 9. Slice breakdown

One slice (this PR): store.rs read API + commands + connectionsStore flag +
syncStore projection + AccountPanel trigger + ConnectionsPrivacy row + docs +
types. ~under 400 changed lines of source (tests/types/docs ride along).

## 10. Risks / open questions

- dim_material label poisoning + score-row dupes across devices: recorded 0006
  trade-off; server-side upsert function can replace the client path later.
- fact_exercise.session_id stays NULL (local exercise_log has no session
  linkage) — doc allows "session linkage when known"; a future T1 addendum can
  add it.

## 11. References

- `docs/architecture/teacher-dashboard-datamodel.md` §2, §Sequencing
- `supabase/migrations/0006_teacher_dashboard_star_schema.sql`, `0007_dashboard_grants.sql`
- `crates/brain/src/store.rs` (T1), `apps/desktop/src/stores/syncStore.ts`
- Issue #449 (T2), #470 (narration-flag semantics, option b), #451/#445-6b (one clock)
