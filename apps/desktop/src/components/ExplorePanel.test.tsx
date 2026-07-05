import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import ExplorePanel from "./ExplorePanel";
import { usePracticeStore } from "../stores/practiceStore";
import type { ExploreDto } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("./CellStaff", () => ({
  default: () => <div data-testid="stub-cell-staff" />,
}));

const dto: ExploreDto = {
  label: "G Dorian · up-down · 3 roots · 60 BPM",
  music_xml: "<score-partwise/>",
  chips: [
    { label: "New keys 🎲", delta: { kind: "reshuffle_roots" } },
    { label: "Make it spicy", delta: { kind: "bump_difficulty", by: 1 } },
    { label: "Different scale", delta: { kind: "different_scale" } },
  ],
  root_pitch_classes: [7, 0, 2],
  can_undo: false,
  staff: {
    fifths: -2,
    beats_per_measure: 4,
    total_beats: 12,
    notes: [
      { midi: 67, start_beat: 0, duration_beats: 1, step: 2, accidental: null },
    ],
  },
};

describe("ExplorePanel (#255)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usePracticeStore.setState({ explore: null });
  });

  it("renders nothing when not exploring", () => {
    const { container } = render(<ExplorePanel />);
    expect(container.firstChild).toBeNull();
  });

  // #255: the surface shows the label, RV cells (play order + brand colors),
  // the ambient score, and ≤3 chips.
  it("renders the variation with cells and chips", () => {
    usePracticeStore.setState({ explore: dto });
    render(<ExplorePanel />);
    expect(screen.getByText(dto.label)).toBeInTheDocument();
    expect(screen.getByTestId("stub-cell-staff")).toBeInTheDocument();
    const cells = screen.getByTestId("explore-root-cells");
    expect(cells.textContent).toBe("GCD"); // play order preserved
    expect(screen.getByTestId("explore-chips").children).toHaveLength(3);
  });

  // #255: tapping a chip echoes back the EXACT delta the backend attached —
  // the frontend performs no theory.
  it("a tapped chip echoes its delta to apply_variation_delta", () => {
    usePracticeStore.setState({ explore: dto });
    mockInvoke.mockResolvedValueOnce({ ...dto, label: "next rep" });
    render(<ExplorePanel />);
    fireEvent.click(screen.getByTestId("chip-bump_difficulty"));
    expect(mockInvoke).toHaveBeenCalledWith("apply_variation_delta", {
      delta: { kind: "bump_difficulty", by: 1 },
    });
  });

  it("Back to listening ends the exploration", () => {
    usePracticeStore.setState({ explore: dto });
    mockInvoke.mockResolvedValueOnce(undefined);
    render(<ExplorePanel />);
    fireEvent.click(screen.getByTestId("explore-end"));
    expect(usePracticeStore.getState().explore).toBeNull();
    expect(mockInvoke).toHaveBeenCalledWith("end_explore", {});
  });
});
