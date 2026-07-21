# Spec: Teacher Dashboard cloud star schema — T3 (#449)

**Spec-lite:** the migration IS the spec here —
`supabase/migrations/0006_teacher_dashboard_star_schema.sql` (SQL-only slice, no app
code). This file records the decisions and the fudge-vector → query mapping. Source
design: `docs/architecture/teacher-dashboard-datamodel.md` §3–§4.

## 1. Summary
The cloud star schema behind the dashboard: `classrooms`/`enrollments` (roster +
consent boundary, issue #449 §2), the four fact tables the device projection P1–P4
lands in, `dim_date`/`dim_material`, the doc's rollup matviews, and two live
RLS-safe views for roster heat + the integrity panel. RLS everywhere; activation
only through a consent-recording definer function; 12-month retention.

## 2. Decisions recorded

| Decision | Rationale |
|---|---|
| `classrooms.seat_product_id` is a **commented TODO**, not a column | `products` (commerce spine) is designed but **not migrated** — checked all of `supabase/migrations/`; an FK to a nonexistent table can't ship, and inventing `products` here would fork the spine. The migration that lands `products` adds the column. |
| Dashboard visibility keys off `enrollments.status='active'` per classroom, **not** `teacher_student_links` | The doc's rule verbatim (§3). 0003's links stay untouched as the 1:1 assignment relationship; the star schema is additive (legacy `sessions` push keeps working). |
| `status='active'` is reachable **only** via `redeem_join_code()` | Client policies allow teachers `invited`/`revoked` only (blocks consent fabrication) and students `revoked` only (self-serve leave). The definer function records consent at activation. |
| Schema-level consent gate | `CHECK (status <> 'active' OR (consent AND consented_at NOT NULL))` binds *every* writer including service role. Under-13 → `consent='parent'` enforced in `redeem_join_code()` against `profiles.age_tier` (COPPA gate, teacher-audit verbatim); the app-side consent screen is a later slice. |
| Revoked students cannot re-redeem a live code | Fail closed: the teacher re-admits by deleting the row. Revocation is otherwise immediate by construction (every teacher policy re-evaluates the active join). |
| Students never SELECT `classrooms` | The row carries the live join code; redemption goes through the definer function, so no student read path exists to harvest codes. |
| Matviews (`mv_student_day`, `mv_material_key`) are **revoked from client roles** | Matviews carry no RLS and are stale between refreshes — never trusted for consent (doc §3). Service-side only; the T4 backend re-applies the active-enrollment filter. |
| Live views `v_roster_heat` / `v_session_integrity` use `security_invoker` | Postgres ≥15 (Supabase/CI). The querying teacher's own policies gate every row, so revocation is immediate on the live views. |
| `spec_json` / `seed` have **no columns** | Doc rule verbatim: replayability is a device concern. Material identity crosses as `spec_hash` → `dim_material`. |
| Idempotent re-push keys | `fact_session (student_id, device_session_id)` upsert; `fact_exercise (student_id, device_log_id)` and `fact_tool_event (session_id, device_event_id)` `ON CONFLICT DO NOTHING`. |
| Retention = a purge **function**, pg_cron **commented** | pg_cron isn't enabled in this project and enabling extensions is an infra action; the exact `cron.schedule` calls are in the migration comment, runnable the moment it is. Until then: service-side invocation. |
| `dim_material` is shared, append-only from clients | Global `spec_hash` dedupe per the doc (12 keys of one cell = one row). Trade-off recorded in-file: label poisoning by a hostile client is possible; facts/accuracy unaffected; a server-side upsert fn can replace the insert path later without schema change. |

## 3. Fudge-vector → schema mapping (doc §5)

| # | Vector | Where the metric lives now |
|---|---|---|
| F1 | open app, walk away | `fact_session.played_secs`/`silence_ratio` → `mv_student_day.thin_sessions`; row in `v_session_integrity` (silence > 0.8) |
| F2 | room noise "practice" | `fact_session.note_count` + `fingerprint` (gates → `null`); notes-per-played-minute from `fact_session`; surfaces via `v_session_integrity` |
| F3 | aimless noodling | `v_session_integrity.graded = 0` while `played_min` high; blank `mv_material_key` coverage |
| F4 | re-grade the easy one | `v_session_integrity.max_retries` (max attempts per `(material_id, tonic)` in session); `mv_material_key.attempts` ≫ distinct materials |
| F5 | camp in one key | `mv_material_key` grouped by `tonic`: 1 hot column, 11 blank per `material_id` |
| F6 | 90-second streak farming | `mv_student_day`: `sessions` vs `played_secs`; `thin_sessions` count |
| F7 | play a recording | `fact_exercise.accuracy` vs the `learner_model` EWMA baseline (0005 table, P5) — nominate-to-human, no automated verdict |
| F8 | let the click be "practice" | range join `fact_tool_event(at_secs)` × `fact_phrase(start_secs, end_secs)` — tool-on spans with ~0 `note_count` |
| F9 | sync off all term, enable before grading | `fact_session.created_at` (server `now()`) ≫ `started_at` — late-arrival annotation |
| F10 | doctored local DB | out of metric scope by design (doc §5): human listening path; no schema element pretends otherwise |

## 4. Acceptance criteria → tests
`supabase/tests/rls_teacher_dashboard_star.sql` (picked up automatically by the
merge-blocking `supabase-rls.yml` CI loop). Asserts: teacher sees active-enrolled
facts only (all four fact tables + profiles); other teacher sees nothing; teacher
cannot write facts or activate enrollments (fabricated consent blocked); student
sees/writes own facts only, upsert doesn't duplicate, cannot attach rows to another
student's session, sees zero classroom rows, cannot self-activate; CHECK refuses
unconsented active rows from any role; redeem happy path, wrong-owner issue,
under-13 student-consent refusal + parent-consent success, revoked-rejoin refusal,
expired-code refusal; revocation closes facts + profile immediately; matviews are
permission-denied for authenticated; `v_roster_heat`/`v_session_integrity` scope by
invoker RLS (PG ≥ 15; version-skipped locally on PG14).

## 5. Validation actually performed
- Local shadow **Postgres 15.18** (initdb scratch cluster + the committed
  `supabase/tests/shadow/shim.sql`: `auth.users`, null-safe `auth.uid()`,
  anon/authenticated/service_role, Supabase blanket default privileges applied
  BEFORE migrations). Migrations 0001→0006 apply clean **unmodified** (PG15
  parses `security_invoker`); all three RLS suites pass — including the star
  suite's PG15-only assertions (the `reloption @> security_invoker=true` check
  on both live views + the invoker-scoped roster/integrity assertions, which
  no longer skip); `refresh_dashboard_rollups()` + `purge_expired_dashboard_facts()`
  execute; `dim_date` spot-checked (3652 rows, school-year labels correct).
- Docker (and therefore `supabase start`) was unavailable on this machine; the
  PG15 shadow harness above is the substitute. CI (`supabase-rls.yml`) remains
  the authority and runs the same files on the real Supabase stack.

### Review round 1 (MF1) — consent-audit forgery, fixed
The reviewer's executed attack: a student, riding the self-revoke policy (whose
`WITH CHECK` can only pin `student_id` + `status`, never compare NEW to OLD),
ran `UPDATE enrollments SET status='revoked', classroom_id=<victim room>,
consent='parent', consented_at=now(), consenting_adult_id=<fabricated>` and
wrote a forged ghost row into an **unrelated** teacher's roster (no fact/profile
leak — revoked grants nothing — but consent-audit forgery + cross-classroom
write). Fix = two belts: (a) `enrollments_guard_student_update` BEFORE UPDATE
trigger freezing every column except `status`/`revoked_at` when a **client role**
acts as the enrolled student; (b) the policy kept as-is. The trigger is
**INVOKER-rights** (not SECURITY DEFINER) so `current_user` stays the acting
role — a definer trigger swaps it to the owner and would exempt the attacker;
`search_path=''` is still pinned. Verified: the attack is RED against the
unfixed schema (row written) and BLOCKED post-fix (`students may only change
enrollment status`), with the legitimate status-only self-revoke still working —
all on the PG15 shadow cluster. Non-blocking hardening also taken: `issue_join_code`
TTL clamped to ≤30 days; a defensive test asserting the `security_invoker`
reloption on both views (that view fails OPEN if it's ever lost); and a
`redeem_join_code` rate-limit TODO (no DB-level throttle — 40-bit code + ≤30-day
TTL makes in-window brute force infeasible; gateway rate-limiting recommended in
the T4/deploy slice, noted at the function).

## 6. Non-goals
Everything the doc's non-goals list says, plus: no `products`/`entitlements`
tables (commerce spine slice), no seat entitlement minting (TODO in
`redeem_join_code`), no cloud `scores` mirror (dim_material.score_id has no FK
yet), no T2 client projection code, no T4 dashboard.
