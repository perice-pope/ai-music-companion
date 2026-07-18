import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  act,
} from "@testing-library/react";
import OpenersPanel from "./OpenersPanel";
import { usePracticeStore } from "../stores/practiceStore";

/**
 * #419 S1 — the Openers builder: bank taps compose items, the preview is
 * PURE (preview_opener), Begin commits (begin_opener) and hands the dto to
 * the explore surface, and refusals surface calmly. The mock invoke is the
 * wire contract: items go over exactly as tagged StarterItem JSON.
 */

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

const PREVIEW_DTO = {
  label: "your opener · 12 keys",
  music_xml: "<score/>",
  chips: [],
  root_pitch_classes: [0],
  root_names: ["C"],
  staff: { fifths: 0, beats_per_measure: 4, total_beats: 4, notes: [] },
};

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(PREVIEW_DTO);
  usePracticeStore.setState({
    openerItems: [],
    openerPreview: null,
    openerNotice: null,
    // S2b state resets too — direction and captured tonic leak across
    // tests otherwise (and did, in review).
    openerTonic: null,
    openerDirection: "forward",
    perception: null,
    explore: null,
    // Begin guards on a live session, like every sibling explore action.
    status: "listening",
  });
});

describe("OpenersPanel (#419 S1)", () => {
  it("opens from the invitation and shows the two live bank rows", () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    screen.getByText("Notes");
    screen.getByText("Note sequence");
    // S2a/S2b made the bank live; only My patterns still rests.
    screen.getByText("Enclosures");
    screen.getByText("Pattern direction");
    screen.getByText("My patterns");
  });

  it("a note tap adds an item and requests a PURE preview", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-note-3"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("preview_opener", {
        // SEMANTIC wire shape — the degree→semitone table lives in Rust
        // only (review MF2). A tap is a degree, never an offset.
        items: [{ type: "note_sequence", degrees: [3] }],
        tonic: null,
        direction: "forward",
      }),
    );
    // Never begin_opener from a preview — a preview that hijacks the
    // session on every tap is the bug this pins against.
    expect(mockInvoke.mock.calls.every(([cmd]) => cmd !== "begin_opener")).toBe(
      true,
    );
    await waitFor(() => screen.getByTestId("opener-preview"));
    screen.getByTestId("opener-chip-0");
  });

  it("a sequence preset adds the classic opener as ONE item", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-seq-1-2-3-5"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
        tonic: null,
        direction: "forward",
      }),
    );
    screen.getByTestId("opener-chip-0");
  });

  it("tapping a chip removes its item and refreshes the preview", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-note-1"));
    fireEvent.click(screen.getByTestId("opener-seq-1-3-5-8"));
    await waitFor(() => screen.getByTestId("opener-chip-1"));
    fireEvent.click(screen.getByTestId("opener-chip-0"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1, 3, 5, 8] }],
        tonic: null,
        direction: "forward",
      }),
    );
  });

  it("Begin is disabled while empty, then commits and hands off to explore", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    expect(screen.getByTestId("opener-begin")).toBeDisabled();

    fireEvent.click(screen.getByTestId("opener-seq-1-2-3-5"));
    await waitFor(() =>
      expect(screen.getByTestId("opener-begin")).not.toBeDisabled(),
    );
    fireEvent.click(screen.getByTestId("opener-begin"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("begin_opener", {
        items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
        tonic: null,
        direction: "forward",
      }),
    );
    // The dto lands on the SAME explore surface a lifted lick uses, and
    // the builder resets for next time.
    await waitFor(() =>
      expect(usePracticeStore.getState().explore?.label).toBe(
        "your opener · 12 keys",
      ),
    );
    expect(usePracticeStore.getState().openerItems).toHaveLength(0);
  });

  it("a late preview response for removed items is dropped, not painted", async () => {
    // Review MF3, reproduced at HEAD: tap → remove-while-in-flight → the
    // late response used to paint an orphan preview over an empty builder.
    let resolveLate: (dto: typeof PREVIEW_DTO) => void = () => {};
    mockInvoke.mockImplementationOnce(
      () => new Promise((res) => (resolveLate = res)),
    );
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-note-1"));
    await waitFor(() => screen.getByTestId("opener-chip-0"));
    // Remove the item while its preview is still in flight…
    fireEvent.click(screen.getByTestId("opener-chip-0"));
    // …then let the stale response land.
    await act(async () => {
      resolveLate(PREVIEW_DTO);
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(usePracticeStore.getState().openerPreview).toBeNull();
    expect(screen.queryByTestId("opener-preview")).toBeNull();
  });

  it("a calm refusal surfaces in the panel, not a crash", async () => {
    mockInvoke.mockRejectedValue("add a note or two first — then Begin");
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-note-1"));
    await waitFor(() => screen.getByTestId("opener-notice"));
    expect(screen.getByTestId("opener-notice").textContent).toContain(
      "add a note or two",
    );
  });

  // ── #419 S2a: the item bank goes live ─────────────────────────────────

  it("every bank row sends its SEMANTIC wire shape", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));

    fireEvent.click(screen.getByTestId("opener-interval-5"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "interval", number: 5 }],
        tonic: null,
        direction: "forward",
      }),
    );
    fireEvent.click(screen.getByTestId("opener-chord-dominant_seventh"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [
          { type: "interval", number: 5 },
          { type: "chord", kind: "dominant_seventh" },
        ],
        tonic: null,
        direction: "forward",
      }),
    );
    fireEvent.click(screen.getByTestId("opener-scale-blues"));
    fireEvent.click(screen.getByTestId("opener-enclosure-one_down_one_up"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [
          { type: "interval", number: 5 },
          { type: "chord", kind: "dominant_seventh" },
          { type: "scale", kind: "blues" },
          { type: "enclosure", style: "one_down_one_up" },
        ],
        tonic: null,
        direction: "forward",
      }),
    );
    // Chip labels read musically.
    expect(screen.getByTestId("opener-chip-0").textContent).toContain("5th");
    // Review MF4: ordinals are real music words — "3rd", never "3th".
    fireEvent.click(screen.getByTestId("opener-interval-3"));
    await waitFor(() => screen.getByTestId("opener-chip-4"));
    expect(screen.getByTestId("opener-chip-4").textContent).toContain("3rd");
    expect(screen.getByTestId("opener-chip-1").textContent).toContain("7");
    expect(screen.getByTestId("opener-chip-2").textContent).toContain("blues");
    expect(screen.getByTestId("opener-chip-3").textContent).toContain(
      "enclose ↓↑",
    );
  });

  it("the resting bank shrank to what actually rests", () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    // S2b took directions live; only My patterns rests now.
    screen.getByText("My patterns");
    expect(screen.queryByText("Pattern directions")).toBeNull();
    // The four S2a rows are live sections now, not resting chips.
    expect(screen.getAllByText("Intervals")).toHaveLength(1);
    expect(screen.getAllByText("Chords")).toHaveLength(1);
  });

  // ── #419 S2b: direction chips + the captured tonic ────────────────────

  it("direction chips are exclusive and re-voice the preview", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-seq-1-2-3-5"));
    await waitFor(() => screen.getByTestId("opener-chip-0"));
    fireEvent.click(screen.getByTestId("opener-direction-reversed"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
        tonic: null,
        direction: "reversed",
      }),
    );
    expect(
      screen
        .getByTestId("opener-direction-reversed")
        .getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("opener-direction-forward")
        .getAttribute("aria-pressed"),
    ).toBe("false");
  });

  it("Begin sends the tonic the last PREVIEW captured, not a fresh read", async () => {
    // A confident A-major room at preview time…
    usePracticeStore.setState({
      perception: {
        tempo_bpm: null,
        swing_ratio: null,
        locked: false,
        key: {
          tonic: 9,
          mode: "major",
          name: "A major",
          confidence: 0.9,
          alternative: null,
        },
        chord: null,
        hearing_polyphony: false,
      } as never,
    });
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-seq-1-2-3-5"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
        tonic: 9,
        direction: "forward",
      }),
    );
    // …then the room drifts to D before Begin: Begin must still send A —
    // the preview IS the exercise.
    usePracticeStore.setState({
      perception: {
        tempo_bpm: null,
        swing_ratio: null,
        locked: false,
        key: {
          tonic: 2,
          mode: "major",
          name: "D major",
          confidence: 0.9,
          alternative: null,
        },
        chord: null,
        hearing_polyphony: false,
      } as never,
    });
    fireEvent.click(screen.getByTestId("opener-begin"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("begin_opener", {
        items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
        tonic: 9,
        direction: "forward",
      }),
    );
  });

  it("custom entry parses spaces/dashes into ONE note_sequence item", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.change(screen.getByTestId("opener-custom-input"), {
      target: { value: " 1-5, 3 2 " },
    });
    fireEvent.click(screen.getByTestId("opener-custom-add"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1, 5, 3, 2] }],
        tonic: null,
        direction: "forward",
      }),
    );
    // Input clears for the next one.
    expect(
      (screen.getByTestId("opener-custom-input") as HTMLInputElement).value,
    ).toBe("");
  });

  it("custom junk gets a calm client notice and NOTHING goes over the wire", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.change(screen.getByTestId("opener-custom-input"), {
      target: { value: "do re mi" },
    });
    fireEvent.click(screen.getByTestId("opener-custom-add"));
    await waitFor(() => screen.getByTestId("opener-custom-notice"));
    expect(mockInvoke).not.toHaveBeenCalled();
    // But out-of-range DEGREES do go over — the backend refuses by name.
    fireEvent.change(screen.getByTestId("opener-custom-input"), {
      target: { value: "9" },
    });
    fireEvent.click(screen.getByTestId("opener-custom-add"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [9] }],
        tonic: null,
        direction: "forward",
      }),
    );
  });
});
