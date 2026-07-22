import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { RosterHeat } from "./RosterHeat";
import { makeClient } from "../test/mockSupabase";

// Fixed clock: the 14-day window is 2026-07-08 .. 2026-07-21 (local).
const TODAY = new Date(2026, 6, 21, 12, 0, 0);

const enrollment = (student_id: string, display_name: string) => ({
  student_id,
  profiles: { display_name },
});

const heatRow = (
  student_id: string,
  date_key: string,
  played_min: number | null,
  wall_min: number | null,
  extra?: Partial<{
    integrity_flag: boolean;
    sessions: number;
    thin_sessions: number;
  }>,
) => ({
  classroom_id: "c1",
  student_id,
  display_name: null,
  date_key,
  played_min,
  wall_min,
  sessions: extra?.sessions ?? 1,
  thin_sessions: extra?.thin_sessions ?? 0,
  integrity_flag: extra?.integrity_flag ?? false,
});

function mount(fixtures: Parameters<typeof makeClient>[0]) {
  const mock = makeClient(fixtures);
  render(
    <RosterHeat
      client={mock.client}
      classroomId="c1"
      onOpenStudent={() => {}}
      today={TODAY}
    />,
  );
  return mock;
}

describe("RosterHeat (spec AC2-AC4, AC11)", () => {
  it("renders a cell per student-day from v_roster_heat rows", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: [heatRow("s1", "2026-07-20", 12, 30)],
      fact_session: [],
    });
    const avaRow = await screen.findByRole("row", { name: /Ava/ });
    expect(within(avaRow).getByText("12")).toBeInTheDocument();
  });

  it("cell exposes played AND wall minutes — the gap is visible (display honesty)", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: [heatRow("s1", "2026-07-20", 12, 30)],
      fact_session: [],
    });
    const cell = await screen.findByTitle("12 min played · 30 min open");
    expect(cell).toHaveTextContent("12");
  });

  it("enrolled student with no rows shows practicing offline, never zero (AC3)", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava"), enrollment("s2", "Ben")],
      v_roster_heat: [heatRow("s1", "2026-07-20", 12, 30)],
      fact_session: [],
    });
    const benRow = await screen.findByRole("row", { name: /Ben/ });
    expect(within(benRow).getByText("practicing offline")).toBeInTheDocument();
    expect(within(benRow).queryByText("0")).not.toBeInTheDocument();
  });

  it("late-synced sessions annotate the row (F9) — shown, not punished", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: [heatRow("s1", "2026-07-10", 8, 10)],
      fact_session: [
        {
          session_id: "sess-1",
          student_id: "s1",
          started_at: "2026-07-10T15:00:00Z",
          created_at: "2026-07-20T09:00:00Z", // arrived 10 days later
        },
      ],
    });
    expect(
      await screen.findByText("Synced 2026-07-20, played 2026-07-10"),
    ).toBeInTheDocument();
  });

  it("prompt syncs get no annotation", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: [heatRow("s1", "2026-07-10", 8, 10)],
      fact_session: [
        {
          session_id: "sess-1",
          student_id: "s1",
          started_at: "2026-07-10T15:00:00Z",
          created_at: "2026-07-10T15:20:00Z",
        },
      ],
    });
    await screen.findByRole("row", { name: /Ava/ });
    expect(screen.queryByText(/^Synced /)).not.toBeInTheDocument();
  });

  it("colors cells by PLAYED minutes, never wall minutes (the flagship honesty rule)", async () => {
    // 12 played / 45 wall: the played bucket (5-15 min) is bg-emerald-200;
    // the wall bucket (>=30 min) would be bg-emerald-600. If anyone wires
    // heatClass to wall_min, this cell jumps buckets and the test dies.
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: [heatRow("s1", "2026-07-20", 12, 45)],
      fact_session: [],
    });
    const cell = await screen.findByTitle("12 min played · 45 min open");
    expect(cell.className).toContain("bg-emerald-200");
    expect(cell.className).not.toContain("bg-emerald-600");
  });

  it("flagged day cells carry the integrity ring", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: [
        heatRow("s1", "2026-07-20", 0.5, 25, {
          integrity_flag: true,
          thin_sessions: 1,
        }),
      ],
      fact_session: [],
    });
    const cell = await screen.findByTitle("0.5 min played · 25 min open");
    expect(cell.className).toContain("ring-amber-400");
  });

  it("empty roster is an honest empty state (AC11)", async () => {
    mount({ enrollments: [], v_roster_heat: [], fact_session: [] });
    expect(
      await screen.findByText(/No students are actively enrolled/),
    ).toBeInTheDocument();
  });

  it("query error surfaces as an error, not an empty grid (AC11)", async () => {
    mount({
      enrollments: [enrollment("s1", "Ava")],
      v_roster_heat: new Error("permission denied"),
      fact_session: [],
    });
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/Could not load the roster/);
    expect(alert).toHaveTextContent(/permission denied/);
  });
});
