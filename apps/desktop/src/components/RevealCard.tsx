import { useEffect, useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";

/** How long a reveal lingers before fading on its own (ms). */
const REVEAL_LINGER_MS = 12000;

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
            <p className="mt-1 text-sm leading-relaxed text-amber-200/90">{why}</p>
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
  const liveKey = usePracticeStore((s) => s.perception?.key ?? null);

  const current =
    revealQueue.length > 0 ? revealQueue[revealQueue.length - 1] : null;

  // Never let a lingering card contradict the live "I hear" header (#266): once
  // the detected key moves to a different (tonic, mode) than this reveal names,
  // dismiss it. A null key (silence) doesn't contradict — keep showing it.
  useEffect(() => {
    if (!current || !liveKey) return;
    const movedOff =
      liveKey.tonic !== current.reveal.tonic ||
      liveKey.mode.toLowerCase() !== current.reveal.mode;
    if (movedOff) {
      dismissReveal(current.id);
    }
  }, [current, liveKey, dismissReveal]);

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
    </div>
  );
}
