import type { DashboardClient } from "../lib/supabase";
import {
  getLateSyncSessions,
  getRosterHeat,
  listRoster,
  type RosterHeatRow,
} from "../lib/queries";
import {
  describeLateSync,
  lateSyncAnnotations,
  type LateSyncAnnotation,
} from "../lib/integrity";
import { dayLabel, fmtMinutes, recentDateKeys } from "../lib/format";
import { useAsync } from "../lib/useAsync";

export const HEAT_WINDOW_DAYS = 14;

/**
 * The landing view (spec AC2-AC4, datamodel §4a): one row per actively
 * enrolled student, one cell per day of the window, colored by PLAYED
 * minutes — never wall minutes. Every populated cell exposes the pair
 * ("X min played · Y min open"): the gap is the story. Students with no
 * synced rows read "practicing offline" — a labeled state, never zeros
 * (honest absence, datamodel §2). Late-arriving rows carry the F9
 * annotation — shown, not punished.
 *
 * Deliberately a CSS grid, not a chart library: two numbers and a color
 * per cell don't justify a dependency (spec §3 non-goals).
 */
export function RosterHeat({
  client,
  classroomId,
  onOpenStudent,
  today,
}: {
  client: DashboardClient;
  classroomId: string;
  onOpenStudent: (studentId: string, displayName: string | null) => void;
  today?: Date;
}) {
  const dateKeys = recentDateKeys(HEAT_WINDOW_DAYS, today);
  const sinceKey = dateKeys[0];

  const state = useAsync(async () => {
    const roster = await listRoster(client, classroomId);
    const heat = await getRosterHeat(client, classroomId, sinceKey);
    const lateSync = await getLateSyncSessions(
      client,
      roster.map((r) => r.student_id),
      `${sinceKey}T00:00:00Z`,
    );
    return { roster, heat, annotations: lateSyncAnnotations(lateSync) };
  }, [classroomId, sinceKey]);

  if (state.status === "loading") {
    return <p className="text-slate-500">Loading roster…</p>;
  }
  if (state.status === "error") {
    return (
      <p role="alert" className="text-red-600">
        Could not load the roster: {state.message}
      </p>
    );
  }

  const { roster, heat, annotations } = state.data;
  if (roster.length === 0) {
    return (
      <p className="text-slate-500">
        No students are actively enrolled in this classroom yet. Students join
        with the classroom code, from inside the app, after the consent screen.
      </p>
    );
  }

  const cellByStudentDay = new Map<string, RosterHeatRow>();
  for (const row of heat) {
    cellByStudentDay.set(`${row.student_id}|${row.date_key}`, row);
  }
  const studentsWithData = new Set(heat.map((r) => r.student_id));
  const annotationByStudent = new Map<string, LateSyncAnnotation>(
    annotations.map((a) => [a.student_id, a]),
  );

  return (
    <div className="overflow-x-auto">
      <table className="border-separate border-spacing-1 text-sm">
        <thead>
          <tr>
            <th className="pr-3 text-left font-medium text-slate-500">
              Student
            </th>
            {dateKeys.map((k) => (
              <th
                key={k}
                scope="col"
                className="w-12 text-center text-xs font-normal text-slate-400"
              >
                {dayLabel(k)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {roster.map((student) => {
            const offline = !studentsWithData.has(student.student_id);
            const annotation = annotationByStudent.get(student.student_id);
            return (
              <tr key={student.student_id}>
                <th scope="row" className="pr-3 text-left font-normal">
                  <button
                    onClick={() =>
                      onOpenStudent(student.student_id, student.display_name)
                    }
                    className="text-slate-900 underline-offset-2 hover:underline"
                  >
                    {student.display_name ?? "Student"}
                  </button>
                  {offline && (
                    <span className="ml-2 rounded bg-slate-100 px-1.5 py-0.5 text-xs text-slate-500">
                      practicing offline
                    </span>
                  )}
                  {annotation && (
                    <span
                      className="ml-2 rounded bg-sky-50 px-1.5 py-0.5 text-xs text-sky-700"
                      title="Sessions arrived well after they were played — worth knowing, not a problem by itself."
                    >
                      {describeLateSync(annotation)}
                    </span>
                  )}
                </th>
                {dateKeys.map((k) => (
                  <HeatCell
                    key={k}
                    row={cellByStudentDay.get(`${student.student_id}|${k}`)}
                    offline={offline}
                  />
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
      <p className="mt-3 text-xs text-slate-400">
        Cell color is minutes of actual sound (played time), never minutes the
        app was open. Hover a cell for both numbers. Blank cells mean no synced
        practice that day — absence, not zero.
      </p>
    </div>
  );
}

/** 4-step green scale by played minutes; amber ring = the view's
 * integrity_flag (thin sessions or high silence that day). */
function heatClass(playedMin: number | null): string {
  if (playedMin === null) return "bg-slate-50";
  if (playedMin >= 30) return "bg-emerald-600 text-white";
  if (playedMin >= 15) return "bg-emerald-400 text-emerald-950";
  if (playedMin >= 5) return "bg-emerald-200 text-emerald-950";
  return "bg-emerald-50 text-emerald-900";
}

function HeatCell({
  row,
  offline,
}: {
  row: RosterHeatRow | undefined;
  offline: boolean;
}) {
  if (!row) {
    // Honest absence: no synced practice this day — blank, never "0".
    return (
      <td
        className="h-9 w-12 rounded bg-slate-50"
        aria-label={offline ? "not syncing" : "no synced practice"}
      />
    );
  }
  const played = fmtMinutes(row.played_min);
  const wall = fmtMinutes(row.wall_min);
  return (
    <td
      className={`h-9 w-12 rounded text-center align-middle ${heatClass(row.played_min)} ${
        row.integrity_flag ? "ring-2 ring-amber-400" : ""
      }`}
      title={`${played} min played · ${wall} min open`}
    >
      {played}
    </td>
  );
}
