/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Supabase project API URL. Falls back to the bundled project default. */
  readonly VITE_SUPABASE_URL?: string;
  /** Supabase publishable (anon) key. Public by design — RLS gates access. */
  readonly VITE_SUPABASE_PUBLISHABLE_KEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
