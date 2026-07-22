import { createClient, type SupabaseClient } from "@supabase/supabase-js";
import type { Database } from "../types/supabase";

/**
 * Supabase client for the teacher dashboard (#449 T4).
 *
 * Same convention as the desktop client (apps/desktop/src/lib/supabase.ts):
 * the URL + publishable key are public by design — the publishable key is
 * RLS-gated, and RLS IS the security boundary of this app. Everything a
 * signed-in teacher can read is decided server-side by the migration-0006
 * policies (teacher SELECT only through an ACTIVE enrollment in a classroom
 * they own) plus the 0007 grants. No service-role key exists anywhere in
 * this app, and none may ever be added: a dashboard that needed one would
 * be bypassing the consent boundary.
 */
const DEFAULT_URL = "https://ttcbaomzgoatunjneuan.supabase.co";
const DEFAULT_PUBLISHABLE_KEY =
  "sb_publishable_gDoRKh1SJG2yecSzHyheiQ_Pe32aXAy";

const url = import.meta.env.VITE_SUPABASE_URL ?? DEFAULT_URL;
const publishableKey =
  import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY ?? DEFAULT_PUBLISHABLE_KEY;

/**
 * The typed client. Sessions persist to localStorage and refresh in the
 * background so a signed-in teacher stays signed in across visits.
 */
export const supabase: SupabaseClient<Database> = createClient<Database>(
  url,
  publishableKey,
  {
    auth: {
      persistSession: true,
      autoRefreshToken: true,
    },
  },
);

/** The client type every query/component in this app is written against. */
export type DashboardClient = SupabaseClient<Database>;
