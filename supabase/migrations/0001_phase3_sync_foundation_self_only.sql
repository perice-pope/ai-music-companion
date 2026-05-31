-- AI Music Companion — Teacher Dashboard track, foundational sync schema.
-- This is the "Phase 2.5 Sync" layer (docs/design/story-phase3-teacher-dashboard.md):
-- a user's OWN profile + synced practice sessions, with RLS locked to the owner.
-- Teacher<->student linking (the FERPA/COPPA-sensitive surface) is intentionally
-- NOT created here — it lands in a later migration after privacy sign-off.

-- ── profiles ────────────────────────────────────────────────────────────────
create table public.profiles (
    id           uuid primary key references auth.users (id) on delete cascade,
    role         text not null default 'student' check (role in ('student', 'teacher')),
    display_name text,
    created_at   timestamptz not null default now()
);
alter table public.profiles enable row level security;

create policy "profiles_select_own" on public.profiles
    for select using ((select auth.uid()) = id);
create policy "profiles_insert_own" on public.profiles
    for insert with check ((select auth.uid()) = id);
create policy "profiles_update_own" on public.profiles
    for update using ((select auth.uid()) = id);

-- Auto-provision a profile row when a new auth user signs up.
create function public.handle_new_user()
    returns trigger
    language plpgsql
    security definer
    set search_path = ''
as $$
begin
    insert into public.profiles (id, display_name)
    values (new.id, new.raw_user_meta_data ->> 'display_name');
    return new;
end;
$$;

create trigger on_auth_user_created
    after insert on auth.users
    for each row execute function public.handle_new_user();

-- ── sessions (synced practice-session recaps) ───────────────────────────────
create table public.sessions (
    id                 uuid primary key default gen_random_uuid(),
    student_id         uuid not null references public.profiles (id) on delete cascade,
    instrument         text not null,
    started_at         timestamptz not null,
    ended_at           timestamptz not null,
    duration_secs      double precision not null,
    phrase_count       integer not null default 0,
    overall_assessment text,
    -- The tone::ToneDescriptor session aggregate (brain SessionRecap.session_tone).
    session_tone       jsonb,
    created_at         timestamptz not null default now()
);
alter table public.sessions enable row level security;
create index sessions_student_started_idx on public.sessions (student_id, started_at desc);

create policy "sessions_select_own" on public.sessions
    for select using ((select auth.uid()) = student_id);
create policy "sessions_insert_own" on public.sessions
    for insert with check ((select auth.uid()) = student_id);
create policy "sessions_update_own" on public.sessions
    for update using ((select auth.uid()) = student_id);
create policy "sessions_delete_own" on public.sessions
    for delete using ((select auth.uid()) = student_id);

-- ── session_phrases (per-phrase detail) ─────────────────────────────────────
create table public.session_phrases (
    id             uuid primary key default gen_random_uuid(),
    session_id     uuid not null references public.sessions (id) on delete cascade,
    phrase_index   integer not null,
    stability      double precision,
    mean_amplitude double precision,
    -- The per-phrase tone::ToneDescriptor (brain PhraseSummary.tone).
    tone           jsonb
);
alter table public.session_phrases enable row level security;
create index session_phrases_session_idx on public.session_phrases (session_id, phrase_index);

-- Phrase access is governed by ownership of the parent session.
create policy "session_phrases_select_own" on public.session_phrases
    for select using (
        exists (
            select 1 from public.sessions s
            where s.id = session_id and s.student_id = (select auth.uid())
        )
    );
create policy "session_phrases_insert_own" on public.session_phrases
    for insert with check (
        exists (
            select 1 from public.sessions s
            where s.id = session_id and s.student_id = (select auth.uid())
        )
    );
