import { usePracticeStore } from "../stores/practiceStore";

/**
 * #285 — the flagship RV gesture in one button: lift what the player just
 * played as a cell and row it through the 12 keys. The backend answers with
 * a calm one-liner when nothing liftable has been played yet.
 */
export default function LiftLickButton() {
  const exploreLastPhrase = usePracticeStore((s) => s.exploreLastPhrase);
  const notice = usePracticeStore((s) => s.exploreNotice);

  return (
    <div className="flex flex-col gap-1">
      <button
        type="button"
        onClick={() => void exploreLastPhrase()}
        data-testid="lift-lick"
        className="rounded-md bg-emerald-700/90 px-3 py-1.5 text-sm font-semibold text-white hover:bg-emerald-600"
      >
        🎲 Work on my last lick
      </button>
      {notice && (
        <p
          className="text-xs italic text-gray-400"
          data-testid="lift-lick-notice"
          role="status"
        >
          {notice}
        </p>
      )}
    </div>
  );
}
