import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

import RevealCard from "./RevealCard";
import { usePracticeStore } from "../stores/practiceStore";
import type { Reveal } from "../types/brain";

function reveal(connection: string): Reveal {
  return {
    concept: "G Dorian",
    connection,
    why: "A cool, jazzy minor.",
    source: "grounded",
    tonic: 7,
    mode: "dorian",
  };
}

// A perception snapshot carrying just the key the card compares against.
function perceptionWithKey(tonic: number, mode: string) {
  return {
    tempo_bpm: null,
    swing_ratio: null,
    locked: false,
    key: {
      tonic,
      mode,
      name: `${tonic} ${mode}`,
      confidence: 0.9,
      alternative: null,
    },
  };
}

function queued(id: string, connection: string) {
  return {
    id,
    reveal: reveal(connection),
    receivedAt: Date.now(),
    phraseIndex: 0,
  };
}

describe("RevealCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    usePracticeStore.setState({
      revealQueue: [],
      perception: null,
      collectionCount: null,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // AC7: empty state when nothing is queued (so the layout stays stable and no
  // stale reveal lingers). Fails if the card rendered with no data.
  it("renders the empty state when no reveal is queued", () => {
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-card-empty")).toBeInTheDocument();
    expect(screen.queryByTestId("reveal-card")).not.toBeInTheDocument();
  });

  // AC7: a queued reveal renders its concept, connection, and why. Fails if any
  // field is dropped from the card.
  it("renders the concept, connection, and why for a queued reveal", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')],
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();
    // Assert the concept shows; don't pin the decorative "In the wild ·" prefix.
    expect(screen.getByText(/G Dorian/)).toBeInTheDocument();
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
    expect(screen.getByText("A cool, jazzy minor.")).toBeInTheDocument();
  });

  // AC7 (render contract): with multiple in the queue only the latest renders.
  it("shows only the most recent reveal, never stacking", () => {
    usePracticeStore.setState({
      revealQueue: [
        queued("r1", "Santana — Oye Como Va"),
        queued("r2", 'Miles Davis — "So What"'),
      ],
    });
    render(<RevealCard />);
    expect(screen.getAllByTestId(/^reveal-r/)).toHaveLength(1);
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
    expect(screen.queryByText("Santana — Oye Como Va")).not.toBeInTheDocument();
  });

  // AC7 (the real push→supersede→dismiss flow): a reveal that arrives while an
  // earlier one is still showing replaces it, and after the newer one's linger
  // the card is empty — the older reveal must NOT resurface. This fails under an
  // appending `pushReveal` (the stale r1 would slide back on and the queue would
  // not be empty).
  it("a newer reveal replaces the older one — the old never resurfaces", () => {
    render(<RevealCard />);
    act(() => {
      usePracticeStore
        .getState()
        .pushReveal(reveal("Santana — Oye Como Va"), 0);
    });
    expect(screen.getByText("Santana — Oye Como Va")).toBeInTheDocument();

    // r2 supersedes r1 partway through r1's linger window.
    act(() => {
      vi.advanceTimersByTime(4000);
      usePracticeStore
        .getState()
        .pushReveal(reveal('Miles Davis — "So What"'), 1);
    });
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
    expect(screen.queryByText("Santana — Oye Como Va")).not.toBeInTheDocument();

    // After the newer reveal's full linger + fade, the queue is empty and
    // nothing slides back on.
    act(() => {
      vi.advanceTimersByTime(12000);
      vi.advanceTimersByTime(300);
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(0);
    expect(screen.queryByTestId("reveal-card")).not.toBeInTheDocument();
  });

  // The card auto-dismisses after its linger window, clearing the queue. Fails
  // if reveals pile up forever (the panel would never return to calm).
  it("auto-dismisses after its linger window", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", "B.B. King & blues-rock solos")],
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();

    vi.advanceTimersByTime(12000); // linger
    vi.advanceTimersByTime(300); // fade-out

    expect(usePracticeStore.getState().revealQueue).toHaveLength(0);
  });

  // #266 AC3 + #277: once the live key CONFIDENTLY moves off the reveal's
  // (tonic, mode), the card is dismissed — but never before its minimum
  // readable dwell, so it can actually be read. Fails if a young card is
  // killed instantly or a stale one lingers past the dwell.
  it("dismisses the card when the live key moves off it — after the readable dwell", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();

    // Live detection confidently moves to a different key/mode while the card
    // is brand new → it must stay readable, not vanish (#277).
    act(() => {
      usePracticeStore.setState({
        perception: perceptionWithKey(5, "phrygian"),
      });
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(1);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();

    // After the readable dwell the contradiction wins.
    act(() => {
      vi.advanceTimersByTime(4000);
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(0);
    expect(screen.queryByTestId("reveal-card")).not.toBeInTheDocument();
  });

  // #277: a confident contradiction arriving MID-dwell dismisses exactly when
  // the readable dwell completes — not immediately, and not a full extra dwell
  // later. Fails if the remaining-time math regresses to a fixed delay.
  it("a mid-age contradiction dismisses when the dwell completes", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')],
    });
    render(<RevealCard />);

    // 2.5s into the card's life, a confident different key arrives.
    act(() => {
      vi.advanceTimersByTime(2500);
      usePracticeStore.setState({
        perception: perceptionWithKey(5, "phrygian"),
      });
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(1);

    // 1.5s later the 4s dwell completes → dismissed (a fixed 4s timer would
    // still be pending here).
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(0);
  });

  // #277: a LOW-CONFIDENCE wander is not a contradiction — the card survives
  // its full linger. Fails if wobbly detection can still evaporate cards.
  it("a low-confidence key wander never dismisses the card", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')],
    });
    render(<RevealCard />);
    act(() => {
      usePracticeStore.setState({
        perception: {
          tempo_bpm: null,
          swing_ratio: null,
          locked: false,
          key: {
            tonic: 5,
            mode: "phrygian",
            name: "F Phrygian",
            confidence: 0.4, // wobble, below the dismissal bar
            alternative: null,
          },
        },
      });
      vi.advanceTimersByTime(5000);
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(1);
  });

  // #266 AC3 (isolate the OR): only the MODE moves (same tonic) → dismiss. Fails
  // if the dismiss condition were `&&` instead of `||`.
  it("dismisses when only the mode moves off (same tonic)", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    act(() => {
      usePracticeStore.setState({
        perception: perceptionWithKey(7, "phrygian"),
      });
      vi.advanceTimersByTime(4000); // readable dwell
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(0);
  });

  // #266 AC3 (isolate the OR): only the TONIC moves (same mode) → dismiss. The
  // other half of the `||`.
  it("dismisses when only the tonic moves off (same mode)", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(5, "dorian") });
      vi.advanceTimersByTime(4000); // readable dwell
    });
    expect(usePracticeStore.getState().revealQueue).toHaveLength(0);
  });

  // #255: the reveal is actionable — "Practice this sound" starts an
  // exploration seeded from the reveal's own key/mode.
  it("Practice this sound starts an exploration from the reveal's key", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
      status: "listening",
    });
    mockInvoke.mockResolvedValueOnce({
      label: "x",
      music_xml: "<score-partwise/>",
      chips: [],
      root_pitch_classes: [7],
    });
    render(<RevealCard />);
    fireEvent.click(screen.getByTestId("reveal-practice-this"));
    expect(mockInvoke).toHaveBeenCalledWith("start_explore_variation", {
      tonic: 7,
      mode: "dorian",
    });
  });

  // #253 S3: the little collection counter renders once a count is known, and
  // stays hidden before the first unlock. Fails if the count wiring is dropped.
  it("shows the collection count once known", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')],
    });
    render(<RevealCard />);
    expect(
      screen.queryByTestId("reveal-collection-count"),
    ).not.toBeInTheDocument();

    act(() => {
      usePracticeStore.setState({ collectionCount: 4 });
    });
    expect(screen.getByTestId("reveal-collection-count")).toHaveTextContent(
      "4 in your collection",
    );
  });

  // #266 AC4: the card is NOT dismissed while the live key still matches (even
  // with different casing) or is momentarily null (silence). Fails if a benign
  // update wrongly clears the card.
  it("keeps the card when the live key is unchanged or silent", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);

    // Same key, different casing → not a change.
    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(7, "Dorian") });
    });
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();

    // Player pauses → key goes null → don't punish silence.
    act(() => {
      usePracticeStore.setState({
        perception: {
          tempo_bpm: null,
          swing_ratio: null,
          locked: false,
          key: null,
        },
      });
    });
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();
  });
});
