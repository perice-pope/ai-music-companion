import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  CoachingTip,
  PhraseSummary,
  PracticeMode,
  SessionRecap,
  ScoreLibraryEntry,
  ScorePosition,
  LoadedScore,
} from "../types/brain";
import { useAudioStore } from "./audioStore";

/**
 * Screen routing enum — keeps Free Play as a state-machine in the
 * store instead of pulling in react-router for four screens (see
 * design doc §2).
 */
export type AppScreen =
  | "selector"
  | "score-picker"
  | "session"
  | "recap"
  | "history"
  | "connections";

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

/**
 * Result of an audio import: the new library entry plus a calm quality
 * signal (field names mirror the Rust `ImportedAudioDto`). We surface
 * approximate-ness; we never show a fake accuracy score.
 */
export interface ImportedAudio {
  entry: ScoreLibraryEntry;
  note_count: number;
  mean_confidence: number;
  polyphony: number;
  /** Input looks polyphonic — basic-pitch is monophonic-first. */
  polyphonic: boolean;
  /** Transcription confidence looks weak. */
  low_confidence: boolean;
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

  // Score mode (story-score-mode PR 1) ------------------------------------
  activeScore: ScoreLibraryEntry | null;
  /**
   * Raw MusicXML for `activeScore`, fetched lazily when a score is
   * selected. `null` until loaded — `ScoreView` renders nothing without
   * it. Kept here (not just in the component) so it survives navigation
   * between score-picker and session.
   */
  activeScoreXml: string | null;
  scoreLibrary: ScoreLibraryEntry[];
  cursorPosition: ScorePosition | null;

  // UI prefs (persisted) --------------------------------------------------
  coachingEnabled: boolean;
  /**
   * Current practice mode. Persisted so a user's last choice survives
   * a restart. Defaults to `"practice"` on first run (matches the Rust
   * `PracticeMode::default()`).
   */
  practiceMode: PracticeMode;

  // Actions ---------------------------------------------------------------
  loadScoreFromFile: (path: string) => Promise<void>;
  /**
   * Import a MIDI file by its raw bytes (read on the frontend) and make it
   * the active score. The backend parses the MIDI, converts it to canonical
   * MusicXML, and stores it — see `import_midi_file`.
   */
  importMidiFromFile: (
    sourceFilename: string,
    bytes: number[],
  ) => Promise<ScoreLibraryEntry>;
  importAudioFromFile: (
    sourceFilename: string,
    bytes: number[],
  ) => Promise<ImportedAudio>;
  loadScoreFromId: (id: string) => Promise<void>;
  refreshScoreLibrary: () => Promise<void>;
  deleteScore: (id: string) => Promise<void>;
  clearActiveScore: () => void;
  startSession: (
    instrument: string,
    vibratoToleranceCents?: number,
    scoreId?: string,
  ) => Promise<void>;
  endSession: () => Promise<void>;
  switchInstrument: (
    name: string,
    vibratoToleranceCents?: number,
  ) => Promise<void>;
  pushPhrase: (phrase: PhraseSummary) => void;
  pushTip: (tip: CoachingTip, phraseIndex: number) => void;
  /**
   * Live coaching loop: ask the backend for a tip on a just-completed phrase,
   * surface it in the tip panel, and persist it in the session recorder.
   *
   * Gated on `coachingEnabled` (the user's opt-in): when off we fire **no**
   * IPC at all — there's nothing to ask for, and the Rust-core airplane switch
   * (`NetworkPolicy::Offline`) would refuse anyway. When on, the backend may
   * still return `null` (rate-limited, API failure, or offline) — that means
   * "no tip", and we honor the silence rather than inventing one.
   */
  requestCoachingTip: (phrase: PhraseSummary) => Promise<void>;
  dismissTip: (id: string) => void;
  setCursorPosition: (pos: ScorePosition | null) => void;
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
  /** Open the Connections & Privacy panel (networked-feature disclosure). */
  goToConnections: () => void;
}

/** localStorage key for the coaching-on/off preference. */
const COACHING_PREF_KEY = "ai-music-companion:coaching-enabled";
/** localStorage key for the last-used practice mode. */
const PRACTICE_MODE_PREF_KEY = "ai-music-companion:practice-mode";

function loadCoachingPref(): boolean {
  try {
    const raw = localStorage.getItem(COACHING_PREF_KEY);
    // Default: DISABLED (off by default). AI coaching narration is a
    // networked feature, and the offline-first principle says every networked
    // feature is opt-in and starts off (see
    // `docs/architecture/offline-first-and-network-transparency.md`). On first
    // run the coach is served entirely by the on-device fallback. Only an
    // explicit "true" turns narration on; the choice is then persisted.
    return raw === "true";
  } catch {
    return false;
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
  activeScore: null,
  activeScoreXml: null,
  scoreLibrary: [],
  cursorPosition: null,
  coachingEnabled: loadCoachingPref(),
  practiceMode: loadPracticeModePref(),

  loadScoreFromFile: async (path: string) => {
    try {
      const entry = await invoke<ScoreLibraryEntry>("import_score", { path });
      // Fetch the just-imported MusicXML so ScoreView can render it
      // without a second user action.
      const loaded = await invoke<LoadedScore>("get_score", { id: entry.id });
      set((state) => ({
        activeScore: entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: [entry, ...state.scoreLibrary],
      }));
    } catch (err) {
      throw new Error(`Failed to load score: ${err}`);
    }
  },

  importMidiFromFile: async (sourceFilename: string, bytes: number[]) => {
    try {
      const entry = await invoke<ScoreLibraryEntry>("import_midi_file", {
        sourceFilename,
        bytes,
      });
      // Load the freshly-imported MusicXML so ScoreView can render it
      // without a second user action (mirrors loadScoreFromFile).
      const loaded = await invoke<LoadedScore>("get_score", { id: entry.id });
      set((state) => ({
        activeScore: entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: [entry, ...state.scoreLibrary],
      }));
      return entry;
    } catch (err) {
      throw new Error(`Failed to import MIDI: ${err}`);
    }
  },

  importAudioFromFile: async (sourceFilename: string, bytes: number[]) => {
    try {
      const result = await invoke<ImportedAudio>("import_audio_file", {
        sourceFilename,
        bytes,
      });
      // Load the freshly-transcribed MusicXML so ScoreView can render it
      // without a second user action (mirrors importMidiFromFile).
      const loaded = await invoke<LoadedScore>("get_score", {
        id: result.entry.id,
      });
      set((state) => ({
        activeScore: result.entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
        scoreLibrary: [result.entry, ...state.scoreLibrary],
      }));
      return result;
    } catch (err) {
      throw new Error(`Failed to import audio: ${err}`);
    }
  },

  loadScoreFromId: async (id: string) => {
    try {
      // `get_score` returns the entry *and* its MusicXML — the library
      // list only carries metadata, so this is where the actual notes
      // come across the IPC boundary.
      const loaded = await invoke<LoadedScore>("get_score", { id });
      set({
        activeScore: loaded.entry,
        activeScoreXml: loaded.music_xml,
        cursorPosition: null,
      });
    } catch (err) {
      throw new Error(`Failed to load score: ${err}`);
    }
  },

  refreshScoreLibrary: async () => {
    try {
      const library = await invoke<ScoreLibraryEntry[]>("list_scores");
      set({ scoreLibrary: library });
    } catch (err) {
      throw new Error(`Failed to refresh score library: ${err}`);
    }
  },

  deleteScore: async (id: string) => {
    try {
      await invoke("delete_score", { id });
      set((state) => ({
        scoreLibrary: state.scoreLibrary.filter((s) => s.id !== id),
        activeScore: state.activeScore?.id === id ? null : state.activeScore,
      }));
    } catch (err) {
      throw new Error(`Failed to delete score: ${err}`);
    }
  },

  clearActiveScore: () =>
    set({ activeScore: null, activeScoreXml: null, cursorPosition: null }),

  startSession: async (
    instrument: string,
    vibratoToleranceCents = 15.0,
    scoreId?: string,
  ) => {
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
        scoreId: scoreId || null,
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
        cursorPosition: null,
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

  requestCoachingTip: async (phrase) => {
    const { coachingEnabled, status, elapsedSecs, phrases } = get();
    // Opt-in gate: no IPC at all when the user hasn't enabled online coaching,
    // and only while a session is live.
    if (!coachingEnabled || status !== "listening") {
      return;
    }
    try {
      const tip = await invoke<CoachingTip | null>("get_coaching_tip", {
        phrase,
        sessionDurationSecs: elapsedSecs,
        phrasesPlayed: phrases.length,
      });
      // `null` is the honest "no tip" signal (offline / rate-limited / API
      // failure). Surface nothing — the panel shows its empty state.
      if (!tip) {
        return;
      }
      get().pushTip(tip, phrase.phrase_index);
      // Persist it into the session recorder so it lands in history + recap.
      // A failure here must not break the live loop — log and move on.
      try {
        await invoke("record_coaching_tip", {
          phraseIndex: phrase.phrase_index,
          tip,
        });
      } catch (err) {
        console.error("Failed to persist coaching tip:", err);
      }
    } catch (err) {
      // The live tip is best-effort; never let a failed request disrupt
      // the session.
      console.error("Failed to fetch coaching tip:", err);
    }
  },

  dismissTip: (id) =>
    set((state) => ({
      tipQueue: state.tipQueue.filter((q) => q.id !== id),
    })),

  setCursorPosition: (pos) => set({ cursorPosition: pos }),

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
      activeScore: null,
      activeScoreXml: null,
      cursorPosition: null,
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

  goToConnections: () =>
    set({
      screen: "connections",
    }),
}));
