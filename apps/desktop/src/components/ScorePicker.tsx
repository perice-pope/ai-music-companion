import { useEffect, useRef, useState } from "react";
import { usePracticeStore } from "../stores/practiceStore";
import { useAudioStore } from "../stores/audioStore";
import ScoreDropZone from "./ScoreDropZone";
import ScoreLibrary from "./ScoreLibrary";

export default function ScorePicker() {
  const {
    activeScore,
    scoreLibrary,
    refreshScoreLibrary,
    clearActiveScore,
    loadScoreFromId,
    startSession,
    returnToSelector,
  } = usePracticeStore();
  // The instrument the user picked on the selector. This is the source of
  // truth here — `practiceStore.instrumentName` is only set once a session is
  // running, so it's always null on this screen. Reading it was the root cause
  // of the dead "Start Practice with This Score" button (#184).
  const selectedInstrument = useAudioStore((s) => s.selectedInstrument);

  const initRef = useRef(false);
  // Surfaced when starting the session fails — previously this only went to
  // console.error, so the button looked like a dead no-op (#184).
  const [startError, setStartError] = useState<string | null>(null);

  // Load library on mount
  useEffect(() => {
    if (!initRef.current) {
      initRef.current = true;
      refreshScoreLibrary().catch(() => {
        // Library may be empty on first run; that's ok
      });
    }
  }, [refreshScoreLibrary]);

  const handleStartWithScore = async () => {
    // The button is disabled in this case, but guard anyway.
    if (!activeScore || !selectedInstrument) return;
    setStartError(null);
    try {
      await startSession(selectedInstrument, 15.0, activeScore.id);
    } catch (err) {
      // Surface the failure in the UI instead of swallowing it (#184).
      setStartError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <main className="flex min-h-screen flex-col bg-gray-900 text-white">
      {/* Header */}
      <div className="border-b border-gray-700 bg-gray-800 px-6 py-4">
        <button
          onClick={returnToSelector}
          className="text-sm text-blue-400 hover:text-blue-300 transition"
        >
          ← Back to instrument selector
        </button>
        <h2 className="mt-2 text-2xl font-bold">Choose a Score</h2>
        <p className="mt-1 text-sm text-gray-400">
          Select an existing score or import a new one.
        </p>
      </div>

      <div className="flex flex-1 gap-6 p-6">
        {/* Left: Drop zone and selection */}
        <div className="flex flex-1 flex-col gap-4">
          <ScoreDropZone />

          {activeScore && (
            <div className="rounded-lg border border-blue-500 bg-blue-900/20 p-4">
              <h3 className="text-lg font-semibold">{activeScore.title}</h3>
              {activeScore.composer && (
                <p className="text-sm text-gray-300">{activeScore.composer}</p>
              )}
              <p className="mt-2 text-xs text-gray-400">
                {activeScore.duration_measures} measures
              </p>
              <div className="mt-4 flex gap-2">
                <button
                  onClick={handleStartWithScore}
                  disabled={!selectedInstrument}
                  title={
                    selectedInstrument ? undefined : "Pick an instrument first"
                  }
                  className={`flex-1 rounded px-4 py-2 font-semibold transition ${
                    selectedInstrument
                      ? "bg-green-600 hover:bg-green-700"
                      : "cursor-not-allowed bg-gray-600 text-gray-400"
                  }`}
                >
                  Start Practice with This Score
                </button>
                <button
                  onClick={clearActiveScore}
                  className="rounded bg-gray-700 px-4 py-2 text-sm hover:bg-gray-600 transition"
                >
                  Clear
                </button>
              </div>

              {/* Tell the user why the button is disabled instead of letting
                  it look like a dead no-op (#184). */}
              {!selectedInstrument && (
                <p className="mt-2 text-xs text-amber-300" role="status">
                  Pick an instrument first —{" "}
                  <button
                    onClick={returnToSelector}
                    className="underline underline-offset-2 hover:text-amber-200"
                  >
                    back to instrument selector
                  </button>
                </p>
              )}

              {/* Surface a start failure rather than swallowing it (#184). */}
              {startError && (
                <p
                  className="mt-2 rounded border border-red-500 bg-red-900/30 px-3 py-2 text-sm text-red-200"
                  role="alert"
                >
                  Couldn't start practice: {startError}
                </p>
              )}
            </div>
          )}
        </div>

        {/* Right: Library list */}
        <div className="w-80 flex-shrink-0">
          <ScoreLibrary scores={scoreLibrary} onSelectScore={loadScoreFromId} />
        </div>
      </div>
    </main>
  );
}
