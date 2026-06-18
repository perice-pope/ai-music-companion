import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

const mockInvoke = vi.fn();
const mockListen = vi.fn().mockResolvedValue(() => {});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

describe("App", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The selector fetches the instrument catalog on mount; it must be an
    // array (an empty one is a valid "loaded but empty" shape). Returning
    // a scalar here would make InstrumentSelector's `.map` throw in an
    // async microtask after the test, which is a test-mock bug, not a
    // product one.
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "list_instruments" ? [] : "unused",
    );
    mockListen.mockResolvedValue(() => {});
  });

  it("renders the practice shell with the app title on the selector screen", () => {
    render(<App />);
    // getByRole throws if missing — no extra assertion needed.
    screen.getByRole("heading", { name: "AI Music Companion" });
    screen.getByText(/Free Play/);
  });

  it("sets up the audio-event and phrase-detected listeners on mount", async () => {
    render(<App />);
    await vi.waitFor(() => {
      expect(mockListen).toHaveBeenCalledTimes(5);
    });
    expect(mockListen).toHaveBeenCalledWith(
      "audio-event",
      expect.any(Function),
    );
    expect(mockListen).toHaveBeenCalledWith(
      "phrase-detected",
      expect.any(Function),
    );
    expect(mockListen).toHaveBeenCalledWith(
      "score-position-updated",
      expect.any(Function),
    );
    expect(mockListen).toHaveBeenCalledWith(
      "accompaniment-status",
      expect.any(Function),
    );
    expect(mockListen).toHaveBeenCalledWith("perception", expect.any(Function));
  });

  it("reflects the backend's perception event in the store", async () => {
    const handlers: Record<string, (e: { payload: unknown }) => void> = {};
    mockListen.mockImplementation(
      async (name: string, cb: (e: { payload: unknown }) => void) => {
        handlers[name] = cb;
        return () => {};
      },
    );

    const { usePracticeStore } = await import("./stores/practiceStore");
    usePracticeStore.getState().setPerception(null);

    render(<App />);
    await vi.waitFor(() => expect(handlers["perception"]).toBeDefined());

    handlers["perception"]({
      payload: {
        tempo_bpm: 100,
        swing_ratio: null,
        locked: true,
        key: {
          tonic: 7,
          mode: "major",
          name: "G major",
          confidence: 0.7,
          alternative: { tonic: 4, minor: true, name: "E minor" },
        },
      },
    });

    const p = usePracticeStore.getState().perception;
    expect(p?.tempo_bpm).toBe(100);
    expect(p?.locked).toBe(true);
    expect(p?.key?.name).toBe("G major");
  });

  it("reflects the backend's accompaniment-status event in the store", async () => {
    // The band's playing state is authoritative from the backend (it can stop
    // on its own at session end), so the event must drive the store flag.
    const handlers: Record<string, (e: { payload: unknown }) => void> = {};
    mockListen.mockImplementation(
      async (name: string, cb: (e: { payload: unknown }) => void) => {
        handlers[name] = cb;
        return () => {};
      },
    );

    const { usePracticeStore } = await import("./stores/practiceStore");
    usePracticeStore.getState().setAccompanimentPlaying(false);

    render(<App />);
    await vi.waitFor(() =>
      expect(handlers["accompaniment-status"]).toBeDefined(),
    );

    handlers["accompaniment-status"]({ payload: { playing: true } });
    expect(usePracticeStore.getState().accompanimentPlaying).toBe(true);

    handlers["accompaniment-status"]({ payload: { playing: false } });
    expect(usePracticeStore.getState().accompanimentPlaying).toBe(false);
  });

  it("advances the cursor on a score-position-updated tick", async () => {
    // The ~10 Hz live-cursor event must move the store cursor directly,
    // with no phrase wrapper and no free-play guard (it only fires in
    // score mode).
    const handlers: Record<string, (e: { payload: unknown }) => void> = {};
    mockListen.mockImplementation(
      async (name: string, cb: (e: { payload: unknown }) => void) => {
        handlers[name] = cb;
        return () => {};
      },
    );

    const { usePracticeStore } = await import("./stores/practiceStore");
    usePracticeStore.getState().setCursorPosition(null);

    render(<App />);
    await vi.waitFor(() =>
      expect(handlers["score-position-updated"]).toBeDefined(),
    );

    handlers["score-position-updated"]({
      payload: { measure_number: 7, beat: 2.5 },
    });

    expect(usePracticeStore.getState().cursorPosition).toEqual({
      measure_number: 7,
      beat: 2.5,
    });
  });

  it("advances the cursor when a score-following phrase arrives", async () => {
    // Capture the handler App registers for `phrase-detected`, fire a
    // synthetic phrase carrying a score position, and assert the store's
    // cursor moved — this is the live cursor-follow path end to end on the
    // frontend side.
    const handlers: Record<string, (e: { payload: unknown }) => void> = {};
    mockListen.mockImplementation(
      async (name: string, cb: (e: { payload: unknown }) => void) => {
        handlers[name] = cb;
        return () => {};
      },
    );

    const { usePracticeStore } = await import("./stores/practiceStore");
    usePracticeStore.getState().setCursorPosition(null);

    render(<App />);
    await vi.waitFor(() => expect(handlers["phrase-detected"]).toBeDefined());

    handlers["phrase-detected"]({
      payload: {
        phrase_index: 0,
        start_time: 0,
        end_time: 1,
        duration_secs: 1,
        note_count: 4,
        pitch_stats: {
          mean_hz: 440,
          min_hz: 440,
          max_hz: 440,
          range_cents: 0,
          pitches: [440],
        },
        dynamics: {
          mean_amplitude: 0.5,
          min_amplitude: 0.4,
          max_amplitude: 0.6,
          dynamic_range: 0.2,
        },
        stability: 0.9,
        score_position: { measure_number: 3, beat: 0 },
      },
    });

    expect(usePracticeStore.getState().cursorPosition).toEqual({
      measure_number: 3,
      beat: 0,
    });
    expect(usePracticeStore.getState().phrases.length).toBe(1);
  });

  it("leaves the cursor put for a free-play phrase with no score position", async () => {
    const handlers: Record<string, (e: { payload: unknown }) => void> = {};
    mockListen.mockImplementation(
      async (name: string, cb: (e: { payload: unknown }) => void) => {
        handlers[name] = cb;
        return () => {};
      },
    );

    const { usePracticeStore } = await import("./stores/practiceStore");
    usePracticeStore
      .getState()
      .setCursorPosition({ measure_number: 5, beat: 0 });

    render(<App />);
    await vi.waitFor(() => expect(handlers["phrase-detected"]).toBeDefined());

    handlers["phrase-detected"]({
      payload: {
        phrase_index: 1,
        start_time: 0,
        end_time: 1,
        duration_secs: 1,
        note_count: 4,
        pitch_stats: {
          mean_hz: 440,
          min_hz: 440,
          max_hz: 440,
          range_cents: 0,
          pitches: [440],
        },
        dynamics: {
          mean_amplitude: 0.5,
          min_amplitude: 0.4,
          max_amplitude: 0.6,
          dynamic_range: 0.2,
        },
        stability: 0.9,
        // no score_position → free play
      },
    });

    // Unchanged — a free-play phrase must not yank the cursor to the start.
    expect(usePracticeStore.getState().cursorPosition).toEqual({
      measure_number: 5,
      beat: 0,
    });
  });

  it("still renders the UI when the event subscription fails", async () => {
    // Design invariant: a broken audio event bus must not take down the
    // whole app — the selector is still reachable and the user can
    // still pick an instrument.
    mockListen.mockRejectedValueOnce(new Error("IPC not ready"));
    render(<App />);
    screen.getByRole("heading", { name: "AI Music Companion" });
    screen.getByText(/Free Play/);
  });
});
