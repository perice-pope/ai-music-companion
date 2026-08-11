import type { StateCreator } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { PracticeState } from "../practiceStore";

/** #421 S2: handoff follows for this long, then freezes. */
export const HANDOFF_FOLLOW_MS = 8_000;
/** #421 S2: follow sends at most once per this window… */
export const FOLLOW_THROTTLE_MS = 1_000;
/** …and only when the reading moved at least this much. */
export const FOLLOW_MIN_DELTA_BPM = 2;

const POCKET_TEMPO_KEY = "ai-music-companion:pocket-tempo";

/** #421 S1: the click tempo survives restarts (spec §4). Default 90. */
function loadPocketTempo(): number {
  try {
    const raw = localStorage.getItem(POCKET_TEMPO_KEY);
    const parsed = raw === null ? NaN : Number(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : 90;
  } catch {
    return 90;
  }
}

function savePocketTempo(bpm: number): void {
  try {
    localStorage.setItem(POCKET_TEMPO_KEY, String(bpm));
  } catch {
    // localStorage unavailable — the tempo still works for this session.
  }
}

/**
 * The Pocket click's own state and actions (#421 S1/S2) — the first focused
 * slice of the practice store (#511). The slice owns the click; the follow
 * POLICY stays with `setPerception` in the store, because perception drives
 * it and the band shares it (#445 pt 9 — one policy, two carriers). Session
 * lifecycle (endSession / openMatchedScore) resets the personality fields
 * from the store side for the same reason: the session owns its scope.
 */
export interface PocketSlice {
  /** #421 S2: the click's personality — anchor holds, follow locks to
   * YOUR pulse, handoff follows then freezes ("now hold it"). */
  pocketMode: "anchor" | "follow" | "handoff";
  setPocketMode: (mode: "anchor" | "follow" | "handoff") => void;
  /** #421 S2: handoff's frozen tempo once the follow window closes. */
  pocketFrozenBpm: number | null;
  /** Internal follow-policy state (throttle + delta gate). */
  _pocketLastSentBpm: number | null;
  _pocketLastSentAt: number;
  _pocketFollowStartedAt: number;

  /** #421 S1: The Pocket — strict Anchor click state. */
  pocketPlaying: boolean;
  pocketTempo: number;
  pocketCountIn: boolean;
  setPocketTempo: (bpm: number) => void;
  setPocketCountIn: (on: boolean) => void;
  startPocket: () => Promise<void>;
  stopPocket: () => Promise<void>;
  setPocketStatus: (playing: boolean, tempoBpm: number) => void;
}

export const createPocketSlice: StateCreator<
  PracticeState,
  [],
  [],
  PocketSlice
> = (set, get) => ({
  pocketMode: "anchor",
  pocketFrozenBpm: null,
  _pocketLastSentBpm: null,
  _pocketLastSentAt: 0,
  _pocketFollowStartedAt: 0,

  setPocketMode: (mode) =>
    set({
      pocketMode: mode,
      // A fresh personality starts a fresh follow window; anchor sends
      // nothing (AC5: switching mid-play stops the stream).
      pocketFrozenBpm: null,
      _pocketLastSentBpm: null,
      _pocketFollowStartedAt: Date.now(),
    }),

  pocketPlaying: false,
  pocketTempo: loadPocketTempo(),
  pocketCountIn: true,

  setPocketTempo: (bpm) => {
    savePocketTempo(bpm);
    set({ pocketTempo: bpm });
  },
  setPocketCountIn: (on) => set({ pocketCountIn: on }),

  startPocket: async () => {
    // Semantic settings only — clamping and validation are the backend's.
    await invoke("start_pocket", {
      tempoBpm: get().pocketTempo,
      beatsPerBar: 4,
      countIn: get().pocketCountIn,
    });
  },

  stopPocket: async () => {
    await invoke("stop_pocket");
  },

  // Authoritative playing state comes from the pocket-status event, same
  // discipline as the band (#421 rule: never optimistically flip).
  setPocketStatus: (playing, tempoBpm) =>
    set((s) => ({
      pocketPlaying: playing,
      // Review MF4: a FRESH click starts a fresh follow life — stale
      // frozen/sent state from a previous click made the drift line lie
      // and the delta gate block against a tempo the click no longer
      // holds. The follow window anchors at the click's real start.
      ...(playing && !s.pocketPlaying
        ? {
            pocketFrozenBpm: null,
            _pocketLastSentBpm: null,
            _pocketFollowStartedAt: Date.now(),
          }
        : {}),
      // The backend reports the CLAMPED tempo it actually plays — mirror
      // it so the pulse and label can never lie.
      pocketTempo: playing && tempoBpm > 0 ? tempoBpm : s.pocketTempo,
    })),
});
