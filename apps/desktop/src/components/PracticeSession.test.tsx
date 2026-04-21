import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import PracticeSession from "./PracticeSession";
import { usePracticeStore } from "../stores/practiceStore";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

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
  });
}

describe("PracticeSession", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    seedListeningSession();
  });

  it("renders the current instrument in the header", () => {
    render(<PracticeSession />);
    const btn = screen.getByTestId("instrument-switch-button");
    expect(btn.textContent).toContain("Trumpet");
  });

  it("toggles the instrument-switch menu on click", () => {
    render(<PracticeSession />);
    expect(screen.queryByTestId("instrument-switch-menu")).toBeNull();

    fireEvent.click(screen.getByTestId("instrument-switch-button"));
    screen.getByTestId("instrument-switch-menu");

    fireEvent.click(screen.getByTestId("instrument-switch-button"));
    expect(screen.queryByTestId("instrument-switch-menu")).toBeNull();
  });

  it("switching instrument mid-session calls invoke and updates header", async () => {
    mockInvoke.mockResolvedValueOnce("seg-piano-1");
    render(<PracticeSession />);
    fireEvent.click(screen.getByTestId("instrument-switch-button"));
    fireEvent.click(screen.getByTestId("instrument-switch-option-piano"));

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("switch_instrument", {
        instrument: "Piano",
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
});
