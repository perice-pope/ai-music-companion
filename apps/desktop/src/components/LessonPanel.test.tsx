import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import LessonPanel from "./LessonPanel";
import { usePracticeStore } from "../stores/practiceStore";
import type { DrillDto, LessonRecapDto } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// ScoreView drags in OpenSheetMusicDisplay; stub it — this suite tests the
// lesson chrome, not notation rendering.
vi.mock("./ScoreView", () => ({
  default: ({ musicXml }: { musicXml: string }) => (
    <div data-testid="stub-score-view">{musicXml.slice(0, 20)}</div>
  ),
}));

function drill(index = 0): DrillDto {
  return {
    index,
    drill_count: 4,
    kind: "WarmupScale",
    label: "C Major · up · 1 roots · 60 BPM",
    tempo_bpm: 60,
    difficulty: 0,
    music_xml: "<score-partwise/>",
    target_len: 8,
  };
}

const recap: LessonRecapDto = {
  drill_labels: ["warmup", "arpeggio"],
  drill_accuracies: [0.9, 0.75],
  start_difficulty: 0,
  end_difficulty: 1,
};

describe("LessonPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usePracticeStore.setState({
      lessonDrill: null,
      lessonScore: null,
      lessonRecap: null,
      lessonSubmitting: false,
    });
  });

  // No lesson → renders nothing (the free-play surface owns the screen).
  it("renders nothing when no lesson is active", () => {
    const { container } = render(<LessonPanel />);
    expect(container.firstChild).toBeNull();
  });

  // #254: a drill shows its position, difficulty step, label, and the score
  // view — and the grade of the previous drill once one exists.
  it("renders the current drill with label, progress, and last score", () => {
    usePracticeStore.setState({
      lessonDrill: drill(1),
      lessonScore: {
        accuracy: 0.8,
        pitch_accuracy: 0.8,
        timing_accuracy: 0.7,
        correct: 8,
        total: 10,
      },
    });
    render(<LessonPanel />);
    expect(screen.getByTestId("lesson-panel")).toBeInTheDocument();
    expect(screen.getByText(/drill 2 of 4/)).toBeInTheDocument();
    expect(screen.getByText(/step 0/)).toBeInTheDocument();
    expect(screen.getByText("C Major · up · 1 roots · 60 BPM")).toBeInTheDocument();
    expect(screen.getByTestId("lesson-last-score")).toHaveTextContent("80%");
    expect(screen.getByTestId("stub-score-view")).toBeInTheDocument();
  });

  // Submitting fires the backend command (the store handles the response).
  it("submit button steps the lesson via submit_drill", () => {
    usePracticeStore.setState({ lessonDrill: drill() });
    mockInvoke.mockResolvedValueOnce({
      seed: 1,
      score: null,
      drill: null,
      recap,
    });
    render(<LessonPanel />);
    fireEvent.click(screen.getByTestId("lesson-submit"));
    expect(mockInvoke).toHaveBeenCalledWith("submit_drill", {});
  });

  // #254 review M2: while a submit is in flight the button is disabled, so a
  // double-tap can't grade the next drill against an empty take.
  it("disables the grade button while submitting", () => {
    usePracticeStore.setState({ lessonDrill: drill(), lessonSubmitting: true });
    render(<LessonPanel />);
    expect(screen.getByTestId("lesson-submit")).toBeDisabled();
    expect(screen.getByTestId("lesson-submit")).toHaveTextContent("Grading…");
  });

  // #254: the recap lists every drill's accuracy and the difficulty movement,
  // and Done clears it.
  it("renders the recap and clears it on Done", () => {
    usePracticeStore.setState({ lessonRecap: recap });
    render(<LessonPanel />);
    expect(screen.getByTestId("lesson-recap")).toBeInTheDocument();
    expect(screen.getByText("90%")).toBeInTheDocument();
    expect(screen.getByText("75%")).toBeInTheDocument();
    expect(screen.getByText(/step 0 → step 1/)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("lesson-recap-done"));
    expect(usePracticeStore.getState().lessonRecap).toBeNull();
  });
});
