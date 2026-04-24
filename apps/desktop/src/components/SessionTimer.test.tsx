import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
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

  it("ticks once per second while listening", async () => {
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

    // Under React 18, store updates from interval callbacks aren't
    // flushed synchronously by `advanceTimersByTime`. The async variant
    // inside `await act(async ...)` yields control between the fake
    // timer firing and the assertion so React can commit. Note that
    // `advanceTimersByTimeAsync(N)` also moves the system clock forward
    // by N, so we do NOT also call `setSystemTime` — doing both would
    // double-advance the clock.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(screen.getByTestId("session-timer").textContent).toBe("00:01");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(64_000);
    });
    expect(screen.getByTestId("session-timer").textContent).toBe("01:05");
  });

  it("stops ticking when status moves to ending", async () => {
    const start = 2_000_000;
    vi.setSystemTime(start);
    usePracticeStore.setState({
      status: "listening",
      startedAtMs: start,
      elapsedSecs: 0,
    });
    const { rerender } = render(<SessionTimer />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_000);
    });
    expect(screen.getByTestId("session-timer").textContent).toBe("00:03");

    // Freeze: status → ending, elapsed should stop advancing.
    act(() => {
      usePracticeStore.setState({ status: "ending" });
    });
    rerender(<SessionTimer />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(17_000);
    });
    expect(screen.getByTestId("session-timer").textContent).toBe("00:03");
  });
});
