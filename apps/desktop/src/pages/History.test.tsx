import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

/**
 * #445-8 — My Sessions, reachable and real. History existed but tapping a
 * card fetched the detail and rendered NOTHING, and the page was a dead
 * end. These pin: back to practice, card → stored recap, honest absence
 * of the score block, calm failure.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));
// AccountPanel talks to its own account store/plumbing — out of scope here.
vi.mock("../components/AccountPanel", () => ({
  default: () => <div data-testid="stub-account-panel" />,
}));

import History from "./History";
import { useHistoryStore } from "../stores/historyStore";
import { usePracticeStore } from "../stores/practiceStore";

const SUMMARY = {
  id: "s-1",
  instrument: "Voice",
  started_at: "2026-07-18T10:00:00Z",
  ended_at: "2026-07-18T10:20:00Z",
  duration_secs: 1200,
  phrase_count: 14,
};

const DETAIL = {
  id: "s-1",
  started_at: "2026-07-18T10:00:00Z",
  ended_at: "2026-07-18T10:20:00Z",
  recap: {
    overall_assessment: "A focused twenty minutes in G Dorian.",
    strengths: ["steady pulse"],
    areas_to_improve: ["intonation above the staff"],
    next_session_suggestions: ["row the 5-3-2-1 cell"],
    duration_secs: 1200,
    phrase_count: 14,
    instrument: "Voice",
  },
};

function route(overrides: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd in overrides) {
      return overrides[cmd] instanceof Error
        ? Promise.reject(overrides[cmd])
        : Promise.resolve(overrides[cmd]);
    }
    if (cmd === "get_session_history") {
      return Promise.resolve([SUMMARY]);
    }
    if (cmd === "get_practice_stats") {
      return Promise.resolve({
        total_sessions: 1,
        total_time_secs: 1200,
      });
    }
    if (cmd === "get_session_detail") {
      return Promise.resolve(DETAIL);
    }
    return Promise.resolve(null);
  });
}

beforeEach(() => {
  mockInvoke.mockReset();
  useHistoryStore.setState({
    sessions: [],
    stats: null,
    selectedSessionId: null,
    selectedSessionDetail: null,
    isLoading: false,
    error: null,
  });
  usePracticeStore.setState({ screen: "history" as never });
});

describe("History (#445-8 My Sessions)", () => {
  it("back returns to the selector — never a dead end", async () => {
    route();
    render(<History />);
    fireEvent.click(await screen.findByTestId("history-back"));
    expect(usePracticeStore.getState().screen).toBe("selector");
  });

  it("tapping a session opens its STORED recap; back returns to the list", async () => {
    route();
    render(<History />);
    fireEvent.click(await screen.findByText(/Voice — 20m/));
    await waitFor(() =>
      expect(screen.getByTestId("past-session-detail")).toBeInTheDocument(),
    );
    expect(mockInvoke).toHaveBeenCalledWith("get_session_detail", {
      session_id: "s-1",
    });
    expect(screen.getByTestId("past-session-assessment").textContent).toBe(
      "A focused twenty minutes in G Dorian.",
    );
    expect(screen.getByText("steady pulse")).toBeInTheDocument();
    expect(screen.getByText("intonation above the staff")).toBeInTheDocument();
    expect(screen.getByText("row the 5-3-2-1 cell")).toBeInTheDocument();
    // Honest absence: no score practiced → no score block.
    expect(screen.queryByTestId("past-session-score")).toBeNull();
    fireEvent.click(screen.getByTestId("past-session-back"));
    expect(screen.queryByTestId("past-session-detail")).toBeNull();
    expect(screen.getByText(/Voice — 20m/)).toBeInTheDocument();
  });

  it("a recap that judged a score shows the score block", async () => {
    route({
      get_session_detail: {
        ...DETAIL,
        recap: {
          ...DETAIL.recap,
          score_summary: {
            score_title: "Für Elise",
            judged: 42,
            accuracy_pct: 87,
            worst_measures: [],
          },
        },
      },
    });
    render(<History />);
    fireEvent.click(await screen.findByText(/Voice — 20m/));
    await waitFor(() =>
      expect(screen.getByTestId("past-session-score")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("past-session-score").textContent).toContain(
      "Für Elise",
    );
  });

  it("a detail fetch failure surfaces calmly; the list survives", async () => {
    route({ get_session_detail: new Error("gone") });
    render(<History />);
    fireEvent.click(await screen.findByText(/Voice — 20m/));
    await waitFor(() =>
      expect(useHistoryStore.getState().error).toContain(
        "Failed to load session",
      ),
    );
    expect(screen.queryByTestId("past-session-detail")).toBeNull();
    expect(screen.getByText(/Voice — 20m/)).toBeInTheDocument();
  });
});
