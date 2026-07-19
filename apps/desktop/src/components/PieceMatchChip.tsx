import { useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";

/**
 * #214 S1b/S2 — the library-match chip: "sounds like {title} from your
 * library". Ambient and honest: it appears only when identification
 * cleared every gate, HOLDS in place (#417 rule 0 — a newer match
 * replaces it, a miss never clears it), and a dismissal quiets that
 * score for the session. S2 adds the consequence: one tap opens the
 * matched score (staged through the selector). A score that can't load
 * (deleted mid-session) gets a calm notice and goes quiet — never a
 * door that doesn't open.
 */
export default function PieceMatchChip() {
  const match = usePracticeStore((s) => s.pieceMatch);
  const dismiss = usePracticeStore((s) => s.dismissPieceMatch);
  const openMatchedScore = usePracticeStore((s) => s.openMatchedScore);
  const [notice, setNotice] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);

  if (!match) {
    if (notice) {
      return (
        <div
          data-testid="piece-match-notice"
          className="flex items-center gap-2 rounded-full border border-indigo-800 bg-indigo-950/40 py-1 pl-3 pr-1.5 text-sm text-indigo-200/80"
          role="status"
        >
          <span>{notice}</span>
          <button
            type="button"
            data-testid="piece-match-notice-dismiss"
            onClick={() => setNotice(null)}
            aria-label="Dismiss notice"
            className="inline-flex h-5 w-5 items-center justify-center rounded-full text-indigo-300/70 hover:text-white"
          >
            ×
          </button>
        </div>
      );
    }
    return null;
  }
  // A live match outranks a lingering notice (S2 review note 3 — the
  // surface must never suppress fresh identification).

  const handleOpen = async () => {
    setOpening(true);
    const opened = await openMatchedScore();
    setOpening(false);
    if (!opened) {
      // The store already quieted that id — say why the tap did nothing.
      setNotice("couldn't open that score — it may have left your library");
    }
  };

  return (
    <div
      data-testid="piece-match-chip"
      className="flex items-center gap-2 rounded-full border border-indigo-700 bg-indigo-950/60 py-1 pl-3 pr-1.5 text-sm text-indigo-100"
      role="status"
      aria-live="polite"
    >
      <span data-testid="piece-match-title">
        🎼 sounds like <strong>{match.title}</strong> from your library
      </span>
      <button
        type="button"
        data-testid="piece-match-open"
        onClick={handleOpen}
        disabled={opening}
        className="rounded-full bg-indigo-700/70 px-2 py-0.5 text-xs font-semibold text-white hover:bg-indigo-600 disabled:opacity-50"
      >
        Open score
      </button>
      <button
        type="button"
        data-testid="piece-match-dismiss"
        onClick={dismiss}
        aria-label={`Not ${match.title} — stay quiet about it`}
        className="inline-flex h-5 w-5 items-center justify-center rounded-full text-indigo-300/70 hover:text-white"
      >
        ×
      </button>
    </div>
  );
}
