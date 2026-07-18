import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  act,
} from "@testing-library/react";

/**
 * #421 S1 — The Pocket control: semantic wire shape, backend-authoritative
 * status, the breathing pulse as ONE persistent element (#417 rule 0),
 * and session gating.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import PocketControl from "./PocketControl";
import { usePracticeStore } from "../stores/practiceStore";

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(undefined);
  usePracticeStore.setState({
    status: "listening",
    pocketPlaying: false,
    pocketTempo: 90,
    pocketCountIn: true,
  });
});

describe("PocketControl (#421 S1)", () => {
  it("start sends the semantic settings — tempo, meter, count-in", async () => {
    render(<PocketControl />);
    fireEvent.change(screen.getByTestId("pocket-tempo-input"), {
      target: { value: "112" },
    });
    fireEvent.click(screen.getByTestId("pocket-start"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("start_pocket", {
        tempoBpm: 112,
        beatsPerBar: 4,
        countIn: true,
      }),
    );
  });

  it("playing state is backend-authoritative and reports the CLAMPED tempo", () => {
    render(<PocketControl />);
    // The backend clamped 300 → 220 and says so; the label must repeat it.
    act(() => usePracticeStore.getState().setPocketStatus(true, 220));
    expect(screen.getByTestId("pocket-tempo").textContent).toBe("220 BPM");
    expect(screen.getByTestId("pocket-pulse")).toBeInTheDocument();
    act(() => usePracticeStore.getState().setPocketStatus(false, 0));
    expect(screen.queryByTestId("pocket-pulse")).toBeNull();
  });

  it("the pulse breathes at the beat period — one element, animation from tempo", () => {
    render(<PocketControl />);
    act(() => usePracticeStore.getState().setPocketStatus(true, 120));
    const pulse = screen.getByTestId("pocket-pulse");
    // 120 BPM → 0.5s per beat, continuous ease — never a blink.
    expect(pulse.style.animation).toContain("pocket-breathe");
    expect(pulse.style.animation).toContain("0.5s");
    expect(pulse.style.animation).toContain("ease-in-out");
    // Tempo change re-times the SAME element (rule 0: no remount).
    act(() => usePracticeStore.getState().setPocketStatus(true, 60));
    expect(screen.getByTestId("pocket-pulse")).toBe(pulse);
    expect(pulse.style.animation).toContain("1s");
  });

  it("stop sends stop_pocket", async () => {
    render(<PocketControl />);
    act(() => usePracticeStore.getState().setPocketStatus(true, 90));
    fireEvent.click(screen.getByTestId("pocket-stop"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("stop_pocket", undefined),
    );
  });

  it("controls are disabled outside a live session", () => {
    usePracticeStore.setState({ status: "idle" });
    render(<PocketControl />);
    expect(screen.getByTestId("pocket-start")).toBeDisabled();
    expect(screen.getByTestId("pocket-tempo-input")).toBeDisabled();
    expect(screen.getByTestId("pocket-count-in")).toBeDisabled();
  });

  it("a cleared tempo field settles into range on blur, never 0", () => {
    render(<PocketControl />);
    const input = screen.getByTestId("pocket-tempo-input");
    fireEvent.change(input, { target: { value: "" } });
    fireEvent.blur(input, { target: { value: "" } });
    expect(usePracticeStore.getState().pocketTempo).toBe(90);
    fireEvent.change(input, { target: { value: "500" } });
    fireEvent.blur(input, { target: { value: "500" } });
    expect(usePracticeStore.getState().pocketTempo).toBe(220);
  });
});
