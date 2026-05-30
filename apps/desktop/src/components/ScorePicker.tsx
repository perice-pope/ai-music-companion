import { useEffect, useRef } from "react";
import { usePracticeStore } from "../stores/practiceStore";
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
    instrumentName,
    returnToSelector,
  } = usePracticeStore();

  const initRef = useRef(false);

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
    if (!activeScore || !instrumentName) return;
    try {
      await startSession(instrumentName, 15.0, activeScore.id);
    } catch (err) {
      console.error("Failed to start session with score:", err);
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
                  className="flex-1 rounded bg-green-600 px-4 py-2 font-semibold hover:bg-green-700 transition"
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
