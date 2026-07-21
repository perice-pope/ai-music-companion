-- #449 T3 follow-up: the explicit table grants the shipped policies presume.
--
-- Root cause (main's red "RLS policy tests" job after #467): on the real
-- stack, client roles reach ONLY what migrations grant explicitly — the
-- 0003 convention (`grant … on public.teacher_student_links to
-- authenticated`). A policy without the matching grant is dead weight:
-- `permission denied` fires at the privilege layer before RLS is consulted.
-- Migration 0001 granted NOTHING on profiles/sessions/session_phrases; its
-- policies (and 0003's teacher-swap policies, and 0006's roster views) have
-- been presuming grants that were never made. The 0006 star suite's
-- teacher-roster read is simply the first test to exercise profiles as
-- `authenticated`:
--     psql:supabase/tests/rls_teacher_dashboard_star.sql:321:
--         ERROR:  permission denied for table profiles
-- Re-running the whole suite set against a grant-faithful shadow stack also
-- exposed the same hole one file later — rls_teacher_linking's teacher read
-- of `sessions` ("permission denied for table sessions"): CI never reached
-- it only because the star suite sorts first in the loop. Both are fixed
-- here.
--
-- Audit performed (every table/view a policy, 0006 view, or RLS suite
-- touches as authenticated/anon):
--   granted already ✓ : teacher_student_links + assignments (0003),
--     taste_profile (0004), learner_model (0005), classrooms, enrollments,
--     fact_session (s/i/u), fact_phrase (s/i), fact_exercise (s/i),
--     fact_tool_event (s/i), dim_date (s), dim_material (s/i),
--     v_roster_heat (s), v_session_integrity (s) — all 0006.
--   missing ✗ (fixed below): profiles, sessions, session_phrases (0001).
--   deliberately ungranted, stays that way: mv_student_day /
--     mv_material_key (no RLS on matviews — service_role only, 0006),
--     everything for anon (no policy anywhere addresses anon).
-- RLS remains the row gate everywhere; these grants only open the table
-- surface the policies then filter.

-- ── profiles ────────────────────────────────────────────────────────────────
-- select: profiles_select_own (0001) + profiles_select_enrolled_teacher
--         (0006 — the roster display_name read), and the invoker-rights
--         v_roster_heat / v_session_integrity joins (security_invoker views
--         require the QUERYING role to hold SELECT on the underlying table).
-- insert: profiles_insert_own (0001) — the client-side self-provision path
--         the policy has promised since 0001.
-- update: profiles_update_own (0001) — display-name edits.
-- (no delete: no delete policy exists; account deletion cascades from
--  auth.users server-side.)
grant select, insert, update on public.profiles to authenticated;

-- ── sessions ────────────────────────────────────────────────────────────────
-- select: sessions_select_owner_or_linked_teacher (0003 swap) — exercised by
--         rls_teacher_linking as authenticated.
-- insert/update: sessions_insert_own / sessions_update_own (0001) — the
--         AccountPanel/accountStore session push runs as authenticated with
--         the publishable key; without this grant that push is
--         permission-denied on any stack provisioned grant-faithfully.
-- delete: sessions_delete_own (0001) — the policy exists; grant matches it.
--         (`sessions` is the legacy recap store, not a star-schema fact
--         table — the facts keep their no-delete append-only posture from
--         0006, which this migration does not touch.)
grant select, insert, update, delete on public.sessions to authenticated;

-- ── session_phrases ─────────────────────────────────────────────────────────
-- select: session_phrases_select_owner_or_linked_teacher (0003 swap).
-- insert: session_phrases_insert_own (0001) — the P2 phrase push (doc §2)
--         activates this table client-side.
-- (no update/delete: no such policies exist — append-only like the facts.)
grant select, insert on public.session_phrases to authenticated;

-- ── rollback notes ──────────────────────────────────────────────────────────
--   revoke select, insert, update on public.profiles from authenticated;
--   revoke select, insert, update, delete on public.sessions from authenticated;
--   revoke select, insert on public.session_phrases from authenticated;
