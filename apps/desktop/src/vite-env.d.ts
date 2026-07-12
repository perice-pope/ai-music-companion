/// <reference types="vite/client" />

/** Emitted-notation fixtures (#356) imported as raw text by the OSMD sweep. */
declare module "*.musicxml?raw" {
  const content: string;
  export default content;
}

interface ImportMetaEnv {
  /** Supabase project API URL. Falls back to the bundled project default. */
  readonly VITE_SUPABASE_URL?: string;
  /** Supabase publishable (anon) key. Public by design — RLS gates access. */
  readonly VITE_SUPABASE_PUBLISHABLE_KEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
