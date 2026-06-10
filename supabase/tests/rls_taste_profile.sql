-- RLS verification for the personalization taste_profile (Phase 4).
--
-- Proves, against a REAL Postgres, that a taste_profile row is owner-only:
-- an owner reads/writes their own row, no other authenticated user can read or
-- mutate it, and — unlike `sessions` — an ACCEPTED-linked teacher gets NO
-- access (preference data is not in the teacher-shared visibility set; see
-- docs/architecture/platform-spine-personalization.md §Privacy).
--
-- Same shape as rls_teacher_linking.sql: one PL/pgSQL block seeds throwaway
-- auth users, switches to the `authenticated` role with each persona's JWT
-- claim, asserts the visibility/mutation matrix, then rolls everything back via
-- an exception so it leaves zero residue. Any leak raises -> psql exits
-- non-zero -> CI fails.
--
-- Run locally against the Supabase stack:
--   supabase start && supabase db reset
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f supabase/tests/rls_taste_profile.sql

do $$
declare
  t uuid; a uuid; b uuid; cnt int; ok boolean;
begin
  t := gen_random_uuid(); a := gen_random_uuid(); b := gen_random_uuid();

  -- Seed auth users (the handle_new_user trigger auto-creates profile rows).
  insert into auth.users (id) values (t), (a), (b);
  update public.profiles set role='teacher', age_tier='adult'    where id=t;
  update public.profiles set role='student', age_tier='under_13' where id=a;
  update public.profiles set role='student', age_tier='adult'    where id=b;

  -- Owner A has a taste profile; teacher T is accepted-linked to A (which DOES
  -- open A's sessions to T, but must NOT open A's taste_profile).
  insert into public.taste_profile (user_id, genres, artists, goals, experience, is_under_13)
  values (a, '["hip-hop","gospel"]'::jsonb, '["Kendrick Lamar"]'::jsonb,
          '["play in church band"]'::jsonb, 'beginner', true);
  insert into public.teacher_student_links (teacher_id, student_id, status, consented_by, consent_at, consenting_adult_id)
  values (t, a, 'accepted', 'parent', now(), t);

  -- ── owner A sees ONLY their own taste profile ─────────────────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', a::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.taste_profile;
  if cnt <> 1 then raise exception 'FAIL: owner A should see exactly their 1 taste_profile, saw %', cnt; end if;
  select count(*) into cnt from public.taste_profile where user_id = a;
  if cnt <> 1 then raise exception 'FAIL: owner A must see their own row'; end if;
  -- Owner can update their own row.
  update public.taste_profile set experience = 'intermediate' where user_id = a;
  get diagnostics cnt = row_count;
  if cnt <> 1 then raise exception 'FAIL: owner A must be able to update own row, updated %', cnt; end if;
  reset role;

  -- ── accepted-linked teacher T sees NO taste profile (not in shared set) ───
  perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.taste_profile;
  if cnt <> 0 then raise exception 'FAIL: linked teacher must NOT see any taste_profile, saw %', cnt; end if;
  -- A teacher write must not silently affect the owner's row.
  update public.taste_profile set experience = 'advanced' where user_id = a;
  get diagnostics cnt = row_count;
  if cnt <> 0 then raise exception 'FAIL: linked teacher must NOT update owner row, updated %', cnt; end if;
  reset role;

  -- ── unrelated user B sees nothing and cannot insert for someone else ──────
  perform set_config('request.jwt.claims', json_build_object('sub', b::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.taste_profile;
  if cnt <> 0 then raise exception 'FAIL: unrelated user B must see no taste_profile, saw %', cnt; end if;
  -- B may not forge a row owned by A (insert WITH CHECK must reject it).
  ok := true;
  begin
    insert into public.taste_profile (user_id, experience) values (a, 'beginner');
    ok := false;  -- should have been rejected by RLS
  exception when others then
    ok := true;   -- RLS rejection is the expected path
  end;
  if not ok then raise exception 'FAIL: user B forged a taste_profile row for A'; end if;
  reset role;

  -- ── confirm the owner edit stuck and no foreign write changed it ──────────
  select (experience = 'intermediate') into ok from public.taste_profile where user_id = a;
  if not ok then raise exception 'FAIL: owner edit lost or overwritten by a foreign write'; end if;

  raise notice 'ALL TASTE_PROFILE RLS ASSERTIONS PASSED';
  raise exception 'ROLLBACK_SENTINEL';  -- undo all seed data, leave no residue
exception
  when others then
    if sqlerrm = 'ROLLBACK_SENTINEL' then
      raise notice 'taste_profile RLS verification passed; test data rolled back';
    else
      raise;  -- a real assertion failure or error: propagate (CI fails)
    end if;
end $$;
