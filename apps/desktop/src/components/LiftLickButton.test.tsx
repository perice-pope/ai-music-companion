import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import LiftLickButton from "./LiftLickButton";
import { usePracticeStore } from "../stores/practiceStore";
import type { ExploreDto } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const dto: ExploreDto = {
  range_notice: null,
  label: "D · your 5-note cell · 3 roots · 60 BPM",
  music_xml: "<score-partwise/>",
  chips: [],
  root_pitch_classes: [2, 9, 4],
  root_names: ["D", "A", "E"],
  can_undo: false,
  staff: {
    fifths: 0,
    beats_per_measure: 4,
    total_beats: 4,
    notes: [
      {
        midi: 62,
        start_beat: 0,
        duration_beats: 1,
        step: -1,
        accidental: null,
        is_root: true,
      },
    ],
  },
};

describe("LiftLickButton (#285)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    usePracticeStore.setState({
      explore: null,
      exploreNotice: null,
      status: "listening",
    });
  });

  // #285 AC: one tap lifts the last phrase into the explore surface.
  it("lifts the last phrase into an exploration", async () => {
    mockInvoke.mockResolvedValueOnce(dto);
    render(<LiftLickButton />);
    fireEvent.click(screen.getByTestId("lift-lick"));
    await waitFor(() =>
      expect(usePracticeStore.getState().explore).toEqual(dto),
    );
    expect(mockInvoke).toHaveBeenCalledWith("explore_last_phrase", {});
    expect(usePracticeStore.getState().exploreNotice).toBeNull();
  });

  // #285 AC (calm refusal): nothing liftable → the backend's own message
  // shows under the button; no exploration starts.
  it("shows the backend's calm notice when nothing was played", async () => {
    mockInvoke.mockRejectedValueOnce(
      "play a little phrase first — then I can lift it",
    );
    render(<LiftLickButton />);
    fireEvent.click(screen.getByTestId("lift-lick"));
    await waitFor(() =>
      expect(screen.getByTestId("lift-lick-notice")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("lift-lick-notice")).toHaveTextContent(
      "play a little phrase first",
    );
    expect(usePracticeStore.getState().explore).toBeNull();
  });
});
