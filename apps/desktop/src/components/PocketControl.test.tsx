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
    pocketMode: "anchor",
    pocketFrozenBpm: null,
    _pocketLastSentBpm: null,
    _pocketLastSentAt: 0,
    _pocketFollowStartedAt: 0,
    perception: null,
  });
});

const perceptionWithTempo = (bpm: number | null) =>
  ({
    tempo_bpm: bpm,
    swing_ratio: null,
    locked: bpm !== null,
    key: null,
    chord: null,
    hearing_polyphony: false,
  }) as never;

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

  // ── #421 S2: Follow / Handoff ─────────────────────────────────────────

  it("mode chips are exclusive, anchor default, disabled off-session", () => {
    render(<PocketControl />);
    expect(
      screen.getByTestId("pocket-mode-anchor").getAttribute("aria-pressed"),
    ).toBe("true");
    fireEvent.click(screen.getByTestId("pocket-mode-follow"));
    expect(
      screen.getByTestId("pocket-mode-follow").getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen.getByTestId("pocket-mode-anchor").getAttribute("aria-pressed"),
    ).toBe("false");
    act(() => usePracticeStore.setState({ status: "idle" }));
    expect(screen.getByTestId("pocket-mode-handoff")).toBeDisabled();
  });

  it("follow sends confident, changed, throttled readings only", () => {
    vi.useFakeTimers();
    vi.setSystemTime(100_000);
    usePracticeStore.setState({ pocketPlaying: true, pocketMode: "follow" });
    const send = (bpm: number | null) =>
      usePracticeStore.getState().setPerception(perceptionWithTempo(bpm));
    send(96);
    expect(mockInvoke).toHaveBeenCalledWith("set_pocket_tempo", {
      tempoBpm: 96,
    });
    // Within the throttle window: nothing, even on a big change.
    send(120);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    // Past the throttle but under the 2-BPM delta: nothing.
    vi.setSystemTime(101_500);
    send(97);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    // Past both gates: sends.
    send(104);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
    // Unconfident/absent tempo: nothing. Out-of-range: nothing.
    vi.setSystemTime(103_000);
    send(null);
    send(500);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
    // Anchor mode: the stream stops (AC5 mid-play switch).
    usePracticeStore.getState().setPocketMode("anchor");
    vi.setSystemTime(105_000);
    send(150);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });

  it("handoff follows, then freezes and the drift line reads honestly", () => {
    vi.useFakeTimers();
    vi.setSystemTime(200_000);
    render(<PocketControl />);
    act(() => {
      usePracticeStore.setState({ pocketPlaying: true });
      usePracticeStore.getState().setPocketMode("handoff");
    });
    const send = (bpm: number) =>
      act(() =>
        usePracticeStore.getState().setPerception(perceptionWithTempo(bpm)),
      );
    send(96); // follows during the window
    expect(mockInvoke).toHaveBeenCalledWith("set_pocket_tempo", {
      tempoBpm: 96,
    });
    // The window closes: the next reading freezes instead of sending.
    vi.setSystemTime(209_000);
    send(98);
    expect(usePracticeStore.getState().pocketFrozenBpm).toBe(96);
    const drift = screen.getByTestId("pocket-drift");
    expect(drift.textContent).toContain("holding your 96");
    // Live drifts ahead: the line updates IN PLACE (rule 0 identity).
    send(101);
    expect(screen.getByTestId("pocket-drift")).toBe(drift);
    expect(drift.textContent).toContain("+5 BPM ahead");
    // Back in the pocket.
    send(97);
    expect(drift.textContent).toContain("in the pocket");
    // Frozen: no further set_pocket_tempo sends.
    expect(
      mockInvoke.mock.calls.filter(([c]) => c === "set_pocket_tempo").length,
    ).toBe(1);
    vi.useRealTimers();
  });
});
