import { create } from "zustand";
import type { Session, User } from "@supabase/supabase-js";
import { supabase } from "../lib/supabase";

/**
 * Auth state for the optional cloud-sync layer. Signing in is entirely
 * opt-in — the practice loop never requires it. We use email + password
 * (no magic-link deep-linking to wrangle in a desktop shell), and a profile
 * row is auto-provisioned server-side by the `handle_new_user` trigger.
 */
export type AuthStatus =
  | "loading" // restoring a persisted session on boot
  | "signed_out"
  | "signed_in"
  | "working"; // a sign-in / sign-up / sign-out round-trip is in flight

export interface AuthState {
  status: AuthStatus;
  user: User | null;
  session: Session | null;
  error: string | null;

  /** Restore any persisted session and subscribe to auth changes. */
  init: () => Promise<void>;
  signUp: (
    email: string,
    password: string,
    displayName?: string,
  ) => Promise<void>;
  signIn: (email: string, password: string) => Promise<void>;
  signOut: () => Promise<void>;
  clearError: () => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  status: "loading",
  user: null,
  session: null,
  error: null,

  init: async () => {
    // onAuthStateChange fires immediately with the restored session (or null)
    // and again on every later sign-in/out/refresh, so it's the single source
    // of truth — we don't separately call getSession().
    supabase.auth.onAuthStateChange((_event, session) => {
      set({
        session,
        user: session?.user ?? null,
        status: session ? "signed_in" : "signed_out",
      });
    });
  },

  signUp: async (email, password, displayName) => {
    set({ status: "working", error: null });
    const { error } = await supabase.auth.signUp({
      email,
      password,
      options: displayName
        ? { data: { display_name: displayName } }
        : undefined,
    });
    if (error) {
      set({ status: "signed_out", error: error.message });
      return;
    }
    // If email confirmation is on, there's no session yet — surface a gentle
    // nudge rather than leaving the form looking stuck. onAuthStateChange will
    // flip us to signed_in if a session did come back.
    set((s) =>
      s.session
        ? s
        : {
            status: "signed_out",
            error: "Check your email to confirm your account, then sign in.",
          },
    );
  },

  signIn: async (email, password) => {
    set({ status: "working", error: null });
    const { error } = await supabase.auth.signInWithPassword({
      email,
      password,
    });
    if (error) {
      set({ status: "signed_out", error: error.message });
    }
    // Success path is handled by onAuthStateChange.
  },

  signOut: async () => {
    set({ status: "working", error: null });
    const { error } = await supabase.auth.signOut();
    if (error) {
      set({ error: error.message });
    }
    // onAuthStateChange flips us to signed_out.
  },

  clearError: () => set({ error: null }),
}));
