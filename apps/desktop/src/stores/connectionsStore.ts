import { create } from "zustand";

/**
 * Opt-in intent for every feature that can touch the network.
 *
 * This store is the single source of truth for *whether the user has chosen*
 * to enable each networked feature. It is the Face-layer expression of the
 * product principle in
 * `docs/architecture/offline-first-and-network-transparency.md`:
 *
 *   Offline by default. The internet is NEVER required for core value.
 *   Every networked feature is opt-in, off by default, and discloses what
 *   leaves the device.
 *
 * Every flag here defaults to **false** (off). The core practice loop —
 * capture → local analysis → recap — never reads this store; it works the
 * same whether everything here is on or off.
 *
 * Note: AI coaching narration is intentionally *not* in this store — it has
 * its own long-standing preference (`coachingEnabled` in `practiceStore`).
 * `ConnectionsPrivacy` surfaces that toggle alongside these so the user sees
 * every networked feature in one place.
 */
export interface ConnectionsState {
  /**
   * Cloud sync of completed-session recaps to Supabase. Off by default; the
   * actual upload still requires a signed-in account (see `authStore` /
   * `syncStore`). This flag records the user's intent to use sync at all.
   */
  cloudSyncEnabled: boolean;
  /**
   * Sharing synced recaps with a linked teacher. Rides on cloud sync; off by
   * default and meaningless unless cloud sync is also on.
   */
  teacherSharingEnabled: boolean;

  /**
   * #58: automatic update checks (on launch + every few hours). Off by
   * default — the shipped promise is "no update request on launch or in
   * the background" unless the user opts in here. The manual "Check for
   * updates" button works regardless of this flag.
   */
  autoUpdateCheckEnabled: boolean;

  setCloudSyncEnabled: (on: boolean) => void;
  setTeacherSharingEnabled: (on: boolean) => void;
  setAutoUpdateCheckEnabled: (on: boolean) => void;
}

const CLOUD_SYNC_KEY = "ai-music-companion:cloud-sync-enabled";
const TEACHER_SHARING_KEY = "ai-music-companion:teacher-sharing-enabled";
const AUTO_UPDATE_KEY = "ai-music-companion:auto-update-check-enabled";

/** Read a persisted opt-in flag. Defaults to false (off) for everything. */
function loadFlag(key: string): boolean {
  try {
    // Off by default: only an explicit "true" enables a networked feature.
    return localStorage.getItem(key) === "true";
  } catch {
    return false;
  }
}

function saveFlag(key: string, on: boolean): void {
  try {
    localStorage.setItem(key, on ? "true" : "false");
  } catch {
    // localStorage unavailable — the toggle still works for this session.
  }
}

export const useConnectionsStore = create<ConnectionsState>((set) => ({
  cloudSyncEnabled: loadFlag(CLOUD_SYNC_KEY),
  teacherSharingEnabled: loadFlag(TEACHER_SHARING_KEY),
  autoUpdateCheckEnabled: loadFlag(AUTO_UPDATE_KEY),

  setCloudSyncEnabled: (on) => {
    saveFlag(CLOUD_SYNC_KEY, on);
    // Turning sync off also withdraws teacher sharing, which depends on it.
    if (!on) {
      saveFlag(TEACHER_SHARING_KEY, false);
      set({ cloudSyncEnabled: false, teacherSharingEnabled: false });
    } else {
      set({ cloudSyncEnabled: true });
    }
  },

  setTeacherSharingEnabled: (on) => {
    saveFlag(TEACHER_SHARING_KEY, on);
    set({ teacherSharingEnabled: on });
  },

  setAutoUpdateCheckEnabled: (on) => {
    saveFlag(AUTO_UPDATE_KEY, on);
    set({ autoUpdateCheckEnabled: on });
  },
}));
