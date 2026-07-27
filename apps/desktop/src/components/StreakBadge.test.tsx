import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

/**
 * #257 AC12 — the badge shows the count lit when today's warmup is done and
 * greyed when not, and tapping "Daily warmup" invokes `start_daily_warmup`.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import StreakBadge from "./StreakBadge";
import { useWarmupStore } from "../stores/warmupStore";

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue({ count: 0, completed_today: false });
  useWarmupStore.setState({
    streak: null,
    phase: "idle",
    challenge: null,
    playedNotes: [],
    result: null,
    notice: null,
    submitting: false,
  });
});

describe("StreakBadge (#257 AC12)", () => {
  it("shows the count lit when today's warmup is done", () => {
    useWarmupStore.setState({ streak: { count: 5, completed_today: true } });
    render(<StreakBadge allowStart={true} />);
    const badge = screen.getByTestId("streak-badge");
    expect(badge).toHaveTextContent("5");
    expect(badge.className).toContain("text-amber-300");
  });

  it("renders greyed when today's warmup is still open", () => {
    useWarmupStore.setState({ streak: { count: 3, completed_today: false } });
    render(<StreakBadge allowStart={true} />);
    const badge = screen.getByTestId("streak-badge");
    expect(badge).toHaveTextContent("3");
    expect(badge.className).toContain("text-gray-500");
    expect(badge.className).not.toContain("text-amber-300");
  });

  it("fetches the streak on mount and renders no badge until it answers", async () => {
    let resolve!: (v: unknown) => void;
    mockInvoke.mockReturnValue(new Promise((r) => (resolve = r)));
    render(<StreakBadge allowStart={true} />);
    expect(mockInvoke).toHaveBeenCalledWith("get_streak", undefined);
    expect(screen.queryByTestId("streak-badge")).toBeNull();
    resolve({ count: 2, completed_today: false });
    await waitFor(() =>
      expect(screen.getByTestId("streak-badge")).toHaveTextContent("2"),
    );
  });

  it("tapping Daily warmup invokes start_daily_warmup", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "get_streak"
        ? Promise.resolve({ count: 0, completed_today: false })
        : Promise.resolve({ seed: 1, label: "C Major", target_notes: [60] }),
    );
    render(<StreakBadge allowStart={true} />);
    fireEvent.click(screen.getByTestId("daily-warmup-entry"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("start_daily_warmup", undefined),
    );
    // The throw is live — the entry hides so it can't double-throw.
    expect(screen.queryByTestId("daily-warmup-entry")).toBeNull();
  });

  it("hides the entry when starting is not allowed (a lesson owns the stage)", () => {
    useWarmupStore.setState({ streak: { count: 1, completed_today: false } });
    render(<StreakBadge allowStart={false} />);
    expect(screen.queryByTestId("daily-warmup-entry")).toBeNull();
    // The badge itself stays — the chain is always visible.
    expect(screen.getByTestId("streak-badge")).toBeInTheDocument();
  });
});
