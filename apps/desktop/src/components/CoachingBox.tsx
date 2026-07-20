import { usePracticeStore } from "../stores/practiceStore";

/**
 * #453 S3 — the coaching box: a persistent free-play surface for AT MOST
 * ONE history-grounded practice suggestion ("your Eb rows are sitting at
 * 54% over 6 attempts…"), sitting just below the reveal card and living
 * by the reveal's rule 0: it HOLDS its state (an empty analyzer result
 * never clears it — the store enforces that), a newer suggestion replaces
 * it in place, and dismissing it quiets the box for the session.
 *
 * Calm by design (founder: "not too bright and alarming"): muted violet —
 * the amber alarm palette stays the reveal's. The suggestion text already
 * embeds its own evidence numbers (silence > lies, #453 S1), so the box
 * renders it verbatim. When history has nothing to say the box renders
 * NOTHING at all — no empty chrome below an empty reveal.
 */
export default function CoachingBox() {
  const suggestion = usePracticeStore((s) => s.coachingSuggestion);
  const dismiss = usePracticeStore((s) => s.dismissCoachingSuggestion);

  if (!suggestion) {
    return null;
  }

  return (
    <div
      data-testid="coaching-box"
      role="status"
      aria-live="polite"
      aria-label="Practice history suggestion"
      className="w-64 rounded-lg border border-violet-800 bg-violet-950/40 p-4 shadow-lg"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1">
          <p className="text-xs font-semibold uppercase tracking-wider text-violet-300/80">
            From your practice history
          </p>
          <p
            data-testid="coaching-box-text"
            className="mt-1 text-sm leading-relaxed text-violet-200"
          >
            {suggestion.text}
          </p>
        </div>
        <button
          type="button"
          data-testid="coaching-box-dismiss"
          onClick={dismiss}
          aria-label="Dismiss practice suggestion"
          className="mt-0.5 inline-flex h-6 w-6 items-center justify-center rounded-full text-sm font-bold text-violet-200 opacity-60 hover:opacity-100"
        >
          ×
        </button>
      </div>
    </div>
  );
}
