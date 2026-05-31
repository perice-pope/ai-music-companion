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

## NOT yet created — gated on privacy sign-off

The **teacher ↔ student linking** tables (`teacher_student_links`, `assignments`)
and the teacher-facing read policies are deliberately **not** in this schema.
That's the surface where a minor's data becomes visible to a third party
(the teacher), and it needs the FERPA/COPPA posture + consent-flow decisions
from the design doc's Open Questions **before** it's built. See
`docs/design/story-phase3-teacher-dashboard.md` §6.

## Applying migrations

Migrations in `migrations/` are the source of truth and have been applied to the
project above. With the Supabase CLI linked to the project:

```
supabase db push          # apply pending migrations
supabase gen types typescript --linked > apps/desktop/src/types/supabase.ts
```
