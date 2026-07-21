-- Shadow-database shim: the minimal slice of the Supabase platform the
-- migrations and RLS suites depend on, for running the whole stack against a
-- plain local Postgres (no Docker) — see README.md in this directory.
-- Validation-only; never applied to a real Supabase project (there these
-- objects already exist). Lives OUTSIDE supabase/tests/*.sql on purpose: the
-- CI loop globs that directory non-recursively and must not run this file
-- against the real stack.
--
-- Agreed harness baseline (#449 T3 review round 1): null-safe auth.uid()
-- (the nullif-before-cast ::jsonb form) and the Supabase blanket default
-- privileges applied BEFORE migrations, so both the builder's and the
-- reviewer's runs exercise identical grant semantics.

create schema if not exists auth;

create table if not exists auth.users (
    id                 uuid primary key default gen_random_uuid(),
    raw_user_meta_data jsonb not null default '{}'::jsonb,
    created_at         timestamptz not null default now()
);

-- Supabase's auth.uid(): 'sub' from the request.jwt.claims GUC. Null-safe:
-- nullif() BEFORE the cast, so an unset/empty GUC yields NULL, never a cast
-- error inside a policy.
create or replace function auth.uid()
    returns uuid
    language sql
    stable
as $$
    select (nullif(current_setting('request.jwt.claims', true), '')::jsonb ->> 'sub')::uuid
$$;

-- Roles used by the migrations and RLS suites (idempotent).
do $$
begin
    if not exists (select 1 from pg_roles where rolname = 'anon') then
        create role anon nologin noinherit;
    end if;
    if not exists (select 1 from pg_roles where rolname = 'authenticated') then
        create role authenticated nologin noinherit;
    end if;
    if not exists (select 1 from pg_roles where rolname = 'service_role') then
        create role service_role nologin noinherit bypassrls;
    end if;
end $$;

grant usage on schema public to anon, authenticated, service_role;
grant usage on schema auth  to anon, authenticated, service_role;
grant execute on function auth.uid() to public;

-- Supabase baseline: client roles get blanket privileges on public; RLS is
-- the real gate. Set BEFORE migrations run so objects they create inherit
-- these (this is what makes 0001's un-granted profiles/sessions reachable,
-- and what the 0006 matview REVOKE meaningfully undoes).
alter default privileges in schema public grant all on tables    to anon, authenticated, service_role;
alter default privileges in schema public grant all on functions to anon, authenticated, service_role;
alter default privileges in schema public grant all on sequences to anon, authenticated, service_role;
grant all on all tables    in schema public to anon, authenticated, service_role;
grant all on all sequences in schema public to anon, authenticated, service_role;
