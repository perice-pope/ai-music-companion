import type {
  IntegrityRow,
  LateSyncSessionRow,
  PhraseSliceRow,
  RosterHeatRow,
  ToolEventSliceRow,
} from "./queries";

/**
 * The fudge-vector metrics (datamodel doc §5) as pure, unit-tested
 * functions. This module is the acceptance-test spec of the integrity
 * panel made executable: each function is one row of the fudge table, with
 * its bar pinned here (and in docs/specs/449-t4-dashboard.md §4) so a
 * change to a threshold is a reviewed edit, not drift.
 *
 * Tone rule, inherited verbatim from teacher-audit and the datamodel §4d:
 * metrics NOMINATE, humans JUDGE. Everything returned here is evidence
 * ("42 min open, 3 min of sound"), never a verdict — no string in this
 * module (or anywhere in the app) accuses anyone.
 */

/** The rule line the integrity panel pins at the top (AC9). */
export const NOMINATE_RULE = "Metrics nominate, humans judge.";

/** The explanatory subline — calm, evidence-first. */
export const NOMINATE_SUBLINE =
  "These are observations worth a conversation, decided by you — never verdicts.";

// ── F1: walk-away sessions ──────────────────────────────────────────────────
// "Start a session, walk away for 30 min" → wall long, almost no sound.
// Bar: wall ≥ 10 min AND played < 60 s AND silence_ratio ≥ 0.9.
export function flagWalkAway(session: {
  wall_min: number | null;
  played_min: number | null;
  silence_ratio: number | null;
}): boolean {
  const wallMin = session.wall_min;
  const playedMin = session.played_min;
  const silence = session.silence_ratio;
  if (wallMin === null || playedMin === null || silence === null) {
    // Honest absence: without the integrity columns we cannot nominate.
    return false;
  }
  return wallMin >= 10 && playedMin * 60 < 60 && silence >= 0.9;
}

// ── F4: retry-farming ───────────────────────────────────────────────────────
// "Re-grade the same easy exercise to farm accuracy" → many fact_exercise
// rows sharing one (material_id, tonic). Bar: max_retries ≥ 6 in a session.
export const RETRY_FARM_BAR = 6;

export function flagRetryFarming(maxRetries: number | null): boolean {
  return maxRetries !== null && maxRetries >= RETRY_FARM_BAR;
}

// ── F6: session-splitting ───────────────────────────────────────────────────
// "Split practice into many 90-second sessions" → high session count, tiny
// played time each. Bar (per student-day): sessions ≥ 3 AND mean played
// per session < 120 s.
export interface SplitDay {
  student_id: string;
  display_name: string | null;
  date_key: string;
  sessions: number;
  playedMinPerSession: number;
}

export function flagSessionSplitting(day: {
  sessions: number;
  played_min: number | null;
}): boolean {
  if (day.played_min === null) return false;
  return day.sessions >= 3 && (day.played_min * 60) / day.sessions < 120;
}

export function splitDays(heatRows: RosterHeatRow[]): SplitDay[] {
  return heatRows
    .filter((r) => flagSessionSplitting(r))
    .map((r) => ({
      student_id: r.student_id,
      display_name: r.display_name,
      date_key: r.date_key,
      sessions: r.sessions,
      playedMinPerSession: (r.played_min ?? 0) / r.sessions,
    }));
}

// ── F8: tool-on-no-notes spans ──────────────────────────────────────────────
// "Run the metronome/band loudly and let the app hear practice" →
// fact_tool_event shows pocket/band running while phrase note_count inside
// the span is zero. Bar: a span ≥ 60 s overlapping zero detected notes.
// The range join is exactly the datamodel §5 F8 query, done client-side
// over the flagged sessions only.

const TOOL_SPAN_STARTS = new Set(["pocket_start", "band_start"]);
const TOOL_SPAN_STOPS = new Set(["pocket_stop", "band_stop"]);
export const TOOL_SPAN_MIN_SECS = 60;

export interface ToolSpan {
  session_id: string;
  start_secs: number;
  end_secs: number;
}

/** Pair *_start / *_stop events into spans (one clock: at_secs). An
 * unterminated span is closed at the last known event time — never
 * extended past the evidence. */
export function toolSpans(
  events: ToolEventSliceRow[],
  sessionId: string,
): ToolSpan[] {
  const inSession = events
    .filter((e) => e.session_id === sessionId)
    .sort((a, b) => a.at_secs - b.at_secs);
  const spans: ToolSpan[] = [];
  const open: Record<"pocket" | "band", number | null> = {
    pocket: null,
    band: null,
  };
  const lastAt = inSession.length ? inSession[inSession.length - 1].at_secs : 0;
  for (const e of inSession) {
    const tool = e.kind.startsWith("pocket") ? "pocket" : "band";
    if (TOOL_SPAN_STARTS.has(e.kind) && open[tool] === null) {
      open[tool] = e.at_secs;
    } else if (TOOL_SPAN_STOPS.has(e.kind) && open[tool] !== null) {
      spans.push({
        session_id: sessionId,
        start_secs: open[tool] as number,
        end_secs: e.at_secs,
      });
      open[tool] = null;
    }
  }
  for (const tool of ["pocket", "band"] as const) {
    const start = open[tool];
    if (start !== null && lastAt > start) {
      spans.push({
        session_id: sessionId,
        start_secs: start,
        end_secs: lastAt,
      });
    }
  }
  return spans;
}

function notesInRange(
  phrases: PhraseSliceRow[],
  sessionId: string,
  start: number,
  end: number,
): number {
  return phrases
    .filter(
      (p) =>
        p.session_id === sessionId && p.end_secs > start && p.start_secs < end,
    )
    .reduce((sum, p) => sum + (p.note_count ?? 0), 0);
}

/** F8: spans ≥ 60 s where a tool ran and zero notes were detected. */
export function toolOnNoNoteSpans(
  events: ToolEventSliceRow[],
  phrases: PhraseSliceRow[],
  sessionId: string,
): ToolSpan[] {
  return toolSpans(events, sessionId).filter(
    (s) =>
      s.end_secs - s.start_secs >= TOOL_SPAN_MIN_SECS &&
      notesInRange(phrases, sessionId, s.start_secs, s.end_secs) === 0,
  );
}

// ── F9: late sync ───────────────────────────────────────────────────────────
// "Practice with sync off all term, enable it before grading" → server
// created_at ≫ session started_at. Bar: arrival ≥ 48 h after play.
// Shown as an annotation, not punished (datamodel §5 F9, verbatim).
export const LATE_SYNC_HOURS = 48;

export interface LateSyncAnnotation {
  student_id: string;
  syncedOn: string; // date of latest late arrival (YYYY-MM-DD)
  playedFrom: string; // earliest late-arriving session's play date
  playedTo: string; // latest late-arriving session's play date
  sessionCount: number;
}

export function lateSyncAnnotations(
  rows: LateSyncSessionRow[],
): LateSyncAnnotation[] {
  const late = rows.filter(
    (r) =>
      new Date(r.created_at).getTime() - new Date(r.started_at).getTime() >=
      LATE_SYNC_HOURS * 3_600_000,
  );
  const byStudent = new Map<string, LateSyncSessionRow[]>();
  for (const r of late) {
    const list = byStudent.get(r.student_id) ?? [];
    list.push(r);
    byStudent.set(r.student_id, list);
  }
  const day = (iso: string): string => iso.slice(0, 10);
  return [...byStudent.entries()].map(([student_id, sessions]) => {
    const played = sessions.map((s) => day(s.started_at)).sort();
    const synced = sessions.map((s) => day(s.created_at)).sort();
    return {
      student_id,
      syncedOn: synced[synced.length - 1],
      playedFrom: played[0],
      playedTo: played[played.length - 1],
      sessionCount: sessions.length,
    };
  });
}

// ── Calm copy builders (evidence phrasing, never verdicts) ──────────────────

const fmtMin = (min: number): string =>
  min >= 10 ? String(Math.round(min)) : (Math.round(min * 10) / 10).toString();

/** "42 min open, 3 min of sound" — the wall-vs-played gap as a fact. */
export function describeSession(
  wallMin: number | null,
  playedMin: number | null,
): string {
  const wall = wallMin === null ? "—" : `${fmtMin(wallMin)} min open`;
  const played =
    playedMin === null
      ? "played time not synced"
      : playedMin * 60 < 60
        ? "under a minute of sound"
        : `${fmtMin(playedMin)} min of sound`;
  return `${wall}, ${played}`;
}

export function describeRetries(maxRetries: number): string {
  return `Same material graded ${maxRetries}× this session`;
}

export function describeSplitDay(day: SplitDay): string {
  return `${day.sessions} short sessions this day, about ${fmtMin(day.playedMinPerSession)} min of sound each`;
}

export function describeToolSpans(spans: ToolSpan[]): string {
  const totalMin =
    spans.reduce((sum, s) => sum + (s.end_secs - s.start_secs), 0) / 60;
  return `Metronome or band ran ${fmtMin(totalMin)} min with no notes detected`;
}

export function describeLateSync(a: LateSyncAnnotation): string {
  const range =
    a.playedFrom === a.playedTo
      ? `played ${a.playedFrom}`
      : `played ${a.playedFrom} to ${a.playedTo}`;
  return `Synced ${a.syncedOn}, ${range}`;
}

/** The chips one integrity row earns. F8 evidence arrives separately. */
export interface SessionChips {
  walkAway: boolean;
  retryFarming: boolean;
}

export function chipsFor(row: IntegrityRow): SessionChips {
  return {
    walkAway: flagWalkAway(row),
    retryFarming: flagRetryFarming(row.max_retries),
  };
}
