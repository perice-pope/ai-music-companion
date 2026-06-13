import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import ScorePicker from "./ScorePicker";
import { usePracticeStore } from "../stores/practiceStore";

// The picker's children hit Tauri IPC; stub them so we test the picker alone.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./ScoreDropZone", () => ({
  default: () => <div data-testid="score-drop-zone" />,
}));
vi.mock("./ScoreLibrary", () => ({
  default: () => <div data-testid="score-library" />,
}));

const SCORE = {
  id: "00000000-0000-0000-0000-000000000001",
  title: "Etude No. 1",
  composer: "Anon",
  source_filename: "etude.musicxml",
  added_at: "2026-06-12T00:00:00Z",
  last_practiced_at: null,
  part_index: 0,
  duration_measures: 12,
};

/** Seed the store with a selected score; override instrument/startSession per test. */
function seed(overrides: Record<string, unknown> = {}) {
  usePracticeStore.setState({
    screen: "score-picker",
    activeScore: SCORE,
    instrumentName: null,
    scoreLibrary: [],
    refreshScoreLibrary: vi.fn().mockResolvedValue(undefined),
    startSession: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as Partial<ReturnType<typeof usePracticeStore.getState>>);
}

const startButton = () =>
  screen.getByRole("button", { name: /start practice with this score/i });

describe("ScorePicker — start-with-score (#184)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("disables Start and explains why when no instrument is selected", () => {
    const startSession = vi.fn().mockResolvedValue(undefined);
    seed({ instrumentName: null, startSession });
    render(<ScorePicker />);

    // The button must not look clickable-but-dead: it is disabled...
    expect(startButton()).toBeDisabled();
    // ...and the UI says why, instead of a silent no-op.
    expect(screen.getByText(/pick an instrument first/i)).toBeInTheDocument();

    fireEvent.click(startButton());
    expect(startSession).not.toHaveBeenCalled();
  });

  it("starts the session with the chosen instrument and score", async () => {
    const startSession = vi.fn().mockResolvedValue(undefined);
    seed({ instrumentName: "Trumpet", startSession });
    render(<ScorePicker />);

    expect(startButton()).toBeEnabled();
    fireEvent.click(startButton());

    await waitFor(() =>
      expect(startSession).toHaveBeenCalledWith("Trumpet", 15.0, SCORE.id),
    );
  });

  it("surfaces a visible error when starting the session fails", async () => {
    const startSession = vi
      .fn()
      .mockRejectedValue(new Error("mic permission denied"));
    seed({ instrumentName: "Trumpet", startSession });
    render(<ScorePicker />);

    fireEvent.click(startButton());

    // The failure reaches the user (was previously swallowed by console.error).
    await screen.findByText(/couldn't start practice: mic permission denied/i);
  });
});
