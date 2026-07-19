import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import AccompanimentToggle from "./AccompanimentToggle";
import { usePracticeStore } from "../stores/practiceStore";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("AccompanimentToggle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue(undefined);
    usePracticeStore.setState({
      status: "listening",
      accompanimentPlaying: false,
    });
  });

  it("is disabled when no session is listening (nothing to follow)", () => {
    usePracticeStore.setState({ status: "idle", accompanimentPlaying: false });
    render(<AccompanimentToggle />);
    expect(screen.getByTestId("accompaniment-toggle")).toBeDisabled();
  });

  it("starts the band via start_accompaniment when clicked during a session", async () => {
    render(<AccompanimentToggle />);
    const btn = screen.getByTestId("accompaniment-toggle");
    expect(btn).toHaveTextContent("Play with me");
    expect(btn).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(btn);

    // #445 pt 9: the band carries the Pocket's clock — the start payload is
    // the set tempo (the exact number is the store's contract, pinned in
    // practiceStore.test.ts).
    await vi.waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("start_accompaniment", {
        tempoBpm: expect.any(Number),
      }),
    );
    expect(mockInvoke).not.toHaveBeenCalledWith("stop_accompaniment");
  });

  it("shows the playing chip and stops the band via stop_accompaniment when playing", async () => {
    usePracticeStore.setState({
      status: "listening",
      accompanimentPlaying: true,
    });
    render(<AccompanimentToggle />);

    // Chip reflects the authoritative backend state.
    screen.getByTestId("accompaniment-status-chip");
    const btn = screen.getByTestId("accompaniment-toggle");
    expect(btn).toHaveTextContent("Stop band");
    expect(btn).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(btn);

    await vi.waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("stop_accompaniment"),
    );
    expect(mockInvoke).not.toHaveBeenCalledWith("start_accompaniment");
  });

  it("hides the chip while the band is not playing", () => {
    render(<AccompanimentToggle />);
    expect(screen.queryByTestId("accompaniment-status-chip")).toBeNull();
  });

  it("surfaces a calm error if starting the band fails", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("no output device xyz"));
    render(<AccompanimentToggle />);
    fireEvent.click(screen.getByTestId("accompaniment-toggle"));
    // The live session must not break — a friendly message shows inline (the
    // raw error goes to the console, not the UI).
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Couldn't start the band/i);
    expect(alert).not.toHaveTextContent("xyz");
  });

  it("ignores a second click while the start command is in flight", async () => {
    // Guards against double-starting two bands: while the command is pending the
    // button is disabled, so a quick double-click invokes start exactly once.
    let resolveStart: (() => void) | undefined;
    mockInvoke.mockImplementation(
      () =>
        new Promise<void>((res) => {
          resolveStart = () => res();
        }),
    );
    render(<AccompanimentToggle />);
    const btn = screen.getByTestId("accompaniment-toggle");

    fireEvent.click(btn);
    await vi.waitFor(() => expect(btn).toBeDisabled());
    fireEvent.click(btn); // ignored — button is disabled mid-flight

    resolveStart?.();
    await vi.waitFor(() => expect(btn).not.toBeDisabled());

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("start_accompaniment", {
      tempoBpm: expect.any(Number),
    });
  });
});
