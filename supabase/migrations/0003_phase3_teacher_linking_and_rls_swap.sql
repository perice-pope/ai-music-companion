-- Teacher Dashboard track, PR 1: teacher<->student linking + the RLS swap that
-- lets an ACCEPTED-linked teacher read a student's recaps. This is the privacy
-- core (see docs/design/story-phase3-teacher-dashboard-privacy.md).
--
-- Safety property: nothing a teacher can read opens up until a link row reaches
-- status='accepted'. With zero accepted links, behaviour is identical to the
-- prior self-only model — so this ships "dark" until the consent flow exists.
-- Verified against real Postgres by supabase/tests/rls_teacher_linking.sql.

-- ── coarse age tier on profiles (never a birthdate) ─────────────────────────
alter table public.profiles
    add column age_tier text check (age_tier in ('under_13', 'teen_13_17', 'adult'));

-- ── teacher <-> student links ───────────────────────────────────────────────
create table public.teacher_student_links (
    id                  uuid primary key default gen_random_uuid(),
    teacher_id          uuid not null references public.profiles (id) on delete cascade,
    student_id          uuid not null references public.profiles (id) on delete cascade,
    status              text not null default 'pending'
                            check (status in ('pending', 'accepted', 'revoked')),
    invited_at          timestamptz not null default now(),
    -- Consent audit trail (privacy doc §3.2). For under-13, consent must come
    -- from a parent; the acting adult account is recorded here.
    consented_by        text check (consented_by in ('student', 'parent')),
    consent_at          timestamptz,
    consenting_adult_id uuid references public.profiles (id) on delete set null,
    constraint teacher_student_distinct check (teacher_id <> student_id),
    unique (teacher_id, student_id)
);
alter table public.teacher_student_links enable row level security;
create index tsl_student_idx on public.teacher_student_links (student_id);
create index tsl_teacher_idx on public.teacher_student_links (teacher_id);
grant select, insert, update, delete on public.teacher_student_links to authenticated;

-- Either party can see their own links.
create policy tsl_select_either_party on public.teacher_student_links
    for select using ((select auth.uid()) in (teacher_id, student_id));
-- A teacher creates a pending invite for a student.
create policy tsl_insert_by_teacher on public.teacher_student_links
    for insert with check ((select auth.uid()) = teacher_id and status = 'pending');
-- Either party may update status (accept / revoke). RLS guarantees only the two
-- parties can touch the row; the *who-may-accept* rule for under-13 (parent
-- only) is enforced at the app layer and recorded in the consent columns —
-- full parent<->child account modelling is a follow-up.
create policy tsl_update_either_party on public.teacher_student_links
    for update using ((select auth.uid()) in (teacher_id, student_id))
    with check ((select auth.uid()) in (teacher_id, student_id));
-- Either party may unlink.
create policy tsl_delete_either_party on public.teacher_student_links
    for delete using ((select auth.uid()) in (teacher_id, student_id));

-- ── the RLS swap: sessions readable by owner OR an accepted-linked teacher ───
drop policy "sessions_select_own" on public.sessions;
create policy "sessions_select_owner_or_linked_teacher" on public.sessions
    for select using (
        (select auth.uid()) = student_id
        or exists (
            select 1 from public.teacher_student_links l
            where l.student_id = sessions.student_id
              and l.teacher_id = (select auth.uid())
              and l.status = 'accepted'
        )
    );
-- insert/update/delete remain owner-only (teachers never write sessions).

drop policy "session_phrases_select_own" on public.session_phrases;
create policy "session_phrases_select_owner_or_linked_teacher" on public.session_phrases
    for select using (
        exists (
            select 1 from public.sessions s
            where s.id = session_phrases.session_id
              and (
                  s.student_id = (select auth.uid())
                  or exists (
                      select 1 from public.teacher_student_links l
                      where l.student_id = s.student_id
                        and l.teacher_id = (select auth.uid())
                        and l.status = 'accepted'
                  )
              )
        )
    );

-- ── assignments (teacher writes for linked students; student reads) ─────────
create table public.assignments (
    id         uuid primary key default gen_random_uuid(),
    teacher_id uuid not null references public.profiles (id) on delete cascade,
    student_id uuid not null references public.profiles (id) on delete cascade,
    score_ref  text,
    goal       text,
    note       text,
    status     text not null default 'assigned'
                   check (status in ('assigned', 'in_progress', 'done')),
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
alter table public.assignments enable row level security;
create index assignments_student_idx on public.assignments (student_id);
create index assignments_teacher_idx on public.assignments (teacher_id);
grant select, insert, update, delete on public.assignments to authenticated;

-- Teacher and the assigned student can read.
create policy assignments_select_party on public.assignments
    for select using ((select auth.uid()) in (teacher_id, student_id));
-- A teacher may create an assignment only for a student they're accepted-linked to.
create policy assignments_insert_by_linked_teacher on public.assignments
    for insert with check (
        (select auth.uid()) = teacher_id
        and exists (
            select 1 from public.teacher_student_links l
            where l.teacher_id = (select auth.uid())
              and l.student_id = assignments.student_id
              and l.status = 'accepted'
        )
    );
-- Teacher edits / deletes their own assignments. (Student status round-trip is PR 3.)
create policy assignments_update_by_teacher on public.assignments
    for update using ((select auth.uid()) = teacher_id)
    with check ((select auth.uid()) = teacher_id);
create policy assignments_delete_by_teacher on public.assignments
    for delete using ((select auth.uid()) = teacher_id);
