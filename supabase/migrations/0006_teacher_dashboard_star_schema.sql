-- Teacher Dashboard track, T3 (#449): the cloud star schema behind the
-- dashboard — classrooms/enrollments (the roster + consent boundary), the
-- fact tables the device projection (P1–P4) lands in, the dimensions BI
-- queries join, and the rollups behind roster heat + the integrity panel.
-- Spec: docs/architecture/teacher-dashboard-datamodel.md §3–§4 and
-- docs/specs/449-t3-cloud-schema.md. SQL-only slice: no app code reads or
-- writes these tables yet (the T2 projection wiring and the T4 dashboard
-- are separate slices).
--
-- Relationship to what already shipped:
--   * 0001/0003 `sessions` / `session_phrases` / `teacher_student_links` /
--     `assignments` are UNTOUCHED — the legacy recap push keeps working.
--     The star schema is additive; the dashboard reads facts, not `sessions`.
--   * Teacher visibility here keys off classrooms/enrollments (issue #449
--     §2), NOT teacher_student_links: the dashboard product is per-CLASSROOM
--     (priced per-classroom/year, doc §Decisions), while the 0003 links stay
--     the 1:1 assignment relationship. Same idiom (status-gated EXISTS
--     join), new roster shape.
--   * Function hardening follows 0002: security definer, pinned empty
--     search_path, EXECUTE revoked from the public RPC surface.
--
-- Safety property (same shape as 0003): nothing a teacher can read opens
-- until an enrollment reaches status='active' — and 'active' is reachable
-- ONLY through redeem_join_code(), which records consent (the schema-level
-- CHECK refuses an active row without it). Revocation, by either party,
-- closes visibility on the next query by construction: every teacher policy
-- re-evaluates the status='active' join. There is no grant to clean up and
-- no cleanup job in the trust path.

-- ── classrooms (the roster container; issue #449 §2) ─────────────────────────
create table public.classrooms (
    id                   uuid primary key default gen_random_uuid(),
    teacher_id           uuid not null references public.profiles (id) on delete cascade,
    name                 text not null,
    -- seat_product_id  uuid references public.products (id),
    --   ^ TODO(#449 / commerce spine): `products` is designed in
    --     docs/architecture/platform-spine-commerce.md but NOT migrated yet
    --     (checked: no migration creates it). Adding an FK to a table that
    --     doesn't exist would fail; inventing the table here would fork the
    --     commerce spine. The migration that lands `products` adds this
    --     column + FK, wiring the per-classroom/year seat license.
    -- Join code: short, expiring, re-issuable (issue #449 §2). Stored plain
    -- because ONLY the owning teacher can select classroom rows (see RLS
    -- below); students never read it — they redeem through the definer
    -- function, which is the only student-side path that touches it.
    join_code            text,
    join_code_expires_at timestamptz,
    created_at           timestamptz not null default now()
);
alter table public.classrooms enable row level security;
create index classrooms_teacher_idx on public.classrooms (teacher_id);
-- Partial unique: two classrooms may not share a live code (redeem must
-- resolve to exactly one roster).
create unique index classrooms_join_code_idx
    on public.classrooms (join_code) where join_code is not null;
grant select, insert, update, delete on public.classrooms to authenticated;

-- Teacher-only, all four verbs. There is deliberately NO student select
-- policy: the row carries the live join_code, so any broader select would
-- let a student (or another teacher) harvest join codes and self-enroll
-- into rosters they were never invited to.
-- blocks: any non-owner selecting a classroom row (code harvesting);
--         creating a classroom owned by someone else; editing/deleting
--         another teacher's classroom.
create policy classrooms_select_teacher on public.classrooms
    for select using ((select auth.uid()) = teacher_id);
create policy classrooms_insert_teacher on public.classrooms
    for insert with check ((select auth.uid()) = teacher_id);
create policy classrooms_update_teacher on public.classrooms
    for update using ((select auth.uid()) = teacher_id)
    with check ((select auth.uid()) = teacher_id);
create policy classrooms_delete_teacher on public.classrooms
    for delete using ((select auth.uid()) = teacher_id);

-- ── enrollments (the consent boundary; issue #449 §2) ────────────────────────
create table public.enrollments (
    id                  uuid primary key default gen_random_uuid(),
    classroom_id        uuid not null references public.classrooms (id) on delete cascade,
    student_id          uuid not null references public.profiles (id) on delete cascade,
    status              text not null default 'invited'
                            check (status in ('invited', 'active', 'revoked')),
    -- Consent audit trail — mirrors the 0003 teacher_student_links columns.
    -- For under-13, consent must come from a parent (teacher-audit COPPA
    -- gate, verbatim); redeem_join_code() enforces that against
    -- profiles.age_tier. consenting_adult_id stays null until the
    -- parent-account model exists (the 0003 follow-up, unchanged).
    consent             text check (consent in ('student', 'parent')),
    consented_at        timestamptz,
    consenting_adult_id uuid references public.profiles (id) on delete set null,
    joined_via_code_at  timestamptz,   -- when the join code was redeemed (server clock)
    revoked_at          timestamptz,
    created_at          timestamptz not null default now(),
    unique (classroom_id, student_id),
    -- THE under-13 schema gate (#449 T3 requirement): an ACTIVE enrollment
    -- must carry consent. This CHECK binds every writer — client policies,
    -- the definer function, even the service role. The app-side consent
    -- screen is a later slice; the schema refuses an unconsented active row
    -- regardless.
    -- blocks: INSERT or UPDATE that lands status='active' with consent or
    --         consented_at null, no matter which role attempts it.
    constraint enrollments_active_requires_consent
        check (status <> 'active' or (consent is not null and consented_at is not null))
);
alter table public.enrollments enable row level security;
create index enrollments_student_idx on public.enrollments (student_id);
grant select, insert, update, delete on public.enrollments to authenticated;

-- Either party sees the rows they are party to.
-- blocks: a student seeing classmates' enrollments; a teacher seeing
--         another classroom's roster.
create policy enrollments_select_student on public.enrollments
    for select using ((select auth.uid()) = student_id);
create policy enrollments_select_teacher on public.enrollments
    for select using (
        exists (select 1 from public.classrooms c
                where c.id = enrollments.classroom_id
                  and c.teacher_id = (select auth.uid()))
    );

-- A teacher may create INVITED rows in their own classroom (the invite
-- path). Activation is NOT insertable over the client API by anyone.
-- blocks: a teacher inserting a pre-activated row (status must be
--         'invited'); inserting into a classroom they don't own; a student
--         self-inserting an enrollment (no student insert policy exists).
create policy enrollments_insert_invite_by_teacher on public.enrollments
    for insert with check (
        status = 'invited'
        and exists (select 1 from public.classrooms c
                    where c.id = enrollments.classroom_id
                      and c.teacher_id = (select auth.uid()))
    );

-- A teacher may update rows in their own classroom, but the resulting row
-- may only be 'invited' or 'revoked' — a teacher can revoke or re-invite,
-- NEVER activate. Activation happens only inside redeem_join_code(), where
-- consent is recorded by the consenting party.
-- blocks: a teacher flipping a row to 'active' (fabricating a student's or
--         parent's consent); a teacher touching another classroom's rows.
create policy enrollments_update_teacher on public.enrollments
    for update using (
        exists (select 1 from public.classrooms c
                where c.id = enrollments.classroom_id
                  and c.teacher_id = (select auth.uid()))
    )
    with check (
        status in ('invited', 'revoked')
        and exists (select 1 from public.classrooms c
                    where c.id = enrollments.classroom_id
                      and c.teacher_id = (select auth.uid()))
    );

-- A student may update their own row only into the 'revoked' state — the
-- self-serve "leave the roster" path. (Column-freeze beyond status is a
-- trigger-level nicety deferred; the row must still be their own and
-- revoked, which grants nothing.)
-- blocks: a student flipping their own (or anyone's) enrollment to
--         'active' or back to 'invited'; touching another student's row.
create policy enrollments_update_student_revoke on public.enrollments
    for update using ((select auth.uid()) = student_id)
    with check ((select auth.uid()) = student_id and status = 'revoked');

-- MF1 guard (review round 1): the student self-revoke policy above pins only
-- student_id + status — WITH CHECK cannot compare NEW to OLD, so on its own
-- it would let a student rewrite the OTHER columns of their row in the same
-- statement: relocate the enrollment into an unrelated teacher's classroom
-- and forge the consent audit trail (consent='parent', a fabricated
-- consenting_adult_id) while setting status='revoked'. No fact/profile leak
-- (revoked grants nothing), but a forged ghost row with a fake consent
-- record in a victim teacher's roster is consent-audit forgery. This BEFORE
-- UPDATE trigger is the second belt: when the updater is a CLIENT role
-- acting as the enrolled student (and not the classroom's owner), every
-- column except status/revoked_at is frozen.
--
-- INVOKER-rights (not SECURITY DEFINER, unlike the 0002 RPC functions):
-- current_user must remain the ROLE that issued the UPDATE. A definer trigger
-- would swap current_user to the function owner even for a client call, so
-- the current_user gate below would exempt the very attacker it must catch.
-- search_path is still pinned to '' (the 0002 hardening that DOES apply to a
-- trigger), and every reference is schema-qualified. The definer redemption
-- path (redeem_join_code, owner-rights) and service_role writers therefore
-- see current_user = the function owner / service_role — roles a client can
-- never SET ROLE to — and are exempt by the current_user check, not a
-- spoofable GUC.
-- blocks: the reviewer's executed attack, verbatim — `UPDATE enrollments
--         SET status='revoked', classroom_id=<victim>, consent='parent',
--         consented_at=now(), consenting_adult_id=<fabricated>` as the
--         student (raises 'students may only change enrollment status').
create function public.enrollments_guard_student_update()
    returns trigger
    language plpgsql
    set search_path = ''
as $$
begin
    if current_user in ('anon', 'authenticated')
       and (select auth.uid()) = old.student_id
       and not exists (select 1 from public.classrooms c
                       where c.id = old.classroom_id
                         and c.teacher_id = (select auth.uid()))
    then
        if new.student_id             is distinct from old.student_id
           or new.classroom_id        is distinct from old.classroom_id
           or new.consent             is distinct from old.consent
           or new.consented_at        is distinct from old.consented_at
           or new.consenting_adult_id is distinct from old.consenting_adult_id
           or new.joined_via_code_at  is distinct from old.joined_via_code_at
           or new.created_at          is distinct from old.created_at
        then
            raise exception 'students may only change enrollment status';
        end if;
    end if;
    return new;
end;
$$;
create trigger enrollments_guard_student_update
    before update on public.enrollments
    for each row execute function public.enrollments_guard_student_update();
-- 0002 precedent: trigger-only function — close the public RPC surface.
revoke execute on function public.enrollments_guard_student_update() from anon, authenticated, public;

-- Teacher may clear rows in their own classroom. Deleting a revoked row is
-- the re-admit path: redeem_join_code() refuses revoked rows, so a revoked
-- student can rejoin only after the teacher deletes the old row.
-- blocks: a student deleting their enrollment history; a teacher clearing
--         another classroom's rows.
create policy enrollments_delete_teacher on public.enrollments
    for delete using (
        exists (select 1 from public.classrooms c
                where c.id = enrollments.classroom_id
                  and c.teacher_id = (select auth.uid()))
    );

-- ── profiles: teachers may see actively-enrolled students' profiles ─────────
-- The dashboard renders display_name on every view; without this the roster
-- would join to nothing (profiles has only the self-select policy from 0001
-- — 0003 deliberately never widened it). Scope: ACTIVE enrollment in a
-- classroom the teacher owns; revocation closes it on the next query, same
-- as the fact policies below.
-- blocks: a teacher reading any profile of a student who is merely invited,
--         revoked, or in someone else's classroom.
create policy profiles_select_enrolled_teacher on public.profiles
    for select using (
        exists (select 1 from public.enrollments e
                join public.classrooms c on c.id = e.classroom_id
                where e.student_id = profiles.id
                  and c.teacher_id = (select auth.uid())
                  and e.status = 'active')
    );

-- ── join-code functions (0002 hardening pattern) ────────────────────────────
-- issue_join_code: teacher mints/rotates the short-lived code for their own
-- classroom. Server-side generation so codes are uniform and unambiguous
-- (no 0/O/1/I/L). Rotating the code invalidates the old one atomically.
create function public.issue_join_code(p_classroom_id uuid, p_ttl interval default interval '7 days')
    returns text
    language plpgsql
    security definer
    set search_path = ''
as $$
declare
    v_teacher uuid;
    v_code    text;
    v_bytes   bytea;
    v_attempt int;
    -- TTL clamp (review round 1): a join code is a roster invite, not a
    -- standing credential — cap the caller-supplied TTL at 30 days so no
    -- client can mint an effectively-permanent code. (A non-positive TTL
    -- just yields an already-expired code: harmless.)
    v_ttl     interval := least(coalesce(p_ttl, interval '7 days'), interval '30 days');
begin
    select c.teacher_id into v_teacher
      from public.classrooms c where c.id = p_classroom_id;
    if v_teacher is null or v_teacher is distinct from (select auth.uid()) then
        -- One message for "no such classroom" and "not yours": don't leak
        -- which classroom ids exist.
        raise exception 'classroom not found or not owned by you';
    end if;
    for v_attempt in 1..5 loop
        -- 8 chars over a 31-symbol unambiguous alphabet (~40 bits), fed from
        -- gen_random_uuid() entropy. Codes are short-lived (default 7 days)
        -- and re-issuable — this is a roster invite, not a long-lived secret.
        v_bytes := decode(md5(gen_random_uuid()::text || clock_timestamp()::text), 'hex');
        select string_agg(substr('ABCDEFGHJKMNPQRSTUVWXYZ23456789', (get_byte(v_bytes, g) % 31) + 1, 1), '')
          into v_code
          from generate_series(0, 7) g;
        begin
            update public.classrooms
               set join_code            = v_code,
                   join_code_expires_at = now() + v_ttl
             where id = p_classroom_id;
            return v_code;
        exception when unique_violation then
            -- collided with another classroom's live code; retry
            null;
        end;
    end loop;
    raise exception 'could not generate a unique join code';
end;
$$;

-- redeem_join_code: THE only path to status='active'. Runs as definer so a
-- student never needs (and never gets) SELECT on classrooms; consent is
-- recorded by the caller at the moment of activation, satisfying the
-- enrollments_active_requires_consent CHECK.
-- blocks (by raising): expired/unknown codes (single message — does not
--         reveal which); a teacher enrolling in their own classroom; an
--         under-13 profile activating with consent <> 'parent' (COPPA gate,
--         teacher-audit verbatim); re-activation of a revoked enrollment
--         (the teacher must re-admit by deleting the row first — a revoked
--         student holding a still-live code cannot let themselves back in).
-- TODO(#449 T4 / deploy slice): redeem_join_code has NO DB-level rate limit.
-- The threat (brute-forcing codes) is already impractical — an 8-char code
-- over a 31-symbol alphabet is ~40 bits and the TTL is ≤30 days, so the
-- expected guesses to hit any live code vastly exceed the window — but add
-- gateway/PostgREST rate-limiting on this RPC when the deploy config lands
-- (belt-and-suspenders; the code space is the primary defense, throttling is
-- the second). This note is the placeholder until supabase/config.toml (or
-- the edge gateway config) exists in the T4 slice.
create function public.redeem_join_code(p_code text, p_consent text)
    returns uuid
    language plpgsql
    security definer
    set search_path = ''
as $$
declare
    v_student   uuid := (select auth.uid());
    v_classroom public.classrooms%rowtype;
    v_age       text;
    v_existing  public.enrollments%rowtype;
    v_id        uuid;
begin
    if v_student is null then
        raise exception 'not signed in';
    end if;
    if p_consent is null or p_consent not in ('student', 'parent') then
        raise exception 'consent must be ''student'' or ''parent''';
    end if;
    select c.* into v_classroom
      from public.classrooms c
     where c.join_code = upper(btrim(p_code))
       and c.join_code_expires_at is not null
       and c.join_code_expires_at > now();
    if not found then
        raise exception 'invalid or expired join code';
    end if;
    if v_classroom.teacher_id = v_student then
        raise exception 'cannot enroll in your own classroom';
    end if;
    select p.age_tier into v_age from public.profiles p where p.id = v_student;
    if v_age = 'under_13' and p_consent <> 'parent' then
        raise exception 'parental consent required';
    end if;
    select e.* into v_existing
      from public.enrollments e
     where e.classroom_id = v_classroom.id and e.student_id = v_student;
    if found then
        if v_existing.status = 'revoked' then
            raise exception 'enrollment was revoked; ask the teacher to re-admit you';
        end if;
        update public.enrollments
           set status             = 'active',
               consent            = p_consent,
               consented_at       = now(),
               joined_via_code_at = now(),
               revoked_at         = null
         where id = v_existing.id
         returning id into v_id;
    else
        insert into public.enrollments
            (classroom_id, student_id, status, consent, consented_at, joined_via_code_at)
        values
            (v_classroom.id, v_student, 'active', p_consent, now(), now())
        returning id into v_id;
    end if;
    -- TODO(#449 S3 / commerce spine): once `products`/`entitlements` are
    -- migrated, activation also mints the seat entitlement here (source =
    -- 'seat_assignment', expires_at = license term).
    return v_id;
end;
$$;

-- 0002 precedent, adapted: these two ARE the intended RPC surface for
-- signed-in users, so `authenticated` keeps EXECUTE; anon and PUBLIC (the
-- implicit default grant 0002 exists to close) are revoked.
revoke execute on function public.issue_join_code(uuid, interval) from anon, public;
revoke execute on function public.redeem_join_code(text, text) from anon, public;
grant execute on function public.issue_join_code(uuid, interval) to authenticated;
grant execute on function public.redeem_join_code(text, text) to authenticated;

-- ── dim_date (calendar, in band-director units; doc §3) ─────────────────────
create table public.dim_date (
    date_key      date primary key,
    year_label    text not null,      -- school year, Aug-1 boundary: '2026-27'
    week_of_year  smallint not null,  -- ISO week
    is_school_day boolean not null    -- Mon–Fri; district holiday calendars are a v2 concern
);
alter table public.dim_date enable row level security;
grant select on public.dim_date to authenticated;
-- Read-only reference data: a select policy for signed-in users and NO
-- write policies (and no write grants).
-- blocks: any client role inserting/rewriting calendar buckets.
create policy dim_date_select_signed_in on public.dim_date
    for select using ((select auth.uid()) is not null);

insert into public.dim_date (date_key, year_label, week_of_year, is_school_day)
select d::date,
       case when extract(month from d) >= 8
            then format('%s-%s', extract(year from d)::int,
                        to_char((extract(year from d)::int + 1) % 100, 'FM00'))
            else format('%s-%s', extract(year from d)::int - 1,
                        to_char(extract(year from d)::int % 100, 'FM00'))
       end,
       extract(week from d)::smallint,
       extract(isodow from d) between 1 and 5
  from generate_series(date '2025-08-01', date '2035-07-31', interval '1 day') d;

-- ── dim_material (what was practiced; doc §3) ───────────────────────────────
-- Grain rule (RV philosophy, doc §3 verbatim): the 12 keys of one cell are
-- ONE material row — tonic lives on the fact. Upserted lazily from P3 rows.
create table public.dim_material (
    material_id uuid primary key default gen_random_uuid(),
    spec_hash   text unique,  -- FNV-1a 64 of spec_json, tonic excluded (generated cells); null for scores
    score_id    uuid,         -- pieces: the local score id. No FK — there is no cloud
                              -- scores mirror yet; the migration that mirrors scores
                              -- adds the FK (TODO #449 T4+).
    label       text not null,   -- "Minor triad, enclosed, descending" / "Haydn Concerto, mvt 1"
    source      text not null,   -- opener|lesson|explore|…|score_practice
    kind        text not null check (kind in ('cell', 'score')),
    created_at  timestamptz not null default now(),
    constraint dim_material_keyed check (
        (kind = 'cell' and spec_hash is not null)
        or (kind = 'score' and score_id is not null))
);
alter table public.dim_material enable row level security;
grant select, insert on public.dim_material to authenticated;

-- Shared dimension: readable by any signed-in user (labels of generated
-- material and piece titles — no per-student data lives here), insertable
-- (lazy upsert from the client projection, ON CONFLICT DO NOTHING), and
-- append-only from clients: no update/delete policies exist.
-- blocks: anon reads; relabeling material another student's facts point at
--         (no update policy); deleting a dim row out from under facts.
-- Known v1 trade-off, recorded: a hostile client could pre-claim a
-- spec_hash with a wrong label (label poisoning). Facts and accuracy are
-- unaffected; a server-side upsert function can replace the client insert
-- path later without schema change.
create policy dim_material_select_signed_in on public.dim_material
    for select using ((select auth.uid()) is not null);
create policy dim_material_insert_signed_in on public.dim_material
    for insert with check ((select auth.uid()) is not null);

-- ── fact_session (P1; grain: one session) ───────────────────────────────────
create table public.fact_session (
    session_id        uuid primary key default gen_random_uuid(),
    student_id        uuid not null references public.profiles (id) on delete cascade,
    device_session_id text not null,  -- the SessionRecorder SessionId — the idempotency key
    started_at        timestamptz not null,
    ended_at          timestamptz,
    duration_secs     double precision not null,  -- wall clock
    -- The integrity columns: computed ONCE, in Rust, at session close
    -- (doc §1b — the #451 played clock, persisted, never re-derived here).
    played_secs       double precision,
    note_count        integer,
    silence_ratio     double precision check (silence_ratio is null or (silence_ratio >= 0 and silence_ratio <= 1)),
    phrase_count      integer not null default 0,
    instrument        text,
    practice_mode     text,
    score_material_id uuid references public.dim_material (material_id) on delete set null,
    fingerprint       jsonb,
    app_version       text,
    created_at        timestamptz not null default now(),  -- server arrival: the F9 late-sync signal
    -- Idempotent re-push: the client upserts on (student, device session).
    unique (student_id, device_session_id)
);
alter table public.fact_session enable row level security;
create index fact_session_student_started_idx on public.fact_session (student_id, started_at desc);
-- select/insert/update only — facts are an append-only projection. There is
-- deliberately no DELETE grant or policy: rows leave via the profile-delete
-- cascade or the 12-month retention purge (service-side), never the client.
grant select, insert, update on public.fact_session to authenticated;

-- Students write/read ONLY their own facts (doc §3 rule).
-- blocks: inserting a session under another student_id; reading or
--         re-pushing (updating) anyone else's session.
create policy fact_session_select_own on public.fact_session
    for select using ((select auth.uid()) = student_id);
create policy fact_session_insert_own on public.fact_session
    for insert with check ((select auth.uid()) = student_id);
create policy fact_session_update_own on public.fact_session
    for update using ((select auth.uid()) = student_id)
    with check ((select auth.uid()) = student_id);

-- Teachers SELECT ONLY through an ACTIVE enrollment in their OWN classroom
-- (doc §3, verbatim). Revocation either direction closes the view on the
-- next query — no cleanup job in the trust path.
-- blocks: a teacher reading facts of invited/revoked/unlinked students, or
--         of students enrolled only in someone else's classroom; teachers
--         writing facts (no teacher insert/update/delete policy exists).
create policy fact_session_select_teacher on public.fact_session
    for select using (
        exists (select 1 from public.enrollments e
                join public.classrooms c on c.id = e.classroom_id
                where e.student_id = fact_session.student_id
                  and c.teacher_id = (select auth.uid())
                  and e.status = 'active')
    );

-- ── fact_phrase (P2; thin rows — no onsets vector, no pitch curves) ─────────
create table public.fact_phrase (
    session_id   uuid not null references public.fact_session (session_id) on delete cascade,
    phrase_index integer not null,
    start_secs   double precision not null,
    end_secs     double precision not null,
    note_count   integer,
    stability    double precision,
    tone         jsonb,       -- flat ToneDescriptor
    key_name     text,        -- key estimate name; null when the gate failed
    primary key (session_id, phrase_index)
);
alter table public.fact_phrase enable row level security;
grant select, insert on public.fact_phrase to authenticated;

-- Access is governed by the owning session (the 0001 session_phrases idiom).
-- blocks: attaching phrases to another student's session; reading phrases
--         of a session you don't own (student) or aren't actively linked to
--         through your own classroom (teacher).
create policy fact_phrase_select_own on public.fact_phrase
    for select using (
        exists (select 1 from public.fact_session s
                where s.session_id = fact_phrase.session_id
                  and s.student_id = (select auth.uid()))
    );
create policy fact_phrase_insert_own on public.fact_phrase
    for insert with check (
        exists (select 1 from public.fact_session s
                where s.session_id = fact_phrase.session_id
                  and s.student_id = (select auth.uid()))
    );
create policy fact_phrase_select_teacher on public.fact_phrase
    for select using (
        exists (select 1 from public.fact_session s
                join public.enrollments e on e.student_id = s.student_id and e.status = 'active'
                join public.classrooms c on c.id = e.classroom_id
                where s.session_id = fact_phrase.session_id
                  and c.teacher_id = (select auth.uid()))
    );

-- ── fact_exercise (P3; grain: one generated exercise) ───────────────────────
-- Doc rule, verbatim: `spec_json` and `seed` STAY LOCAL — replayability is a
-- device concern, not a dashboard one. There are deliberately no columns
-- for them; the material identity crossing the wire is spec_hash → dim.
create table public.fact_exercise (
    exercise_id   uuid primary key default gen_random_uuid(),
    student_id    uuid not null references public.profiles (id) on delete cascade,
    session_id    uuid references public.fact_session (session_id) on delete set null,
    device_log_id bigint,  -- local exercise_log.id; idempotency for re-push
    logged_at     timestamptz not null,
    material_id   uuid not null references public.dim_material (material_id),
    tonic         smallint not null check (tonic >= 0 and tonic <= 11),
    difficulty    smallint not null,
    accuracy      real,   -- NULL = generated but never graded (kept: abandonment is a signal — F3)
    created_at    timestamptz not null default now(),
    unique (student_id, device_log_id)
);
alter table public.fact_exercise enable row level security;
create index fact_exercise_student_logged_idx on public.fact_exercise (student_id, logged_at desc);
create index fact_exercise_material_idx on public.fact_exercise (material_id, tonic);
grant select, insert on public.fact_exercise to authenticated;

-- blocks: logging an exercise under another student_id; hanging your
--         exercise on another student's session (the session, when given,
--         must be your own); teachers writing or grading anything.
create policy fact_exercise_select_own on public.fact_exercise
    for select using ((select auth.uid()) = student_id);
create policy fact_exercise_insert_own on public.fact_exercise
    for insert with check (
        (select auth.uid()) = student_id
        and (session_id is null or exists
                (select 1 from public.fact_session s
                 where s.session_id = fact_exercise.session_id
                   and s.student_id = (select auth.uid())))
    );
create policy fact_exercise_select_teacher on public.fact_exercise
    for select using (
        exists (select 1 from public.enrollments e
                join public.classrooms c on c.id = e.classroom_id
                where e.student_id = fact_exercise.student_id
                  and c.teacher_id = (select auth.uid())
                  and e.status = 'active')
    );

-- ── fact_tool_event (P4; grain: one tool event) ─────────────────────────────
create table public.fact_tool_event (
    event_id        uuid primary key default gen_random_uuid(),
    session_id      uuid not null references public.fact_session (session_id) on delete cascade,
    student_id      uuid not null references public.profiles (id) on delete cascade,
    device_event_id bigint,  -- local practice_events.id; idempotency for re-push
    at_secs         double precision not null,  -- one clock: seconds from session start (doc §1a)
    kind            text not null,
    params          jsonb not null default '{}'::jsonb,
    created_at      timestamptz not null default now(),
    unique (session_id, device_event_id)
);
alter table public.fact_tool_event enable row level security;
create index fact_tool_event_session_idx on public.fact_tool_event (session_id, at_secs);
grant select, insert on public.fact_tool_event to authenticated;

-- blocks: attaching events to another student's session (both the
--         student_id column AND the parent session must be the caller's);
--         reading tool events without ownership or an active enrollment.
create policy fact_tool_event_select_own on public.fact_tool_event
    for select using ((select auth.uid()) = student_id);
create policy fact_tool_event_insert_own on public.fact_tool_event
    for insert with check (
        (select auth.uid()) = student_id
        and exists (select 1 from public.fact_session s
                    where s.session_id = fact_tool_event.session_id
                      and s.student_id = (select auth.uid()))
    );
create policy fact_tool_event_select_teacher on public.fact_tool_event
    for select using (
        exists (select 1 from public.enrollments e
                join public.classrooms c on c.id = e.classroom_id
                where e.student_id = fact_tool_event.student_id
                  and c.teacher_id = (select auth.uid())
                  and e.status = 'active')
    );

-- ── rollups: materialized views (doc §3 — never trusted for consent) ────────
-- Materialized views carry NO row security. They exist for the dashboard
-- backend (service side), which re-applies the active-enrollment filter in
-- every query; they are NOT exposed to client roles — the revokes below
-- make a PostgREST select fail with permission denied. Consent/revocation
-- is enforced only by the live RLS policies above, never by rollup
-- contents (which are stale by design between refreshes).

-- The workhorse: one row per student per calendar day (doc §3 verbatim;
-- day bucketing uses the server default timezone, UTC — local-day display
-- is a dashboard concern).
create materialized view public.mv_student_day as
select s.student_id, d.date_key,
       count(*)                              as sessions,
       sum(s.duration_secs)                  as wall_secs,
       sum(s.played_secs)                    as played_secs,
       sum(s.note_count)                     as notes,
       avg(s.silence_ratio)                  as avg_silence_ratio,
       count(*) filter (where s.played_secs < 20 or s.phrase_count < 3)
                                             as thin_sessions   -- the #451 thresholds, verbatim
  from public.fact_session s
  join public.dim_date d on d.date_key = s.started_at::date
 group by 1, 2;
create unique index mv_student_day_idx on public.mv_student_day (student_id, date_key);

-- Material × key coverage: the RV question as a table (doc §3 verbatim).
create materialized view public.mv_material_key as
select e.student_id, e.material_id, e.tonic,
       count(*)                              as attempts,
       count(accuracy)                       as graded,
       avg(accuracy)                         as avg_accuracy,
       max(logged_at)                        as last_at
  from public.fact_exercise e
 group by 1, 2, 3;
create unique index mv_material_key_idx on public.mv_material_key (student_id, material_id, tonic);

-- blocks: any authenticated user (teacher OR student) selecting the whole
--         school's rollups through the API — matviews bypass RLS, so client
--         roles get no grant at all. service_role reads them server-side.
revoke all on public.mv_student_day from anon, authenticated;
revoke all on public.mv_material_key from anon, authenticated;
grant select on public.mv_student_day to service_role;
grant select on public.mv_material_key to service_role;

-- Refresh entrypoint (scheduled — see the retention section for the cron
-- wiring). Plain (non-CONCURRENT) refresh: CONCURRENTLY cannot run inside a
-- function (functions always execute in a transaction block), and these
-- matviews are service-side only — a brief lock blocks no dashboard user.
-- The unique indexes above are kept so a future scheduler that issues
-- `refresh materialized view concurrently …` as a direct statement can.
create function public.refresh_dashboard_rollups()
    returns void
    language plpgsql
    security definer
    set search_path = ''
as $$
begin
    refresh materialized view public.mv_student_day;
    refresh materialized view public.mv_material_key;
end;
$$;
revoke execute on function public.refresh_dashboard_rollups() from anon, authenticated, public;
grant execute on function public.refresh_dashboard_rollups() to service_role;

-- ── live RLS-safe views (doc §4a roster heat, §4d integrity panel) ──────────
-- security_invoker (Postgres ≥15, which Supabase runs): the querying user's
-- OWN policies gate every underlying row, so these views are safe to expose
-- to authenticated — a teacher sees exactly their active-enrollment
-- students, a student sees exactly themselves, always live (revocation is
-- immediate here, unlike the scheduled matviews).

-- §4a roster heat: "Across my classroom, who practiced this week — really?"
-- Cell renders played_min; wall_min rides along — the gap IS the story.
-- A student with no synced rows simply has no row here: the dashboard
-- renders "practicing offline", never a zero (honest absence, doc §2).
create view public.v_roster_heat with (security_invoker = true) as
select e.classroom_id,
       s.student_id,
       p.display_name,
       s.started_at::date                                   as date_key,
       round((sum(s.played_secs) / 60.0)::numeric, 1)       as played_min,
       round((sum(s.duration_secs) / 60.0)::numeric, 1)     as wall_min,
       count(*)                                             as sessions,
       count(*) filter (where s.played_secs < 20 or s.phrase_count < 3)
                                                            as thin_sessions,
       (count(*) filter (where s.played_secs < 20 or s.phrase_count < 3) > 0
        or avg(s.silence_ratio) > 0.8)                      as integrity_flag
  from public.fact_session s
  join public.enrollments e on e.student_id = s.student_id and e.status = 'active'
  left join public.profiles p on p.id = s.student_id
 group by 1, 2, 3, 4;
grant select on public.v_roster_heat to authenticated;

-- §4d engagement-integrity panel: "Which practice claims should I not take
-- at face value?" Thresholds verbatim from the doc. Tone rule, inherited
-- from "coach, don't judge": this surfaces EVIDENCE ("45 min open, 3 min of
-- sound"), never verdicts — no column here says "cheating".
create view public.v_session_integrity with (security_invoker = true) as
select p.display_name,
       s.student_id,
       s.session_id,
       s.started_at,
       s.duration_secs / 60                  as wall_min,
       s.played_secs / 60                    as played_min,
       s.silence_ratio, s.note_count, s.phrase_count,
       (select count(*) from public.fact_exercise e
         where e.session_id = s.session_id and e.accuracy is not null) as graded,
       (select max(r.cnt) from (select count(*) as cnt
                                  from public.fact_exercise e
                                 where e.session_id = s.session_id
                                 group by e.material_id, e.tonic) r)   as max_retries
  from public.fact_session s
  left join public.profiles p on p.id = s.student_id
 where s.silence_ratio > 0.8
    or (s.duration_secs > 600 and s.played_secs < 120)
    or s.phrase_count < 3;
grant select on public.v_session_integrity to authenticated;

-- ── retention: the 12-month window (doc §Decisions, verbatim) ───────────────
-- "A lapsed license keeps its cloud rows for a school year, so renewal
-- restores the dashboard exactly; local data is never touched." Facts older
-- than 12 months are purged; fact_phrase / fact_tool_event ride the
-- fact_session cascade. Dimensions are retained (shared, no per-student
-- data). The purge is deliberately date-based, not license-based: rows age
-- out uniformly, so a renewal inside the window restores everything and a
-- revocation never triggers deletion (revocation is a visibility event,
-- not a data event).
create function public.purge_expired_dashboard_facts()
    returns table (purged_sessions bigint, purged_exercises bigint)
    language plpgsql
    security definer
    set search_path = ''
as $$
declare
    v_sessions  bigint;
    v_exercises bigint;
begin
    delete from public.fact_session s
     where s.started_at < now() - interval '12 months';
    get diagnostics v_sessions = row_count;
    delete from public.fact_exercise e
     where e.logged_at < now() - interval '12 months';
    get diagnostics v_exercises = row_count;
    return query select v_sessions, v_exercises;
end;
$$;
revoke execute on function public.purge_expired_dashboard_facts() from anon, authenticated, public;
grant execute on function public.purge_expired_dashboard_facts() to service_role;

-- Scheduling: the project does not have pg_cron enabled today (no prior
-- migration uses it, and enabling extensions is a dashboard/infra action —
-- not something this migration can assume). The functions above are
-- schedulable the moment it is; until then they are invoked from the
-- service side (the T4 dashboard backend, or a scheduled Edge Function).
-- Once pg_cron is enabled, wire exactly this:
--
--   select cron.schedule('amc-dashboard-rollups',  '*/30 * * * *',
--                        $$select public.refresh_dashboard_rollups()$$);
--   select cron.schedule('amc-dashboard-retention', '17 4 * * 0',
--                        $$select public.purge_expired_dashboard_facts()$$);

-- ── rollback notes ──────────────────────────────────────────────────────────
-- This migration is purely additive (plus one new SELECT policy on
-- profiles); nothing shipped earlier is altered or dropped. To roll back:
--
--   drop view public.v_session_integrity;
--   drop view public.v_roster_heat;
--   drop function public.purge_expired_dashboard_facts();
--   drop function public.refresh_dashboard_rollups();
--   drop materialized view public.mv_material_key;
--   drop materialized view public.mv_student_day;
--   drop table public.fact_tool_event;
--   drop table public.fact_exercise;
--   drop table public.fact_phrase;
--   drop table public.fact_session;
--   drop table public.dim_material;
--   drop table public.dim_date;
--   drop function public.redeem_join_code(text, text);
--   drop function public.issue_join_code(uuid, interval);
--   drop policy profiles_select_enrolled_teacher on public.profiles;
--   drop trigger enrollments_guard_student_update on public.enrollments;
--   drop function public.enrollments_guard_student_update();
--   drop table public.enrollments;
--   drop table public.classrooms;
