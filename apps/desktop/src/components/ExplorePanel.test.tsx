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
    { label: "Shuffle 🎲", delta: { kind: "reshuffle_roots" }, enabled: false },
    {
      label: "Add keys",
      delta: { kind: "bump_difficulty", by: 1 },
      enabled: true,
    },
    {
      label: "Simpler",
      delta: { kind: "bump_difficulty", by: -1 },
      enabled: false,
    },
    {
      label: "Try a pattern 🎲",
      delta: { kind: "try_pattern" },
      enabled: true,
    },
    {
      label: "Different scale",
      delta: { kind: "different_scale" },
      enabled: true,
    },
  ],
  root_pitch_classes: [7, 0, 2],
  root_names: ["G", "C", "D"],
  can_undo: false,
  staff: {
    fifths: -2,
    beats_per_measure: 4,
    total_beats: 12,
    notes: [
      {
        midi: 67,
        start_beat: 0,
        duration_beats: 1,
        step: 2,
        accidental: null,
        is_root: true,
      },
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
    expect(screen.getByTestId("explore-chips").children).toHaveLength(5);
  });

  // #445-4: gated chips stay IN the row, dimmed and inert — they never
  // vanish (the old row shuffled identities under the player's finger).
  it("disabled chips render in place and do not fire", () => {
    usePracticeStore.setState({ explore: dto });
    render(<ExplorePanel />);
    const shuffle = screen.getByTestId("chip-reshuffle_roots");
    expect(shuffle).toBeDisabled();
    fireEvent.click(shuffle);
    expect(mockInvoke).not.toHaveBeenCalled();
    expect(screen.getByTestId("chip-bump_difficulty-down")).toBeDisabled();
    expect(screen.getByTestId("chip-bump_difficulty-up")).not.toBeDisabled();
  });

  // #255: tapping a chip echoes back the EXACT delta the backend attached —
  // the frontend performs no theory.
  it("a tapped chip echoes its delta to apply_variation_delta", () => {
    usePracticeStore.setState({ explore: dto });
    mockInvoke.mockResolvedValueOnce({ ...dto, label: "next rep" });
    render(<ExplorePanel />);
    fireEvent.click(screen.getByTestId("chip-bump_difficulty-up"));
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
