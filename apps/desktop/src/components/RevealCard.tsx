import { useState } from "react";
import {
  usePracticeStore,
  KEY_ASSERT_CONFIDENCE,
} from "../stores/practiceStore";

/**
 * A key change only DIMS a card when the NEW reading is at least this
 * confident — early-session detection wanders, and a wobble is not a
 * contradiction. Matches the "I hear" header's assert threshold (the point
 * where it drops the "maybe" hedge), so an asserted header can never sit
 * contradicting a lingering card.
 */
const DISMISS_MIN_CONFIDENCE = KEY_ASSERT_CONFIDENCE;

/**
 * A single real-world music "reveal". Slides in from the right and STAYS
 * (#417 rule 0) until explicitly dismissed or replaced by a newer reveal;
 * a confidently-contradicted card dims in place via `stale`.
 * Mirrors the coaching `TipCard` so the two share a calm, consistent voice.
 */
function RevealCardItem({
  id,
  concept,
  connection,
  why,
  stale,
  matchTitle,
  onDismiss,
}: {
  id: string;
  concept: string;
  connection: string;
  why: string;
  /** #417 rule 0: a confidently-contradicted card DIMS, it never vanishes. */
  stale: boolean;
  /** #214 S2: the confirmed library match, when identification has one. */
  matchTitle: string | null;
  onDismiss: (id: string) => void;
}) {
  const [isDismissing, setIsDismissing] = useState(false);

  // #417: no auto-dismiss timer. The card stays until the player dismisses
  // it or a better reveal REPLACES it — "it stays until it has something
  // better to say."

  const handleDismiss = () => {
    setIsDismissing(true);
    setTimeout(() => onDismiss(id), 300);
  };

  return (
    <div
      data-testid={`reveal-${id}`}
      className={`
        transform transition-all duration-300 ease-out
        ${
          isDismissing
            ? "translate-x-full opacity-0"
            : stale
              ? "translate-x-0 opacity-40"
              : "translate-x-0 opacity-100"
        }
      `}
    >
      <div className="rounded-lg border border-amber-700 bg-amber-900/40 p-4 shadow-lg">
        <div className="flex items-start justify-between gap-3">
          <div className="flex-1">
            <p className="text-xs font-semibold uppercase tracking-wider text-amber-300/80">
              In the wild · {concept}
            </p>
            {/* #417-2a → #214 S2: the card is a KEY/MODE catalog, and next
                to live playing a bare title reads as song identification —
                worse than nothing when it's the wrong Beethoven. The framing
                line makes that unmistakable, and it retires ONLY while real
                identification (#214) has a confirmed match to say instead —
                then the card can finally assert, and the catalog reframes
                as company the piece keeps in this key. */}
            {matchTitle !== null ? (
              <>
                <p
                  className="text-[13px] font-semibold text-amber-100"
                  data-testid={`reveal-identified-${id}`}
                >
                  You&apos;re playing — {matchTitle}
                </p>
                <p
                  className="text-[11px] italic text-amber-300/60"
                  data-testid={`reveal-catalog-reframe-${id}`}
                >
                  also lives in this key:
                </p>
              </>
            ) : (
              <p
                className="text-[11px] italic text-amber-300/60"
                data-testid={`reveal-framing-${id}`}
              >
                other music that lives in this sound
              </p>
            )}
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
  // #214 S2: a confirmed library match upgrades the card's voice from
  // catalog to identification. Clears with the match (store rule 0 keeps
  // the MATCH sticky; this only mirrors it).
  const matchTitle = usePracticeStore((s) => s.pieceMatch?.title ?? null);
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

  // #417 rule 0 rewrite of the #266/#277 balance: a lingering card that
  // contradicts the live "I hear" header DIMS instead of vanishing — the
  // staleness is honest, the screen stays calm, and the card still leaves
  // the moment a better reveal replaces it. Only a CONFIDENT contradiction
  // dims; a null or shaky key reading changes nothing.
  const stale = Boolean(
    current &&
    liveTonic !== null &&
    liveMode !== null &&
    liveKeyConfident &&
    (liveTonic !== current.reveal.tonic ||
      liveMode.toLowerCase() !== current.reveal.mode),
  );

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
        stale={stale}
        matchTitle={matchTitle}
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
