import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import ChordLane from "./ChordLane";
import { usePracticeStore } from "../stores/practiceStore";
import type { PerceptionSnapshot } from "../types/brain";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

/** A perception snapshot showing one labeled chord. */
function hearing(
  label: string,
  rootPc: number,
  quality: string,
  confidence = 0.8,
): PerceptionSnapshot {
  return {
    tempo_bpm: null,
    swing_ratio: null,
    locked: false,
    key: null,
    chord: { root_pc: rootPc, quality, label, bass_pc: null, confidence },
    hearing_polyphony: false,
  };
}

const SEVERAL: PerceptionSnapshot = {
  tempo_bpm: null,
  swing_ratio: null,
  locked: false,
  key: null,
  chord: null,
  hearing_polyphony: true,
};

const SILENCE: PerceptionSnapshot = {
  tempo_bpm: null,
  swing_ratio: null,
  locked: false,
  key: null,
  chord: null,
  hearing_polyphony: false,
};

describe("ChordLane (#349 T4a)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue({});
    usePracticeStore.setState({
      status: "listening",
      listenToRoom: true,
      chordLane: [],
      explore: null,
      exploreNotice: null,
    });
  });

  // The lane accumulates from perception CHANGES: a ringing chord lands
  // once, a new chord appends, an unresolved stretch is one honest entry.
  // Fails if the lane spams per-snapshot or fabricates labels.
  it("accumulates chord changes, not frames — with honest unresolved entries", () => {
    const set = usePracticeStore.getState().setPerception;
    for (let i = 0; i < 5; i++) set(hearing("Cmaj7", 0, "maj7"));
    set(hearing("F", 5, "maj"));
    for (let i = 0; i < 3; i++) set(SEVERAL);
    render(<ChordLane />);
    const chords = screen.getAllByTestId("lane-chord");
    expect(chords).toHaveLength(2);
    expect(chords[0]).toHaveTextContent("Cmaj7");
    expect(chords[1]).toHaveTextContent("F");
    expect(screen.getAllByTestId("lane-unresolved")).toHaveLength(1);
    expect(screen.getByTestId("lane-unresolved")).toHaveTextContent(
      "several notes",
    );
  });

  // AC1's "tapping one rows it": a tap invokes the bridge with the chord's
  // root and quality. Unresolved chips are not tappable.
  it("taps a chord into the 12-key bridge", () => {
    const set = usePracticeStore.getState().setPerception;
    set(hearing("Bb7", 10, "dom7"));
    render(<ChordLane />);
    fireEvent.click(screen.getByTestId("lane-chord"));
    expect(mockInvoke).toHaveBeenCalledWith("explore_chord", {
      rootPc: 10,
      quality: "dom7",
    });
  });

  // The lane rolls: only the last 8 entries stay.
  it("keeps a rolling window of eight", () => {
    const set = usePracticeStore.getState().setPerception;
    const roots = [0, 2, 4, 5, 7, 9, 11, 1, 3, 6];
    for (const r of roots) {
      set(hearing(`chord-${r}`, r, "maj"));
    }
    render(<ChordLane />);
    const chords = screen.getAllByTestId("lane-chord");
    expect(chords).toHaveLength(8);
    expect(chords[0]).toHaveTextContent("chord-4", { normalizeWhitespace: true });
  });

  // Silence between two strikes of the SAME chord re-records it (the lane
  // is a timeline, not a set) — and the privacy copy is always present.
  it("re-records a chord after silence and states the privacy line", () => {
    const set = usePracticeStore.getState().setPerception;
    set(hearing("C", 0, "maj"));
    set(SILENCE);
    set(hearing("C", 0, "maj"));
    render(<ChordLane />);
    expect(screen.getAllByTestId("lane-chord")).toHaveLength(2);
    expect(screen.getByTestId("lane-privacy")).toHaveTextContent(
      "nothing is recorded",
    );
  });

  // Lane accumulation only runs in room mode — normal sessions must not
  // grow a hidden lane.
  it("does not accumulate when the room mode is off", () => {
    usePracticeStore.setState({ listenToRoom: false });
    usePracticeStore.getState().setPerception(hearing("C", 0, "maj"));
    expect(usePracticeStore.getState().chordLane).toHaveLength(0);
  });

  // Honesty is not tappable: an unresolved chip has nothing to row —
  // clicking it must never invoke the bridge. Fails if unresolved entries
  // become buttons (test-auditor probe: this mutation survived before).
  it("unresolved chips are not tappable", () => {
    usePracticeStore.getState().setPerception(SEVERAL);
    render(<ChordLane />);
    fireEvent.click(screen.getByTestId("lane-unresolved"));
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  // The confidence dots ARE the honesty cue (AC2 "confidence dots
  // pinned"): a hesitant label shows fewer dots. Fails if dotCount stops
  // reflecting confidence (that mutation survived before this test).
  it("draws dots that reflect the label's confidence", () => {
    const set = usePracticeStore.getState().setPerception;
    set(hearing("C", 0, "maj", 0.9)); // 3 dots
    set(hearing("F", 5, "maj", 0.6)); // 2 dots
    set(hearing("G", 7, "maj", 0.4)); // 1 dot
    render(<ChordLane />);
    const dots = screen
      .getAllByTestId("lane-confidence")
      .map((d) => d.textContent);
    expect(dots).toEqual(["●●●", "●●", "●"]);
  });

  // A slash arriving mid-ring ("C" → "C/E") refreshes the chip in place —
  // same (root, quality) identity rule as the backend recorder, so the
  // lane and the recap chart can never disagree (test-auditor probe).
  it("refreshes a slash change in place, never a duplicate chip", () => {
    const set = usePracticeStore.getState().setPerception;
    set(hearing("C", 0, "maj", 0.8));
    set(hearing("C/E", 0, "maj", 0.8));
    render(<ChordLane />);
    const chords = screen.getAllByTestId("lane-chord");
    expect(chords).toHaveLength(1);
    expect(chords[0]).toHaveTextContent("C/E");
  });
});
