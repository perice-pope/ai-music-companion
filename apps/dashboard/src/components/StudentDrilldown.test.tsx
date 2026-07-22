import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StudentDrilldown } from "./StudentDrilldown";
import { makeClient } from "../test/mockSupabase";

const session = (
  overrides: Partial<Record<string, unknown>> = {},
): Record<string, unknown> => ({
  session_id: "sess-1",
  student_id: "s1",
  device_session_id: "dev-1",
  started_at: "2026-07-20T15:00:00Z",
  ended_at: "2026-07-20T15:30:00Z",
  duration_secs: 1800,
  played_secs: 240,
  note_count: 350,
  silence_ratio: 0.87,
  phrase_count: 9,
  instrument: "trumpet",
  practice_mode: "explore",
  score_material_id: null,
  fingerprint: null,
  app_version: "2.46.0",
  created_at: "2026-07-20T15:31:00Z",
  ...overrides,
});

const exercise = (
  tonic: number,
  accuracy: number | null,
  materialId = "m1",
  label = "Minor triad, enclosed, descending",
) => ({
  exercise_id: `${materialId}-${tonic}-${accuracy}`,
  logged_at: "2026-07-20T15:05:00Z",
  tonic,
  difficulty: 2,
  accuracy,
  session_id: "sess-1",
  dim_material: {
    material_id: materialId,
    label,
    source: "explore",
    kind: "cell",
    spec_hash: "abc123",
  },
});

function mount(fixtures: Parameters<typeof makeClient>[0]) {
  const mock = makeClient(fixtures);
  render(
    <StudentDrilldown
      client={mock.client}
      studentId="s1"
      displayName="Ava"
      onBack={() => {}}
    />,
  );
  return mock;
}

describe("StudentDrilldown (spec AC5-AC7)", () => {
  it("session rows show wall and played separately (AC5 — one clock, never conflated)", async () => {
    mount({
      fact_session: [session()],
      fact_exercise: [],
      fact_tool_event: [],
    });
    const row = await screen.findByRole("row", { name: /trumpet/ });
    // 1800 s wall = 30 min; 240 s played = 4 min — both present, distinct.
    expect(within(row).getByText("30")).toBeInTheDocument();
    expect(within(row).getByText("4")).toBeInTheDocument();
    expect(within(row).getByText("350")).toBeInTheDocument(); // notes
    expect(within(row).getByText("87%")).toBeInTheDocument(); // silence
  });

  it("null integrity columns render em dash — absence, not zero (AC5)", async () => {
    mount({
      fact_session: [
        session({ played_secs: null, note_count: null, silence_ratio: null }),
      ],
      fact_exercise: [],
      fact_tool_event: [],
    });
    const row = await screen.findByRole("row", { name: /trumpet/ });
    expect(within(row).getAllByText("—")).toHaveLength(3);
    expect(within(row).queryByText("0")).not.toBeInTheDocument();
  });

  it("renders the stored recap facts as fingerprint chips when the device claimed them", async () => {
    mount({
      fact_session: [
        session({ fingerprint: { tone: "warm", key: "F major" } }),
      ],
      fact_exercise: [],
      fact_tool_event: [],
    });
    expect(await screen.findByText("tone: warm")).toBeInTheDocument();
    expect(screen.getByText("key: F major")).toBeInTheDocument();
  });

  it("coverage matrix blank means never drilled, not zero (AC6)", async () => {
    mount({
      fact_session: [],
      fact_exercise: [
        exercise(0, 0.8),
        exercise(0, 0.9),
        exercise(7, null), // attempted in G, never graded
      ],
      fact_tool_event: [],
    });
    const matrixRow = await screen.findByRole("row", {
      name: /Minor triad, enclosed, descending/,
    });
    // C column: two graded attempts averaging 85%.
    expect(within(matrixRow).getByText("85%")).toBeInTheDocument();
    // G column: attempted, ungraded — the dot, not a percentage.
    expect(within(matrixRow).getByText("·")).toBeInTheDocument();
    // The other 10 tonics: blank cells labeled honestly, and no "0" anywhere.
    expect(within(matrixRow).getAllByLabelText("never drilled")).toHaveLength(
      10,
    );
    expect(within(matrixRow).queryByText(/0\s*%/)).not.toBeInTheDocument();
  });

  it("summarizes tool events (AC7 — how they practiced)", async () => {
    mount({
      fact_session: [],
      fact_exercise: [],
      fact_tool_event: [
        {
          event_id: "e1",
          session_id: "sess-1",
          student_id: "s1",
          device_event_id: 1,
          at_secs: 10,
          kind: "pocket_start",
          params: { bpm: 90, mode: "anchor" },
          created_at: "2026-07-20T15:00:10Z",
        },
        {
          event_id: "e2",
          session_id: "sess-1",
          student_id: "s1",
          device_event_id: 2,
          at_secs: 310,
          kind: "pocket_stop",
          params: { bpm: 110 },
          created_at: "2026-07-20T15:05:10Z",
        },
        {
          event_id: "e3",
          session_id: "sess-1",
          student_id: "s1",
          device_event_id: 3,
          at_secs: 400,
          kind: "narration_used",
          params: { kind: "tip" },
          created_at: "2026-07-20T15:06:40Z",
        },
      ],
    });
    expect(
      await screen.findByText(
        /Metronome: on in 1 session, about 5 min total, tempos 90–110 bpm/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/Coaching narration used 1×/)).toBeInTheDocument();
  });

  it("no tool events → honest absence line, not 'not used' (AC7)", async () => {
    mount({ fact_session: [], fact_exercise: [], fact_tool_event: [] });
    expect(
      await screen.findByText(/No tool events synced/),
    ).toBeInTheDocument();
    expect(screen.getByText(/different from .not used./)).toBeInTheDocument();
  });

  it("no synced sessions → honest offline copy, not an empty table", async () => {
    mount({ fact_session: [], fact_exercise: [], fact_tool_event: [] });
    expect(
      await screen.findByText(/absence of sync, not absence of practice/),
    ).toBeInTheDocument();
  });
});
