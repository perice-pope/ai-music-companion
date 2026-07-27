import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { AudioEvent } from "./audioStore";
import type {
  StreakDto,
  WarmupChallengeDto,
  WarmupResultDto,
} from "../types/brain";

/**
 * #257 S4 — the Daily Warmup Roulette's frontend state machine.
 *
 * All scoring and streak math lives in the Rust core; this store only
 * throws (`start_daily_warmup`), collects what the live mic stream heard,
 * and hands the take back (`complete_daily_warmup`). An abandoned throw
 * never calls complete, so it writes nothing (spec §6).
 */
export type WarmupPhase = "idle" | "active" | "unheard" | "done";

/** The ritual's pacing budget (spec §1): throw → play → score in ~60 s.
 * The countdown paces, it doesn't punish — a take in progress at 0:00 is
 * graded, and an unheard one costs nothing. */
export const WARMUP_SECONDS = 60;

/** Consecutive audio events that must agree on a MIDI note before it counts
 * as played. The pipeline streams at ~40–50 Hz, so two events ≈ 45 ms —
 * far shorter than any deliberate 72 BPM warmup note, but longer than the
 * single-event pitch flicker this rejects (the same idea as the backend
 * drill grader's `DRILL_MIN_PITCH_RUN`). */
const STABLE_RUN = 2;

/** Client-side bound on the collected take. A 60 s warmup at 72 BPM is
 * ~15 notes; this only keeps a stuck session from growing an unbounded
 * IPC payload. The backend truncates at its own documented bound anyway. */
const MAX_COLLECTED_NOTES = 1024;

interface WarmupState {
  /** Badge truth from the last `get_streak` / completion. `null` until the
   * first successful read — the badge renders nothing rather than guess. */
  streak: StreakDto | null;
  phase: WarmupPhase;
  challenge: WarmupChallengeDto | null;
  /** The collected take: stable MIDI notes in played order. */
  playedNotes: number[];
  /** The graded outcome once `phase === "done"`. */
  result: WarmupResultDto | null;
  /** Calm error copy when a completion fails; the take is kept for retry. */
  notice: string | null;
  submitting: boolean;

  /** Note-collector scratch: the last recorded note (suppresses the held
   * note's stream), and the candidate still earning its `STABLE_RUN`. */
  _lastRecordedMidi: number | null;
  _pendingMidi: number | null;
  _pendingRun: number;

  fetchStreak: () => Promise<void>;
  startWarmup: () => Promise<void>;
  hearEvent: (event: AudioEvent) => void;
  finishWarmup: () => Promise<void>;
  expireUnheard: () => void;
  closeWarmup: () => void;
}

export const useWarmupStore = create<WarmupState>((set, get) => ({
  streak: null,
  phase: "idle",
  challenge: null,
  playedNotes: [],
  result: null,
  notice: null,
  submitting: false,
  _lastRecordedMidi: null,
  _pendingMidi: null,
  _pendingRun: 0,

  fetchStreak: async () => {
    try {
      const streak = await invoke<StreakDto>("get_streak");
      set({ streak });
    } catch (err) {
      // Badge stays absent — better than a made-up count.
      console.error("get_streak failed:", err);
    }
  },

  startWarmup: async () => {
    const challenge = await invoke<WarmupChallengeDto>("start_daily_warmup");
    set({
      phase: "active",
      challenge,
      playedNotes: [],
      result: null,
      notice: null,
      _lastRecordedMidi: null,
      _pendingMidi: null,
      _pendingRun: 0,
    });
  },

  hearEvent: (event) => {
    const s = get();
    if (s.phase !== "active") {
      return;
    }
    const midi =
      event.pitch_hz !== null && event.note_info !== null
        ? event.note_info.midi_note
        : null;
    if (midi === null) {
      // Silence: the next attack is a new note even if it's the same pitch —
      // an up-down pass revisits every degree on the way back.
      set({ _lastRecordedMidi: null, _pendingMidi: null, _pendingRun: 0 });
      return;
    }
    if (midi === s._lastRecordedMidi) {
      // Still the held note that was already recorded; a flicker candidate
      // that resolved back to it is dropped.
      if (s._pendingMidi !== null) {
        set({ _pendingMidi: null, _pendingRun: 0 });
      }
      return;
    }
    const run = midi === s._pendingMidi ? s._pendingRun + 1 : 1;
    if (run < STABLE_RUN) {
      set({ _pendingMidi: midi, _pendingRun: run });
      return;
    }
    if (s.playedNotes.length >= MAX_COLLECTED_NOTES) {
      // Take is full: keep tracking identity so a later cap-raise stays
      // honest, but record nothing more.
      set({ _lastRecordedMidi: midi, _pendingMidi: null, _pendingRun: 0 });
      return;
    }
    set({
      playedNotes: [...s.playedNotes, midi],
      _lastRecordedMidi: midi,
      _pendingMidi: null,
      _pendingRun: 0,
    });
  },

  finishWarmup: async () => {
    const s = get();
    // Double-tap guard, same reason as the lesson's (#254 review M2): a
    // second in-flight complete would double-grade the take.
    if (s.phase !== "active" || s.challenge === null || s.submitting) {
      return;
    }
    set({ submitting: true });
    try {
      const result = await invoke<WarmupResultDto>("complete_daily_warmup", {
        seed: s.challenge.seed,
        playedNotes: s.playedNotes,
      });
      if (get().phase === "active") {
        set({ phase: "done", result, streak: result.streak, notice: null });
      } else {
        // Panel was closed while the grade was in flight: the completion
        // still landed backend-side, so the badge gets the truth — but a
        // dismissed panel must not resurrect to show it.
        set({ streak: result.streak });
      }
    } catch (err) {
      // Keep the take on screen for a retry and say why — a silent blink
      // would eat the day's completion.
      console.error("complete_daily_warmup failed:", err);
      if (get().phase === "active") {
        set({ notice: String(err) });
      }
    } finally {
      set({ submitting: false });
    }
  },

  expireUnheard: () => {
    const s = get();
    // Only an empty take expires — a heard one is graded instead.
    if (s.phase === "active" && s.playedNotes.length === 0) {
      set({ phase: "unheard" });
    }
  },

  closeWarmup: () =>
    // No IPC call: an abandoned throw writes nothing (spec §6). The badge
    // keeps whatever truth it last read.
    set({
      phase: "idle",
      challenge: null,
      playedNotes: [],
      result: null,
      notice: null,
      _lastRecordedMidi: null,
      _pendingMidi: null,
      _pendingRun: 0,
    }),
}));
