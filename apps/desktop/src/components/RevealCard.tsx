import { useEffect, useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";

/** How long a reveal lingers before fading on its own (ms). */
const REVEAL_LINGER_MS = 12000;

/**
 * Minimum time a reveal stays readable even when the live key confidently
 * moves off it (#277: cards were vanishing before they could be read).
 */
const MIN_READABLE_MS = 4000;

/**
 * A key change only dismisses a card when the NEW reading is at least this
 * confident — early-session detection wanders, and a wobble is not a
 * contradiction. Matches the "I hear" header's assert threshold (the point
 * where it drops the "maybe" hedge), so an asserted header can never sit
 * contradicting a lingering card.
 */
const DISMISS_MIN_CONFIDENCE = 0.55;

/**
 * A single real-world music "reveal" with an auto-dismiss timer. Slides in from
 * the right and fades after {@link REVEAL_LINGER_MS}, or on explicit dismiss.
 * Mirrors the coaching `TipCard` so the two share a calm, consistent voice.
 */
function RevealCardItem({
  id,
  concept,
  connection,
  why,
  onDismiss,
}: {
  id: string;
  concept: string;
  connection: string;
  why: string;
  onDismiss: (id: string) => void;
}) {
  const [isDismissing, setIsDismissing] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => {
      setIsDismissing(true);
      setTimeout(() => onDismiss(id), 300); // wait for fade-out
    }, REVEAL_LINGER_MS);

    return () => clearTimeout(timer);
  }, [id, onDismiss]);

  const handleDismiss = () => {
    setIsDismissing(true);
    setTimeout(() => onDismiss(id), 300);
  };

  return (
    <div
      data-testid={`reveal-${id}`}
      className={`
        transform transition-all duration-300 ease-out
        ${isDismissing ? "translate-x-full opacity-0" : "translate-x-0 opacity-100"}
      `}
    >
      <div className="rounded-lg border border-amber-700 bg-amber-900/40 p-4 shadow-lg">
        <div className="flex items-start justify-between gap-3">
          <div className="flex-1">
            <p className="text-xs font-semibold uppercase tracking-wider text-amber-300/80">
              In the wild · {concept}
            </p>
            <p className="mt-1 text-sm font-semibold leading-relaxed text-amber-100">
              {connection}
            </p>
            <p className="mt-1 text-sm leading-relaxed text-amber-200/90">
              {why}
            </p>
          </div>

          <button
            type="button"
            onClick={handleDismiss}
            data-testid={`reveal-dismiss-${id}`}
            className="mt-0.5 inline-flex h-6 w-6 items-center justify-center rounded-full text-sm font-bold text-amber-200 opacity-60 hover:opacity-100"
            aria-label={`Dismiss reveal: ${connection}`}
          >
            ×
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * The reveal card surface in free play. Shows only the most recent reveal (never
 * stacks), echoing {@link CoachingTipPanel}. While the player explores, the AI
 * occasionally names the real-world music that lives in what they're playing.
 */
export default function RevealCard() {
  const revealQueue = usePracticeStore((s) => s.revealQueue);
  const dismissReveal = usePracticeStore((s) => s.dismissReveal);
  const collectionCount = usePracticeStore((s) => s.collectionCount);
  const startExplore = usePracticeStore((s) => s.startExplore);
  // Subscribe to the live key's primitives (not the object), so this only
  // re-runs when the detected tonic/mode/confidence band actually change — not
  // on every ~8 Hz perception tick.
  const liveTonic = usePracticeStore((s) => s.perception?.key?.tonic ?? null);
  const liveMode = usePracticeStore((s) => s.perception?.key?.mode ?? null);
  const liveKeyConfident = usePracticeStore(
    (s) => (s.perception?.key?.confidence ?? 0) >= DISMISS_MIN_CONFIDENCE,
  );

  const current =
    revealQueue.length > 0 ? revealQueue[revealQueue.length - 1] : null;

  // Never let a lingering card contradict the live "I hear" header (#266) —
  // but never make it unreadable either (#277: early-session key detection
  // wanders, and dismissing on every wobble made cards vanish before they
  // could be read). So a card is dismissed for a key change only when the
  // contradicting reading is CONFIDENT, and never before it has been on
  // screen for a minimum readable dwell. A null/shaky key never dismisses.
  useEffect(() => {
    if (!current || liveTonic === null || liveMode === null) return;
    if (!liveKeyConfident) return; // a wobble is not a contradiction
    const movedOff =
      liveTonic !== current.reveal.tonic ||
      liveMode.toLowerCase() !== current.reveal.mode;
    if (!movedOff) return;

    const age = Date.now() - current.receivedAt;
    if (age >= MIN_READABLE_MS) {
      dismissReveal(current.id);
      return;
    }
    // Confident contradiction while the card is still young: let it finish its
    // readable dwell, then dismiss.
    const timer = setTimeout(
      () => dismissReveal(current.id),
      MIN_READABLE_MS - age,
    );
    return () => clearTimeout(timer);
  }, [current, liveTonic, liveMode, liveKeyConfident, dismissReveal]);

  if (!current) {
    return (
      <div
        className="h-28 w-64"
        data-testid="reveal-card-empty"
        aria-label="No music reveal right now"
      />
    );
  }

  return (
    <div
      className="w-64"
      data-testid="reveal-card"
      role="status"
      aria-live="polite"
      aria-label="Real-world music reveal"
    >
      <RevealCardItem
        id={current.id}
        concept={current.reveal.concept}
        connection={current.reveal.connection}
        why={current.reveal.why}
        onDismiss={dismissReveal}
      />
      {/* #255: the reveal becomes actionable — one tap turns the named sound
          into an RV variation on the free-play surface. */}
      <button
        type="button"
        onClick={() =>
          void startExplore(current.reveal.tonic, current.reveal.mode)
        }
        data-testid="reveal-practice-this"
        className="mt-2 w-full rounded-md bg-amber-600/90 px-3 py-1.5 text-sm font-semibold text-white hover:bg-amber-500"
      >
        🎲 Practice this sound
      </button>
      {collectionCount !== null && (
        <p
          className="mt-1 text-right text-xs text-amber-300/60"
          data-testid="reveal-collection-count"
        >
          {collectionCount} in your collection
        </p>
      )}
    </div>
  );
}
