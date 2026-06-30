import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import RevealCard from "./RevealCard";
import { usePracticeStore } from "../stores/practiceStore";
import type { Reveal } from "../types/brain";

function reveal(connection: string): Reveal {
  return {
    concept: "G Dorian",
    connection,
    why: "A cool, jazzy minor.",
    source: "grounded",
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
    usePracticeStore.setState({ revealQueue: [] });
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
    expect(screen.getByText(/In the wild · G Dorian/)).toBeInTheDocument();
    expect(screen.getByText('Miles Davis — "So What"')).toBeInTheDocument();
    expect(screen.getByText("A cool, jazzy minor.")).toBeInTheDocument();
  });

  // AC7: a second reveal replaces the first (the card never stacks). Fails if
  // an old reveal stays on screen alongside a new one.
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
});
