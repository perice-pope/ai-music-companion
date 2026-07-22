import type { DashboardClient } from "../lib/supabase";
import {
  getIntegrityRows,
  getRosterHeat,
  getSessionEvidence,
  listRoster,
} from "../lib/queries";
import {
  chipsFor,
  describeRetries,
  describeSession,
  describeSplitDay,
  describeToolSpans,
  NOMINATE_RULE,
  NOMINATE_SUBLINE,
  splitDays,
  toolOnNoNoteSpans,
} from "../lib/integrity";
import { fmtCount, fmtDateTime, fmtRatio, recentDateKeys } from "../lib/format";
import { useAsync } from "../lib/useAsync";

const WINDOW_DAYS = 14;

/**
 * The engagement-integrity panel (spec AC8-AC9, datamodel §4d and the §5
 * fudge-vector table). Every row is EVIDENCE a fudge vector left behind —
 * F1 walk-away, F4 retry-farming, F6 session-splitting, F8
 * tool-on-no-notes — surfaced calmly. The rule is pinned in the header
 * and enforced by the copy: metrics nominate, humans judge. Nothing here
 * issues a verdict, and the word for what a student did is always a
 * description of the data, never an accusation.
 */
export function IntegrityPanel({
  client,
  classroomId,
  today,
}: {
  client: DashboardClient;
  classroomId: string;
  today?: Date;
}) {
  const sinceKey = recentDateKeys(WINDOW_DAYS, today)[0];

  const state = useAsync(async () => {
    const roster = await listRoster(client, classroomId);
    const rosterIds = new Set(roster.map((r) => r.student_id));
    // v_session_integrity has no classroom column (0006 L725-744); RLS has
    // already scoped rows to this teacher's active students (and their own
    // sessions) — the roster filter narrows to the selected classroom.
    const allRows = await getIntegrityRows(client);
    const rows = allRows.filter((r) => rosterIds.has(r.student_id));
    const heat = await getRosterHeat(client, classroomId, sinceKey);
    const evidence = await getSessionEvidence(
      client,
      rows.map((r) => r.session_id),
    );
    return { rows, days: splitDays(heat), evidence };
  }, [classroomId, sinceKey]);

  return (
    <div>
      <div className="mb-4 rounded border border-slate-200 bg-slate-50 p-3">
        <p className="text-sm font-semibold text-slate-800">{NOMINATE_RULE}</p>
        <p className="text-xs text-slate-500">{NOMINATE_SUBLINE}</p>
      </div>

      {state.status === "loading" && (
        <p className="text-slate-500">Gathering evidence rows…</p>
      )}
      {state.status === "error" && (
        <p role="alert" className="text-red-600">
          Could not load the integrity panel: {state.message}
        </p>
      )}
      {state.status === "ready" && (
        <>
          {state.data.days.length > 0 && (
            <section className="mb-5">
              <h3 className="mb-1 text-sm font-medium text-slate-700">
                Split-up days (last {WINDOW_DAYS} days)
              </h3>
              <ul className="space-y-0.5 text-sm text-slate-700">
                {state.data.days.map((d) => (
                  <li key={`${d.student_id}|${d.date_key}`}>
                    <span className="font-medium">
                      {d.display_name ?? "Student"}
                    </span>{" "}
                    · {d.date_key} · {describeSplitDay(d)}
                  </li>
                ))}
              </ul>
            </section>
          )}

          {state.data.rows.length === 0 ? (
            <p className="text-slate-500">
              Nothing to look at — no sessions in this classroom currently meet
              the evidence thresholds.
            </p>
          ) : (
            <table className="w-full text-left text-sm">
              <thead>
                <tr className="border-b border-slate-200 text-xs text-slate-500">
                  <th className="py-1.5 pr-3 font-medium">Student</th>
                  <th className="py-1.5 pr-3 font-medium">When</th>
                  <th className="py-1.5 pr-3 font-medium">
                    What the data shows
                  </th>
                  <th className="py-1.5 pr-3 font-medium">Notes</th>
                  <th className="py-1.5 pr-3 font-medium">Silence</th>
                  <th className="py-1.5 pr-3 font-medium">Graded</th>
                  <th className="py-1.5 font-medium">Observations</th>
                </tr>
              </thead>
              <tbody>
                {state.data.rows.map((row) => {
                  const chips = chipsFor(row);
                  const f8 = toolOnNoNoteSpans(
                    state.data.evidence.events,
                    state.data.evidence.phrases,
                    row.session_id,
                  );
                  return (
                    <tr
                      key={row.session_id}
                      className="border-b border-slate-100 align-top"
                    >
                      <td className="py-1.5 pr-3 whitespace-nowrap">
                        {row.display_name ?? "Student"}
                      </td>
                      <td className="py-1.5 pr-3 whitespace-nowrap">
                        {fmtDateTime(row.started_at)}
                      </td>
                      <td className="py-1.5 pr-3">
                        {describeSession(row.wall_min, row.played_min)}
                      </td>
                      <td className="py-1.5 pr-3">
                        {fmtCount(row.note_count)}
                      </td>
                      <td className="py-1.5 pr-3">
                        {fmtRatio(row.silence_ratio)}
                      </td>
                      <td className="py-1.5 pr-3">{fmtCount(row.graded)}</td>
                      <td className="py-1.5">
                        <span className="flex flex-wrap gap-1">
                          {chips.walkAway && (
                            <Chip label="Open with almost no sound" />
                          )}
                          {chips.retryFarming && row.max_retries !== null && (
                            <Chip label={describeRetries(row.max_retries)} />
                          )}
                          {f8.length > 0 && (
                            <Chip label={describeToolSpans(f8)} />
                          )}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </>
      )}
    </div>
  );
}

function Chip({ label }: { label: string }) {
  return (
    <span className="rounded bg-amber-50 px-1.5 py-0.5 text-xs text-amber-800">
      {label}
    </span>
  );
}
