-- Personalization data foundation (Phase 4, personalization spine).
-- See docs/architecture/platform-spine-personalization.md.
--
-- Two additive changes, no destructive migration:
--   1. A JSONB `fingerprint` column on `sessions` — the unified
--      brain::fingerprint::MusicalFingerprint. Forward-compatible as the
--      fingerprint grows dimensions (tone/key/intonation/groove today), so we
--      store the whole blob rather than normalising columns. The existing
--      `session_tone` column is KEPT working untouched: sync continues to
--      project tone into it for backward compatibility while the richer
--      fingerprint lands alongside it.
--   2. A `taste_profile` table (stated preferences: genres, artists, goals,
--      experience, plus the existing under-13 flag). Owner-only RLS, modelled
--      exactly on the self-only sessions posture from migration 0001. It is
--      deliberately NOT in the teacher-shared visibility set — preference data
--      is not part of what crosses to a teacher (personalization spine §Privacy).

-- ── sessions.fingerprint (additive; session_tone stays as-is) ────────────────
-- The full MusicalFingerprint (tone, key, intonation, groove) as a single
-- forward-compatible JSONB blob. `session_tone` is intentionally left in place:
-- it remains the tone-only projection for existing readers; nothing reads or
-- writes through it differently because of this column.
alter table public.sessions
    add column fingerprint jsonb;

-- ── taste_profile (1 row per owning user; local-first, synced if sync on) ────
-- Stated preferences, distinct from the measured MusicalFingerprint. Keyed to
-- the owner's profile; preference arrays are stored as JSONB for the same
-- forward-compatibility reason the fingerprint is. `is_under_13` reuses the
-- shipped age precedent (teacher-audit.md): we keep the profile minimal for
-- minors and never store a birthdate.
create table public.taste_profile (
    user_id      uuid primary key references public.profiles (id) on delete cascade,
    genres       jsonb not null default '[]'::jsonb,
    artists      jsonb not null default '[]'::jsonb,
    goals        jsonb not null default '[]'::jsonb,
    experience   text  not null default 'beginner'
                     check (experience in ('beginner', 'intermediate', 'advanced')),
    is_under_13  boolean not null default false,
    updated_at   timestamptz not null default now()
);
alter table public.taste_profile enable row level security;
grant select, insert, update, delete on public.taste_profile to authenticated;

-- Owner-only RLS — identical posture to the self-only `sessions` policies in
-- migration 0001 (auth.uid() = the owning key). The taste profile is NOT shared
-- with linked teachers: there is deliberately no owner-or-linked-teacher SELECT
-- policy here, unlike the sessions RLS swap in migration 0003. Preference data
-- only crosses to a teacher via a future, separately-consented story.
create policy "taste_profile_select_own" on public.taste_profile
    for select using ((select auth.uid()) = user_id);
create policy "taste_profile_insert_own" on public.taste_profile
    for insert with check ((select auth.uid()) = user_id);
create policy "taste_profile_update_own" on public.taste_profile
    for update using ((select auth.uid()) = user_id)
    with check ((select auth.uid()) = user_id);
create policy "taste_profile_delete_own" on public.taste_profile
    for delete using ((select auth.uid()) = user_id);
