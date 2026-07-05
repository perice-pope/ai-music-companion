import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CellStaff, { MEASURES_PER_PAGE } from "./CellStaff";
import { RV_LETTER_COLORS } from "../lib/rvColors";
import type { CellStaffViewDto, CellStaffNoteDto } from "../types/brain";

function note(overrides: Partial<CellStaffNoteDto>): CellStaffNoteDto {
  return {
    midi: 60,
    start_beat: 0,
    duration_beats: 1,
    step: -2,
    accidental: null,
    ...overrides,
  };
}

function view(notes: CellStaffNoteDto[], overrides?: Partial<CellStaffViewDto>): CellStaffViewDto {
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
