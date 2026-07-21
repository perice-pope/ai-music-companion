# Shadow-database harness (no Docker)

Runs the full migration chain + every RLS suite against a plain local Postgres
when the Supabase stack (Docker) isn't available. `shim.sql` provides the
minimal platform surface (auth schema, null-safe `auth.uid()`, client roles,
Supabase's blanket default privileges) — apply it **before** the migrations.

Use Postgres **15+** so `security_invoker` views (migration 0006) parse and
their version-gated assertions run; on 14 the suite skips them with a NOTICE.

```sh
PGBIN=/usr/local/opt/postgresql@15/bin   # or wherever 15+ lives
D=/tmp/amc-shadow
"$PGBIN/initdb" -D $D/data -U postgres --auth=trust -E UTF8
"$PGBIN/pg_ctl" -D $D/data -o "-p 55432 -k $D" -l $D/pg.log start
"$PGBIN/createdb" -h $D -p 55432 -U postgres amc
q() { "$PGBIN/psql" -h $D -p 55432 -U postgres -d amc -v ON_ERROR_STOP=1 "$@"; }
q -f supabase/tests/shadow/shim.sql
for f in supabase/migrations/*.sql; do q -f "$f"; done
for f in supabase/tests/*.sql; do q -f "$f"; done
```

This directory is deliberately **outside** the `supabase/tests/*.sql` glob the
CI job loops over — the shim must never run against the real stack. CI remains
the authority (`.github/workflows/supabase-rls.yml`, real Supabase Postgres);
this harness exists so policy changes can be attacked locally first.
