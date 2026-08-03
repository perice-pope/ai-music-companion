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
    label: "C Major · up · 1 root · 60 BPM",
    tempo_bpm: 60,
    difficulty: 0,
    music_xml: "<score-partwise/>",
    target_len: 8,
    root_pitch_classes: [0, 7, 2],
    root_names: ["C", "G", "D"],
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
    // #391: difficulty 0 displays as "step 1" — the player never sees "STEP 0".
    expect(screen.getByText(/step 1/)).toBeInTheDocument();
    expect(screen.queryByText(/step 0/)).toBeNull();
    expect(
      screen.getByText("C Major · up · 1 root · 60 BPM"),
    ).toBeInTheDocument();
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

  // #278: the drill's roots render as RV brand cells — right count, PLAY
  // order, note names, and the exact brand colors. Fails if the cells strip
  // is dropped or the palette mapping drifts.
  it("renders the RV root cells in play order with brand colors", () => {
    usePracticeStore.setState({ lessonDrill: drill() }); // roots [0, 7, 2]
    render(<LessonPanel />);
    const cells = screen.getByTestId("lesson-root-cells");
    expect(cells).toBeInTheDocument();
    expect(screen.getByTestId("root-cell-0")).toHaveTextContent("C");
    expect(screen.getByTestId("root-cell-1")).toHaveTextContent("G");
    expect(screen.getByTestId("root-cell-2")).toHaveTextContent("D");
    // Exact RV brand colors (jsdom normalizes to rgb()).
    expect(screen.getByTestId("root-cell-0")).toHaveStyle({
      backgroundColor: "rgb(142, 250, 0)", // C #8EFA00
    });
    expect(screen.getByTestId("root-cell-1")).toHaveStyle({
      backgroundColor: "rgb(255, 80, 151)", // G #FF5097
    });
    // The header accent carries the leading root's color.
    expect(screen.getByTestId("lesson-header-accent")).toHaveStyle({
      borderColor: "rgb(142, 250, 0)",
    });
  });

  // #254 review M2: while a submit is in flight the button is disabled, so a
  // double-tap can't grade the next drill against an empty take.
  // #335 — the VA's C#-major lesson drew a flat signature under cells
  // saying "C#". Cells render the BACKEND's signature-spelled names: a
  // flat-key drill says Db, never C#. Fails if the UI re-derives names
  // from pitch classes with the sharp-only table.
  it("renders backend-spelled root names — Db, not C#, in flat keys", () => {
    usePracticeStore.setState({
      lessonDrill: {
        ...drill(),
        root_pitch_classes: [1, 8, 3],
        root_names: ["Db", "Ab", "Eb"],
      },
    });
    render(<LessonPanel />);
    expect(screen.getByTestId("root-cell-0")).toHaveTextContent("Db");
    expect(screen.getByTestId("root-cell-1")).toHaveTextContent("Ab");
    expect(screen.queryByText("C#")).toBeNull();
  });

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
    // #391: 1-based in the recap too, so it matches the header's numbering.
    expect(screen.getByText(/step 1 → step 2/)).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("lesson-recap-done"));
    expect(usePracticeStore.getState().lessonRecap).toBeNull();
  });
});
