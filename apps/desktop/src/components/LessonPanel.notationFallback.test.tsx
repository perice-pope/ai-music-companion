import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import LessonPanel from "./LessonPanel";
import { usePracticeStore } from "../stores/practiceStore";
import type { DrillDto } from "../types/brain";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

/**
 * #327 AC: on any residual notation failure the lesson stays a lesson —
 * the RV cell row keeps showing every key and the player reads a calm
 * notice, not a raw TypeError. Unlike LessonPanel.test.tsx this suite does
 * NOT stub ScoreView: the drill's MusicXML goes through the real component
 * and the real OSMD loader, and fails there for real.
 */
describe("LessonPanel when the drill notation cannot render (#327)", () => {
  beforeEach(() => {
    usePracticeStore.setState({
      lessonDrill: null,
      lessonScore: null,
      lessonRecap: null,
      lessonSubmitting: false,
    });
  });

  it("keeps the RV cells and shows a calm notice instead of the error", async () => {
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    const brokenDrill: DrillDto = {
      index: 0,
      drill_count: 4,
      kind: "WarmupScale",
      label: "G Major · up-down · 4 roots · 72 BPM",
      tempo_bpm: 72,
      difficulty: 3,
      music_xml: "<score-partwise", // unloadable — OSMD throws
      target_len: 8,
      root_pitch_classes: [7, 2, 9, 4],
      root_names: ["G", "D", "A", "E"],
    };
    usePracticeStore.setState({ lessonDrill: brokenDrill });
    render(<LessonPanel />);

    const notice = await waitFor(() => screen.getByTestId("score-view-error"));

    // The drill is still playable: header, cells, and grading chrome stand.
    expect(screen.getByTestId("lesson-panel")).toBeInTheDocument();
    const cells = screen.getByTestId("lesson-root-cells");
    expect(cells).toBeInTheDocument();
    expect(cells.textContent).toContain("G");
    expect(screen.getByTestId("root-cell-3")).toBeInTheDocument();
    expect(screen.getByTestId("lesson-submit")).toBeInTheDocument();

    // Calm words, no leaked exception text anywhere on the surface.
    expect(notice.textContent).toContain("notation couldn't be drawn");
    const surface = screen.getByTestId("lesson-panel").textContent ?? "";
    expect(surface).not.toMatch(/TypeError|undefined is not an object|Error:/);

    consoleError.mockRestore();
  });
});
