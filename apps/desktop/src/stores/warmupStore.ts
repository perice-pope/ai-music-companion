import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { useAudioStore, type AudioEvent } from "./audioStore";
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
 * drill grader's `DRILL_MIN_PITCH_RUN`).
 *
 * This client-side segmentation exists because S3's IPC contract takes
 * `played_notes` from the frontend (documented drift on the spec: no
 * `Pitch` type in the leaf crate). The GRADE still lives entirely in Rust
 * (`score_warmup`); a follow-up could move collection onto the backend
 * phrase buffer the way `submit_drill` grades. */
const STABLE_RUN = 2;

/** Consecutive silent events before a gap counts as a re-attack boundary —
 * symmetric with `STABLE_RUN`, so a single unvoiced frame inside a held
 * note (breath, bow change, vibrato dip) can't split it into two recorded
 * notes and dent the grade. */
const SILENCE_RUN = 2;

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
   * note's stream), the candidate still earning its `STABLE_RUN`, the
   * silence run earning a re-attack boundary, and the last event object
   * already fed (each event counts once — StrictMode re-runs the feed
   * effect with the same object, and the pre-throw stale event is
   * pre-seeded here by `startWarmup` so it can never be collected). */
  _lastRecordedMidi: number | null;
  _pendingMidi: number | null;
  _pendingRun: number;
  _silenceRun: number;
  _lastEventFed: AudioEvent | null;

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
  _silenceRun: 0,
  _lastEventFed: null,

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
      // A previous throw's in-flight completion no longer owns the flag.
      submitting: false,
      _lastRecordedMidi: null,
      _pendingMidi: null,
      _pendingRun: 0,
      _silenceRun: 0,
      // What the mic heard BEFORE the throw is not part of the take: the
      // feed effect replays the store's latest event on mount, so consume
      // it here — spec §6 says an untouched warmup writes nothing.
      _lastEventFed: useAudioStore.getState().latestEvent,
    });
  },

  hearEvent: (event) => {
    const s = get();
    // Each event object counts exactly once — StrictMode's doubled feed
    // effect must not turn one flicker frame into a stable run.
    if (s.phase !== "active" || event === s._lastEventFed) {
      return;
    }
    set({ _lastEventFed: event });
    const midi =
      event.pitch_hz !== null && event.note_info !== null
        ? event.note_info.midi_note
        : null;
    if (midi === null) {
      // Silence: after a real gap (`SILENCE_RUN`) the next attack is a new
      // note even at the same pitch — an up-down pass revisits every degree
      // on the way back. A single unvoiced frame is NOT a gap: it must not
      // split a held note in two.
      const silence = s._silenceRun + 1;
      if (silence >= SILENCE_RUN) {
        set({
          _lastRecordedMidi: null,
          _pendingMidi: null,
          _pendingRun: 0,
          _silenceRun: silence,
        });
      } else {
        set({ _silenceRun: silence });
      }
      return;
    }
    if (midi === s._lastRecordedMidi) {
      // Still the held note that was already recorded; a flicker candidate
      // that resolved back to it is dropped.
      set({ _pendingMidi: null, _pendingRun: 0, _silenceRun: 0 });
      return;
    }
    const run = midi === s._pendingMidi ? s._pendingRun + 1 : 1;
    if (run < STABLE_RUN) {
      set({ _pendingMidi: midi, _pendingRun: run, _silenceRun: 0 });
      return;
    }
    if (s.playedNotes.length >= MAX_COLLECTED_NOTES) {
      // Take is full: keep tracking identity so a later cap-raise stays
      // honest, but record nothing more.
      set({
        _lastRecordedMidi: midi,
        _pendingMidi: null,
        _pendingRun: 0,
        _silenceRun: 0,
      });
      return;
    }
    set({
      playedNotes: [...s.playedNotes, midi],
      _lastRecordedMidi: midi,
      _pendingMidi: null,
      _pendingRun: 0,
      _silenceRun: 0,
    });
  },

  finishWarmup: async () => {
    const s = get();
    // Double-tap guard, same reason as the lesson's (#254 review M2): a
    // second in-flight complete would double-grade the take.
    if (s.phase !== "active" || s.challenge === null || s.submitting) {
      return;
    }
    const seed = s.challenge.seed;
    // "Still this throw": close (or close + a fresh throw) while the grade
    // was in flight means this completion no longer owns the panel — a
    // dismissed throw must not resurrect, and throw B must never wear
    // throw A's grade.
    const ownsThrow = () =>
      get().phase === "active" && get().challenge?.seed === seed;
    set({ submitting: true });
    try {
      const result = await invoke<WarmupResultDto>("complete_daily_warmup", {
        seed,
        playedNotes: s.playedNotes,
      });
      if (ownsThrow()) {
        set({ phase: "done", result, streak: result.streak, notice: null });
      } else {
        // The completion still landed backend-side, so the badge gets the
        // truth either way.
        set({ streak: result.streak });
      }
    } catch (err) {
      // Keep the take on screen for a retry and say why — a silent blink
      // would eat the day's completion.
      console.error("complete_daily_warmup failed:", err);
      if (ownsThrow()) {
        set({ notice: String(err) });
      }
    } finally {
      // Release the flag — unless a DIFFERENT throw is already active
      // (its own submission owns it now).
      const cur = get();
      if (!(cur.phase === "active" && cur.challenge?.seed !== seed)) {
        set({ submitting: false });
      }
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
      _silenceRun: 0,
      _lastEventFed: null,
    }),
}));
