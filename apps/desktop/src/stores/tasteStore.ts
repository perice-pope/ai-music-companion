import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import {
  emptyTasteProfile,
  type ExperienceLevel,
  type TasteProfile,
} from "../types/brain";

/**
 * Local taste-profile state (Phase 4, personalization spine — capture only).
 *
 * The Rust core is the source of truth: this store loads the locally-stored
 * profile over IPC and writes edits back through `set_taste_profile`. It holds
 * NO relevance/coaching logic — it only captures stated preferences (genres,
 * artists, goals, experience). Consumption of the profile lives elsewhere.
 *
 * Local-first: no sign-in is required to capture or edit the profile; it lives
 * on-device and only reaches the cloud through the separate, opt-in
 * `syncStore.syncTasteProfile` switch.
 */
export type TasteStatus = "idle" | "loading" | "saving" | "ready" | "error";

export interface TasteState {
  profile: TasteProfile;
  status: TasteStatus;
  error: string | null;
  /** True once a non-empty profile has been captured (drives onboarding). */
  hasProfile: boolean;

  /** Load the locally-stored profile over IPC. */
  load: () => Promise<void>;
  /** Persist a full profile locally and update state. */
  save: (profile: TasteProfile) => Promise<void>;
}

/** A profile counts as "captured" once it carries any stated preference. */
export function isProfileCaptured(p: TasteProfile): boolean {
  return p.genres.length > 0 || p.artists.length > 0 || p.goals.length > 0;
}

export const useTasteStore = create<TasteState>((set) => ({
  profile: emptyTasteProfile(),
  status: "idle",
  error: null,
  hasProfile: false,

  load: async () => {
    set({ status: "loading", error: null });
    try {
      const profile = await invoke<TasteProfile>("get_taste_profile");
      set({
        profile,
        status: "ready",
        hasProfile: isProfileCaptured(profile),
      });
    } catch (err: unknown) {
      set({ status: "error", error: String(err) });
    }
  },

  save: async (profile) => {
    set({ status: "saving", error: null });
    try {
      await invoke("set_taste_profile", { profile });
      set({
        profile,
        status: "ready",
        hasProfile: isProfileCaptured(profile),
      });
    } catch (err: unknown) {
      set({ status: "error", error: String(err) });
    }
  },
}));

export type { ExperienceLevel };
