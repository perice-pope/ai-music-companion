-- RLS verification for the teacher-linking privacy core (PR 1).
--
-- This is the merge-blocking privacy test (privacy doc §5): it proves, against a
-- REAL Postgres, that a teacher can read a student's recaps ONLY through an
-- accepted link, and never through a pending/absent link or after a revoke.
--
-- It runs as a single PL/pgSQL block that seeds throwaway auth users + sessions,
-- switches to the `authenticated` role with each persona's JWT claim, asserts
-- the visibility matrix, and then rolls everything back via an exception so it
-- leaves zero residue. Any leak raises an exception -> psql exits non-zero ->
-- CI fails.
--
-- Run locally against the Supabase stack:
--   supabase start && supabase db reset
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f supabase/tests/rls_teacher_linking.sql

do $$
declare
  t uuid; a uuid; b uuid; c uuid; cnt int;
begin
  t := gen_random_uuid(); a := gen_random_uuid(); b := gen_random_uuid(); c := gen_random_uuid();

  -- Seed auth users (the handle_new_user trigger auto-creates profile rows).
  insert into auth.users (id) values (t), (a), (b), (c);
  update public.profiles set role='teacher', age_tier='adult'      where id=t;
  update public.profiles set role='student', age_tier='under_13'   where id=a;
  update public.profiles set role='student', age_tier='teen_13_17' where id=b;
  update public.profiles set role='student', age_tier='adult'      where id=c;

  insert into public.sessions (student_id, instrument, started_at, ended_at, duration_secs, phrase_count)
  values (a,'Trumpet',now(),now(),60,1),
         (b,'Voice',  now(),now(),60,1),
         (c,'Trumpet',now(),now(),60,1);

  -- T->A accepted, T->B pending; C unlinked.
  insert into public.teacher_student_links (teacher_id, student_id, status, consented_by, consent_at, consenting_adult_id)
  values (t,a,'accepted','parent',now(),t),
         (t,b,'pending', null,    null,null);

  -- ── teacher T sees ONLY the accepted student's (A) session ────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.sessions;
  if cnt <> 1 then raise exception 'FAIL: teacher should see exactly 1 session (accepted A), saw %', cnt; end if;
  select count(*) into cnt from public.sessions where student_id = a;
  if cnt <> 1 then raise exception 'FAIL: teacher should see A''s session'; end if;
  select count(*) into cnt from public.sessions where student_id in (b,c);
  if cnt <> 0 then raise exception 'FAIL: teacher must NOT see pending(B)/unlinked(C), saw %', cnt; end if;
  reset role;

  -- ── student A sees ONLY their own session ─────────────────────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', a::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.sessions;
  if cnt <> 1 then raise exception 'FAIL: student A should see only own 1 session, saw %', cnt; end if;
  select count(*) into cnt from public.sessions where student_id <> a;
  if cnt <> 0 then raise exception 'FAIL: student A must not see other students'; end if;
  reset role;

  -- ── unlinked student C sees no links ──────────────────────────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', c::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.teacher_student_links;
  if cnt <> 0 then raise exception 'FAIL: C has no links and must see none, saw %', cnt; end if;
  reset role;

  -- ── revoke closes teacher access immediately ──────────────────────────────
  update public.teacher_student_links set status='revoked' where teacher_id=t and student_id=a;
  perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.sessions;
  if cnt <> 0 then raise exception 'FAIL: after unlink teacher must see 0 sessions, saw %', cnt; end if;
  reset role;

  raise notice 'ALL RLS ASSERTIONS PASSED';
  raise exception 'ROLLBACK_SENTINEL';  -- undo all seed data, leave no residue
exception
  when others then
    if sqlerrm = 'ROLLBACK_SENTINEL' then
      raise notice 'RLS verification passed; test data rolled back';
    else
      raise;  -- a real assertion failure or error: propagate (CI fails)
    end if;
end $$;
