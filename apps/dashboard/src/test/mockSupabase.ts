import type { DashboardClient } from "../lib/supabase";

/**
 * A Supabase client mock ROUTED BY RELATION NAME (spec §7): a test declares
 * fixture rows (or an Error) per table/view, and any `from()` chain against
 * that name resolves to them. Filters (.eq/.in/.gte) are recorded but not
 * applied — a test's fixtures are already the rows the real RLS + filters
 * would admit, which keeps every fixture an explicit statement of what the
 * policies return. Querying a relation with NO fixture resolves to an
 * error, so a component quietly reading something a test didn't declare
 * fails loudly instead of passing on accident.
 */

export type Fixture = unknown[] | Error;

export interface MockSupabase {
  client: DashboardClient;
  /** Relation names in the order components queried them. */
  calls: string[];
}

interface QueryResult {
  data: unknown;
  error: { message: string } | null;
}

export function makeClient(fixtures: Record<string, Fixture>): MockSupabase {
  const calls: string[] = [];

  const from = (relation: string) => {
    calls.push(relation);
    const resolve = (): QueryResult => {
      const fixture = fixtures[relation];
      if (fixture === undefined) {
        return {
          data: null,
          error: { message: `no fixture declared for "${relation}"` },
        };
      }
      if (fixture instanceof Error) {
        return { data: null, error: { message: fixture.message } };
      }
      return { data: fixture, error: null };
    };

    const builder: Record<string, unknown> = {};
    const chainMethods = ["select", "eq", "in", "gte", "lte", "order", "limit"];
    for (const method of chainMethods) {
      builder[method] = () => builder;
    }
    builder.maybeSingle = () => {
      const r = resolve();
      return Promise.resolve({
        data: Array.isArray(r.data) ? ((r.data[0] as unknown) ?? null) : null,
        error: r.error,
      });
    };
    // Awaiting the builder itself (the common path) resolves the fixture.
    builder.then = (
      onFulfilled: (value: QueryResult) => unknown,
      onRejected?: (reason: unknown) => unknown,
    ) => Promise.resolve(resolve()).then(onFulfilled, onRejected);
    return builder;
  };

  return {
    client: { from } as unknown as DashboardClient,
    calls,
  };
}
