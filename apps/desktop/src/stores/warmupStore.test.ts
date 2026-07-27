import { describe, it, expect, vi, beforeEach } from "vitest";

/**
 * #257 S4 — the warmup store: the note collector's honesty (a held note is
 * ONE note, a flicker is none, silence separates re-attacks) and the state
 * machine's contract (an abandoned throw never calls complete; a failed
 * completion keeps the take for retry).
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import { useWarmupStore } from "./warmupStore";
import type { AudioEvent } from "./audioStore";

/** A pitched event at `midi`, or silence when `midi` is null. */
const ev = (midi: number | null): AudioEvent => ({
  pitch_hz: midi === null ? null : 440,
  confidence: 0.9,
  amplitude: 0.5,
  timestamp_secs: 0,
  is_onset: false,
  note_info:
    midi === null
      ? null
      : { midi_note: midi, note_name: "X", octave: 4, cents_deviation: 0 },
});

const hear = (...midis: Array<number | null>) => {
  for (const m of midis) {
    useWarmupStore.getState().hearEvent(ev(m));
  }
};

const CHALLENGE = {
  seed: 42,
  label: "C# Mixolydian scale · up-down · 72 BPM",
  target_notes: [61, 63, 65],
};

beforeEach(() => {
  mockInvoke.mockReset();
  useWarmupStore.setState({
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
  });
});

const activeState = () =>
  useWarmupStore.setState({ phase: "active", challenge: CHALLENGE });

describe("note collector", () => {
  it("a held note streaming many events records exactly once", () => {
    activeState();
    hear(60, 60, 60, 60, 60, 60, 60, 60);
    expect(useWarmupStore.getState().playedNotes).toEqual([60]);
  });

  it("a single-event flicker between stable notes records nothing", () => {
    activeState();
    // Held C, one bogus D event, C continues: the blip never reaches the
    // stability run, so the take stays [C].
    hear(60, 60, 60, 62, 60, 60);
    expect(useWarmupStore.getState().playedNotes).toEqual([60]);
  });

  it("silence separates re-attacks of the same pitch", () => {
    activeState();
    // Up-down passes revisit degrees: C … rest … C must be TWO notes.
    hear(60, 60, null, 60, 60);
    expect(useWarmupStore.getState().playedNotes).toEqual([60, 60]);
  });

  it("a note change without silence records once stable", () => {
    activeState();
    hear(60, 60, 62, 62, 64, 64);
    expect(useWarmupStore.getState().playedNotes).toEqual([60, 62, 64]);
  });

  it("ignores events outside the active phase", () => {
    hear(60, 60);
    expect(useWarmupStore.getState().playedNotes).toEqual([]);
    useWarmupStore.setState({ phase: "done" });
    hear(62, 62);
    expect(useWarmupStore.getState().playedNotes).toEqual([]);
  });

  it("stops recording at the client-side bound", () => {
    activeState();
    // Alternate two pitches with a rest between pairs so every attack is
    // distinct; push past the 1024-note cap.
    for (let i = 0; i < 1100; i++) {
      hear(60, 60, null);
    }
    expect(useWarmupStore.getState().playedNotes.length).toBe(1024);
  });
});

describe("state machine", () => {
  it("startWarmup throws the roulette and resets the take", async () => {
    mockInvoke.mockResolvedValue(CHALLENGE);
    useWarmupStore.setState({
      playedNotes: [1, 2, 3],
      result: { score: 0.5, streak: { count: 1, completed_today: true } },
      notice: "old",
    });
    await useWarmupStore.getState().startWarmup();
    expect(mockInvoke).toHaveBeenCalledWith("start_daily_warmup", undefined);
    const s = useWarmupStore.getState();
    expect(s.phase).toBe("active");
    expect(s.challenge).toEqual(CHALLENGE);
    expect(s.playedNotes).toEqual([]);
    expect(s.result).toBeNull();
    expect(s.notice).toBeNull();
  });

  it("finishWarmup grades the collected take against the echoed seed", async () => {
    activeState();
    useWarmupStore.setState({ playedNotes: [61, 63, 65] });
    const result = { score: 0.8, streak: { count: 3, completed_today: true } };
    mockInvoke.mockResolvedValue(result);
    await useWarmupStore.getState().finishWarmup();
    expect(mockInvoke).toHaveBeenCalledWith("complete_daily_warmup", {
      seed: 42,
      playedNotes: [61, 63, 65],
    });
    const s = useWarmupStore.getState();
    expect(s.phase).toBe("done");
    expect(s.result).toEqual(result);
    // The badge truth updates from the completion, no extra get_streak.
    expect(s.streak).toEqual(result.streak);
  });

  it("a failed completion keeps the take on screen for retry", async () => {
    activeState();
    useWarmupStore.setState({ playedNotes: [61] });
    mockInvoke.mockRejectedValue("store is busy");
    await useWarmupStore.getState().finishWarmup();
    const s = useWarmupStore.getState();
    expect(s.phase).toBe("active");
    expect(s.playedNotes).toEqual([61]);
    expect(s.notice).toContain("store is busy");
    expect(s.submitting).toBe(false);
  });

  it("a second finish while one is in flight is a no-op", async () => {
    activeState();
    let resolve!: (v: unknown) => void;
    mockInvoke.mockReturnValue(new Promise((r) => (resolve = r)));
    const first = useWarmupStore.getState().finishWarmup();
    const second = useWarmupStore.getState().finishWarmup();
    resolve({ score: 1, streak: { count: 1, completed_today: true } });
    await Promise.all([first, second]);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("a completion resolving after close updates the badge but not the panel", async () => {
    activeState();
    let resolve!: (v: unknown) => void;
    mockInvoke.mockReturnValue(new Promise((r) => (resolve = r)));
    const pending = useWarmupStore.getState().finishWarmup();
    useWarmupStore.getState().closeWarmup();
    resolve({ score: 1, streak: { count: 7, completed_today: true } });
    await pending;
    const s = useWarmupStore.getState();
    expect(s.phase).toBe("idle");
    expect(s.result).toBeNull();
    expect(s.streak).toEqual({ count: 7, completed_today: true });
  });

  it("closeWarmup abandons without any IPC call — the throw writes nothing", () => {
    activeState();
    useWarmupStore.setState({ playedNotes: [61, 63] });
    useWarmupStore.getState().closeWarmup();
    expect(mockInvoke).not.toHaveBeenCalled();
    const s = useWarmupStore.getState();
    expect(s.phase).toBe("idle");
    expect(s.challenge).toBeNull();
    expect(s.playedNotes).toEqual([]);
  });

  it("expireUnheard only fires on an empty take", () => {
    activeState();
    useWarmupStore.setState({ playedNotes: [61] });
    useWarmupStore.getState().expireUnheard();
    expect(useWarmupStore.getState().phase).toBe("active");
    useWarmupStore.setState({ playedNotes: [] });
    useWarmupStore.getState().expireUnheard();
    expect(useWarmupStore.getState().phase).toBe("unheard");
  });

  it("fetchStreak reads the badge truth and survives failure", async () => {
    mockInvoke.mockResolvedValue({ count: 5, completed_today: false });
    await useWarmupStore.getState().fetchStreak();
    expect(mockInvoke).toHaveBeenCalledWith("get_streak", undefined);
    expect(useWarmupStore.getState().streak).toEqual({
      count: 5,
      completed_today: false,
    });
    mockInvoke.mockRejectedValue("ipc down");
    await useWarmupStore.getState().fetchStreak();
    // Last good truth is kept — no crash, no invented count.
    expect(useWarmupStore.getState().streak).toEqual({
      count: 5,
      completed_today: false,
    });
  });
});
