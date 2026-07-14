import React from "react";
import { usePracticeStore } from "../stores/practiceStore";
import type { ScoreLibraryEntry } from "../types/brain";

interface ScoreLibraryProps {
  scores: ScoreLibraryEntry[];
  onSelectScore: (id: string) => Promise<void>;
}

export default function ScoreLibrary({
  scores,
  onSelectScore,
}: ScoreLibraryProps) {
  const { deleteScore, activeScore } = usePracticeStore();
  const [confirmDelete, setConfirmDelete] = React.useState<string | null>(null);

  const handleDelete = async (id: string) => {
    try {
      await deleteScore(id);
      setConfirmDelete(null);
    } catch (err) {
      console.error("Failed to delete score:", err);
    }
  };

  const handleSelect = async (id: string) => {
    try {
      await onSelectScore(id);
    } catch (err) {
      console.error("Failed to select score:", err);
    }
  };

  if (scores.length === 0) {
    return (
      <div className="rounded-lg border border-gray-700 bg-gray-800 p-6 text-center">
        <p className="text-sm text-gray-400">
          Drop your first score to get started.
        </p>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-gray-700 bg-gray-800 p-4">
      <h3 className="font-semibold text-lg mb-4">My Scores</h3>
      <div className="space-y-3 max-h-[600px] overflow-y-auto">
        {scores.map((score) => (
          <div
            key={score.id}
            className={`rounded-lg border p-3 transition cursor-pointer ${
              activeScore?.id === score.id
                ? "border-blue-500 bg-blue-900/20"
                : "border-gray-700 bg-gray-700/50 hover:bg-gray-700"
            }`}
          >
            <button
              onClick={() => handleSelect(score.id)}
              className="text-left flex-1 w-full"
            >
              <p className="font-medium text-sm">{score.title}</p>
              {score.composer && (
                <p className="text-xs text-gray-400">{score.composer}</p>
              )}
              <p className="text-xs text-gray-500 mt-1">
                {score.duration_measures} measures
              </p>
            </button>

            {confirmDelete === score.id ? (
              <div className="mt-2 flex gap-2 text-xs">
                <button
                  onClick={() => handleDelete(score.id)}
                  className="flex-1 rounded bg-red-600 px-2 py-1 hover:bg-red-700 transition"
                >
                  Confirm
                </button>
                <button
                  onClick={() => setConfirmDelete(null)}
                  className="flex-1 rounded bg-gray-600 px-2 py-1 hover:bg-gray-700 transition"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmDelete(score.id)}
                className="mt-2 w-full rounded bg-gray-600 px-2 py-1 text-xs hover:bg-gray-700 transition"
              >
                Delete
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
