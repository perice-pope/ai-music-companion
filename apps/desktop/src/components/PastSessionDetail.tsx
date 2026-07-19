import type { StoredSessionDto } from "../types/brain";

/**
 * #445-8: a PAST session's recap, read-only — renders exactly what the
 * stored recap carries and invents nothing. Empty lists simply don't
 * render their section (honest absence); the score block appears only
 * when the session judged a score.
 */
export default function PastSessionDetail({
  session,
  onBack,
}: {
  session: StoredSessionDto;
  onBack: () => void;
}) {
  const { recap } = session;
  const date = new Date(session.started_at).toLocaleDateString();
  const time = new Date(session.started_at).toLocaleTimeString();
  const durationMins = Math.round(recap.duration_secs / 60);

  return (
    <div data-testid="past-session-detail">
      <button
        type="button"
        onClick={onBack}
        data-testid="past-session-back"
        className="mb-4 text-sm text-gray-400 underline-offset-2 hover:text-gray-200 hover:underline"
      >
        ← All sessions
      </button>
      <div className="rounded-lg border border-gray-700 bg-gray-800 p-6">
        <h3 className="text-xl font-semibold text-white">
          {recap.instrument} — {durationMins}m
        </h3>
        <p className="text-xs text-gray-400">
          {date} at {time} · {recap.phrase_count} phrase
          {recap.phrase_count !== 1 ? "s" : ""}
        </p>
        <p
          className="mt-4 leading-relaxed text-gray-200"
          data-testid="past-session-assessment"
        >
          {recap.overall_assessment}
        </p>
        {recap.strengths.length > 0 && (
          <section className="mt-4" data-testid="past-session-strengths">
            <h4 className="text-sm font-semibold uppercase tracking-wider text-emerald-400/80">
              What worked
            </h4>
            <ul className="mt-1 list-inside list-disc text-sm text-gray-300">
              {recap.strengths.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </section>
        )}
        {recap.areas_to_improve.length > 0 && (
          <section className="mt-4" data-testid="past-session-areas">
            <h4 className="text-sm font-semibold uppercase tracking-wider text-amber-400/80">
              Worth another look
            </h4>
            <ul className="mt-1 list-inside list-disc text-sm text-gray-300">
              {recap.areas_to_improve.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </section>
        )}
        {recap.next_session_suggestions.length > 0 && (
          <section className="mt-4" data-testid="past-session-suggestions">
            <h4 className="text-sm font-semibold uppercase tracking-wider text-sky-400/80">
              Next time, try
            </h4>
            <ul className="mt-1 list-inside list-disc text-sm text-gray-300">
              {recap.next_session_suggestions.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
          </section>
        )}
        {recap.score_summary && (
          <section className="mt-4" data-testid="past-session-score">
            <h4 className="text-sm font-semibold uppercase tracking-wider text-indigo-400/80">
              Score practice
            </h4>
            <p className="mt-1 text-sm text-gray-300">
              {recap.score_summary.score_title} —{" "}
              {recap.score_summary.accuracy_pct}% over{" "}
              {recap.score_summary.judged} judged notes
            </p>
          </section>
        )}
      </div>
    </div>
  );
}
