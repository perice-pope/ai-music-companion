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
    // #388: the backend attributes by mode only — never the heard key.
    concept: "Dorian",
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
      pieceMatch: null,
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
    // #388 AC2: the headline claims the MODE and nothing else — pinned
    // exactly, so a frontend change re-deriving a note name into it
    // ("In the wild · G# Dorian") fails here even with the backend clean.
    expect(screen.getByText("In the wild · Dorian")).toBeInTheDocument();
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
    expect(screen.getByText("A cool, jazzy minor.")).toBeInTheDocument();
  });

  // #417-2a: the card is a key/mode catalog, and next to live playing a bare
  // title reads as song identification ("I played Für Elise and it said
  // Moonlight Sonata"). The framing line makes the catalog nature explicit
  // on every card. Fails if the line is dropped — which would reopen the
  // founder's wrong-Beethoven misread until item 5 ships real piece ID.
  it("frames the connection as other-music-in-this-sound, never song ID", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Beethoven — "Moonlight Sonata"')],
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-framing-r1").textContent).toBe(
      "other music that lives in this sound",
    );
  });

  // #214 S2: with a confirmed library match live, the card finally EARNS
  // identification — "You're playing — {title}" — the 2a framing retires
  // on this card only, and the catalog reframes as company the piece
  // keeps in this sound. #388: the reframe says "sound", not "key" — the
  // catalog exemplars share the identified piece's MODE, not its key, so
  // "in this key" was the same fabrication the headline made. Fails if
  // the hedge survives next to a real ID or the key claim returns.
  it("a confirmed match upgrades the card: identified, framing retired", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Beethoven — "Moonlight Sonata"')],
      pieceMatch: { scoreId: "id-9", title: "Für Elise", confirmed: true },
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-identified-r1").textContent).toBe(
      "You're playing — Für Elise",
    );
    expect(screen.queryByTestId("reveal-framing-r1")).toBeNull();
    expect(screen.getByTestId("reveal-catalog-reframe-r1").textContent).toBe(
      "also lives in this sound:",
    );
  });

  // #214 S2: the framing retires ONLY while the match is live — cleared
  // or dismissed, the 2a hedge returns verbatim. Fails if identification
  // leaves a permanent hole in the catalog voice. (Distinct title from
  // the other S2 test, so a hardcoded header can't pass both.)
  it("the framing returns verbatim when the match clears", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Beethoven — "Moonlight Sonata"')],
      pieceMatch: {
        scoreId: "id-9",
        title: "Preexisting Prelude",
        confirmed: true,
      },
    });
    render(<RevealCard />);
    expect(screen.getByTestId("reveal-identified-r1").textContent).toBe(
      "You're playing — Preexisting Prelude",
    );
    expect(screen.queryByTestId("reveal-framing-r1")).toBeNull();
    act(() => usePracticeStore.getState().dismissPieceMatch());
    expect(screen.getByTestId("reveal-framing-r1").textContent).toBe(
      "other music that lives in this sound",
    );
    expect(screen.queryByTestId("reveal-identified-r1")).toBeNull();
  });

  // #214 S2 review MF2: "You're playing —" is an ASSERTION riding a
  // rule-0 sticky match. When the live key confidently contradicts the
  // card (the same signal that dims it), the assertion must step back
  // down to the hedged framing — a dimmed card asserting a title would
  // be the wrong-Beethoven misread wearing a confident voice.
  it("a confident key contradiction demotes the assertion to the hedge", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Miles Davis — "So What"')],
      pieceMatch: { scoreId: "id-9", title: "Für Elise", confirmed: true },
      // The reveal is G Dorian (tonic 7); a confident D major reading
      // contradicts it — the card dims AND the assertion retires.
      perception: perceptionWithKey(2, "major") as never,
    });
    render(<RevealCard />);
    expect(screen.queryByTestId("reveal-identified-r1")).toBeNull();
    expect(screen.getByTestId("reveal-framing-r1").textContent).toBe(
      "other music that lives in this sound",
    );
  });

  // #214 S2b AC4: a retrieval-only match (the judge refused — e.g. a
  // half-right window) must NOT upgrade the card. The hedged 2a framing
  // stays, exactly as if there were no match. Fails if the assertion
  // ever rides an unconfirmed match again.
  it("an unconfirmed match keeps the hedged catalog voice", () => {
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Beethoven — "Moonlight Sonata"')],
      pieceMatch: { scoreId: "id-9", title: "Für Elise", confirmed: false },
    });
    render(<RevealCard />);
    expect(screen.queryByTestId("reveal-identified-r1")).toBeNull();
    expect(screen.getByTestId("reveal-framing-r1").textContent).toBe(
      "other music that lives in this sound",
    );
  });

  // #214 S2b AC7: the asserted voice is rule-0 sticky per score — a
  // later retrieval-only re-sight of the SAME score never steps the
  // card back down (only the key contradiction does, tested above).
  it("a confirmed voice holds through a retrieval-only re-sight", async () => {
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-9",
      title: "Für Elise",
      coherent_hits: 9,
      confirmed: true,
    });
    usePracticeStore.setState({
      revealQueue: [queued("r1", 'Beethoven — "Moonlight Sonata"')],
      dismissedPieceIds: [],
    });
    render(<RevealCard />);
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.getByTestId("reveal-identified-r1").textContent).toBe(
      "You're playing — Für Elise",
    );
    // The next ambient check finds the same piece at chip-tier evidence.
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-9",
      title: "Für Elise",
      coherent_hits: 7,
      confirmed: false,
    });
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.getByTestId("reveal-identified-r1").textContent).toBe(
      "You're playing — Für Elise",
    );
    // A DIFFERENT score arriving unconfirmed replaces match AND voice.
    mockInvoke.mockResolvedValueOnce({
      score_id: "id-2",
      title: "Gymnopédie No. 1",
      coherent_hits: 7,
      confirmed: false,
    });
    await act(() => usePracticeStore.getState().requestPieceMatch());
    expect(screen.queryByTestId("reveal-identified-r1")).toBeNull();
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
      usePracticeStore.setState({
        perception: perceptionWithKey(5, "phrygian"),
      });
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
      usePracticeStore.setState({
        perception: perceptionWithKey(5, "phrygian"),
      });
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
      usePracticeStore.setState({
        perception: perceptionWithKey(7, "phrygian"),
      });
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
