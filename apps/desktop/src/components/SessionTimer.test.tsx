import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import SessionTimer, { formatMmSs } from "./SessionTimer";
import { usePracticeStore } from "../stores/practiceStore";

describe("formatMmSs", () => {
  it.each([
    [0, "00:00"],
    [1, "00:01"],
    [59, "00:59"],
    [60, "01:00"],
    [61, "01:01"],
    [600, "10:00"],
    [3661, "61:01"], // > 1 hour — overflows minutes cleanly
  ])("formats %i as %s", (input, expected) => {
    expect(formatMmSs(input)).toBe(expected);
  });

  it("clamps negative input to 00:00", () => {
    expect(formatMmSs(-5)).toBe("00:00");
  });
});

describe("SessionTimer", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    usePracticeStore.setState({
      status: "idle",
      startedAtMs: null,
      elapsedSecs: 0,
    });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows 00:00 while idle and does not tick", () => {
    render(<SessionTimer />);
    expect(screen.getByTestId("session-timer").textContent).toBe("00:00");
    vi.advanceTimersByTime(5_000);
    expect(screen.getByTestId("session-timer").textContent).toBe("00:00");
  });

  it("ticks once per second while listening", () => {
    const start = 1_000_000;
    vi.setSystemTime(start);
    usePracticeStore.setState({
      status: "listening",
      startedAtMs: start,
      elapsedSecs: 0,
    });
    render(<SessionTimer />);
    // First tick on mount.
    expect(screen.getByTestId("session-timer").textContent).toBe("00:00");

    vi.setSystemTime(start + 1_000);
    vi.advanceTimersByTime(1_000);
    expect(screen.getByTestId("session-timer").textContent).toBe("00:01");

    vi.setSystemTime(start + 65_000);
    vi.advanceTimersByTime(64_000);
    expect(screen.getByTestId("session-timer").textContent).toBe("01:05");
  });

  it("stops ticking when status moves to ending", () => {
    const start = 2_000_000;
    vi.setSystemTime(start);
    usePracticeStore.setState({
      status: "listening",
      startedAtMs: start,
      elapsedSecs: 0,
    });
    const { rerender } = render(<SessionTimer />);

    vi.setSystemTime(start + 3_000);
    vi.advanceTimersByTime(3_000);
    expect(screen.getByTestId("session-timer").textContent).toBe("00:03");

    // Freeze: status → ending, elapsed should stop advancing.
    usePracticeStore.setState({ status: "ending" });
    rerender(<SessionTimer />);

    vi.setSystemTime(start + 20_000);
    vi.advanceTimersByTime(17_000);
    expect(screen.getByTestId("session-timer").textContent).toBe("00:03");
  });
});
