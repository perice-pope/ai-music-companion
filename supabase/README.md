# Supabase — AI Music Companion

Backend for the **Teacher Dashboard** track (`docs/design/story-phase3-teacher-dashboard.md`).
This is the cloud layer; the core practice loop stays fully offline (architecture-v2 §6) —
sync is optional and additive.

## Project

| | |
|---|---|
| Project ref | `ttcbaomzgoatunjneuan` |
| API URL | `https://ttcbaomzgoatunjneuan.supabase.co` |
| Region | `us-east-2` |
| Publishable (anon) key | `sb_publishable_gDoRKh1SJG2yecSzHyheiQ_Pe32aXAy` |

The publishable key is safe to ship in the client (it's the public, RLS-gated key).
**Never** commit the service-role/secret key. Client config belongs in env, e.g.:

```
VITE_SUPABASE_URL=https://ttcbaomzgoatunjneuan.supabase.co
VITE_SUPABASE_PUBLISHABLE_KEY=sb_publishable_gDoRKh1SJG2yecSzHyheiQ_Pe32aXAy
```

## Schema (this PR — "Phase 2.5 Sync")

The foundational, **self-only** sync layer. Every table has RLS enabled and a
user can touch **only their own rows**.

- `profiles` — one row per `auth.users` (role `student`/`teacher`, display name).
  Auto-created on signup via the `handle_new_user` trigger.
- `sessions` — synced practice-session recaps (instrument, timing, `overall_assessment`,
  and the `session_tone` aggregate from `brain::SessionRecap`).
- `session_phrases` — per-phrase detail (`stability`, `mean_amplitude`, per-phrase `tone`).

RLS: `profiles`/`sessions` are gated on `auth.uid() = id|student_id`; `session_phrases`
inherit access from the owning session. Security advisors: **clean**.

Generated TypeScript types live at `apps/desktop/src/types/supabase.ts`
(regenerate after any migration).

## Teacher linking (PR 1 — the privacy core)

Migration `0003` adds the teacher-facing layer, ratified against the
[privacy decision doc](../docs/design/story-phase3-teacher-dashboard-privacy.md):

- `profiles.age_tier` — coarse `under_13` / `teen_13_17` / `adult` (**never a birthdate**).
- `teacher_student_links` — `pending` → `accepted` → `revoked`, with a consent
  audit trail (`consented_by`, `consent_at`, `consenting_adult_id`).
- `assignments` — a teacher writes for an accepted-linked student; the student reads.
- **The RLS swap:** `sessions` / `session_phrases` SELECT changes from self-only to
  **owner OR an accepted-linked teacher**. INSERT/UPDATE/DELETE stay owner-only.

**Safety property:** nothing a teacher can read opens up until a link reaches
`status='accepted'`. With zero accepted links the behaviour is identical to the
self-only model — it ships *dark* until the consent flow (a later PR) exists.

The *who-may-accept* rule for under-13 (parent only) is enforced at the app layer
and recorded in the consent columns; full parent↔child account modelling is a
follow-up. RLS guarantees only the two linked parties can touch a link row.

### RLS tests (merge-blocking)

`tests/rls_teacher_linking.sql` proves the visibility matrix against a **real
Postgres**: a teacher reads a student's recaps *only* via an accepted link, never
via a pending/absent link, and a revoke closes access immediately. It seeds
throwaway data, asserts as each persona (`set role authenticated` + JWT claim),
and rolls back to leave zero residue. Wired into CI as the **Supabase RLS** job
(`.github/workflows/supabase-rls.yml`); run locally with:

```
supabase start
for f in supabase/migrations/*.sql; do psql "$DB_URL" -v ON_ERROR_STOP=1 -f "$f"; done
psql "$DB_URL" -v ON_ERROR_STOP=1 -f supabase/tests/rls_teacher_linking.sql
```

## Still gated — pending later PRs

The consent/invite UI flow, the teacher web app (roster → feed → assignments →
analytics), and the under-13 parent-account model are the remaining work, sliced
in `docs/design/story-phase3-teacher-dashboard.md` §4. **Public launch** of
teacher-linking is gated on counsel review of these RLS tests + the privacy
notice (privacy doc §1).

## Applying migrations

Migrations in `migrations/` are the source of truth and have been applied to the
project above. With the Supabase CLI linked to the project:

```
supabase db push          # apply pending migrations
supabase gen types typescript --linked > apps/desktop/src/types/supabase.ts
```
