import { usePracticeStore } from "../stores/practiceStore";

/**
 * #453 S3 / #454 S3 — the coaching box: a persistent free-play surface that
 * can speak two voices, showing AT MOST ONE thing at a time:
 *
 * - a history-grounded practice suggestion ("your Eb rows are sitting at
 *   54% over 6 attempts…") — the #453 voice, and
 * - a method-book tip the last session's measured evidence earned
 *   ("there are drills for exactly this in Schlossberg's…") — the #454
 *   voice, always with its attribution line visible.
 *
 * DISPLAY POLICY: history outranks the tip when both exist — the history
 * claim is about THIS player's measured trajectory, while the book tip
 * generalizes one session's fingerprint to canonical technique guidance;
 * the specific, personal claim wins the single slot. The tip fills the box
 * otherwise.
 *
 * The box lives by the reveal's rule 0: it HOLDS its state (empty fetches
 * never clear — the store enforces that, per voice), newer results replace
 * in place, and one dismissal quiets BOTH voices for the session.
 *
 * Calm by design (founder: "not too bright and alarming"): muted violet —
 * the amber alarm palette stays the reveal's. Text renders verbatim
 * (silence > lies: the history text embeds its own citations, the tip's
 * guidance is the attributed founder-voice paraphrase). When neither voice
 * has anything to say the box renders NOTHING at all — no empty chrome.
 */
export default function CoachingBox() {
  const suggestion = usePracticeStore((s) => s.coachingSuggestion);
  const tip = usePracticeStore((s) => s.coachingTip);
  const dismiss = usePracticeStore((s) => s.dismissCoachingSuggestion);

  if (!suggestion && !tip) {
    return null;
  }

  // History outranks the book tip — see the display policy above.
  const showingHistory = suggestion !== null;

  return (
    <div
      data-testid="coaching-box"
      role="status"
      aria-live="polite"
      aria-label={
        showingHistory ? "Practice history suggestion" : "Method-book tip"
      }
      className="w-64 rounded-lg border border-violet-800 bg-violet-950/40 p-4 shadow-lg"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1">
          <p className="text-xs font-semibold uppercase tracking-wider text-violet-300/80">
            {showingHistory
              ? "From your practice history"
              : "From the method books"}
          </p>
          <p
            data-testid="coaching-box-text"
            className="mt-1 text-sm leading-relaxed text-violet-200"
          >
            {showingHistory ? suggestion.text : tip?.guidance}
          </p>
          {!showingHistory && tip && (
            // Attribution is non-negotiable (#454's copyright posture):
            // the source line is always visible with the guidance.
            <p
              data-testid="coaching-box-attribution"
              className="mt-1.5 text-xs italic text-violet-400/70"
            >
              — {tip.source_line}
            </p>
          )}
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
