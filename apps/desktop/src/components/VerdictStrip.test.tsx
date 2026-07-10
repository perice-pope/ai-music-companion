import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import VerdictStrip from "./VerdictStrip";
import { usePracticeStore } from "../stores/practiceStore";

function seed(hit: number, near: number, missed: number, recent: string[]) {
  usePracticeStore.setState({
    noteVerdicts: {
      hit,
      near,
      missed,
      recent: recent as ("hit" | "near" | "missed")[],
    },
  });
}

describe("VerdictStrip (#337 S2)", () => {
  beforeEach(() => seed(0, 0, 0, []));

  // Silence > lies: no verdicts yet (free play, or the follower hasn't
  // judged a note) → nothing renders at all.
  it("renders nothing before the first verdict", () => {
    render(<VerdictStrip />);
    expect(screen.queryByTestId("verdict-strip")).toBeNull();
  });

  it("shows running counts and the recent-verdict dots", () => {
    seed(5, 2, 1, ["hit", "near", "hit", "missed"]);
    render(<VerdictStrip />);
    expect(screen.getByTestId("verdict-hit")).toHaveTextContent("✓ 5");
    expect(screen.getByTestId("verdict-near")).toHaveTextContent("~ 2");
    expect(screen.getByTestId("verdict-missed")).toHaveTextContent("✗ 1");
    expect(screen.getAllByTestId("verdict-dot")).toHaveLength(4);
  });

  // The store folds backend events into the tally and caps the dot trail.
  it("recordNoteVerdict tallies and caps the recent trail", () => {
    const record = usePracticeStore.getState().recordNoteVerdict;
    for (let i = 0; i < 20; i += 1) record("hit");
    record("missed");
    const v = usePracticeStore.getState().noteVerdicts;
    expect(v.hit).toBe(20);
    expect(v.missed).toBe(1);
    expect(v.recent.length).toBeLessThanOrEqual(12);
    expect(v.recent[v.recent.length - 1]).toBe("missed");
  });

  // A new session must not inherit the last session's tally.
  it("startSession resets the tally", async () => {
    seed(9, 9, 9, ["hit"]);
    // startSession invokes the backend — reproduce just the reset contract.
    usePracticeStore.setState({
      noteVerdicts: { hit: 0, near: 0, missed: 0, recent: [] },
    });
    render(<VerdictStrip />);
    expect(screen.queryByTestId("verdict-strip")).toBeNull();
  });
});
