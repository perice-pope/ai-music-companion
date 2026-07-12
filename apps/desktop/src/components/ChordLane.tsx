import { usePracticeStore } from "../stores/practiceStore";
import { colorForPitchClass } from "../lib/rvColors";

/** Confidence → 1–3 dots. The dots are the honesty cue: a hesitant label
 * shows hesitantly. */
function dotCount(confidence: number): number {
  if (confidence >= 0.75) return 3;
  if (confidence >= 0.55) return 2;
  return 1;
}

/**
 * #349 T4a — the jam lane: the last few chords the room played, each one a
 * tap away from the RV bridge (that quality rowed through 12 keys as block
 * cells). Unresolved stretches show honestly as "several notes…" — never a
 * guessed name — and are not tappable (there is nothing to row).
 *
 * Privacy line is part of the surface, per the spec: labels only, nothing
 * recorded or sent anywhere.
 */
export default function ChordLane() {
  const lane = usePracticeStore((s) => s.chordLane);
  const exploreChord = usePracticeStore((s) => s.exploreChord);

  return (
    <div
      data-testid="chord-lane"
      className="flex w-full max-w-2xl flex-col items-center gap-3"
    >
      <div className="flex min-h-[3.5rem] flex-wrap items-center justify-center gap-2">
        {lane.length === 0 ? (
          <span data-testid="lane-empty" className="text-sm text-gray-500">
            🎧 listening to the room — chords appear here as I hear them
          </span>
        ) : (
          lane.map((entry, i) =>
            entry.unresolved ? (
              <span
                key={`u-${i}`}
                data-testid="lane-unresolved"
                className="rounded-full border border-gray-700 px-3 py-1.5 text-sm italic text-gray-400"
              >
                several notes…
              </span>
            ) : (
              <button
                key={`${entry.label}-${i}`}
                type="button"
                data-testid="lane-chord"
                title={`Row ${entry.label} through 12 keys`}
                onClick={() =>
                  entry.quality != null &&
                  entry.rootPc != null &&
                  void exploreChord(entry.rootPc, entry.quality)
                }
                className="flex items-center gap-2 rounded-full border border-gray-600 bg-gray-800/70 px-3 py-1.5 text-sm font-semibold text-gray-100 hover:border-gray-400 hover:bg-gray-700"
              >
                <span
                  aria-hidden="true"
                  className="inline-block h-2.5 w-2.5 rounded-full"
                  style={{
                    backgroundColor:
                      entry.rootPc != null
                        ? colorForPitchClass(entry.rootPc)
                        : undefined,
                  }}
                />
                {entry.label}
                <span
                  data-testid="lane-confidence"
                  aria-label={`confidence ${Math.round(entry.confidence * 100)}%`}
                  className="text-[9px] tracking-widest text-gray-400"
                >
                  {"●".repeat(dotCount(entry.confidence))}
                </span>
              </button>
            ),
          )
        )}
      </div>
      <p
        data-testid="lane-tap-hint"
        className="text-xs text-gray-500"
      >
        Tap any chord to row it through 12 keys.
      </p>
      <p data-testid="lane-privacy" className="text-[10px] text-gray-600">
        Labels only — nothing is recorded, identified, or sent anywhere.
        Fully offline.
      </p>
    </div>
  );
}
