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
  // The list-shaped and recall commands resolve to their honest empties;
  // every other command previews. Routed BY NAME — the E2 review lesson.
  mockInvoke.mockImplementation((cmd: string) => {
    if (cmd === "my_patterns" || cmd === "list_opener_recipes") {
      return Promise.resolve([]);
    }
    if (cmd === "recall_last_opener") {
      return Promise.resolve(null);
    }
    return Promise.resolve(PREVIEW_DTO);
  });
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
    let deferredArmed = true;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "my_patterns") {
        return Promise.resolve([]);
      }
      if (cmd === "preview_opener" && deferredArmed) {
        deferredArmed = false;
        return new Promise((res) => (resolveLate = res));
      }
      return Promise.resolve(PREVIEW_DTO);
    });
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
    // Scoped re-pin (#419 S3): the panel's OPEN fetches my_patterns —
    // the junk contract is that no OPENER command fires.
    expect(
      mockInvoke.mock.calls.some(
        ([cmd]) => cmd === "preview_opener" || cmd === "begin_opener",
      ),
    ).toBe(false);
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

  it("the capture respects the assert threshold — 0.54 is a wobble", async () => {
    // Review MF5: >= 0.55 captures; below stays C. Exactly-at included.
    const withConfidence = (confidence: number) =>
      ({
        tempo_bpm: null,
        swing_ratio: null,
        locked: false,
        key: {
          tonic: 9,
          mode: "major",
          name: "A major",
          confidence,
          alternative: null,
        },
        chord: null,
        hearing_polyphony: false,
      }) as never;
    usePracticeStore.setState({ perception: withConfidence(0.54) });
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-note-1"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1] }],
        tonic: null,
        direction: "forward",
      }),
    );
    usePracticeStore.setState({ perception: withConfidence(0.55) });
    fireEvent.click(screen.getByTestId("opener-note-2"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [
          { type: "note_sequence", degrees: [1] },
          { type: "note_sequence", degrees: [2] },
        ],
        tonic: 9,
        direction: "forward",
      }),
    );
  });

  it("Begin resets direction and tonic — the next recipe starts fresh", async () => {
    // Review MF6: a stale captured tonic leaking into the next recipe is
    // the drift class this slice exists to prevent.
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
    fireEvent.click(screen.getByTestId("opener-direction-reversed"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
        tonic: 9,
        direction: "reversed",
      }),
    );
    fireEvent.click(screen.getByTestId("opener-begin"));
    await waitFor(() =>
      expect(usePracticeStore.getState().openerItems).toHaveLength(0),
    );
    expect(usePracticeStore.getState().openerDirection).toBe("forward");
    expect(usePracticeStore.getState().openerTonic).toBeNull();
    // The room went silent; the NEXT recipe previews from C, forward.
    usePracticeStore.setState({ perception: null });
    fireEvent.click(screen.getByTestId("opener-note-1"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "note_sequence", degrees: [1] }],
        tonic: null,
        direction: "forward",
      }),
    );
  });

  it("a stale direction refresh cannot outpaint a newer one — seq token", async () => {
    // Review MF2: two direction-triggered refreshes share the items array
    // identity; only a monotonic token can order them. The FIRST refresh
    // resolves LAST and must be dropped — preview, tonic, and previewed
    // direction all stay with the newer response.
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-seq-1-2-3-5"));
    await waitFor(() => screen.getByTestId("opener-chip-0"));

    let resolveSlow: (v: unknown) => void = () => {};
    mockInvoke.mockImplementationOnce(
      () => new Promise((res) => (resolveSlow = res)),
    );
    fireEvent.click(screen.getByTestId("opener-direction-reversed"));
    // The second flip's refresh resolves immediately.
    fireEvent.click(screen.getByTestId("opener-direction-varied"));
    await waitFor(() =>
      expect(usePracticeStore.getState().openerPreviewedDirection).toBe(
        "varied",
      ),
    );
    // The slow "reversed" response lands late — and changes nothing.
    await act(async () => {
      resolveSlow({ ...PREVIEW_DTO, label: "stale reversed preview" });
      await Promise.resolve();
    });
    expect(usePracticeStore.getState().openerPreviewedDirection).toBe("varied");
    expect(usePracticeStore.getState().openerPreview?.label).not.toBe(
      "stale reversed preview",
    );
  });

  it("Begin's reset kills an in-flight refresh — no ghost repaint", async () => {
    // Round-3 review MF1: a preview refresh airborne when Begin resolves
    // must NOT repaint the just-reset builder with a ghost preview and a
    // stale captured tonic.
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    fireEvent.click(screen.getByTestId("opener-seq-1-2-3-5"));
    await waitFor(() => screen.getByTestId("opener-chip-0"));

    // A second item's preview goes airborne (slow)…
    let resolveSlow: (v: unknown) => void = () => {};
    mockInvoke.mockImplementationOnce(
      () => new Promise((res) => (resolveSlow = res)),
    );
    fireEvent.click(screen.getByTestId("opener-note-1"));
    await waitFor(() =>
      expect(usePracticeStore.getState().openerItems).toHaveLength(2),
    );
    // …while Begin resolves fast and resets the builder.
    fireEvent.click(screen.getByTestId("opener-begin"));
    await waitFor(() =>
      expect(usePracticeStore.getState().openerItems).toHaveLength(0),
    );
    // The slow preview lands late — and changes NOTHING.
    await act(async () => {
      resolveSlow({ ...PREVIEW_DTO, label: "ghost preview" });
      await Promise.resolve();
    });
    expect(usePracticeStore.getState().openerPreview).toBeNull();
    expect(usePracticeStore.getState().openerTonic).toBeNull();
    expect(usePracticeStore.getState().openerPreviewedDirection).toBe(
      "forward",
    );
  });

  // ── #419 S3: My patterns ──────────────────────────────────────────────

  it("your patterns surface as chips and add through the Notes wire", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      Promise.resolve(
        cmd === "my_patterns"
          ? [
              {
                label: "your 5-note cell · 3×, last in A",
                offsets: [0, 4, 2, 7, 4],
                times_practiced: 3,
                last_tonic: 9,
              },
            ]
          : PREVIEW_DTO,
      ),
    );
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    await waitFor(() => screen.getByTestId("opener-my-pattern-0"));
    expect(screen.getByTestId("opener-my-pattern-0").textContent).toContain(
      "3×, last in A",
    );
    fireEvent.click(screen.getByTestId("opener-my-pattern-0"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith("preview_opener", {
        items: [{ type: "notes", offsets: [0, 4, 2, 7, 4] }],
        tonic: null,
        direction: "forward",
      }),
    );
  });

  it("an empty practice history reads honestly, never hides", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    await waitFor(() => screen.getByTestId("my-patterns-empty"));
    expect(screen.getByTestId("my-patterns-empty").textContent).toContain(
      "your patterns appear here",
    );
  });
});

/**
 * #419 S4 — recipe persistence + recall: save round-trips through the
 * wire, a tapped recipe repopulates the builder through the EXISTING
 * preview path, delete forgets, and yesterday's opener replays via
 * begin_opener_recall (the stored-seed law lives backend-side — the
 * frontend only carries semantic gestures).
 */
describe("OpenersPanel (#419 S4 recipes)", () => {
  const RECIPE = {
    id: 3,
    name: "The classic",
    items: [{ type: "note_sequence", degrees: [1, 2, 3, 5] }],
    direction: "reversed",
  };

  it("save: named builder goes over the wire and lands on the strip", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "my_patterns" || cmd === "list_opener_recipes") {
        return Promise.resolve([]);
      }
      if (cmd === "recall_last_opener") {
        return Promise.resolve(null);
      }
      if (cmd === "save_opener_recipe") {
        return Promise.resolve({
          id: 7,
          name: "Morning",
          items: [{ type: "note_sequence", degrees: [3] }],
          direction: "forward",
        });
      }
      return Promise.resolve(PREVIEW_DTO);
    });
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    // Empty builder → no save controls at all (AC6: nothing to save).
    expect(screen.queryByTestId("opener-recipe-save")).toBeNull();
    fireEvent.click(screen.getByTestId("opener-note-3"));
    await waitFor(() => screen.getByTestId("opener-recipe-save"));
    // Unnamed → disabled; named → live.
    expect(screen.getByTestId("opener-recipe-save")).toBeDisabled();
    fireEvent.change(screen.getByTestId("opener-recipe-name"), {
      target: { value: "Morning" },
    });
    expect(screen.getByTestId("opener-recipe-save")).not.toBeDisabled();
    fireEvent.click(screen.getByTestId("opener-recipe-save"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("save_opener_recipe", {
        name: "Morning",
        items: [{ type: "note_sequence", degrees: [3] }],
        direction: "forward",
      }),
    );
    await waitFor(() => screen.getByTestId("opener-recipe-7"));
    expect(screen.getByTestId("opener-recipe-7").textContent).toBe("Morning");
    expect(
      (screen.getByTestId("opener-recipe-name") as HTMLInputElement).value,
    ).toBe("");
  });

  it("a tapped recipe repopulates the builder through the preview path", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "my_patterns") {
        return Promise.resolve([]);
      }
      if (cmd === "list_opener_recipes") {
        return Promise.resolve([RECIPE]);
      }
      if (cmd === "recall_last_opener") {
        return Promise.resolve(null);
      }
      return Promise.resolve(PREVIEW_DTO);
    });
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    await waitFor(() => screen.getByTestId("opener-recipe-3"));
    fireEvent.click(screen.getByTestId("opener-recipe-3"));
    // AC2: the EXISTING preview command, carrying the recipe's items AND
    // direction — no new generation path frontend-side.
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("preview_opener", {
        items: RECIPE.items,
        tonic: null,
        direction: "reversed",
      }),
    );
    expect(usePracticeStore.getState().openerItems).toEqual(RECIPE.items);
    expect(usePracticeStore.getState().openerDirection).toBe("reversed");
  });

  it("delete forgets the recipe and the honest empty state returns", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "my_patterns") {
        return Promise.resolve([]);
      }
      if (cmd === "list_opener_recipes") {
        return Promise.resolve([RECIPE]);
      }
      if (cmd === "recall_last_opener") {
        return Promise.resolve(null);
      }
      if (cmd === "delete_opener_recipe") {
        return Promise.resolve(null);
      }
      return Promise.resolve(PREVIEW_DTO);
    });
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    await waitFor(() => screen.getByTestId("opener-recipe-delete-3"));
    fireEvent.click(screen.getByTestId("opener-recipe-delete-3"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("delete_opener_recipe", {
        id: 3,
      }),
    );
    await waitFor(() => screen.getByTestId("opener-recipes-empty"));
    expect(screen.queryByTestId("opener-recipe-3")).toBeNull();
  });

  it("yesterday's opener replays via begin_opener_recall", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "my_patterns" || cmd === "list_opener_recipes") {
        return Promise.resolve([]);
      }
      if (cmd === "recall_last_opener") {
        return Promise.resolve({ label: "1-2-3-5 · 12 keys", tonic: 9 });
      }
      return Promise.resolve(PREVIEW_DTO);
    });
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    await waitFor(() => screen.getByTestId("opener-yesterday"));
    expect(screen.getByTestId("opener-yesterday").textContent).toContain(
      "1-2-3-5 · 12 keys",
    );
    fireEvent.click(screen.getByTestId("opener-yesterday"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("begin_opener_recall", undefined),
    );
    // The recalled opener lands on the explore surface, like Begin.
    await waitFor(() =>
      expect(usePracticeStore.getState().explore).toEqual(PREVIEW_DTO),
    );
  });

  it("no opener begun yet → the honest empty chip", async () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    await waitFor(() => screen.getByTestId("opener-yesterday-empty"));
    expect(screen.queryByTestId("opener-yesterday")).toBeNull();
    // And the recipes strip is honestly empty too.
    screen.getByTestId("opener-recipes-empty");
  });
});

describe("OpenersPanel (#445 pt 6a — reachable panel + indigo palette)", () => {
  // The dead-zone fix is a layout contract: the open builder is an unbounded
  // wall taller than the viewport, so its live feedback (chips + preview) and
  // its primary action (Begin) render below an invisible fold — the founder's
  // "static UI, cant click." jsdom can't measure layout, so we pin the
  // STRUCTURAL fix that makes the surface reachable at any window height: a
  // bounded, internally-scrollable card with Begin as an always-visible sticky
  // footer. (The pixel-level reachability is proven by the headless-browser
  // probe in docs/specs/445-openers-fix.md §2/§3.)
  it("the open panel is a bounded, internally-scrollable card", () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    const panel = screen.getByTestId("openers-panel");
    // Bounded height: the card can never exceed the viewport…
    expect(panel.className).toMatch(/\bmax-h-/);
    // …and its own content scrolls INSIDE it (the visible affordance) instead
    // of pushing the page past an invisible fold.
    expect(panel.className).toContain("overflow-y-auto");
  });

  it("Begin is pinned as a sticky footer — always reachable", () => {
    render(<OpenersPanel />);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    const begin = screen.getByTestId("opener-begin");
    // Sticky to the bottom of the scroll container: Begin stays on-screen and
    // clickable however tall the bank/preview grows. Un-pinning it (the revert)
    // recreates the off-fold dead zone.
    expect(begin.className).toContain("sticky");
    expect(begin.className).toContain("bottom-0");
  });

  it("the whole openers surface is off teal — closed AND open", () => {
    // Cheap, honest palette regression pin: any teal- class reappearing on the
    // invitation or the builder is the old green the founder rejected.
    const { container } = render(<OpenersPanel />);
    // Closed: the invitation button.
    expect(container.innerHTML).not.toMatch(/teal-/);
    fireEvent.click(screen.getByTestId("openers-toggle"));
    // Open: the full builder.
    expect(container.innerHTML).not.toMatch(/teal-/);
  });
});
