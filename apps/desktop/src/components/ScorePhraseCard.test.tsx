import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import ScorePhraseCard from "./ScorePhraseCard";
import { usePracticeStore } from "../stores/practiceStore";
import type { PhraseSummary } from "../types/brain";

function phrase(overrides: Partial<PhraseSummary> = {}): PhraseSummary {
  return {
    phrase_index: 0,
    start_time: 0,
    end_time: 2,
    duration_secs: 2,
    note_count: 8,
    pitch_stats: {
      mean_hz: 440,
      min_hz: 430,
      max_hz: 450,
      pitches: [],
    },
    dynamics: {
      mean_amplitude: 0.5,
      min_amplitude: 0.2,
      max_amplitude: 0.8,
      dynamic_range: 0.6,
    },
    stability: 0.9,
    ...overrides,
  } as PhraseSummary;
}

describe("ScorePhraseCard (#337 S3, closes #210)", () => {
  beforeEach(() => usePracticeStore.setState({ phrases: [] }));

  // #210's original complaint: no per-phrase feedback in score practice.
  it("shows the latest phrase's measure-anchored card", () => {
    usePracticeStore.setState({
      phrases: [
        phrase({ phrase_index: 0, score_card: "Measure 1 — 4 clean" }),
        phrase({
          phrase_index: 1,
          score_card: "Measures 5-8 — 6 clean, 1 rough, 2 missed",
        }),
      ],
    });
    render(<ScorePhraseCard />);
    expect(screen.getByTestId("score-phrase-card")).toHaveTextContent(
      "Measures 5-8 — 6 clean, 1 rough, 2 missed",
    );
  });

  // Free play: phrases carry no card → the component renders nothing.
  it("renders nothing when no phrase carries a card", () => {
    usePracticeStore.setState({ phrases: [phrase()] });
    render(<ScorePhraseCard />);
    expect(screen.queryByTestId("score-phrase-card")).toBeNull();
  });

  // A cardless trailing phrase must not hide the last real card.
  it("falls back to the most recent phrase WITH a card", () => {
    usePracticeStore.setState({
      phrases: [
        phrase({ phrase_index: 0, score_card: "Measure 2 — 3 clean" }),
        phrase({ phrase_index: 1 }),
      ],
    });
    render(<ScorePhraseCard />);
    expect(screen.getByTestId("score-phrase-card")).toHaveTextContent(
      "Measure 2 — 3 clean",
    );
  });
});
