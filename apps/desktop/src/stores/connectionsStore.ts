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
   * #449 T2: the teacher-dashboard sync projection (doc §2 P1–P4): session
   * facts + phrase timings + exercise labels/hashes + tool events. Its OWN
   * opt-in on top of cloud sync — off by default and meaningless unless
   * cloud sync is also on (same dependency rule as teacher sharing). The
   * T-enrollment slice will prompt for this at classroom join; until then
   * it lives in Connections & Privacy like every networked switch. With
   * this off, the existing session-recap push keeps its behavior and
   * nothing else leaves the device.
   */
  dashboardSyncEnabled: boolean;

  /**
   * #58: automatic update checks (on launch + every few hours). Off by
   * default — the shipped promise is "no update request on launch or in
   * the background" unless the user opts in here. The manual "Check for
   * updates" button in ConnectionsPrivacy works regardless of this flag
   * (user-initiated is always allowed).
   */
  autoUpdateCheckEnabled: boolean;

  /**
   * #465: the once-only first-launch question about automatic update checks
   * has been answered — by either prompt button, or made moot by using the
   * Connections & Privacy toggle directly. The prompt never returns once true.
   */
  updatePromptAnswered: boolean;

  setCloudSyncEnabled: (on: boolean) => void;
  setTeacherSharingEnabled: (on: boolean) => void;
  setDashboardSyncEnabled: (on: boolean) => void;
  setAutoUpdateCheckEnabled: (on: boolean) => void;
  /**
   * #465: answer the first-launch prompt. "Yes" enables the auto-check
   * toggle; "No thanks" changes nothing beyond recording the answer, so
   * everything stays exactly as today.
   */
  answerUpdatePrompt: (enable: boolean) => void;
}

const CLOUD_SYNC_KEY = "ai-music-companion:cloud-sync-enabled";
const TEACHER_SHARING_KEY = "ai-music-companion:teacher-sharing-enabled";
const DASHBOARD_SYNC_KEY = "ai-music-companion:dashboard-sync-enabled";
const AUTO_UPDATE_KEY = "ai-music-companion:auto-update-check-enabled";
const UPDATE_PROMPT_KEY = "ai-music-companion:update-prompt-answered";

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
  dashboardSyncEnabled: loadFlag(DASHBOARD_SYNC_KEY),
  autoUpdateCheckEnabled: loadFlag(AUTO_UPDATE_KEY),
  updatePromptAnswered: loadFlag(UPDATE_PROMPT_KEY),

  setCloudSyncEnabled: (on) => {
    saveFlag(CLOUD_SYNC_KEY, on);
    // Turning sync off also withdraws the features that depend on it:
    // teacher sharing and the dashboard projection (#449 T2).
    if (!on) {
      saveFlag(TEACHER_SHARING_KEY, false);
      saveFlag(DASHBOARD_SYNC_KEY, false);
      set({
        cloudSyncEnabled: false,
        teacherSharingEnabled: false,
        dashboardSyncEnabled: false,
      });
    } else {
      set({ cloudSyncEnabled: true });
    }
  },

  setTeacherSharingEnabled: (on) => {
    saveFlag(TEACHER_SHARING_KEY, on);
    set({ teacherSharingEnabled: on });
  },

  setDashboardSyncEnabled: (on) => {
    saveFlag(DASHBOARD_SYNC_KEY, on);
    set({ dashboardSyncEnabled: on });
  },

  setAutoUpdateCheckEnabled: (on) => {
    saveFlag(AUTO_UPDATE_KEY, on);
    // An explicit choice in Connections & Privacy answers the #465
    // first-launch question too — it must never ask after that.
    saveFlag(UPDATE_PROMPT_KEY, true);
    set({ autoUpdateCheckEnabled: on, updatePromptAnswered: true });
  },

  answerUpdatePrompt: (enable) => {
    saveFlag(UPDATE_PROMPT_KEY, true);
    if (enable) {
      saveFlag(AUTO_UPDATE_KEY, true);
      set({ updatePromptAnswered: true, autoUpdateCheckEnabled: true });
    } else {
      // "No thanks" records only the answer — the toggle (and its
      // persisted value) stays exactly as it was.
      set({ updatePromptAnswered: true });
    }
  },
}));
