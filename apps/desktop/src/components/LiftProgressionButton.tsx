import { usePracticeStore } from "../stores/practiceStore";

/**
 * #349 T3c — "Work on my last progression": lift the chord sequence the
 * app just heard (the session's chord chart) and row it through 12 keys
 * as stacked cells. The backend answers with a calm one-liner when fewer
 * than two distinct chords have been heard yet.
 */
export default function LiftProgressionButton() {
  const exploreProgression = usePracticeStore((s) => s.exploreProgression);

  return (
    <button
      type="button"
      onClick={() => void exploreProgression()}
      data-testid="lift-progression"
      className="rounded-md bg-indigo-700/90 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-600"
    >
      🎲 Work on my last progression
    </button>
  );
}
