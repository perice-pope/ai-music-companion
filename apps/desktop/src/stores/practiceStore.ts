import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  CoachingTip,
  PhraseSummary,
  PracticeMode,
  SessionRecap,
} from "../types/brain";
import { useAudioStore } from "./audioStore";

/**
 * Screen routing enum — keeps Free Play as a state-machine in the
 * store instead of pulling in react-router for four screens (see
 * design doc §2).
 */
export type AppScreen = "selector" | "session" | "recap" | "history";

/**
 * Finite lifecycle states for a session. `recap_ready` exists so we
 * can distinguish "Rust returned the recap and we're about to nav"
 * from "already on the recap screen" in PR 2+.
 */
export type SessionStatus =
  | "idle"
  | "starting"
  | "listening"
  | "ending"
  | "recap_ready";

/**
 * A coaching tip queued in the panel. UUID keys keep React's
 * reconciler stable when tips rotate through.
 */
export interface QueuedTip {
  id: string;
  tip: CoachingTip;
  receivedAt: number;
  phraseIndex: number;
}

/** All the state the free-play flow needs. */
export interface PracticeState {
  // Routing ---------------------------------------------------------------
  screen: AppScreen;

  // Session lifecycle -----------------------------------------------------
  status: SessionStatus;
  sessionId: string | null;
  instrumentName: string | null;
  /** Opaque id for the current instrument segment (updated on switch). */
  segmentId: string | null;
  startedAtMs: number | null;
  /** Ticked by `tick()` on a 1Hz setInterval. Not derived per-render. */
  elapsedSecs: number;

  // Live session data -----------------------------------------------------
  phrases: PhraseSummary[];
  tipQueue: QueuedTip[];

  // Recap -----------------------------------------------------------------
  recap: SessionRecap | null;
  recapError: string | null;

  // UI prefs (persisted) --------------------------------------------------
  coachingEnabled: boolean;
  /**
   * Current practice mode. Persisted so a user's last choice survives
   * a restart. Defaults to `"practice"` on first run (matches the Rust
   * `PracticeMode::default()`).
   */
  practiceMode: PracticeMode;

  // Actions ---------------------------------------------------------------
  startSession: (instrument: string, vibratoToleranceCents?: number) => Promise<void>;
  endSession: () => Promise<void>;
  switchInstrument: (name: string, vibratoToleranceCents?: number) => Promise<void>;
  pushPhrase: (phrase: PhraseSummary) => void;
  pushTip: (tip: CoachingTip, phraseIndex: number) => void;
  dismissTip: (id: string) => void;
  tick: () => void;
  returnToSelector: () => void;
  setCoachingEnabled: (on: boolean) => void;
  /**
   * Update the mode. If a session is already running, this only updates
   * local state — the Rust side is notified on the next `switchInstrument`
   * (which now carries the mode). Callers that want an immediate mid-session
   * change should call `switchInstrument` with the current instrument after
   * setting the new mode.
   */
  setPracticeMode: (mode: PracticeMode) => void;
  goToHistory: () => void;
}

/** localStorage key for the coaching-on/off preference. */
const COACHING_PREF_KEY = "ai-music-companion:coaching-enabled";
/** localStorage key for the last-used practice mode. */
const PRACTICE_MODE_PREF_KEY = "ai-music-companion:practice-mode";

function loadCoachingPref(): boolean {
  try {
    const raw = localStorage.getItem(COACHING_PREF_KEY);
    // Default: enabled. Only "false" disables.
    return raw === null ? true : raw !== "false";
  } catch {
    return true;
  }
}

function saveCoachingPref(on: boolean): void {
  try {
    localStorage.setItem(COACHING_PREF_KEY, on ? "true" : "false");
  } catch {
    // localStorage unavailable — silently ignore.
  }
}

function isPracticeMode(v: string | null): v is PracticeMode {
  return v === "warmup" || v === "practice" || v === "run_through";
}

function loadPracticeModePref(): PracticeMode {
  try {
    const raw = localStorage.getItem(PRACTICE_MODE_PREF_KEY);
    return isPracticeMode(raw) ? raw : "practice";
  } catch {
    return "practice";
  }
}

function savePracticeModePref(mode: PracticeMode): void {
  try {
    localStorage.setItem(PRACTICE_MODE_PREF_KEY, mode);
  } catch {
    // localStorage unavailable — silently ignore.
  }
}

/**
 * Tiny UUID generator that works in jsdom (crypto.randomUUID is available
 * in Vitest's jsdom >= 25). Falls back to a timestamp-based string if
 * for some reason crypto isn't present.
 */
function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `tip-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  }
}

export const usePracticeStore = create<PracticeState>((set, get) => ({
  screen: "selector",
  status: "idle",
  sessionId: null,
  instrumentName: null,
  segmentId: null,
  startedAtMs: null,
  elapsedSecs: 0,
  phrases: [],
  tipQueue: [],
  recap: null,
  recapError: null,
  coachingEnabled: loadCoachingPref(),
  practiceMode: loadPracticeModePref(),

  startSession: async (instrument: string, vibratoToleranceCents = 15.0) => {
    const { status } = get();
    if (status !== "idle") {
      throw new Error(
        `cannot start session from status=${status} — call endSession first`,
      );
    }
    set({ status: "starting", recap: null, recapError: null });
    try {
      const sessionId = await invoke<string>("start_practice_session", {
        instrument,
        practiceMode: get().practiceMode,
        coachingEnabled: get().coachingEnabled,
      });
      set({
        status: "listening",
        screen: "session",
        sessionId,
        instrumentName: instrument,
        segmentId: null,
        startedAtMs: Date.now(),
        elapsedSecs: 0,
        phrases: [],
        tipQueue: [],
      });
      useAudioStore.getState().setInstrument(instrument, vibratoToleranceCents);
      // NOTE: `isListening` is *not* flipped here. It's driven by the
      // `audio-event` listener — the first event that arrives is
      // evidence that the backend pipeline actually opened the mic.
      // If the mic failed to open, the Rust side logs and continues
      // the session without emitting events, and `isListening` stays
      // false (so `PitchDisplay` shows the idle copy instead of lying
      // about a dead mic).
    } catch (err) {
      // Roll back to idle so the UI can retry. Store the error text so
      // the caller (or a future error banner) can surface it. Clear
      // any stale listening flag from a previous session.
      set({ status: "idle", recapError: String(err) });
      useAudioStore.getState().setListening(false);
      throw err;
    }
  },

  endSession: async () => {
    const { status } = get();
    if (status !== "listening") {
      throw new Error(
        `cannot end session from status=${status} — nothing to end`,
      );
    }
    set({ status: "ending" });
    // Mic pipeline is torn down on the Rust side by the time the
    // `end_practice_session` command even starts executing — flip the
    // listening flag immediately so `PitchDisplay` drops back to its
    // idle copy while the recap round-trip is in flight.
    useAudioStore.getState().setListening(false);
    try {
      const recap = await invoke<SessionRecap>("end_practice_session");
      set({
        status: "idle",
        screen: "recap",
        recap,
        recapError: null,
        sessionId: null,
      });
    } catch (err) {
      // Recap failure: still exit the session, still go to the recap
      // screen — the screen has a fallback copy for `recapError`. This
      // matches the design doc: "never fail loudly on a recap."
      set({
        status: "idle",
        screen: "recap",
        recap: null,
        recapError: String(err),
        sessionId: null,
      });
    }
  },

  switchInstrument: async (name: string, vibratoToleranceCents = 15.0) => {
    const { status } = get();
    if (status !== "listening") {
      throw new Error(
        `cannot switch instrument from status=${status} — start a session first`,
      );
    }
    const segmentId = await invoke<string>("switch_instrument", {
      instrument: name,
      practiceMode: get().practiceMode,
    });
    set({ instrumentName: name, segmentId });
    useAudioStore.getState().setInstrument(name, vibratoToleranceCents);
  },

  pushPhrase: (phrase) =>
    set((state) => ({ phrases: [...state.phrases, phrase] })),

  pushTip: (tip, phraseIndex) =>
    set((state) => ({
      tipQueue: [
        ...state.tipQueue,
        { id: newId(), tip, receivedAt: Date.now(), phraseIndex },
      ],
    })),

  dismissTip: (id) =>
    set((state) => ({
      tipQueue: state.tipQueue.filter((q) => q.id !== id),
    })),

  tick: () => {
    const { status, startedAtMs } = get();
    if (status !== "listening" || startedAtMs === null) {
      return;
    }
    // Integer seconds from start. Rendering MM:SS from this derived
    // display string is trivial and re-renders at most 1Hz.
    set({ elapsedSecs: Math.floor((Date.now() - startedAtMs) / 1000) });
  },

  returnToSelector: () =>
    set({
      screen: "selector",
      status: "idle",
      recap: null,
      recapError: null,
      sessionId: null,
      instrumentName: null,
      segmentId: null,
      startedAtMs: null,
      elapsedSecs: 0,
      phrases: [],
      tipQueue: [],
    }),

  setCoachingEnabled: (on) => {
    saveCoachingPref(on);
    set({ coachingEnabled: on });
  },

  setPracticeMode: (mode) => {
    savePracticeModePref(mode);
    set({ practiceMode: mode });
  },

  goToHistory: () =>
    set({
      screen: "history",
    }),
}));
