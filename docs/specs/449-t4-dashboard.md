# Spec: Teacher Dashboard v1 — web app (#449 T4)

> Slice T4 of the teacher-dashboard track
> (`docs/architecture/teacher-dashboard-datamodel.md` §4, §5). T1 (local
> telemetry), T2 (projection P1–P4) and T3 (cloud star schema, migrations
> 0006/0007) have landed; this slice is the surface that reads them.

## 1. Summary

A new web app, `apps/dashboard/`, for band directors: sign in with the same
Supabase auth the desktop uses, pick a classroom, and see (a) a roster heat
grid of played-minutes per student per day, (b) a per-student drill-down
(sessions with integrity columns, material × 12-key coverage, tool usage,
stored recap facts), and (c) an engagement-integrity panel that surfaces the
fudge-vector evidence calmly. Founder decisions honored: BI-grade granularity,
per-classroom (per-year pricing shape), **no parent view**.

## 2. Problem / why

T1–T3 put honest practice data in the cloud behind RLS; nothing reads it yet.
The product being sold ("visibility into the practice session data") does not
exist until a teacher can see it.

## 3. Non-goals

- No assignment-push (v2 — doc §Decisions), no messaging, no audio anywhere
  on this path (teacher-audit stays separate), **no parent view** (founder).
- No writes to any fact/dim table from this app: it is a **read-only** surface
  (auth + profile self-provision aside). The cloud never writes back to a
  device; this app never writes at all beyond auth.
- No chart library. v1 is a CSS-grid heat map + tables — no dependency is
  justified for that (per the dataviz restraint rule in the task brief).
- No service keys, no server component, no edge functions. The app talks to
  PostgREST with the **publishable key only**; RLS is the entire security
  boundary (0006 policies + 0007 grants). A future backend may add the
  matview-powered rollups (`mv_student_day` is service-role-only by design —
  0006 lines 667-670); v1 uses the live `security_invoker` views instead.
- No automated verdicts. The integrity panel reports evidence; copy never
  accuses (teacher-audit non-goal, inherited verbatim).
- No pnpm workspace conversion. There is **no `pnpm-workspace.yaml` in the
  repo** (checked 2026-07-22); `apps/desktop` is a standalone pnpm project
  with its own lockfile, and CI installs it with `--frozen-lockfile` from
  that lockfile. Introducing a root workspace file would re-root pnpm for the
  desktop app and break that install. `apps/dashboard` therefore mirrors the
  standing convention: standalone project, own `pnpm-lock.yaml`, own CI lane.

## 4. Contract / interface

### Screens

| Screen | Route (state-based, no router dep) | Data |
|---|---|---|
| Sign in | `signed_out` | `supabase.auth` (email+password — same flow as desktop `authStore`) |
| Teacher gate | after sign-in | own `profiles` row; `role !== 'teacher'` → polite dead-end, zero data queries |
| Classroom picker (header) | persistent | `classrooms` (teacher's own) |
| Roster heat | `roster` tab | `v_roster_heat` (14-day window) + `enrollments`+`profiles` (full roster incl. offline) + `fact_session` created_at (F9) |
| Student drill-down | click a roster row | `fact_session`, `fact_exercise`+`dim_material`, `fact_tool_event` |
| Integrity panel | `integrity` tab | `v_session_integrity` + `v_roster_heat` (F6 day rows) + `fact_tool_event`×`fact_phrase` (F8 evidence) |

### Query → relation → policy/grant map (every read this app performs)

The app authenticates as a teacher with the publishable key; every row it can
see is decided by these shipped policies. Citations are to
`supabase/migrations/0006_teacher_dashboard_star_schema.sql` (0006) and
`0007_dashboard_grants.sql` (0007) line numbers.

| # | Query (queries.ts) | Relation | Policy that admits the rows | Grant |
|---|---|---|---|---|
| Q1 | `getOwnProfile` | `profiles` | `profiles_select_own` (0001) | 0007 L45 |
| Q2 | `listClassrooms` | `classrooms` | `classrooms_select_teacher` (0006 L65-66) | 0006 L56 |
| Q3 | `listRoster` | `enrollments` + embedded `profiles` | `enrollments_select_teacher` (0006 L113-118); `profiles_select_enrolled_teacher` (0006 L240-247) | 0006 L106, 0007 L45 |
| Q4 | `getRosterHeat` | `v_roster_heat` (security_invoker, 0006 L703-719) | underlying: `fact_session_select_teacher` (0006 L498-505), `enrollments_select_teacher`, profiles as Q3 | 0006 L719 |
| Q5 | `getLateSyncSessions` (F9) | `fact_session` | `fact_session_select_teacher` (0006 L498-505) | 0006 L479 |
| Q6 | `getStudentSessions` | `fact_session` | same as Q5 | 0006 L479 |
| Q7 | `getStudentExercises` | `fact_exercise` + embedded `dim_material` | `fact_exercise_select_teacher` (0006 L582-589); `dim_material_select_signed_in` (0006 L446-447) | 0006 L567, L434 |
| Q8 | `getStudentToolEvents` | `fact_tool_event` | `fact_tool_event_select_teacher` (0006 L619-626) | 0006 L605 |
| Q9 | `getIntegrityRows` (roster `.in()` before the 100-row limit) | `v_session_integrity` (security_invoker, 0006 L725-744) | underlying: `fact_session_select_teacher`, `fact_exercise` teacher policy (subselects), profiles as Q3 | 0006 L744 |
| Q10 | `getSessionEvidence` (F8) | `fact_tool_event`, `fact_phrase` | Q8's policy; `fact_phrase_select_teacher` (0006 L538-545) | 0006 L605, L520 |

Deliberately NOT queried: `mv_student_day` / `mv_material_key` (service-role
only — 0006 L667-670; a client select is `permission denied` by design),
legacy `sessions`/`session_phrases` (the dashboard reads facts, not the recap
push — 0006 header L11-13), `teacher_student_links` (the roster boundary is
classrooms/enrollments — 0006 header L14-18).

### Fudge vector → metric → dashboard copy (datamodel §5, implemented)

Rule, pinned verbatim in the UI: **"Metrics nominate, humans judge."**
(teacher-audit's line). Evidence phrasing, never verdicts; the words
"cheat"/"fake"/"lie" never appear in the app.

Provenance: the vector *directions* are datamodel §5's, verbatim; the
**numeric bars are T4's** — the datamodel names the metrics but not
per-vector thresholds, so the numbers below are declared here (and pinned
in `integrity.ts` + its tests) as this slice's decisions, revisable by
editing this table. The F1 bar sits **strictly inside** the
`v_session_integrity` nomination filter (0006 L741-743: `silence_ratio >
0.8 OR (wall > 600 AND played < 120) OR phrase_count < 3`), so every
session F1 can flag is already a row the view returned — the chip narrows
the view's nomination, it never needs rows the view withholds.

| F# | Metric (integrity.ts) | Bar (fires only above) | Calm copy shown |
|---|---|---|---|
| F1 walk-away | `flagWalkAway`: `silence_ratio ≥ 0.9` AND `played_secs < 60` AND `duration_secs ≥ 600` | below bar → no chip | "App open ⟨wall⟩ min, under a minute of sound" |
| F4 retry-farming | `flagRetryFarming`: `max_retries ≥ 6` | 5 retries → no chip | "Same material graded ⟨n⟩× this session" |
| F6 session-splitting | `flagSessionSplitting` per day-row: `sessions ≥ 3` AND `played_secs/sessions < 120` | 2 sessions, or healthy per-session time → no flag | "⟨n⟩ short sessions this day, about ⟨m⟩ min of sound each" |
| F8 tool-on-no-notes | `toolOnNoNoteSpans`: pocket/band span ≥ 60 s overlapping zero phrase notes | shorter spans / spans with notes → none | "Metronome or band ran ⟨m⟩ min with no notes detected" |
| F9 late sync | `lateSyncAnnotations`: `created_at − started_at ≥ 48 h` | prompt syncs → none | heat annotation "synced ⟨date⟩, played ⟨date⟩" — shown, not punished |
| F2/F3/F5/F7 | visible as data, not chips in v1: empty fingerprint chips, `graded = 0`, one hot tonic column in the coverage matrix, — | — | the drill-down surfaces them; F7/F10 are human-judgment/off-scope (doc §5) |

### Honest absences (everywhere)

- An actively-enrolled student with **no** heat rows renders **"practicing
  offline"** — a labeled state, never a zero (datamodel §2 "Honest absence").
- A day cell with no row is blank ("no synced practice"), distinct from a
  synced row whose `played_min` is 0 (which renders 0 and is evidence).
- `silence_ratio`/`played_secs`/`accuracy` NULL render "—", never 0.
- Coverage-matrix cells never drilled are blank, not 0 %.
- Query errors render an error state, never an empty-but-confident grid.

## 5. Acceptance criteria (numbered, testable)

1. **Teacher gate:** signed-in user whose profile `role` is not `teacher`
   sees the "for teachers" dead-end and the app issues **no** data queries
   beyond the own-profile read. A `teacher` role reaches the shell.
2. **Roster heat renders from `v_roster_heat` rows:** given fixture rows,
   the grid shows one row per enrolled student × 14 day columns, cell
   intensity from `played_min`, and each populated cell exposes both played
   and wall minutes (the gap is visible on hover/inspect — display honesty).
3. **Honest absence:** an active enrollment with zero heat rows renders
   "practicing offline", and no cell for that student reads "0".
4. **F9:** a session fixture with `created_at` ≥ 48 h after `started_at`
   produces the late-sync annotation on that student's heat row; a prompt
   sync produces none.
5. **Drill-down display honesty:** session list shows wall and played as
   separate values (never summed/interchanged), notes, and silence ratio;
   NULL integrity columns render "—".
6. **Coverage matrix:** `fact_exercise` fixtures spanning tonics render a
   material × 12-tonic matrix; attempted cells show accuracy/attempts;
   never-drilled cells are blank, not 0.
7. **Tool usage:** `fact_tool_event` fixtures summarize (pocket time/tempo
   range, band time, narration count); no events → honest "no tool events
   synced" line.
8. **Integrity flags fire only above bars:** for each of F1/F4/F6/F8, a
   fixture just below the bar produces no chip and one above produces the
   chip (both directions tested).
9. **Calm copy pinned:** the integrity panel renders the exact rule line
   "Metrics nominate, humans judge." and never renders "cheat"
   (case-insensitive) anywhere.
10. **Signed-out → sign-in screen;** sign-in uses the same
    email+password Supabase flow as the desktop store.
11. **Empty/error states:** classroom with no active enrollments → honest
    empty roster message; a failed view query → error message, not an empty
    grid.

## 6. Edge cases & failure modes

- Zero classrooms → "create a classroom in the app" empty state (classroom
  creation is not in this slice).
- Enrollment `status='invited'`/`'revoked'`: excluded from roster (query
  filters `status=eq.active`; RLS would hide their facts regardless).
- `v_session_integrity` returns rows for the **teacher's own** sessions too
  (a teacher who practices) — panel filters to the selected classroom's
  roster ids.
- Networkless / Supabase down: every fetch has an error state; nothing spins
  forever.
- 12-month retention purge (0006) means old students go blank — the
  "practicing offline" state covers them honestly.

## 7. Test plan

All tests run against a mocked Supabase client routed **by relation name**
(`src/test/mockSupabase.ts`) — no network in tests.

| AC | Test | Asserts |
|---|---|---|
| 1 | `TeacherGate.test.tsx` › "blocks non-teacher role", "admits teacher", "issues no data queries for non-teacher" | dead-end copy; children rendered; `from()` call log ⊆ {profiles} |
| 2 | `RosterHeat.test.tsx` › "renders a cell per student-day from v_roster_heat rows", "cell exposes played and wall minutes" | grid content + title attr "X min played · Y min open" |
| 3 | `RosterHeat.test.tsx` › "enrolled student with no rows shows practicing offline, never zero" | label present; no "0" cell for that student |
| 4 | `RosterHeat.test.tsx` › "late-synced sessions annotate the row (F9)" + `integrity.test.ts` › lateSyncAnnotations | annotation text; 47 h fixture → none |
| 5 | `StudentDrilldown.test.tsx` › "session rows show wall and played separately", "null integrity columns render em dash" | both numbers; "—" |
| 6 | `StudentDrilldown.test.tsx` › "coverage matrix blank means never drilled, not zero" | attempted cells labeled; blank cells have no "0" |
| 7 | `StudentDrilldown.test.tsx` › "summarizes tool events", "no tool events → honest absence line" | pocket/band/narration summary; absence copy |
| 8 | `integrity.test.ts` › one `fires above the bar` / `stays quiet below the bar` pair per F1/F4/F6/F8; `IntegrityPanel.test.tsx` › "chips appear only for sessions above the bars" | boolean/metric outputs both sides of each bar; chip presence |
| 9 | `IntegrityPanel.test.tsx` › "pins the nominate-not-judge rule", "never renders accusatory language" | exact rule string; `/cheat|fake|lied/i` absent |
| 10 | `App.test.tsx` › "signed out shows the sign-in screen" (+ authStore reuse is copied verbatim from the desktop pattern) | form present |
| 11 | `RosterHeat.test.tsx` › "empty roster is an honest empty state"; "query error surfaces as an error, not an empty grid" | copy |

## 8. Architecture / approach

- `apps/dashboard/` — Vite + React + TS(strict) + Tailwind + zustand +
  vitest/RTL, config copied from `apps/desktop` (tsconfig, eslint flat
  config, prettier defaults, same dep versions). Standalone pnpm project
  (see Non-goals for why no workspace file).
- `src/lib/supabase.ts` — same URL + **publishable key only** convention as
  the desktop client; no service key exists anywhere in this app.
- `src/lib/queries.ts` — the ONLY module that touches PostgREST; each
  function's doc comment cites the policy/grant that admits its rows (the
  table in §4). Components receive the client as a prop (`App` passes the
  real one; tests pass the mock) — that is what makes "routed by
  table/view name" mocking possible.
- `src/lib/integrity.ts` — pure functions for F1/F4/F6/F8/F9 + the copy
  builders; fully unit-tested (the fudge table is the test spec).
- Navigation is a zustand store (`navStore`) — three states, no router dep.
- Heat map is a CSS grid with a 4-step green scale + amber integrity ring;
  no chart library (dataviz restraint: two-value cells and tables don't
  justify a dependency).
- Offline-first law: this is a teacher web surface, not the practice loop —
  nothing here touches the desktop app's offline guarantees; the app makes
  no outbound call beyond the already-disclosed Supabase origin.
- CI: new `dashboard` job in `.github/workflows/ci.yml` mirroring the
  `frontend` job (own lockfile cache key, lint → format check → test →
  build). The existing `frontend` job is untouched (its name is a required
  status check).

## 9. Slice breakdown

This IS slice T4; it ships as one PR (screens are thin over §4's queries).
Follow-ups recorded: dashboard deploy config + `redeem_join_code` gateway
rate-limit (0006 L310-317 TODO), classroom management UI, scores mirror FK.

## 10. Risks / open questions

- `v_session_integrity` has no classroom column. Classroom relevance is
  applied server-side as `.in(student_id, roster)` **before** the 100-row
  page limit (Q9), so a multi-classroom teacher's other rosters — or their
  own practice sessions, which RLS also admits — can never consume the
  page; the client-side roster filter stays as a second belt. Known
  completeness bound: the panel shows at most the 100 most recent evidence
  rows for the selected roster; older rows fall off the page, not out of
  the data. Revisit with pagination if a roster outgrows it.
- F8 needs per-session `fact_tool_event`+`fact_phrase` fetches; bounded to
  the flagged sessions (the integrity panel page, ≤ 100 rows — the same
  Q9 limit), and the evidence ids passed to Q10 are already
  roster-filtered rows.
- **F4 structural scope, inherited from 0006:** the view's WHERE
  (0006 L741-743) nominates on silence/wall-vs-played/phrase-count only —
  it never filters on retries. A retry-farmer whose sessions are otherwise
  healthy (low silence, real phrases) therefore never reaches the panel,
  and the F4 chip can only decorate rows nominated for other reasons.
  This is the shipped view's shape, not a T4 choice. Named follow-up: a
  0008 migration extending `v_session_integrity` (or adding a dedicated
  retry-evidence view) so `max_retries >= 6` nominates on its own; the F4
  bar and copy here carry over unchanged.
- Label poisoning on `dim_material` is a recorded v1 trade-off (0006
  L442-445); display-only impact here.

## 11. References

- `docs/architecture/teacher-dashboard-datamodel.md` (§3–§5 verbatim source)
- `supabase/migrations/0006_teacher_dashboard_star_schema.sql`, `0007_dashboard_grants.sql`
- `docs/architecture/teacher-audit.md` (tone rule), issue #449 + 2026-07-19 founder comment
- `apps/desktop/src/lib/supabase.ts`, `apps/desktop/src/stores/authStore.ts` (conventions copied)
