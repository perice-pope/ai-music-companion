import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import PracticeSession from "./PracticeSession";
import { usePracticeStore } from "../stores/practiceStore";
import type { InstrumentInfo } from "../stores/audioStore";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const TEST_INSTRUMENTS: InstrumentInfo[] = [
  { name: "Trumpet", family: "Brass", freqMinHz: 165, freqMaxHz: 1047, vibratoToleranceCents: 20, emoji: "\uD83C\uDFBA" },
  { name: "Piano", family: "Keyboard", freqMinHz: 28, freqMaxHz: 4186, vibratoToleranceCents: 10, emoji: "\uD83C\uDFB9" },
  { name: "Violin", family: "Strings", freqMinHz: 196, freqMaxHz: 3136, vibratoToleranceCents: 30, emoji: "\uD83C\uDFBB" },
];

function seedListeningSession() {
  usePracticeStore.setState({
    screen: "session",
    status: "listening",
    sessionId: "sid",
    instrumentName: "Trumpet",
    segmentId: null,
    startedAtMs: Date.now(),
    elapsedSecs: 0,
    phrases: [],
    tipQueue: [],
    recap: null,
    recapError: null,
    practiceMode: "practice",
    lessonDrill: null,
    lessonScore: null,
    lessonRecap: null,
    lessonSubmitting: false,
  });
}

const LESSON_DRILL = {
  index: 0,
  drill_count: 4,
  kind: "warmup_scale",
  label: "C Major · up · 1 roots · 60 BPM",
  tempo_bpm: 60,
  difficulty: 0,
  music_xml: "<score-partwise/>",
  target_len: 8,
  root_pitch_classes: [0, 7, 2],
  root_names: ["C", "G", "D"],
};

describe("PracticeSession", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    seedListeningSession();
    // Default: only `list_instruments` is handled; every other command
    // must be explicitly mocked by the test that triggers it.
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      return Promise.reject(new Error(`no mock configured for "${cmd}"`));
    });
  });

  it("renders the current instrument in the header", () => {
    render(<PracticeSession />);
    const btn = screen.getByTestId("instrument-switch-button");
    expect(btn.textContent).toContain("Trumpet");
  });

  it("toggles the instrument-switch menu on click", async () => {
    render(<PracticeSession />);
    // Let the `list_instruments` effect resolve so the menu can render.
    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("list_instruments");
    });
    expect(screen.queryByTestId("instrument-switch-menu")).toBeNull();

    fireEvent.click(screen.getByTestId("instrument-switch-button"));
    screen.getByTestId("instrument-switch-menu");

    fireEvent.click(screen.getByTestId("instrument-switch-button"));
    expect(screen.queryByTestId("instrument-switch-menu")).toBeNull();
  });

  it("switching instrument mid-session calls invoke and updates header", async () => {
    mockInvoke.mockImplementation((cmd: string, _args?: unknown) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "switch_instrument") return Promise.resolve("seg-piano-1");
      return Promise.reject(new Error(`unexpected cmd ${cmd}`));
    });
    render(<PracticeSession />);
    // Wait for the switch menu items to be populated before opening it.
    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("list_instruments");
    });
    fireEvent.click(screen.getByTestId("instrument-switch-button"));
    fireEvent.click(await screen.findByTestId("instrument-switch-option-piano"));

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("switch_instrument", {
        instrument: "Piano",
        practiceMode: "practice",
      });
    });
    await vi.waitFor(() => {
      expect(usePracticeStore.getState().instrumentName).toBe("Piano");
    });
    await vi.waitFor(() => {
      expect(usePracticeStore.getState().segmentId).toBe("seg-piano-1");
    });
    const btn = screen.getByTestId("instrument-switch-button");
    expect(btn.textContent).toContain("Piano");
  });

  it("clicking End Session triggers end flow and navigates to recap", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "end_practice_session") {
        return Promise.resolve({
          overall_assessment: "Done",
          strengths: ["s"],
          areas_to_improve: ["a"],
          next_session_suggestions: ["n"],
          duration_secs: 30,
          phrase_count: 2,
          instrument: "Trumpet",
        });
      }
      return Promise.reject(new Error(`unexpected cmd ${cmd}`));
    });

    render(<PracticeSession />);
    fireEvent.click(screen.getByTestId("end-session-button"));

    await vi.waitFor(() => {
      expect(usePracticeStore.getState().screen).toBe("recap");
    });
    expect(usePracticeStore.getState().recap?.phrase_count).toBe(2);
  });

  it("renders the coaching tip panel", () => {
    render(<PracticeSession />);
    expect(screen.getByTestId("coaching-tip-panel-empty")).toBeDefined();
  });

  it("renders the current practice mode in the header", () => {
    render(<PracticeSession />);
    const btn = screen.getByTestId("practice-mode-switch-button");
    expect(btn.textContent).toContain("Practice");
  });

  it("switching mode mid-session re-invokes switch_instrument under the new mode", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "switch_instrument") return Promise.resolve("seg-new");
      return Promise.reject(new Error(`unexpected cmd ${cmd}`));
    });
    render(<PracticeSession />);

    fireEvent.click(screen.getByTestId("practice-mode-switch-button"));
    fireEvent.click(
      await screen.findByTestId("practice-mode-switch-option-run_through"),
    );

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("switch_instrument", {
        instrument: "Trumpet",
        practiceMode: "run_through",
      });
    });
    expect(usePracticeStore.getState().practiceMode).toBe("run_through");
  });

  it("picking the currently-active mode is a no-op (no invoke)", async () => {
    render(<PracticeSession />);
    fireEvent.click(screen.getByTestId("practice-mode-switch-button"));
    fireEvent.click(
      await screen.findByTestId("practice-mode-switch-option-practice"),
    );

    // No switch_instrument call should have fired (only list_instruments
    // from the mount effect).
    const switchCalls = mockInvoke.mock.calls.filter(
      (args) => args[0] === "switch_instrument",
    );
    expect(switchCalls.length).toBe(0);
  });

  it("displays coaching tips when they are queued", async () => {
    render(<PracticeSession />);

    // Queue a coaching tip
    usePracticeStore.setState({
      tipQueue: [
        {
          id: "test-tip",
          tip: {
            text: "Keep steady breathing",
            severity: "suggestion",
            category: "technique",
          },
          receivedAt: Date.now(),
          phraseIndex: 0,
        },
      ],
    });

    await vi.waitFor(() => {
      expect(screen.getByText("Keep steady breathing")).toBeDefined();
    });
  });

  // #254: the "Give me a lesson" button starts a lesson, and while a lesson is
  // active it takes the stage — even over score mode — and the button hides.
  it("lesson takes precedence over score mode and hides the start button", () => {
    usePracticeStore.setState({
      lessonDrill: LESSON_DRILL,
      activeScoreXml: "<score-partwise/>",
    });
    render(<PracticeSession />);
    expect(screen.getByTestId("lesson-panel")).toBeInTheDocument();
    expect(screen.queryByTestId("session-score-pane")).not.toBeInTheDocument();
    expect(screen.queryByTestId("start-lesson")).not.toBeInTheDocument();
  });

  it("the start-lesson button fires start_lesson", () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_instruments") return Promise.resolve(TEST_INSTRUMENTS);
      if (cmd === "start_lesson")
        return Promise.resolve({ seed: 1, score: null, drill: LESSON_DRILL, recap: null });
      return Promise.reject(new Error(`no mock configured for "${cmd}"`));
    });
    render(<PracticeSession />);
    fireEvent.click(screen.getByTestId("start-lesson"));
    expect(mockInvoke).toHaveBeenCalledWith("start_lesson", {});
  });

  // #349 T4a: the "Listen to the room" toggle swaps the free-play
  // centerpiece for the jam chord lane (and back). Fails if the mode stops
  // reaching the layout.
  it("listen-to-room swaps the centerpiece for the chord lane", () => {
    seedListeningSession();
    usePracticeStore.setState({
      activeScoreXml: null,
      explore: null,
      listenToRoom: false,
      chordLane: [],
    });
    render(<PracticeSession />);
    expect(screen.queryByTestId("chord-lane")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("listen-to-room-toggle"));
    expect(screen.getByTestId("chord-lane")).toBeInTheDocument();
    expect(screen.getByTestId("listen-to-room-toggle")).toHaveTextContent(
      "Listening to the room",
    );
    fireEvent.click(screen.getByTestId("listen-to-room-toggle"));
    expect(screen.queryByTestId("chord-lane")).not.toBeInTheDocument();
  });
});
