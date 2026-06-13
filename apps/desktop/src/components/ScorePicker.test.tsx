import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import ScorePicker from "./ScorePicker";
import { usePracticeStore } from "../stores/practiceStore";
import { useAudioStore } from "../stores/audioStore";

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

/**
 * Seed a selected score (practiceStore) and the instrument the user picked on
 * the selector (audioStore — the source of truth the picker must read; #184).
 */
function seed(
  selectedInstrument: string | null,
  startSession = vi.fn().mockResolvedValue(undefined),
) {
  useAudioStore.setState({ selectedInstrument });
  usePracticeStore.setState({
    screen: "score-picker",
    activeScore: SCORE,
    scoreLibrary: [],
    refreshScoreLibrary: vi.fn().mockResolvedValue(undefined),
    startSession,
  } as Partial<ReturnType<typeof usePracticeStore.getState>>);
  return startSession;
}

const startButton = () =>
  screen.getByRole("button", { name: /start practice with this score/i });

describe("ScorePicker — start-with-score (#184)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("disables Start and explains why when no instrument is selected", () => {
    const startSession = seed(null);
    render(<ScorePicker />);

    // The button must not look clickable-but-dead: it is disabled...
    expect(startButton()).toBeDisabled();
    // ...and the UI says why, instead of a silent no-op.
    expect(screen.getByText(/pick an instrument first/i)).toBeInTheDocument();

    fireEvent.click(startButton());
    expect(startSession).not.toHaveBeenCalled();
  });

  it("starts the session with the picked instrument and score", async () => {
    // Regression for #184: the picker must use the *selected* instrument, not
    // practiceStore.instrumentName (which is null until a session is running).
    const startSession = seed("Trumpet");
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
    seed("Trumpet", startSession);
    render(<ScorePicker />);

    fireEvent.click(startButton());

    // The failure reaches the user (was previously swallowed by console.error).
    await screen.findByText(/couldn't start practice: mic permission denied/i);
  });
});
