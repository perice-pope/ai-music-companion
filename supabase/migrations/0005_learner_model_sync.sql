-- Learner Model cloud sync (epic #252 F2; deferred from the S3 slice, now due).
--
-- The Learner Model is the "sessions get smarter" spine: the versioned,
-- forward-compatible JSON blob holding the reveal collection, per-key mastery,
-- and the adaptive difficulty step. Locally it lives in SQLite
-- (`learner_model` table, brain::store); this table is its cloud mirror so a
-- reinstall or second machine doesn't erase a student's progress.
--
-- One additive table, no destructive migration. Stored as a single JSONB blob
-- (the same forward-compat strategy as sessions.fingerprint in 0004): the blob
-- is versioned and preserves unknown fields, so schema growth (streaks, sound
-- profile) needs no further migration here.

create table public.learner_model (
    user_id    uuid primary key references public.profiles (id) on delete cascade,
    model      jsonb not null,
    updated_at timestamptz not null default now()
);
alter table public.learner_model enable row level security;
grant select, insert, update, delete on public.learner_model to authenticated;

-- Owner-only RLS — identical posture to taste_profile in 0004. Mastery and
-- collection data is the student's; it is deliberately NOT in the
-- teacher-shared visibility set (that would need its own consented story).
create policy "learner_model_select_own" on public.learner_model
    for select using ((select auth.uid()) = user_id);
create policy "learner_model_insert_own" on public.learner_model
    for insert with check ((select auth.uid()) = user_id);
create policy "learner_model_update_own" on public.learner_model
    for update using ((select auth.uid()) = user_id)
    with check ((select auth.uid()) = user_id);
create policy "learner_model_delete_own" on public.learner_model
    for delete using ((select auth.uid()) = user_id);
