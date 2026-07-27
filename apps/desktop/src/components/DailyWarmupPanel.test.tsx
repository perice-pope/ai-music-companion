import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";

/**
 * #257 S4 — the warmup stage: the thrown label + countdown, the live-stream
 * feed into the collector, grading on "I'm done" or on a heard expiry, and
 * the free unheard expiry (no write).
 *
 * Fake timers drive the countdown; async settling is flushed with
 * `act(async …)` microtask drains instead of `waitFor` (which polls on the
 * real clock and hangs under fake timers).
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import DailyWarmupPanel from "./DailyWarmupPanel";
import { useWarmupStore, WARMUP_SECONDS } from "../stores/warmupStore";
import { useAudioStore, type AudioEvent } from "../stores/audioStore";

const CHALLENGE = {
  seed: 42,
  label: "C# Mixolydian scale · up-down · 72 BPM",
  target_notes: [61, 63, 65],
};

const ev = (midi: number): AudioEvent => ({
  pitch_hz: 440,
  confidence: 0.9,
  amplitude: 0.5,
  timestamp_secs: 0,
  is_onset: false,
  note_info: { midi_note: midi, note_name: "X", octave: 4, cents_deviation: 0 },
});

/** Drain pending microtasks (resolved invoke promises) inside act. */
const settle = () => act(async () => {});

beforeEach(() => {
  vi.useFakeTimers();
  mockInvoke.mockReset();
  useAudioStore.setState({ latestEvent: null });
  useWarmupStore.setState({
    streak: null,
    phase: "active",
    challenge: CHALLENGE,
    playedNotes: [],
    result: null,
    notice: null,
    submitting: false,
    _lastRecordedMidi: null,
    _pendingMidi: null,
    _pendingRun: 0,
    _silenceRun: 0,
    _lastEventFed: null,
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("DailyWarmupPanel (#257 S4)", () => {
  it("shows the thrown label and counts the pacing clock down", () => {
    render(<DailyWarmupPanel />);
    expect(screen.getByTestId("warmup-label")).toHaveTextContent(
      CHALLENGE.label,
    );
    expect(screen.getByTestId("warmup-countdown")).toHaveTextContent("1:00");
    act(() => {
      vi.advanceTimersByTime(13_000);
    });
    expect(screen.getByTestId("warmup-countdown")).toHaveTextContent("0:47");
  });

  it("feeds live audio events into the collector", () => {
    render(<DailyWarmupPanel />);
    expect(screen.getByTestId("warmup-heard")).toHaveTextContent("Listening…");
    act(() => {
      useAudioStore.getState().setEvent(ev(61));
    });
    act(() => {
      useAudioStore.getState().setEvent(ev(61));
    });
    expect(useWarmupStore.getState().playedNotes).toEqual([61]);
    expect(screen.getByTestId("warmup-heard")).toHaveTextContent(
      "Heard 1 note",
    );
  });

  it("I'm done grades the take and shows the score + streak", async () => {
    mockInvoke.mockResolvedValue({
      score: 0.8,
      streak: { count: 3, completed_today: true },
    });
    useWarmupStore.setState({ playedNotes: [61, 63, 65] });
    render(<DailyWarmupPanel />);
    fireEvent.click(screen.getByTestId("warmup-finish"));
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith("complete_daily_warmup", {
      seed: 42,
      playedNotes: [61, 63, 65],
    });
    expect(screen.getByTestId("warmup-grade")).toHaveTextContent("80%");
    expect(screen.getByTestId("warmup-streak-line")).toHaveTextContent(
      "3 days in a row",
    );
  });

  it("expiry with a heard take auto-grades it", async () => {
    mockInvoke.mockResolvedValue({
      score: 0.5,
      streak: { count: 1, completed_today: true },
    });
    useWarmupStore.setState({ playedNotes: [61] });
    render(<DailyWarmupPanel />);
    act(() => {
      vi.advanceTimersByTime(WARMUP_SECONDS * 1000);
    });
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith("complete_daily_warmup", {
      seed: 42,
      playedNotes: [61],
    });
    expect(screen.getByTestId("warmup-result")).toBeInTheDocument();
  });

  it("expiry with nothing heard is free — no IPC call, throw-again offered", async () => {
    render(<DailyWarmupPanel />);
    act(() => {
      vi.advanceTimersByTime(WARMUP_SECONDS * 1000);
    });
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(screen.getByTestId("warmup-unheard")).toBeInTheDocument();
    // Throw again re-deals a fresh challenge and restarts the clock.
    mockInvoke.mockResolvedValue({ ...CHALLENGE, seed: 43 });
    fireEvent.click(screen.getByTestId("warmup-try-again"));
    await settle();
    expect(mockInvoke).toHaveBeenCalledWith("start_daily_warmup", undefined);
    expect(screen.getByTestId("warmup-countdown")).toHaveTextContent("1:00");
  });

  it("closing mid-throw abandons without grading", () => {
    useWarmupStore.setState({ playedNotes: [61, 63] });
    render(<DailyWarmupPanel />);
    fireEvent.click(screen.getByTestId("warmup-close"));
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(useWarmupStore.getState().phase).toBe("idle");
  });

  it("a failed grade keeps the throw on screen and says why", async () => {
    mockInvoke.mockRejectedValue("store is busy");
    useWarmupStore.setState({ playedNotes: [61] });
    render(<DailyWarmupPanel />);
    fireEvent.click(screen.getByTestId("warmup-finish"));
    await settle();
    expect(screen.getByTestId("warmup-notice")).toHaveTextContent(
      "store is busy",
    );
    expect(screen.getByTestId("daily-warmup-panel")).toBeInTheDocument();
    expect(useWarmupStore.getState().playedNotes).toEqual([61]);
  });

  it("a grade that fails AT expiry is not auto-retried once per second", async () => {
    // The clock stops at zero and expiry is one-shot: a backend that said
    // "busy" must not be hammered by the countdown's corpse ticking on.
    mockInvoke.mockRejectedValue("store is busy");
    useWarmupStore.setState({ playedNotes: [61] });
    render(<DailyWarmupPanel />);
    act(() => {
      vi.advanceTimersByTime(WARMUP_SECONDS * 1000);
    });
    await settle();
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    await settle();
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    // The player's own retry still works.
    mockInvoke.mockResolvedValue({
      score: 0.4,
      streak: { count: 1, completed_today: true },
    });
    fireEvent.click(screen.getByTestId("warmup-finish"));
    await settle();
    expect(screen.getByTestId("warmup-result")).toBeInTheDocument();
  });
});
