import { usePracticeStore } from "../stores/practiceStore";
import ToneSummary from "./ToneSummary";

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
  const recapError = usePracticeStore((s) => s.recapError);
  const returnToSelector = usePracticeStore((s) => s.returnToSelector);

  // Error path — takes precedence over any partial recap.
  if (recapError) {
    return (
      <section
        className="mx-auto flex max-w-2xl flex-col items-center gap-6 px-4 py-12 text-gray-100"
        data-testid="session-recap"
        data-variant="error"
      >
        <h2 className="text-2xl font-semibold">Recap unavailable</h2>
        <p className="text-center text-gray-300">
          I had trouble generating your recap, but your session is safe.
          Come back whenever you're ready to play again.
        </p>
        <RecapActions onDone={returnToSelector} />
      </section>
    );
  }

  if (!recap) {
    // Shouldn't happen in the normal flow — the router only sends us
    // here after endSession sets either `recap` or `recapError`.
    return (
      <section
        className="mx-auto flex max-w-2xl flex-col items-center gap-6 px-4 py-12 text-gray-100"
        data-testid="session-recap"
        data-variant="empty"
      >
        <p>No recap yet.</p>
        <RecapActions onDone={returnToSelector} />
      </section>
    );
  }

  const durationMinutes = Math.max(1, Math.round(recap.duration_secs / 60));
  const isEmptyState = recap.phrase_count === 0;

  return (
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
      </header>

      <p className="text-lg leading-relaxed text-gray-200" data-testid="recap-assessment">
        {recap.overall_assessment}
      </p>

      {/* Tone read-out (secondary to the coaching text above). Only shown
          when tone analysis produced a session aggregate. */}
      {!isEmptyState && recap.session_tone && (
        <ToneSummary tone={recap.session_tone} />
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
  );
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
