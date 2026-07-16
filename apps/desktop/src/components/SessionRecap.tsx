import type { ReactNode } from "react";
import { usePracticeStore } from "../stores/practiceStore";
import { confidenceDots } from "../lib/confidenceDots";
import type {
  ChartEntry,
  GrooveDescriptor,
  IntonationSummary,
} from "../types/brain";
import { keyName } from "../types/brain";
import ToneSummary from "./ToneSummary";

const DEGREE_NAMES = [
  "tonic",
  "flat 2nd",
  "2nd",
  "minor 3rd",
  "major 3rd",
  "4th",
  "tritone",
  "5th",
  "minor 6th",
  "major 6th",
  "minor 7th",
  "leading tone",
];

/**
 * One quiet line summarising intonation: overall direction + in-tune ratio,
 * plus the single most out-of-tune degree when a key anchored the analysis.
 * Mirrors the computed facts the recap was grounded on — no invented numbers.
 */
function intonationLine(s: IntonationSummary): string {
  const direction =
    s.mean_cents > 1
      ? "tends sharp"
      : s.mean_cents < -1
        ? "tends flat"
        : "centered";
  const sign = s.mean_cents >= 0 ? "+" : "";
  const pct = Math.round(s.in_tune_ratio * 100);
  let line = `mean ${sign}${Math.round(s.mean_cents)} cents (${direction}), ${pct}% in tune`;
  const worst = s.tendencies
    .filter((t) => t.count >= 2 && Math.abs(t.mean_cents) >= 5)
    .sort((a, b) => Math.abs(b.mean_cents) - Math.abs(a.mean_cents))[0];
  if (worst) {
    const degree = DEGREE_NAMES[worst.semitones_from_tonic] ?? "degree";
    const dir = worst.mean_cents >= 0 ? "sharp" : "flat";
    const wSign = worst.mean_cents >= 0 ? "+" : "";
    line += `; the ${degree} ran ${wSign}${Math.round(worst.mean_cents)} cents (${dir})`;
  }
  return line;
}

/** One quiet line summarising rhythmic feel: tempo, swing, steadiness. */
function grooveLine(g: GrooveDescriptor): string {
  const parts: string[] = [];
  if (g.tempo_bpm != null) parts.push(`~${Math.round(g.tempo_bpm)} BPM`);
  if (g.swing_ratio != null) {
    parts.push(
      g.swing_ratio >= 1.25
        ? `swung ~${g.swing_ratio.toFixed(1)}:1`
        : "straight feel",
    );
  }
  const steadiness =
    g.timing_consistency >= 0.9
      ? "steady"
      : g.timing_consistency >= 0.7
        ? "mostly steady"
        : "uneven";
  parts.push(steadiness);
  return parts.join(", ");
}

/**
 * Post-session recap screen.
 *
 * Product invariant: strengths render BEFORE areas-to-improve in the
 * DOM. This is intentional (design doc §2) — the UI itself enforces
 * the "coach, don't judge" tone, not just the LLM prompt.
 *
 * Empty-state handling: when `recap.phrase_count === 0` we show the
 * calm "come back when you're ready" copy. The backend already
 * populates this shape from the empty-state recap helper, so here
 * we just render what we're given.
 *
 * Error handling: `recapError` is present when `end_practice_session`
 * threw. The user still lands on this screen (design invariant — the
 * session happened) with a short fallback message and the same two
 * action buttons.
 */
export default function SessionRecap() {
  const recap = usePracticeStore((s) => s.recap);
  const jamChart = usePracticeStore((s) => s.jamChart);
  const recapError = usePracticeStore((s) => s.recapError);
  const returnToSelector = usePracticeStore((s) => s.returnToSelector);
  const exploreMeasure = usePracticeStore((s) => s.exploreMeasure);
  const bridgeNotice = usePracticeStore((s) => s.bridgeNotice);

  // Error path — takes precedence over any partial recap.
  if (recapError) {
    return (
      <RecapScreen>
        <section
          className="mx-auto flex max-w-2xl flex-col items-center gap-6 px-4 py-12 text-gray-100"
          data-testid="session-recap"
          data-variant="error"
        >
          <h2 className="text-2xl font-semibold">Recap unavailable</h2>
          <p className="text-center text-gray-300">
            I had trouble generating your recap, but your session is safe. Come
            back whenever you're ready to play again.
          </p>
          <RecapActions onDone={returnToSelector} />
        </section>
      </RecapScreen>
    );
  }

  if (!recap) {
    // Shouldn't happen in the normal flow — the router only sends us
    // here after endSession sets either `recap` or `recapError`.
    return (
      <RecapScreen>
        <section
          className="mx-auto flex max-w-2xl flex-col items-center gap-6 px-4 py-12 text-gray-100"
          data-testid="session-recap"
          data-variant="empty"
        >
          <p>No recap yet.</p>
          <RecapActions onDone={returnToSelector} />
        </section>
      </RecapScreen>
    );
  }

  const durationMinutes = Math.max(1, Math.round(recap.duration_secs / 60));
  const isEmptyState = recap.phrase_count === 0;
  // The session's unified musical fingerprint. Each dimension is read from here
  // and rendered only when present (the backend gates decide presence).
  const fingerprint = recap.fingerprint;

  return (
    <RecapScreen>
      <section
        className="mx-auto flex max-w-2xl flex-col gap-6 px-4 py-12 text-gray-100"
        data-testid="session-recap"
        data-variant={isEmptyState ? "empty" : "summary"}
      >
        <header className="flex flex-col gap-1">
          <h2 className="text-2xl font-semibold">Nice session.</h2>
          <p className="text-sm text-gray-400">
            {recap.instrument
              ? `${recap.instrument} · ${durationMinutes} minute${
                  durationMinutes === 1 ? "" : "s"
                }`
              : `${durationMinutes} minute${durationMinutes === 1 ? "" : "s"}`}
          </p>
          {/* Detected key/mode, stated only as firmly as the live tracking
            earned (#316): asserted keys read flat, late-settling keys lean,
            and a session that never settled says so instead of guessing.
            A missing key_claim is a pre-#316 recap → the flat form. */}
          {!isEmptyState && fingerprint?.key && (
            <p className="text-sm text-gray-400" data-testid="recap-key">
              Key:{" "}
              <span className="text-gray-200">
                {fingerprint.key_claim === "leaning"
                  ? `leaning ${keyName(fingerprint.key)} toward the end`
                  : keyName(fingerprint.key)}
              </span>
            </p>
          )}
          {/* "Kept moving" only when the tracking actually observed motion
            (the explicit unsettled claim) — a percussive/groove-only session
            with no tonal readings says nothing about key at all. */}
          {!isEmptyState && fingerprint?.key_claim === "unsettled" && (
            <p className="text-sm text-gray-400" data-testid="recap-key-unsettled">
              Key:{" "}
              <span className="text-gray-200">
                kept moving — normal for exploratory playing
              </span>
            </p>
          )}
        </header>

        <p
          className="text-lg leading-relaxed text-gray-200"
          data-testid="recap-assessment"
        >
          {recap.overall_assessment}
        </p>

        {/* Score-practice summary (#337 S4): honest accuracy over the notes
          the follower judged, and the measures that most need work. The
          worst-measure rows carry the RV bridge (#337 S5): one tap rows
          that measure through 12 keys. */}
        {!isEmptyState && recap.score_summary && (
          <div
            className="rounded-md border border-gray-700 bg-gray-900/60 p-3"
            data-testid="recap-score-summary"
          >
            <p className="text-sm text-gray-200">
              <span className="font-semibold">
                {recap.score_summary.score_title}
              </span>
              {" — "}
              {Math.round(recap.score_summary.accuracy_pct)}% of{" "}
              {recap.score_summary.judged} notes clean, as the app followed
              along.
            </p>
            {bridgeNotice && (
              <p
                className="mt-2 text-xs italic text-amber-300"
                data-testid="bridge-notice"
                role="status"
              >
                {bridgeNotice}
              </p>
            )}
            {recap.score_summary.worst_measures.length > 0 && (
              <ul className="mt-2 flex flex-col gap-1.5">
                {recap.score_summary.worst_measures.map((m) => (
                  <li
                    key={m.measure_number}
                    className="flex items-center justify-between gap-3 text-sm text-gray-300"
                    data-testid={`worst-measure-${m.measure_number}`}
                  >
                    <span>
                      Measure {m.measure_number} —{" "}
                      {[
                        m.missed > 0 ? `${m.missed} missed` : null,
                        m.near > 0 ? `${m.near} rough` : null,
                      ]
                        .filter(Boolean)
                        .join(", ")}
                    </span>
                    <button
                      type="button"
                      onClick={() => void exploreMeasure(m.measure_number)}
                      data-testid={`row-measure-${m.measure_number}`}
                      className="shrink-0 rounded bg-emerald-700/90 px-2 py-1 text-xs font-semibold text-white hover:bg-emerald-600"
                    >
                      🎲 Row it through 12 keys
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        {/* #349 T4a / #390: the jam's chord chart sketch — the label sequence
          the room played, timestamped, with the same honesty cues as the lane.
          Consecutive unresolved entries collapse into ONE muted span with the
          stretch's duration, so a mumbly recording reads like a chart, not a
          log; a session that resolved nothing gets one honest line instead. */}
        {jamChart && jamChart.length > 0 && (
          <div
            className="rounded border border-gray-700 bg-gray-800/60 p-3"
            data-testid="recap-chord-chart"
          >
            <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-gray-400">
              What the room played
            </h3>
            {jamChart.some((e) => !e.unresolved) ? (
              <ol className="flex flex-wrap gap-x-4 gap-y-1">
                {chartRows(jamChart).map((row, i) =>
                  row.kind === "chord" ? (
                    <li
                      key={`${row.entry.at_secs}-${i}`}
                      data-testid="chart-chord"
                      className="flex items-baseline gap-1.5 text-sm"
                    >
                      <span className="font-mono text-[10px] text-gray-500">
                        {formatChartTime(row.entry.at_secs)}
                      </span>
                      <span className="font-semibold text-gray-100">
                        {row.entry.label}
                        <span className="ml-1 text-[9px] tracking-widest text-gray-500">
                          {confidenceDots(row.entry.confidence)}
                        </span>
                      </span>
                    </li>
                  ) : (
                    <li
                      key={`${row.atSecs}-${i}`}
                      data-testid="chart-unresolved"
                      className="flex items-baseline gap-1.5 text-sm"
                    >
                      <span className="font-mono text-[10px] text-gray-500">
                        {formatChartTime(row.atSecs)}
                      </span>
                      <span className="italic text-gray-500">
                        · · ·
                        {row.durationSecs >= 1
                          ? ` (${formatUnclearSpan(row.durationSecs)} unclear)`
                          : ""}
                      </span>
                    </li>
                  ),
                )}
              </ol>
            ) : (
              <p
                className="text-sm italic text-gray-400"
                data-testid="chart-all-unclear"
              >
                Mostly blended sound — nothing settled long enough to name.
              </p>
            )}
            <p className="mt-2 text-[10px] text-gray-600">
              Labels only — nothing was recorded or sent anywhere.
            </p>
          </div>
        )}

        {/* Tone read-out (secondary to the coaching text above). Only shown
          when tone analysis produced a session aggregate. */}
        {!isEmptyState && fingerprint?.tone && (
          <ToneSummary tone={fingerprint.tone} />
        )}

        {/* Intonation read-out — quiet, factual. Only shown when enough notes
          were observed to report honestly (backend gate). */}
        {!isEmptyState && fingerprint?.intonation && (
          <p className="text-sm text-gray-400" data-testid="recap-intonation">
            Intonation:{" "}
            <span className="text-gray-200">
              {intonationLine(fingerprint.intonation)}
            </span>
          </p>
        )}

        {/* Rhythmic-feel read-out — quiet, factual. Only shown when enough
          onsets were observed to estimate groove (backend gate). */}
        {!isEmptyState && fingerprint?.groove && (
          <p className="text-sm text-gray-400" data-testid="recap-groove">
            Feel:{" "}
            <span className="text-gray-200">
              {grooveLine(fingerprint.groove)}
            </span>
          </p>
        )}

        {/* Flavour read-out — quiet, hedged, theory-grounded. Derived in the
          Rust core from the reliable signal the app measures (key mode + swing
          ratio), not the placeholder idiom corpus (#208/#209). Shown only when
          there's a clear signal (`recap.flavour` present); silent otherwise
          ("silence > lies"). */}
        {!isEmptyState && recap.flavour && (
          <p className="text-sm text-gray-400" data-testid="recap-flavour">
            Flavour: <span className="text-gray-200">{recap.flavour}</span>
          </p>
        )}

        {/* Cross-genre connections — a quiet, warm bridge from what they played
          to the music in their world. Shown ONLY when the Rust core grounded
          one (profile present + enough measured signal + the coach returned a
          hedged reference). Empty/absent → nothing rendered, mirroring the
          tone/key/intonation/groove treatment: silence over a hollow link. */}
        {!isEmptyState &&
          recap.connections != null &&
          recap.connections.length > 0 && (
            <div data-testid="recap-connections">
              <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-gray-400">
                In your world
              </h3>
              <ul className="list-disc space-y-1 pl-6 text-gray-200">
                {recap.connections.map((c) => (
                  <li key={c}>{c}</li>
                ))}
              </ul>
            </div>
          )}

        {/* Strengths first — product invariant enforced by DOM order. */}
        {recap.strengths.length > 0 && (
          <div data-testid="recap-strengths">
            <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-gray-400">
              Strengths
            </h3>
            <ul className="list-disc space-y-1 pl-6">
              {recap.strengths.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </div>
        )}

        {recap.areas_to_improve.length > 0 && (
          <div data-testid="recap-areas">
            <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-gray-400">
              Areas to work on
            </h3>
            <ul className="list-disc space-y-1 pl-6">
              {recap.areas_to_improve.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </div>
        )}

        {recap.next_session_suggestions.length > 0 && (
          <div data-testid="recap-next">
            <h3 className="mb-2 text-sm font-semibold uppercase tracking-wider text-gray-400">
              Next time, try
            </h3>
            <ul className="list-disc space-y-1 pl-6">
              {recap.next_session_suggestions.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </div>
        )}

        <RecapActions onDone={returnToSelector} />
      </section>
    </RecapScreen>
  );
}

/** m:ss for chart timestamps. */
function formatChartTime(secs: number): string {
  const m = Math.floor(secs / 60);
  const ss = Math.floor(secs % 60);
  return `${m}:${ss.toString().padStart(2, "0")}`;
}

type ChartRow =
  | { kind: "chord"; entry: ChartEntry }
  | { kind: "unclear"; atSecs: number; durationSecs: number };

/**
 * Collapse consecutive unresolved entries into one span (#390). The span's
 * duration is the observed first→last reading of the run — never extended
 * to the next label or session end, because the recorder writes nothing
 * during silence, so anything past the last reading may be rest and the
 * chart must not bill rest as unclear sound. A lone blip spans 0 s and
 * renders without a duration claim.
 */
function chartRows(entries: ChartEntry[]): ChartRow[] {
  const rows: ChartRow[] = [];
  let i = 0;
  while (i < entries.length) {
    const e = entries[i];
    if (!e.unresolved) {
      rows.push({ kind: "chord", entry: e });
      i += 1;
      continue;
    }
    const start = e.at_secs;
    let last = start;
    while (i < entries.length && entries[i].unresolved) {
      last = entries[i].at_secs;
      i += 1;
    }
    rows.push({ kind: "unclear", atSecs: start, durationSecs: last - start });
  }
  return rows;
}

/** Impressionistic duration for a collapsed unclear span: "45s" / "2m". */
function formatUnclearSpan(secs: number): string {
  const s = Math.round(secs);
  return s < 60 ? `${s}s` : `${Math.round(secs / 60)}m`;
}



/**
 * Full-height dark surface for the recap. Every other screen owns its own
 * `bg-gray-900` (PracticeShell mounts each screen bare), so without this the
 * recap's light text rendered on the browser's default white — unreadable
 * (#186). Kept as a wrapper so all three recap branches share one surface.
 */
function RecapScreen({ children }: { children: ReactNode }) {
  return <div className="min-h-screen bg-gray-900">{children}</div>;
}

/**
 * Two-button action row for the recap screen. Both buttons go to
 * selector — the labels differentiate framing ("Practice again" is a
 * positive invitation, "Done" is a neutral close).
 */
function RecapActions({ onDone }: { onDone: () => void }) {
  return (
    <div className="mt-4 flex gap-3">
      <button
        type="button"
        onClick={onDone}
        data-testid="recap-practice-again"
        className="rounded border border-blue-500 bg-blue-600/40 px-4 py-2 text-sm text-blue-100 hover:bg-blue-600/60"
      >
        Practice again
      </button>
      <button
        type="button"
        onClick={onDone}
        data-testid="recap-done"
        className="rounded border border-gray-500 bg-gray-700 px-4 py-2 text-sm text-gray-100 hover:bg-gray-600"
      >
        Done
      </button>
    </div>
  );
}
