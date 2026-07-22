import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { IntegrityPanel } from "./IntegrityPanel";
import { NOMINATE_RULE } from "../lib/integrity";
import { makeClient } from "../test/mockSupabase";

const TODAY = new Date(2026, 6, 21, 12, 0, 0);

const integrityRow = (
  session_id: string,
  display_name: string,
  student_id: string,
  overrides: Partial<Record<string, unknown>> = {},
) => ({
  session_id,
  student_id,
  display_name,
  started_at: "2026-07-20T15:00:00Z",
  wall_min: 30,
  played_min: 10,
  silence_ratio: 0.5,
  note_count: 200,
  phrase_count: 2, // in the view because phrase_count < 3
  graded: 1,
  max_retries: 2,
  ...overrides,
});

function mount(fixtures: Parameters<typeof makeClient>[0]) {
  const mock = makeClient(fixtures);
  render(
    <IntegrityPanel client={mock.client} classroomId="c1" today={TODAY} />,
  );
  return mock;
}

const baseFixtures = {
  enrollments: [
    { student_id: "s1", profiles: { display_name: "Ava" } },
    { student_id: "s2", profiles: { display_name: "Ben" } },
  ],
  v_roster_heat: [],
  fact_tool_event: [],
  fact_phrase: [],
};

describe("IntegrityPanel (spec AC8-AC9)", () => {
  it("pins the nominate-not-judge rule (AC9)", async () => {
    mount({ ...baseFixtures, v_session_integrity: [] });
    await screen.findByText(/no sessions in this classroom currently meet/i);
    // The literal string is asserted on purpose: a drive-by edit to the
    // NOMINATE_RULE constant must fail this test, not silently reword it.
    expect(
      screen.getByText("Metrics nominate, humans judge."),
    ).toBeInTheDocument();
    expect(screen.getByText(NOMINATE_RULE)).toBeInTheDocument();
  });

  it("chips appear only for sessions above the bars (AC8)", async () => {
    mount({
      ...baseFixtures,
      v_session_integrity: [
        // Above the F1 bar: 30 min open, 30 s of sound, 95% silence.
        integrityRow("above-f1", "Ava", "s1", {
          played_min: 0.5,
          silence_ratio: 0.95,
        }),
        // Below every bar (in the view for phrase_count, but no chip fires).
        integrityRow("below-all", "Ben", "s2", { max_retries: 5 }),
      ],
    });
    const avaRow = await screen.findByRole("row", { name: /Ava/ });
    expect(
      within(avaRow).getByText("Open with almost no sound"),
    ).toBeInTheDocument();

    const benRow = screen.getByRole("row", { name: /Ben/ });
    expect(
      within(benRow).queryByText("Open with almost no sound"),
    ).not.toBeInTheDocument();
    expect(
      within(benRow).queryByText(/Same material graded/),
    ).not.toBeInTheDocument();
  });

  it("F4 retry-farming chip fires at the bar with evidence copy", async () => {
    mount({
      ...baseFixtures,
      v_session_integrity: [
        integrityRow("retries", "Ava", "s1", { max_retries: 8 }),
      ],
    });
    expect(
      await screen.findByText("Same material graded 8× this session"),
    ).toBeInTheDocument();
  });

  it("F8 chip fires from tool events over silent phrases", async () => {
    mount({
      ...baseFixtures,
      v_session_integrity: [integrityRow("f8", "Ava", "s1")],
      fact_tool_event: [
        { session_id: "f8", at_secs: 0, kind: "pocket_start", params: {} },
        { session_id: "f8", at_secs: 720, kind: "pocket_stop", params: {} },
      ],
      fact_phrase: [],
    });
    expect(
      await screen.findByText(
        "Metronome or band ran 12 min with no notes detected",
      ),
    ).toBeInTheDocument();
  });

  it("F6 split-days strip fires from day rows above the bar", async () => {
    mount({
      ...baseFixtures,
      v_roster_heat: [
        {
          classroom_id: "c1",
          student_id: "s1",
          display_name: "Ava",
          date_key: "2026-07-19",
          played_min: 4,
          wall_min: 12,
          sessions: 4,
          thin_sessions: 4,
          integrity_flag: true,
        },
        {
          // Below the bar: two sessions is a normal day.
          classroom_id: "c1",
          student_id: "s2",
          display_name: "Ben",
          date_key: "2026-07-19",
          played_min: 2,
          wall_min: 6,
          sessions: 2,
          thin_sessions: 0,
          integrity_flag: false,
        },
      ],
      v_session_integrity: [],
    });
    expect(
      await screen.findByText(/4 short sessions this day/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/2 short sessions/)).not.toBeInTheDocument();
  });

  it("filters to the selected classroom's roster (view has no classroom column)", async () => {
    mount({
      ...baseFixtures,
      v_session_integrity: [
        integrityRow("mine", "Ava", "s1", {
          played_min: 0.5,
          silence_ratio: 0.95,
        }),
        // RLS admits this row (another classroom of the same teacher, or the
        // teacher's own practice) — the panel must not show it here.
        integrityRow("other", "Zoe", "sX", {
          played_min: 0.5,
          silence_ratio: 0.95,
        }),
      ],
    });
    await screen.findByRole("row", { name: /Ava/ });
    expect(screen.queryByText("Zoe")).not.toBeInTheDocument();
  });

  it("never renders accusatory language (AC9 — evidence, not verdicts)", async () => {
    const { container } = render(
      <IntegrityPanel
        client={
          makeClient({
            ...baseFixtures,
            v_session_integrity: [
              integrityRow("above-f1", "Ava", "s1", {
                played_min: 0.5,
                silence_ratio: 0.95,
                max_retries: 9,
              }),
            ],
          }).client
        }
        classroomId="c1"
        today={TODAY}
      />,
    );
    await screen.findByRole("row", { name: /Ava/ });
    expect(container.textContent).not.toMatch(/cheat|fake|lied|liar|fraud/i);
  });

  it("empty evidence is an honest empty state", async () => {
    mount({ ...baseFixtures, v_session_integrity: [] });
    expect(
      await screen.findByText(/no sessions in this classroom currently meet/i),
    ).toBeInTheDocument();
  });
});
