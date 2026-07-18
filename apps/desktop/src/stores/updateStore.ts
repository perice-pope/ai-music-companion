import { create } from "zustand";
import { check } from "@tauri-apps/plugin-updater";

/**
 * #58 — the update pill's state machine.
 *
 * The signing/manifest pipeline has been live since v2.28.12; this store is
 * the missing surface. It only ever runs when the user has opted in to
 * automatic checks (`connectionsStore.autoUpdateCheckEnabled`) or asks
 * manually — the caller gates, this store just talks to the plugin.
 *
 * #417 rule 0 applies to the pill it drives: phases REPLACE each other in
 * one persistent element; nothing here should cause a remount.
 */

export type UpdatePhase =
  | "idle"
  | "available"
  | "downloading"
  | "ready"
  | "error";

const DISMISSED_KEY = "ai-music-companion:dismissed-update";

function loadDismissed(): string | null {
  try {
    return localStorage.getItem(DISMISSED_KEY);
  } catch {
    return null;
  }
}

function saveDismissed(version: string): void {
  try {
    localStorage.setItem(DISMISSED_KEY, version);
  } catch {
    // localStorage unavailable — dismissal still holds for this session.
  }
}

export interface UpdateState {
  phase: UpdatePhase;
  /** The newer version's tag, e.g. "2.31.4", when phase !== "idle". */
  availableVersion: string | null;
  /** Calm player-facing line for the error phase. */
  notice: string | null;
  /** Version the user dismissed — that one stays hidden; a newer shows. */
  dismissedVersion: string | null;

  /**
   * Ask the updater endpoint whether a newer signed build exists. Failures
   * are SILENT — offline is normal, not an error surface (spec AC5/edge).
   */
  checkForUpdate: () => Promise<void>;
  /** Download + install the available update; pill dims while working. */
  installUpdate: () => Promise<void>;
  /** Hide the currently offered version (a newer one will re-surface). */
  dismiss: () => void;
}

/**
 * The plugin's update handle for the currently offered version. Kept out of
 * the store state: it is not renderable data, and zustand should not diff it.
 */
let pendingUpdate: {
  version: string;
  downloadAndInstall: () => Promise<void>;
} | null = null;

export const useUpdateStore = create<UpdateState>((set, get) => ({
  phase: "idle",
  availableVersion: null,
  notice: null,
  dismissedVersion: loadDismissed(),

  checkForUpdate: async () => {
    // Never interrupt an in-flight download with a re-check.
    if (get().phase === "downloading" || get().phase === "ready") {
      return;
    }
    try {
      const update = await check();
      // Review MF2 (TOCTOU): the entry guard above ran BEFORE the await —
      // if an install started while this check was in flight, a late
      // resolution must not stomp the phase back to "available" (which
      // re-enabled the button mid-download and could fire a second,
      // concurrent install).
      if (get().phase === "downloading" || get().phase === "ready") {
        return;
      }
      if (!update) {
        return; // Up to date — stay idle, render nothing.
      }
      if (update.version === get().dismissedVersion) {
        return; // The user said "not this one" — honor it.
      }
      pendingUpdate = update;
      set({
        phase: "available",
        availableVersion: update.version,
        notice: null,
      });
    } catch {
      // Offline or endpoint unreachable — normal life, stay silent.
    }
  },

  installUpdate: async () => {
    const update = pendingUpdate;
    if (get().phase !== "available" || !update) {
      return;
    }
    set({ phase: "downloading" });
    try {
      await update.downloadAndInstall();
      // The new build is on disk; it takes over on next launch (the
      // in-app relaunch button is a follow-up slice — no new plugin dep).
      set({ phase: "ready" });
    } catch (err) {
      // Raw plugin errors (network, signature verification) are not calm
      // reading — log them, show a fixed friendly line (review SF6).
      console.error("update install failed:", err);
      set({
        phase: "error",
        notice: "The update didn't finish. It will offer again next check.",
      });
      pendingUpdate = null;
    }
  },

  dismiss: () => {
    const version = get().availableVersion;
    if (version) {
      saveDismissed(version);
    }
    pendingUpdate = null;
    set({
      phase: "idle",
      availableVersion: null,
      notice: null,
      dismissedVersion: version,
    });
  },
}));
