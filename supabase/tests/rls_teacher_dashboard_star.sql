-- RLS verification for the Teacher Dashboard star schema (#449 T3).
--
-- Proves, against a REAL Postgres, the doc's rule (teacher-dashboard-
-- datamodel.md §3): students write/read ONLY their own facts; a teacher
-- SELECTs facts ONLY through an ACTIVE enrollment in their OWN classroom;
-- activation is reachable ONLY via redeem_join_code() (with consent — the
-- COPPA gate included); revocation closes visibility immediately; the
-- rollup matviews are not client-readable at all.
--
-- Same harness as rls_teacher_linking.sql: one PL/pgSQL block, throwaway
-- seed data, persona switching via `set local role authenticated` + JWT
-- claim, everything rolled back via an exception sentinel. Any leak raises
-- -> psql exits non-zero -> CI fails.
--
-- Run locally against the Supabase stack:
--   supabase start && supabase db reset
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f supabase/tests/rls_teacher_dashboard_star.sql

do $$
declare
  t uuid; t2 uuid; a uuid; b uuid; c uuid; d uuid; e uuid;
  k uuid; k2 uuid; mat uuid; sess_a uuid; sess_c uuid; sess_d uuid;
  v_code text; cnt int; ok boolean;
begin
  t := gen_random_uuid(); t2 := gen_random_uuid();
  a := gen_random_uuid(); b := gen_random_uuid(); c := gen_random_uuid();
  d := gen_random_uuid(); e := gen_random_uuid();

  -- Seed auth users (handle_new_user auto-creates profiles).
  insert into auth.users (id) values (t), (t2), (a), (b), (c), (d), (e);
  update public.profiles set role='teacher', age_tier='adult',      display_name='Ms T'  where id=t;
  update public.profiles set role='teacher', age_tier='adult',      display_name='Mr T2' where id=t2;
  update public.profiles set role='student', age_tier='teen_13_17', display_name='A'     where id=a;
  update public.profiles set role='student', age_tier='teen_13_17', display_name='B'     where id=b;
  update public.profiles set role='student', age_tier='adult',      display_name='C'     where id=c;
  update public.profiles set role='student', age_tier='teen_13_17', display_name='D'     where id=d;
  update public.profiles set role='student', age_tier='under_13',   display_name='E'     where id=e;

  insert into public.classrooms (teacher_id, name) values (t, 'Band 1') returning id into k;
  insert into public.classrooms (teacher_id, name) values (t2, 'Band 2') returning id into k2;

  -- A active (student consent), B invited, C active (parent consent), D unlinked.
  insert into public.enrollments (classroom_id, student_id, status, consent, consented_at)
  values (k, a, 'active', 'student', now()),
         (k, b, 'invited', null, null),
         (k, c, 'active', 'parent', now());

  -- ── the schema gate: an active enrollment WITHOUT consent must be refused ──
  -- (even from the superuser seeding path — the CHECK binds every writer)
  ok := false;
  begin
    insert into public.enrollments (classroom_id, student_id, status)
    values (k, d, 'active');
  exception when check_violation then ok := true;
  end;
  if not ok then raise exception 'FAIL: active enrollment without consent must violate CHECK'; end if;

  -- Facts. A: silence-heavy session (integrity-flagged) + exercise + tool event.
  -- C: healthy session. D: thin session (flagged, but D is UNLINKED — no
  -- teacher may ever see it).
  insert into public.dim_material (spec_hash, label, source, kind)
  values ('fnv-cell-1', 'Minor triad, enclosed, descending', 'explore', 'cell')
  returning material_id into mat;

  insert into public.fact_session (student_id, device_session_id, started_at, duration_secs,
                                   played_secs, note_count, silence_ratio, phrase_count, instrument)
  values (a, 'dev-a-1', now(), 1800, 120, 40, 0.93, 4, 'Trumpet') returning session_id into sess_a;
  insert into public.fact_session (student_id, device_session_id, started_at, duration_secs,
                                   played_secs, note_count, silence_ratio, phrase_count, instrument)
  values (c, 'dev-c-1', now(), 700, 600, 800, 0.14, 8, 'Voice') returning session_id into sess_c;
  insert into public.fact_session (student_id, device_session_id, started_at, duration_secs,
                                   played_secs, note_count, silence_ratio, phrase_count, instrument)
  values (d, 'dev-d-1', now(), 1200, 10, 2, 0.99, 1, 'Trumpet') returning session_id into sess_d;

  insert into public.fact_phrase (session_id, phrase_index, start_secs, end_secs, note_count)
  values (sess_a, 0, 10, 40, 20);
  insert into public.fact_exercise (student_id, session_id, device_log_id, logged_at, material_id, tonic, difficulty, accuracy)
  values (a, sess_a, 1, now(), mat, 7, 2, 0.8);
  insert into public.fact_tool_event (student_id, session_id, device_event_id, at_secs, kind, params)
  values (a, sess_a, 1, 12.5, 'pocket_start', '{"bpm": 90}');

  -- ── teacher T sees ONLY active-enrolled students' facts (A + C) ───────────
  perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.fact_session;
  if cnt <> 2 then raise exception 'FAIL: teacher must see exactly A+C sessions (2), saw %', cnt; end if;
  select count(*) into cnt from public.fact_session where student_id in (b, d);
  if cnt <> 0 then raise exception 'FAIL: teacher must NOT see invited(B)/unlinked(D) sessions'; end if;
  select count(*) into cnt from public.fact_phrase;
  if cnt <> 1 then raise exception 'FAIL: teacher should see A''s phrase through the session, saw %', cnt; end if;
  select count(*) into cnt from public.fact_exercise;
  if cnt <> 1 then raise exception 'FAIL: teacher should see A''s exercise, saw %', cnt; end if;
  select count(*) into cnt from public.fact_tool_event;
  if cnt <> 1 then raise exception 'FAIL: teacher should see A''s tool event, saw %', cnt; end if;
  -- Roster names: active students' profiles only (plus the teacher's own row).
  select count(*) into cnt from public.profiles where id in (a, b, c, d, e);
  if cnt <> 2 then raise exception 'FAIL: teacher must see exactly A+C profiles, saw %', cnt; end if;

  -- Teachers never write facts.
  ok := false;
  begin
    insert into public.fact_session (student_id, device_session_id, started_at, duration_secs)
    values (a, 'forged-by-teacher', now(), 60);
  exception when insufficient_privilege then ok := true;
  end;
  if not ok then raise exception 'FAIL: teacher insert into fact_session must be blocked'; end if;

  -- A teacher cannot ACTIVATE an enrollment — even with consent columns
  -- filled in (fabricated consent). Only redeem_join_code() activates.
  ok := false;
  begin
    update public.enrollments set status='active', consent='student', consented_at=now()
     where classroom_id = k and student_id = b;
  exception when insufficient_privilege then ok := true;
  end;
  if not ok then raise exception 'FAIL: teacher must not be able to activate an enrollment'; end if;

  -- Rollup matviews are not client-readable (no RLS on matviews — revoked).
  ok := false;
  begin
    select count(*) into cnt from public.mv_student_day;
  exception when insufficient_privilege then ok := true;
  end;
  if not ok then raise exception 'FAIL: mv_student_day must be revoked from authenticated'; end if;
  reset role;

  -- ── other teacher T2 sees nothing of classroom K ──────────────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', t2::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.fact_session;
  if cnt <> 0 then raise exception 'FAIL: T2 has no active enrollments and must see 0 sessions, saw %', cnt; end if;
  select count(*) into cnt from public.enrollments;
  if cnt <> 0 then raise exception 'FAIL: T2 must not see classroom K''s roster, saw %', cnt; end if;
  reset role;

  -- ── student A: own facts only; own-row writes only; no classroom reads ────
  perform set_config('request.jwt.claims', json_build_object('sub', a::text)::text, true);
  set local role authenticated;
  select count(*) into cnt from public.fact_session;
  if cnt <> 1 then raise exception 'FAIL: student A must see only own session, saw %', cnt; end if;
  -- Idempotent re-push: same (student, device session) upserts, no duplicate.
  insert into public.fact_session (student_id, device_session_id, started_at, duration_secs)
  values (a, 'dev-a-1', now(), 1801)
  on conflict (student_id, device_session_id)
  do update set duration_secs = excluded.duration_secs;
  select count(*) into cnt from public.fact_session where student_id = a;
  if cnt <> 1 then raise exception 'FAIL: re-push must upsert, not duplicate; saw % rows', cnt; end if;
  -- Writing under another student_id is blocked.
  ok := false;
  begin
    insert into public.fact_session (student_id, device_session_id, started_at, duration_secs)
    values (b, 'forged-by-a', now(), 60);
  exception when insufficient_privilege then ok := true;
  end;
  if not ok then raise exception 'FAIL: student A must not insert facts for B'; end if;
  -- Hanging an exercise on someone else's session is blocked.
  ok := false;
  begin
    insert into public.fact_exercise (student_id, session_id, device_log_id, logged_at, material_id, tonic, difficulty)
    values (a, sess_c, 99, now(), mat, 3, 1);
  exception when insufficient_privilege then ok := true;
  end;
  if not ok then raise exception 'FAIL: student A must not attach an exercise to C''s session'; end if;
  -- Students never read classroom rows (join codes live there).
  select count(*) into cnt from public.classrooms;
  if cnt <> 0 then raise exception 'FAIL: student must see 0 classroom rows, saw %', cnt; end if;
  -- A student cannot self-activate (only revoke).
  ok := false;
  begin
    update public.enrollments set status='invited' where student_id = a;
  exception when insufficient_privilege then ok := true;
  end;
  if not ok then raise exception 'FAIL: student must not be able to set a non-revoked status'; end if;

  -- ── MF1 (review round 1): consent-audit forgery / cross-classroom write ──
  -- The reviewer's executed attack: riding the self-revoke policy (whose
  -- WITH CHECK pins only student_id + status), a student relocates their own
  -- enrollment row into an UNRELATED teacher's classroom while forging the
  -- parent-consent audit columns. The victim teacher would then see a ghost
  -- row with a fabricated consent trail. MUST-BLOCK: the freeze trigger
  -- raises and the row is untouched.
  ok := false;
  begin
    update public.enrollments
       set status = 'revoked', classroom_id = k2, consent = 'parent',
           consented_at = now(), consenting_adult_id = t2
     where student_id = a;
  exception when others then ok := true;
  end;
  if not ok then raise exception 'FAIL: MF1 — student rewrote consent/classroom columns (forged ghost row written)'; end if;
  reset role;
  select count(*) into cnt from public.enrollments
   where student_id = a and classroom_id = k and status = 'active'
     and consent = 'student' and consenting_adult_id is null;
  if cnt <> 1 then raise exception 'FAIL: MF1 — A''s enrollment row was mutated by the blocked update'; end if;

  -- ── the join-code path: the ONLY road to status=active ────────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
  set local role authenticated;
  select public.issue_join_code(k) into v_code;
  -- Issuing for someone else's classroom is refused.
  ok := false;
  begin
    perform public.issue_join_code(k2);
  exception when others then ok := (sqlerrm like '%not owned%' or sqlerrm like '%not found%');
  end;
  if not ok then raise exception 'FAIL: issue_join_code must refuse a classroom you don''t own'; end if;
  reset role;

  -- Invited student B redeems -> active; teacher immediately sees B.
  perform set_config('request.jwt.claims', json_build_object('sub', b::text)::text, true);
  set local role authenticated;
  perform public.redeem_join_code(v_code, 'student');
  reset role;
  select count(*) into cnt from public.enrollments
   where classroom_id = k and student_id = b and status = 'active' and consent = 'student';
  if cnt <> 1 then raise exception 'FAIL: redeem must activate B with recorded consent'; end if;

  -- Under-13 E: student consent refused (COPPA gate), parent consent works.
  perform set_config('request.jwt.claims', json_build_object('sub', e::text)::text, true);
  set local role authenticated;
  ok := false;
  begin
    perform public.redeem_join_code(v_code, 'student');
  exception when others then ok := (sqlerrm like '%parental consent%');
  end;
  if not ok then raise exception 'FAIL: under-13 redeem with student consent must be refused'; end if;
  perform public.redeem_join_code(v_code, 'parent');
  reset role;
  select count(*) into cnt from public.enrollments
   where classroom_id = k and student_id = e and status = 'active' and consent = 'parent';
  if cnt <> 1 then raise exception 'FAIL: under-13 redeem with parent consent must activate'; end if;

  -- ── revocation closes teacher visibility immediately ──────────────────────
  perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
  set local role authenticated;
  update public.enrollments set status='revoked', revoked_at=now()
   where classroom_id = k and student_id = c;
  select count(*) into cnt from public.fact_session where student_id = c;
  if cnt <> 0 then raise exception 'FAIL: after revoke teacher must see 0 of C''s sessions, saw %', cnt; end if;
  select count(*) into cnt from public.profiles where id = c;
  if cnt <> 0 then raise exception 'FAIL: after revoke teacher must not see C''s profile'; end if;
  reset role;

  -- A revoked student cannot let themselves back in with a still-live code.
  perform set_config('request.jwt.claims', json_build_object('sub', c::text)::text, true);
  set local role authenticated;
  ok := false;
  begin
    perform public.redeem_join_code(v_code, 'student');
  exception when others then ok := (sqlerrm like '%revoked%');
  end;
  if not ok then raise exception 'FAIL: revoked enrollment must not be re-activatable by code'; end if;
  reset role;

  -- Expired codes are dead.
  update public.classrooms set join_code_expires_at = now() - interval '1 minute' where id = k;
  perform set_config('request.jwt.claims', json_build_object('sub', d::text)::text, true);
  set local role authenticated;
  ok := false;
  begin
    perform public.redeem_join_code(v_code, 'student');
  exception when others then ok := (sqlerrm like '%invalid or expired%');
  end;
  if not ok then raise exception 'FAIL: expired join code must be refused'; end if;
  reset role;

  -- ── live views scope by invoker RLS (security_invoker; Postgres >= 15) ────
  if current_setting('server_version_num')::int >= 150000 then
    -- Defensive (review round 1): the reloption must actually be present on
    -- both live views. If security_invoker is ever lost (e.g. a careless
    -- CREATE OR REPLACE), v_session_integrity fails OPEN school-wide —
    -- owner-rights views bypass RLS entirely.
    select count(*) into cnt from pg_class
     where relnamespace = 'public'::regnamespace
       and relname in ('v_roster_heat', 'v_session_integrity')
       and reloptions @> array['security_invoker=true'];
    if cnt <> 2 then
      raise exception 'FAIL: v_roster_heat/v_session_integrity must carry security_invoker=true (fails OPEN otherwise)';
    end if;
    perform set_config('request.jwt.claims', json_build_object('sub', t::text)::text, true);
    set local role authenticated;
    -- Roster heat: active students with synced sessions only (A + B; B has
    -- no facts so no row; C revoked; D unlinked).
    select count(*) into cnt from public.v_roster_heat where student_id not in (a, b);
    if cnt <> 0 then raise exception 'FAIL: v_roster_heat must only show active-enrolled students'; end if;
    select count(*) into cnt from public.v_roster_heat where student_id = a and display_name = 'A';
    if cnt < 1 then raise exception 'FAIL: v_roster_heat must carry A''s display_name for the teacher'; end if;
    -- Integrity panel: A's silence-heavy session is flagged; D's thin
    -- session exists but is invisible (unlinked).
    select count(*) into cnt from public.v_session_integrity where student_id = a;
    if cnt <> 1 then raise exception 'FAIL: integrity panel must flag A''s silence-heavy session'; end if;
    select count(*) into cnt from public.v_session_integrity where student_id in (c, d);
    if cnt <> 0 then raise exception 'FAIL: integrity panel must not leak revoked/unlinked students'; end if;
    reset role;
  else
    raise notice 'SKIP: security_invoker view assertions need Postgres >= 15 (this is %)',
      current_setting('server_version');
  end if;

  -- ── the legitimate self-serve leave still works after the MF1 freeze ──────
  -- (status + revoked_at only — the columns the trigger leaves writable)
  perform set_config('request.jwt.claims', json_build_object('sub', b::text)::text, true);
  set local role authenticated;
  update public.enrollments set status = 'revoked', revoked_at = now()
   where student_id = b and classroom_id = k;
  reset role;
  select count(*) into cnt from public.enrollments
   where student_id = b and classroom_id = k and status = 'revoked' and revoked_at is not null;
  if cnt <> 1 then raise exception 'FAIL: student self-revoke (status-only) must still work'; end if;

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
