import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CellStaff, { MEASURES_PER_PAGE } from "./CellStaff";
import { RV_LETTER_COLORS, RV_NON_ROOT_FILL } from "../lib/rvColors";
import type { CellStaffViewDto, CellStaffNoteDto } from "../types/brain";

function note(overrides: Partial<CellStaffNoteDto>): CellStaffNoteDto {
  return {
    midi: 60,
    start_beat: 0,
    duration_beats: 1,
    step: -2,
    accidental: null,
    // Default fixtures to root notes so the palette assertions keep testing
    // the colored path; the RV root-vs-white rule has its own test below.
    is_root: true,
    ...overrides,
  };
}

function view(
  notes: CellStaffNoteDto[],
  overrides?: Partial<CellStaffViewDto>,
): CellStaffViewDto {
  const last = notes[notes.length - 1];
  return {
    fifths: 0,
    beats_per_measure: 4,
    total_beats: last ? last.start_beat + last.duration_beats : 4,
    notes,
    ...overrides,
  };
}

describe("CellStaff (#292 slice 1)", () => {
  // #292 AC (brand): stemless dots in the RV palette — a G renders in G's
  // exact brand color, and nothing draws a stem.
  it("renders stemless dots in brand colors", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 67, step: 2 }), // G4
          note({ midi: 60, step: -2, start_beat: 1 }), // C4
        ])}
      />,
    );
    const dots = screen.getAllByTestId("staff-dot");
    expect(dots).toHaveLength(2);
    expect(dots[0]).toHaveAttribute("fill", RV_LETTER_COLORS.G);
    expect(dots[1]).toHaveAttribute("fill", RV_LETTER_COLORS.C);
  });

  // RV color rule (founder, 2026-07-08): only ROOT notes carry their
  // pitch-class color — every other note draws white so the roots pop.
  // Fails if the renderer colors by pitch class unconditionally.
  it("colors only root notes; scale tones draw white", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 64, step: 0 }), // E root — colored
          note({ midi: 67, step: 2, start_beat: 1, is_root: false }), // G — white
          note({ midi: 71, step: 4, start_beat: 2, is_root: false }), // B — white
        ])}
      />,
    );
    const dots = screen.getAllByTestId("staff-dot");
    expect(dots[0]).toHaveAttribute("fill", RV_LETTER_COLORS.E);
    expect(dots[1]).toHaveAttribute("fill", RV_NON_ROOT_FILL);
    expect(dots[2]).toHaveAttribute("fill", RV_NON_ROOT_FILL);
  });

  // Accidentals draw only when the backend says so (spelling lives in Rust).
  it("draws accidental glyphs only when provided", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 61, step: -2, accidental: 1 }),
          note({ midi: 62, step: -1, start_beat: 1, accidental: null }),
        ])}
      />,
    );
    expect(screen.getAllByTestId("staff-accidental")).toHaveLength(1);
    expect(screen.getAllByTestId("staff-accidental")[0]).toHaveTextContent("♯");
  });

  // Key signature: fifths = -2 draws two flats; +3 draws three sharps.
  it("draws the key signature from fifths", () => {
    const { unmount } = render(
      <CellStaff staff={view([note({})], { fifths: -2 })} />,
    );
    expect(screen.getAllByTestId("staff-signature")).toHaveLength(2);
    expect(screen.getAllByTestId("staff-signature")[0]).toHaveTextContent("♭");
    unmount();
    render(<CellStaff staff={view([note({})], { fifths: 3 })} />);
    expect(screen.getAllByTestId("staff-signature")).toHaveLength(3);
    expect(screen.getAllByTestId("staff-signature")[0]).toHaveTextContent("♯");
  });

  // #292 AC (founder): 2–4 measures at a time — an 8-measure cell pages, the
  // second page shows only its own notes.
  it("windows long cells with the pager", () => {
    const notes = Array.from({ length: 8 }, (_, m) =>
      note({ midi: 60 + m, step: m - 2, start_beat: m * 4 }),
    );
    render(<CellStaff staff={view(notes, { total_beats: 32 })} />);
    expect(screen.getAllByTestId("staff-dot")).toHaveLength(MEASURES_PER_PAGE);
    const pager = screen.getByTestId("staff-pager");
    expect(pager).toHaveTextContent("1 / 2");
    fireEvent.click(screen.getByLabelText("Next measures"));
    expect(pager).toHaveTextContent("2 / 2");
    expect(screen.getAllByTestId("staff-dot")).toHaveLength(
      8 - MEASURES_PER_PAGE,
    );
  });

  // A short cell shows no pager at all — slim by default.
  it("hides the pager within one window", () => {
    render(<CellStaff staff={view([note({})])} />);
    expect(screen.queryByTestId("staff-pager")).not.toBeInTheDocument();
  });
});

describe("CellStaff — rhythm layer (#292 slice 2)", () => {
  const mixed = () =>
    view([
      note({ midi: 60, step: -2, duration_beats: 0.5 }), // eighth
      note({ midi: 62, step: -1, start_beat: 1, duration_beats: 1 }), // quarter
      note({ midi: 64, step: 0, start_beat: 2, duration_beats: 2 }), // half
      note({ midi: 65, step: 1, start_beat: 4, duration_beats: 4 }), // whole
    ]);

  // #292 hard rule: toggling rhythms changes GLYPHS only — every notehead
  // stays at exactly the same position. Fails if the layer causes a reflow.
  it("never moves a notehead when the layer toggles", () => {
    const positions = (dots: HTMLElement[]) =>
      dots.map((d) => `${d.getAttribute("cx")},${d.getAttribute("cy")}`);
    const a = render(<CellStaff staff={mixed()} defaultShowRhythms={false} />);
    const before = positions(a.getAllByTestId("staff-dot"));
    a.unmount();
    const b = render(<CellStaff staff={mixed()} defaultShowRhythms={true} />);
    expect(positions(b.getAllByTestId("staff-dot"))).toEqual(before);
  });

  // Stems/flags obey duration: whole notes stay bare, sub-beat notes flag.
  it("draws stems and flags by duration, whole notes bare", () => {
    render(<CellStaff staff={mixed()} defaultShowRhythms={true} />);
    expect(screen.getAllByTestId("staff-stem")).toHaveLength(3);
    expect(screen.getAllByTestId("staff-flag")).toHaveLength(1);
  });

  // Default is the RV brand: dots only; the toggle reveals the layer and
  // persists the choice on the device.
  it("defaults to dots and persists the toggle", () => {
    window.localStorage.removeItem("amc.cellstaff.showRhythms");
    render(<CellStaff staff={mixed()} />);
    expect(screen.queryAllByTestId("staff-stem")).toHaveLength(0);
    fireEvent.click(screen.getByTestId("rhythm-toggle"));
    expect(screen.getAllByTestId("staff-stem")).toHaveLength(3);
    expect(window.localStorage.getItem("amc.cellstaff.showRhythms")).toBe("1");
  });
});

// jsdom has no PointerEvent; React's onPointerDown still fires from a
// MouseEvent-shaped dispatch, which carries clientY.
if (typeof window.PointerEvent === "undefined") {
  (window as unknown as { PointerEvent: unknown }).PointerEvent = MouseEvent;
}

describe("CellStaff — editing (#292 slice 3)", () => {
  const three = () =>
    view([
      note({ midi: 60, step: -2 }),
      note({ midi: 64, step: 0, start_beat: 1 }),
      note({ midi: 67, step: 2, start_beat: 2 }),
    ]);

  // Read-only surfaces stay read-only: no handler → no selection affordances.
  it("without onEditNote the staff is inert", () => {
    render(<CellStaff staff={three()} />);
    fireEvent.pointerDown(screen.getAllByTestId("staff-dot")[0]);
    expect(screen.queryByTestId("edit-actions")).not.toBeInTheDocument();
  });

  // Tap selects (halo + quick actions); actions emit SEMANTIC gestures with
  // the note's sequence index — never a pitch.
  it("selection reveals quick actions that emit semantic gestures", () => {
    const onEdit = vi.fn();
    render(<CellStaff staff={three()} onEditNote={onEdit} />);
    fireEvent.pointerDown(screen.getByTestId("staff-note-64-1"));
    fireEvent.pointerUp(window);
    expect(screen.getByTestId("staff-halo")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("edit-octave-up"));
    expect(onEdit).toHaveBeenCalledWith(1, { kind: "octaves", by: 1 });
    fireEvent.click(screen.getByTestId("edit-sharp"));
    expect(onEdit).toHaveBeenCalledWith(1, { kind: "semitones", by: 1 });
    fireEvent.click(screen.getByTestId("edit-remove"));
    expect(onEdit).toHaveBeenCalledWith(1, { kind: "remove" });
  });

  // Vertical drag emits staff_steps quantized to staff positions (5px = one
  // step at the unscaled viewBox size jsdom reports).
  it("a vertical drag emits a staff_steps gesture", () => {
    const onEdit = vi.fn();
    render(<CellStaff staff={three()} onEditNote={onEdit} />);
    fireEvent.pointerDown(screen.getByTestId("staff-note-60-0"), {
      clientY: 100,
    });
    fireEvent.pointerMove(window, { clientY: 90 });
    fireEvent.pointerUp(window, { clientY: 90 });
    expect(onEdit).toHaveBeenCalledWith(0, { kind: "staff_steps", by: 2 });
  });

  // A drag that ends where it started edits nothing (tap = select only).
  it("a no-move release edits nothing", () => {
    const onEdit = vi.fn();
    render(<CellStaff staff={three()} onEditNote={onEdit} />);
    fireEvent.pointerDown(screen.getByTestId("staff-note-60-0"), {
      clientY: 100,
    });
    fireEvent.pointerUp(window, { clientY: 100 });
    expect(onEdit).not.toHaveBeenCalled();
  });

  // Undo appears only when the backend says there is history.
  it("undo shows only with history", () => {
    const onUndo = vi.fn();
    const { unmount } = render(
      <CellStaff
        staff={three()}
        onEditNote={() => {}}
        onUndo={onUndo}
        canUndo={false}
      />,
    );
    expect(screen.queryByTestId("edit-undo")).not.toBeInTheDocument();
    unmount();
    render(
      <CellStaff
        staff={three()}
        onEditNote={() => {}}
        onUndo={onUndo}
        canUndo={true}
      />,
    );
    fireEvent.click(screen.getByTestId("edit-undo"));
    expect(onUndo).toHaveBeenCalled();
  });
});

describe("CellStaff — review regressions", () => {
  const three = () =>
    view([
      note({ midi: 60, step: -2 }),
      note({ midi: 64, step: 0, start_beat: 1 }),
      note({ midi: 67, step: 2, start_beat: 2 }),
    ]);

  // Review M6/G: drag quantization at a REAL render size — the px-per-step
  // scale must divide by the actual viewBox height. At 2× render scale a
  // 2-step drag is 20 real px; a mismatched constant would misquantize it.
  it("quantizes drags correctly at a scaled render size", () => {
    const onEdit = vi.fn();
    render(<CellStaff staff={three()} onEditNote={onEdit} />);
    const svg = document.querySelector("svg")!;
    const vb = svg.getAttribute("viewBox")!.split(" ").map(Number);
    const spy = vi
      .spyOn(svg, "getBoundingClientRect")
      .mockReturnValue({ height: vb[3] * 2, width: 1280 } as DOMRect);
    fireEvent.pointerDown(screen.getByTestId("staff-note-60-0"), {
      clientY: 200,
    });
    fireEvent.pointerUp(window, { clientY: 180 }); // 20px up at 2× = 2 steps
    expect(onEdit).toHaveBeenCalledWith(0, { kind: "staff_steps", by: 2 });
    spy.mockRestore();
  });

  // Review nice-to-have: when the staff changes (chip / undo / new rep), a
  // kept selection would retarget an arbitrary note — it must clear instead.
  it("clears the selection when the staff changes", () => {
    const onEdit = vi.fn();
    const { rerender } = render(
      <CellStaff staff={three()} onEditNote={onEdit} />,
    );
    fireEvent.pointerDown(screen.getByTestId("staff-note-64-1"));
    fireEvent.pointerUp(window);
    expect(screen.getByTestId("staff-halo")).toBeInTheDocument();
    rerender(
      <CellStaff
        staff={view([note({ midi: 62, step: -1 })])}
        onEditNote={onEdit}
      />,
    );
    expect(screen.queryByTestId("staff-halo")).not.toBeInTheDocument();
    expect(screen.queryByTestId("edit-actions")).not.toBeInTheDocument();
  });

  // Review nice-to-have: a cancelled pointer (touch scroll) ends the drag
  // without editing.
  it("pointercancel abandons the drag without an edit", () => {
    const onEdit = vi.fn();
    render(<CellStaff staff={three()} onEditNote={onEdit} />);
    fireEvent.pointerDown(screen.getByTestId("staff-note-60-0"), {
      clientY: 100,
    });
    fireEvent.pointerMove(window, { clientY: 60 });
    fireEvent.pointerCancel(window);
    expect(onEdit).not.toHaveBeenCalled();
    // And a fresh drag afterwards still works.
    fireEvent.pointerDown(screen.getByTestId("staff-note-60-0"), {
      clientY: 100,
    });
    fireEvent.pointerUp(window, { clientY: 95 });
    expect(onEdit).toHaveBeenCalledWith(0, { kind: "staff_steps", by: 1 });
  });
});

describe("CellStaff — extreme registers (Monday review)", () => {
  // An 8va-edited note far above the staff must stay ON canvas: the viewBox
  // grows to cover it (review must-fix: it used to silently vanish).
  it("grows the viewBox to keep high/low notes visible", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 96, step: 23 }), // ~3 octaves up: deep ledger territory
          note({ midi: 40, step: -12, start_beat: 1 }),
        ])}
      />,
    );
    const svg = document.querySelector("svg")!;
    const [, minY, , height] = svg
      .getAttribute("viewBox")!
      .split(" ")
      .map(Number);
    const yHigh = 50 - 23 * 5; // yFor(23)
    const yLow = 50 - -12 * 5; // yFor(-12)
    expect(minY).toBeLessThanOrEqual(yHigh);
    expect(minY + height).toBeGreaterThanOrEqual(yLow);
    expect(screen.getAllByTestId("staff-dot")).toHaveLength(2);
  });

  // #349 T2a: a stacked cell (notes sharing a beat) draws as one vertical
  // stack — same x for every tone — with distinct staff heights.
  it("renders simultaneous notes as a vertical stack at one x position", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 60, step: -2 }), // C root
          note({ midi: 64, step: 0, is_root: false }), // E
          note({ midi: 67, step: 2, is_root: false }), // G
          note({ midi: 70, step: 4, accidental: -1, is_root: false }), // Bb
        ])}
      />,
    );
    const dots = screen.getAllByTestId("staff-dot");
    expect(dots).toHaveLength(4);
    const xs = new Set(dots.map((d) => d.getAttribute("cx")));
    expect(xs.size).toBe(1); // struck together → drawn together
    const ys = new Set(dots.map((d) => d.getAttribute("cy")));
    expect(ys.size).toBe(4); // four distinct staff heights
  });

  // Engraving rule for stacked SECONDS: adjacent-step simultaneous notes
  // must not overlap — the upper notehead shifts right. Fails if the
  // second-interval offset is dropped (all dots would share cx).
  it("offsets the upper note of a stacked second so both read", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 60, step: -2 }), // C
          note({ midi: 62, step: -1, is_root: false }), // D — a second above
          note({ midi: 67, step: 2, is_root: false }), // G — a third+ away
        ])}
      />,
    );
    const dots = screen.getAllByTestId("staff-dot");
    const byCx = dots.map((d) => Number(d.getAttribute("cx")));
    // C and G share the column; D (the upper of the second) shifts right.
    expect(byCx[0]).toBe(byCx[2]);
    expect(byCx[1]).toBeGreaterThan(byCx[0]);
  });

  // A CHROMATIC second (same staff step, different pitches — C + C#) must
  // also offset, or the stack draws two dots perfectly superimposed and
  // lies about its own size. Fails if the offset rule keys on step
  // difference alone.
  it("offsets a chromatic second sharing one staff step", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 72, step: 8 }), // C5
          note({ midi: 73, step: 8, accidental: 1, is_root: false }), // C#5
        ])}
      />,
    );
    const dots = screen.getAllByTestId("staff-dot");
    const byCx = dots.map((d) => Number(d.getAttribute("cx")));
    expect(byCx[1]).toBeGreaterThan(byCx[0]);
    // Engraving: the accidental stays LEFT of the whole column — it must
    // not ride the offset onto the lower notehead.
    const accX = Number(
      screen.getByTestId("staff-accidental").getAttribute("x"),
    );
    expect(accX).toBeLessThan(byCx[0] - 5.5);
  });

  // MELODIC notes never take the stack offset: adjacent-step neighbors at
  // DIFFERENT beats keep their own beat-grid columns, unshifted. Fails if
  // the offset predicate stops requiring simultaneity (every melodic
  // neighbor would drift +9px off the grid).
  it("keeps melodic neighbors on the beat grid — no stack offset", () => {
    render(
      <CellStaff
        staff={view([
          note({ midi: 60, step: -2 }), // C on beat 0
          note({ midi: 62, step: -1, start_beat: 1, is_root: false }), // D, beat 1
          note({ midi: 64, step: 0, start_beat: 2, is_root: false }), // E, beat 2
        ])}
      />,
    );
    const dots = screen.getAllByTestId("staff-dot");
    const byCx = dots.map((d) => Number(d.getAttribute("cx")));
    // Strictly increasing AND evenly spaced: pure beat-grid positions.
    expect(byCx[1] - byCx[0]).toBeCloseTo(byCx[2] - byCx[1], 5);
    expect(byCx[0]).toBeLessThan(byCx[1]);
  });
});
