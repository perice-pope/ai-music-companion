import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import PracticeShell from "./PracticeShell";
import { usePracticeStore } from "../stores/practiceStore";

// Mock Tauri invoke so PracticeSession's child components don't error
// on their own subscribe calls. Routed BY COMMAND NAME (review MF1): a
// blanket string once fed InstrumentSelector's list fetch, and
// `instruments.find` blew up asynchronously in a LATER test.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) =>
    Promise.resolve(cmd === "list_instruments" ? [] : "mock-id"),
  ),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
// The shell ROUTES; the History page has its own suite (History.test.tsx).
// Mounting the real page under this file's blanket string-resolving invoke
// mock crashes its render (review MF1: sessions.map on "mock-id").
vi.mock("../pages/History", () => ({
  default: () => <div data-testid="stub-history" />,
}));

function resetStore() {
  usePracticeStore.setState({
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
  });
}

describe("PracticeShell routing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("renders the selector screen by default", () => {
    render(<PracticeShell />);
    // The selector heading doubles as the app title on this screen.
    screen.getByTestId("practice-shell-selector");
    screen.getByTestId("instrument-selector");
  });

  // #445-8: History was only reachable through Connections & Privacy —
  // the selector now offers the door directly.
  it("the selector offers My sessions and it lands on History", () => {
    render(<PracticeShell />);
    fireEvent.click(screen.getByTestId("open-history"));
    expect(usePracticeStore.getState().screen).toBe("history");
    screen.getByTestId("stub-history");
  });

  it("renders the session screen when screen=session", () => {
    usePracticeStore.setState({
      screen: "session",
      status: "listening",
      instrumentName: "Trumpet",
      startedAtMs: Date.now(),
    });
    render(<PracticeShell />);
    screen.getByTestId("practice-session");
    // Selector must be hidden — we're in-session.
    expect(screen.queryByTestId("instrument-selector")).toBeNull();
  });

  it("renders the recap screen when screen=recap", () => {
    usePracticeStore.setState({
      screen: "recap",
      recap: {
        overall_assessment: "Nice work",
        strengths: ["A"],
        areas_to_improve: ["B"],
        next_session_suggestions: ["C"],
        duration_secs: 60,
        phrase_count: 3,
        instrument: "Trumpet",
      },
    });
    render(<PracticeShell />);
    screen.getByTestId("session-recap");
    expect(screen.queryByTestId("instrument-selector")).toBeNull();
    expect(screen.queryByTestId("practice-session")).toBeNull();
  });
});
