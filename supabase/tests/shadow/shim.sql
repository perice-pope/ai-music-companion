-- Shadow-database shim: the minimal slice of the Supabase platform the
-- migrations and RLS suites depend on, for running the whole stack against a
-- plain local Postgres (no Docker) — see README.md in this directory.
-- Validation-only; never applied to a real Supabase project (there these
-- objects already exist). Lives OUTSIDE supabase/tests/*.sql on purpose: the
-- CI loop globs that directory non-recursively and must not run this file
-- against the real stack.
--
-- Agreed harness baseline (#449 T3 review round 1, amended post-#467): the
-- null-safe auth.uid() (the nullif-before-cast ::jsonb form), and — the
-- amendment — NO blanket default-privileges grant. The CI/real stack gives
-- client roles only what migrations grant EXPLICITLY (the 0003 per-table
-- convention); the original shim's ALTER DEFAULT PRIVILEGES was MORE
-- generous than reality and masked a missing-grant hole in 0006's surface
-- ("permission denied for table profiles" on main). The shim must mirror
-- the real stack's stinginess so the shadow fails exactly where CI fails.

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

-- Deliberately NOTHING else: no ALTER DEFAULT PRIVILEGES, no blanket table
-- grants. Table/view privileges for client roles must come from the
-- migrations themselves (explicit per-table grants, the 0003 convention) —
-- a table a migration forgets to grant is unreachable here exactly as it is
-- on the CI stack. (Functions still default to EXECUTE for PUBLIC, which is
-- ordinary Postgres and why the 0002-style revokes matter.)
