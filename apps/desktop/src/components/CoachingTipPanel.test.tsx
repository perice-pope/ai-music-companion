import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CoachingTipPanel from "./CoachingTipPanel";
import { usePracticeStore } from "../stores/practiceStore";
import type { CoachingTip } from "../types/brain";

describe("CoachingTipPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    usePracticeStore.setState({
      tipQueue: [],
      screen: "session",
      status: "listening",
      sessionId: "test-session",
      instrumentName: "Trumpet",
      segmentId: null,
      startedAtMs: Date.now(),
      elapsedSecs: 0,
      phrases: [],
      recap: null,
      recapError: null,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders empty state when no tips are queued", () => {
    render(<CoachingTipPanel />);
    expect(screen.getByTestId("coaching-tip-panel-empty")).toBeInTheDocument();
  });

  it("renders the most recent tip when tips are queued", () => {
    const tip: CoachingTip = {
      text: "Try a warmer tone in the high register",
      severity: "suggestion",
      category: "tone",
    };

    usePracticeStore.setState({
      tipQueue: [{ id: "tip-1", tip, receivedAt: Date.now(), phraseIndex: 0 }],
    });

    render(<CoachingTipPanel />);
    const tipText = screen.getByText("Try a warmer tone in the high register");
    expect(tipText).toBeInTheDocument();
    expect(tipText.textContent).toBe("Try a warmer tone in the high register");
    const tipCard = screen.getByTestId("coaching-tip-tip-1");
    expect(tipCard).toBeInTheDocument();
  });

  it("displays severity and category badges", () => {
    const tip: CoachingTip = {
      text: "Focus on steady rhythm",
      severity: "focus",
      category: "rhythm",
    };

    usePracticeStore.setState({
      tipQueue: [{ id: "tip-1", tip, receivedAt: Date.now(), phraseIndex: 0 }],
    });

    render(<CoachingTipPanel />);
    const severityBadge = screen.getByText("focus");
    expect(severityBadge).toBeInTheDocument();
    expect(severityBadge.className).toContain("bg-yellow-900");
    const categoryBadge = screen.getByText("rhythm");
    expect(categoryBadge).toBeInTheDocument();
    expect(categoryBadge.className).toContain("indigo");
  });

  it("shows only the most recent tip when multiple are queued", () => {
    const tip1: CoachingTip = {
      text: "First tip",
      severity: "encouragement",
      category: "tone",
    };
    const tip2: CoachingTip = {
      text: "Second tip",
      severity: "suggestion",
      category: "rhythm",
    };

    usePracticeStore.setState({
      tipQueue: [
        { id: "tip-1", tip: tip1, receivedAt: Date.now(), phraseIndex: 0 },
        {
          id: "tip-2",
          tip: tip2,
          receivedAt: Date.now() + 100,
          phraseIndex: 1,
        },
      ],
    });

    render(<CoachingTipPanel />);
    expect(screen.getByText("Second tip")).toBeInTheDocument();
    expect(screen.queryByText("First tip")).not.toBeInTheDocument();
  });

  it("allows manual dismissal via close button", () => {
    const tip: CoachingTip = {
      text: "Keep your shoulders relaxed",
      severity: "suggestion",
      category: "technique",
    };

    usePracticeStore.setState({
      tipQueue: [{ id: "tip-1", tip, receivedAt: Date.now(), phraseIndex: 0 }],
    });

    render(<CoachingTipPanel />);
    const dismissBtn = screen.getByTestId("tip-dismiss-tip-1");
    fireEvent.click(dismissBtn);

    // Fast-forward animation
    vi.advanceTimersByTime(300);

    // Tip should be removed from store
    expect(usePracticeStore.getState().tipQueue).toHaveLength(0);
  });

  it("auto-dismisses after 10 seconds", () => {
    const tip: CoachingTip = {
      text: "Great intonation on that phrase!",
      severity: "encouragement",
      category: "intonation",
    };

    usePracticeStore.setState({
      tipQueue: [{ id: "tip-1", tip, receivedAt: Date.now(), phraseIndex: 0 }],
    });

    render(<CoachingTipPanel />);
    expect(
      screen.getByText("Great intonation on that phrase!")
    ).toBeInTheDocument();

    // Fast-forward 10 seconds
    vi.advanceTimersByTime(10000);
    // Plus animation time
    vi.advanceTimersByTime(300);

    // Tip should be removed from store
    expect(usePracticeStore.getState().tipQueue).toHaveLength(0);
  });

  it("renders with appropriate accessibility attributes", () => {
    const tip: CoachingTip = {
      text: "Smooth air support",
      severity: "suggestion",
      category: "technique",
    };

    usePracticeStore.setState({
      tipQueue: [{ id: "tip-1", tip, receivedAt: Date.now(), phraseIndex: 0 }],
    });

    render(<CoachingTipPanel />);
    const panel = screen.getByTestId("coaching-tip-panel");
    expect(panel).toHaveAttribute("role", "status");
    expect(panel).toHaveAttribute("aria-live", "polite");
    expect(panel).toHaveAttribute("aria-label", "Coaching tip panel");
  });

  it("handles all severity types correctly", () => {
    const severities: Array<CoachingTip["severity"]> = [
      "encouragement",
      "suggestion",
      "focus",
    ];

    severities.forEach((severity, idx) => {
      usePracticeStore.setState({
        tipQueue: [
          {
            id: `tip-${idx}`,
            tip: {
              text: `Tip with ${severity} severity`,
              severity,
              category: "tone",
            },
            receivedAt: Date.now(),
            phraseIndex: 0,
          },
        ],
      });

      const { unmount } = render(<CoachingTipPanel />);
      const severityBadge = screen.getByText(severity);
      expect(severityBadge).toBeInTheDocument();
      expect(severityBadge.className).toContain("uppercase");
      unmount();
    });
  });

  it("handles all category types correctly", () => {
    const categories: Array<CoachingTip["category"]> = [
      "tone",
      "intonation",
      "rhythm",
      "dynamics",
      "expression",
      "technique",
    ];

    categories.forEach((category, idx) => {
      usePracticeStore.setState({
        tipQueue: [
          {
            id: `tip-${idx}`,
            tip: {
              text: `Tip for ${category}`,
              severity: "suggestion",
              category,
            },
            receivedAt: Date.now(),
            phraseIndex: 0,
          },
        ],
      });

      const { unmount } = render(<CoachingTipPanel />);
      const categoryBadge = screen.getByText(category);
      expect(categoryBadge).toBeInTheDocument();
      expect(categoryBadge.className).toContain("uppercase");
      unmount();
    });
  });
});
