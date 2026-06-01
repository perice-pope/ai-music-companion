import { createClient, type SupabaseClient } from "@supabase/supabase-js";
import type { Database } from "../types/supabase";

/**
 * Supabase client for the optional cloud-sync layer (Phase 3, Teacher
 * Dashboard track — see supabase/README.md).
 *
 * The core practice loop is fully offline; this client only ever touches the
 * network when a user has explicitly signed in. The URL + publishable key are
 * public by design (the publishable key is RLS-gated), so we ship sane
 * defaults pointing at the project and allow an env override for local dev or
 * a future self-host. Nothing secret lives here — the service-role key never
 * touches the client.
 */
const DEFAULT_URL = "https://ttcbaomzgoatunjneuan.supabase.co";
const DEFAULT_PUBLISHABLE_KEY =
  "sb_publishable_gDoRKh1SJG2yecSzHyheiQ_Pe32aXAy";

const url = import.meta.env.VITE_SUPABASE_URL ?? DEFAULT_URL;
const publishableKey =
  import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY ?? DEFAULT_PUBLISHABLE_KEY;

/**
 * The typed client. Sessions persist to localStorage and refresh in the
 * background so a signed-in user stays signed in across restarts.
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
