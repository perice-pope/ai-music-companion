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
    key: { tonic, mode, name: `${tonic} ${mode}`, confidence: 0.9, alternative: null },
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
      usePracticeStore.getState().pushReveal(reveal("Santana — Oye Como Va"), 0);
    });
    expect(screen.getByText("Santana — Oye Como Va")).toBeInTheDocument();

    // r2 supersedes r1 partway through r1's linger window.
    act(() => {
      vi.advanceTimersByTime(4000);
      usePracticeStore.getState().pushReveal(reveal('Miles Davis — "So What"'), 1);
    });
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
    expect(screen.queryByText("Santana — Oye Como Va")).not.toBeInTheDocument();

    // #417 rule 0: nothing auto-clears — the newest card STAYS until
    // dismissed or replaced ("until it has something better to say").
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
  });

  // #417 rule 0 (founder, 2026-07-18): the card NEVER auto-dismisses —
  // "quick blinking … these things need to be static." It stays until the
  // player dismisses it or a better reveal replaces it. Fails if a linger
  // timer sneaks back in.
  it("never auto-dismisses — it stays until dismissed or replaced", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", "B.B. King & blues-rock solos")],
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();

    vi.advanceTimersByTime(120_000); // two full minutes of nothing

    expect(usePracticeStore.getState().revealQueue).toHaveLength(1);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();
  });

  // #417 rewrite of #266/#277: a CONFIDENT contradiction DIMS the card in
  // place (honest staleness) — it never removes it. Fails if dismissal
  // returns, or if the dim never engages.
  it("a confident key contradiction dims the card in place", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-100");

    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(5, "phrygian") });
    });
    // Still present, still in the queue — but visibly stale.
    expect(usePracticeStore.getState().revealQueue).toHaveLength(1);
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-40");

    // And it STAYS (dimmed) indefinitely — static, not blinking.
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();
  });

  // The dim heals: if the live key comes back to the card's key, full
  // opacity returns — the card was never stale after all.
  it("the dim heals when the live key returns to the card's key", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(5, "phrygian") });
    });
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-40");
    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(7, "dorian") });
    });
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-100");
  });

  // #277: a LOW-CONFIDENCE wander is not a contradiction — the card stays
  // at FULL opacity. Fails if wobbly detection can dim (or remove) cards.
  it("a low-confidence key wander never dims the card", () => {
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
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-100");
  });

  // Isolate the staleness OR: only the MODE moves (same tonic) → dims.
  // Fails if the condition regresses to `&&`.
  it("dims when only the mode moves off (same tonic)", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(7, "phrygian") });
    });
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-40");
  });

  // The other half of the `||`: only the TONIC moves (same mode) → dims.
  it("dims when only the tonic moves off (same mode)", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')], // G(7) dorian
    });
    render(<RevealCard />);
    act(() => {
      usePracticeStore.setState({ perception: perceptionWithKey(5, "dorian") });
    });
    expect(screen.getByTestId("reveal-r1").className).toContain("opacity-40");
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
        perception: { tempo_bpm: null, swing_ratio: null, locked: false, key: null },
      });
    });
    expect(screen.getByTestId("reveal-card")).toBeInTheDocument();
  });
});
