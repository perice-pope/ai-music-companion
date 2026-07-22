import { create } from "zustand";
import type { Session, User } from "@supabase/supabase-js";
import { supabase } from "../lib/supabase";

/**
 * Auth state for the teacher dashboard — the same email + password Supabase
 * flow as the desktop app (apps/desktop/src/stores/authStore.ts, copied
 * pattern). Sign-UP is deliberately absent here: accounts are created in
 * the app; the dashboard only signs existing teachers in. The teacher gate
 * itself (profiles.role) is a component concern — see TeacherGate.tsx.
 */
export type AuthStatus =
  | "loading" // restoring a persisted session on boot
  | "signed_out"
  | "signed_in"
  | "working"; // a sign-in / sign-out round-trip is in flight

export interface AuthState {
  status: AuthStatus;
  user: User | null;
  session: Session | null;
  error: string | null;

  /** Restore any persisted session and subscribe to auth changes. */
  init: () => Promise<void>;
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
