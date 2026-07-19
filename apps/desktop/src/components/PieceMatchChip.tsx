import { usePracticeStore } from "../stores/practiceStore";

/**
 * #214 S1b — the library-match chip: "sounds like {title} from your
 * library". Ambient and honest: it appears only when identification
 * cleared every gate, HOLDS in place (#417 rule 0 — a newer match
 * replaces it, a miss never clears it), and a dismissal quiets that
 * score for the session. Actions ("open the score at your measure",
 * reveal integration) ride #214 S2 — this surface only ever informs.
 */
export default function PieceMatchChip() {
  const match = usePracticeStore((s) => s.pieceMatch);
  const dismiss = usePracticeStore((s) => s.dismissPieceMatch);

  if (!match) {
    return null;
  }

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
