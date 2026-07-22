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
// ClassroomPanel likewise (#449 enrollment slice) — it has its own tests.
vi.mock("../components/ClassroomPanel", () => ({
  default: () => <div data-testid="stub-classroom-panel" />,
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
  usePracticeStore.setState({ screen: "history" });
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

  it("a detail fetch failure is VISIBLE next to the list", async () => {
    route({ get_session_detail: new Error("gone") });
    render(<History />);
    fireEvent.click(await screen.findByText(/Voice — 20m/));
    // Review MF2: the user must SEE the failure — a store-only error is
    // the dead-tap pattern this page exists to kill.
    await waitFor(() =>
      expect(screen.getByTestId("history-error").textContent).toContain(
        "Failed to load session",
      ),
    );
    expect(screen.queryByTestId("past-session-detail")).toBeNull();
    expect(screen.getByText(/Voice — 20m/)).toBeInTheDocument();
  });

  // Review SF3: a detail left open on a PREVIOUS visit must not greet
  // the player as stale navigation state — re-entry lands on the list.
  it("re-entering History lands on the list, not last visit's detail", async () => {
    route();
    useHistoryStore.setState({
      selectedSessionId: "s-1",
      selectedSessionDetail: DETAIL as never,
    });
    render(<History />);
    expect(await screen.findByText(/Voice — 20m/)).toBeInTheDocument();
    expect(screen.queryByTestId("past-session-detail")).toBeNull();
  });

  // Review SF4: leaving History must not fire returnToSelector's resets —
  // a score staged by Open score survives the detour.
  it("back to practice preserves a staged score", async () => {
    route();
    usePracticeStore.setState({
      activeScore: { id: "id-1", title: "Für Elise" } as never,
      activeScoreXml: "<score-partwise/>",
    });
    render(<History />);
    fireEvent.click(await screen.findByTestId("history-back"));
    const s = usePracticeStore.getState();
    expect(s.screen).toBe("selector");
    expect(s.activeScore).toMatchObject({ id: "id-1" });
    expect(s.activeScoreXml).toBe("<score-partwise/>");
  });
});
