import type { DashboardClient } from "../lib/supabase";
import {
  getStudentExercises,
  getStudentSessions,
  getStudentToolEvents,
  type FactSessionRow,
  type ToolEventRow,
} from "../lib/queries";
import { toolSpans } from "../lib/integrity";
import {
  EM_DASH,
  fmtCount,
  fmtDateTime,
  fmtRatio,
  fmtSecsAsMin,
} from "../lib/format";
import { useAsync } from "../lib/useAsync";
import { CoverageMatrix } from "./CoverageMatrix";

/**
 * Per-student drill-down (spec AC5-AC7, datamodel §4b): session timeline
 * with the integrity columns (played vs wall SEPARATE, never conflated —
 * the #451 one-clock rule resold upward), the material × 12-key coverage
 * matrix, the tool-usage summary, and the stored recap facts (fingerprint
 * chips — blank when the device's evidence gate said nothing; the
 * dashboard never out-claims the recap).
 */
export function StudentDrilldown({
  client,
  studentId,
  displayName,
  onBack,
}: {
  client: DashboardClient;
  studentId: string;
  displayName: string | null;
  onBack: () => void;
}) {
  const state = useAsync(async () => {
    const sessions = await getStudentSessions(client, studentId);
    const exercises = await getStudentExercises(client, studentId);
    const toolEvents = await getStudentToolEvents(client, studentId);
    return { sessions, exercises, toolEvents };
  }, [studentId]);

  return (
    <div>
      <button
        onClick={onBack}
        className="mb-4 text-sm text-slate-500 hover:text-slate-900"
      >
        &larr; Back to roster
      </button>
      <h2 className="text-lg font-semibold text-slate-900">
        {displayName ?? "Student"}
      </h2>

      {state.status === "loading" && (
        <p className="mt-4 text-slate-500">Loading practice data…</p>
      )}
      {state.status === "error" && (
        <p role="alert" className="mt-4 text-red-600">
          Could not load this student&apos;s data: {state.message}
        </p>
      )}
      {state.status === "ready" && (
        <div className="mt-4 space-y-8">
          <section>
            <h3 className="mb-2 font-medium text-slate-700">Sessions</h3>
            <SessionTable sessions={state.data.sessions} />
          </section>
          <section>
            <h3 className="mb-2 font-medium text-slate-700">
              Material coverage — the cell through 12 keys
            </h3>
            <CoverageMatrix exercises={state.data.exercises} />
          </section>
          <section>
            <h3 className="mb-2 font-medium text-slate-700">
              How they practiced
            </h3>
            <ToolUsageSummary events={state.data.toolEvents} />
          </section>
        </div>
      )}
    </div>
  );
}

function SessionTable({ sessions }: { sessions: FactSessionRow[] }) {
  if (sessions.length === 0) {
    return (
      <p className="text-sm text-slate-500">
        No synced sessions yet. This student may be practicing offline — absence
        of data here is absence of sync, not absence of practice.
      </p>
    );
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead>
          <tr className="border-b border-slate-200 text-xs text-slate-500">
            <th className="py-1.5 pr-3 font-medium">When</th>
            <th className="py-1.5 pr-3 font-medium">Open (min)</th>
            <th className="py-1.5 pr-3 font-medium">Played (min)</th>
            <th className="py-1.5 pr-3 font-medium">Notes</th>
            <th className="py-1.5 pr-3 font-medium">Silence</th>
            <th className="py-1.5 pr-3 font-medium">Phrases</th>
            <th className="py-1.5 pr-3 font-medium">Mode</th>
            <th className="py-1.5 font-medium">Heard</th>
          </tr>
        </thead>
        <tbody>
          {sessions.map((s) => (
            <tr key={s.session_id} className="border-b border-slate-100">
              <td className="py-1.5 pr-3 whitespace-nowrap">
                {fmtDateTime(s.started_at)}
              </td>
              <td className="py-1.5 pr-3">{fmtSecsAsMin(s.duration_secs)}</td>
              <td className="py-1.5 pr-3">{fmtSecsAsMin(s.played_secs)}</td>
              <td className="py-1.5 pr-3">{fmtCount(s.note_count)}</td>
              <td className="py-1.5 pr-3">{fmtRatio(s.silence_ratio)}</td>
              <td className="py-1.5 pr-3">{s.phrase_count}</td>
              <td className="py-1.5 pr-3">
                {s.practice_mode ?? EM_DASH}
                {s.instrument ? ` · ${s.instrument}` : ""}
              </td>
              <td className="py-1.5">
                <FingerprintChips fingerprint={s.fingerprint} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-2 text-xs text-slate-400">
        &ldquo;Open&rdquo; is wall-clock; &ldquo;played&rdquo; is time the
        instrument actually made sound — always shown side by side, never mixed.
        &ldquo;{EM_DASH}&rdquo; means the device didn&apos;t sync that number,
        not that it was zero.
      </p>
    </div>
  );
}

/** The stored recap facts: each fingerprint dimension the device was
 * willing to claim renders as a chip; a gated (null) dimension renders
 * nothing — silence over lies, same as the student's own recap. */
function FingerprintChips({
  fingerprint,
}: {
  fingerprint: FactSessionRow["fingerprint"];
}) {
  if (
    fingerprint === null ||
    typeof fingerprint !== "object" ||
    Array.isArray(fingerprint)
  ) {
    return null;
  }
  const chips = Object.entries(fingerprint).filter(
    (entry): entry is [string, string | number] =>
      typeof entry[1] === "string" || typeof entry[1] === "number",
  );
  if (chips.length === 0) return null;
  return (
    <span className="flex flex-wrap gap-1">
      {chips.map(([k, v]) => (
        <span
          key={k}
          className="rounded bg-slate-100 px-1.5 py-0.5 text-xs text-slate-600"
        >
          {k}: {String(v)}
        </span>
      ))}
    </span>
  );
}

interface ToolTotals {
  sessions: Set<string>;
  totalSecs: number;
  bpms: number[];
}

/** Summarize fact_tool_event rows (datamodel §4b "HOW they practiced"). */
export function ToolUsageSummary({ events }: { events: ToolEventRow[] }) {
  if (events.length === 0) {
    return (
      <p className="text-sm text-slate-500">
        No tool events synced — nothing known about metronome or band usage,
        which is different from &ldquo;not used&rdquo;.
      </p>
    );
  }

  const sessionsOf = (kinds: string[]) =>
    new Set(
      events.filter((e) => kinds.includes(e.kind)).map((e) => e.session_id),
    );

  const slice = events.map((e) => ({
    session_id: e.session_id,
    at_secs: e.at_secs,
    kind: e.kind,
    params: e.params,
  }));
  const spanSecs = (prefix: "pocket" | "band"): ToolTotals => {
    const totals: ToolTotals = { sessions: new Set(), totalSecs: 0, bpms: [] };
    for (const sid of new Set(slice.map((e) => e.session_id))) {
      for (const span of toolSpans(
        slice.filter((e) => e.kind.startsWith(prefix)),
        sid,
      )) {
        totals.totalSecs += span.end_secs - span.start_secs;
        totals.sessions.add(sid);
      }
    }
    return totals;
  };

  const pocket = spanSecs("pocket");
  for (const e of events) {
    if (e.kind.startsWith("pocket")) {
      const params = e.params;
      if (params && typeof params === "object" && !Array.isArray(params)) {
        const bpm = params["bpm"];
        if (typeof bpm === "number") pocket.bpms.push(bpm);
      }
    }
  }
  const band = spanSecs("band");
  const narrations = events.filter((e) => e.kind === "narration_used").length;
  const openers = sessionsOf(["opener_begin"]).size;
  const scores = sessionsOf(["score_open"]).size;

  const lines: string[] = [];
  if (pocket.sessions.size > 0) {
    const tempo =
      pocket.bpms.length > 0
        ? `, tempos ${Math.min(...pocket.bpms)}–${Math.max(...pocket.bpms)} bpm`
        : "";
    lines.push(
      `Metronome: on in ${pocket.sessions.size} session${pocket.sessions.size === 1 ? "" : "s"}, about ${Math.round(pocket.totalSecs / 60)} min total${tempo}`,
    );
  }
  if (band.sessions.size > 0) {
    lines.push(
      `Band (play-along): on in ${band.sessions.size} session${band.sessions.size === 1 ? "" : "s"}, about ${Math.round(band.totalSecs / 60)} min total`,
    );
  }
  if (openers > 0)
    lines.push(
      `Openers begun in ${openers} session${openers === 1 ? "" : "s"}`,
    );
  if (scores > 0)
    lines.push(`Scores opened in ${scores} session${scores === 1 ? "" : "s"}`);
  if (narrations > 0) lines.push(`Coaching narration used ${narrations}×`);
  if (lines.length === 0) {
    lines.push(
      "Tool events synced, but none of the known kinds — app versions may differ.",
    );
  }

  return (
    <ul className="list-inside list-disc space-y-1 text-sm text-slate-700">
      {lines.map((line) => (
        <li key={line}>{line}</li>
      ))}
    </ul>
  );
}
