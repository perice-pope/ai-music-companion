import type { SessionSummaryDto } from "../types/brain";

interface SessionCardProps {
  session: SessionSummaryDto;
  onClick: () => void;
}

export default function SessionCard({ session, onClick }: SessionCardProps) {
  const date = new Date(session.started_at).toLocaleDateString();
  const time = new Date(session.started_at).toLocaleTimeString();
  const durationMins = Math.round(session.duration_secs / 60);

  return (
    <button
      onClick={onClick}
      className="block w-full rounded-lg border border-gray-700 bg-gray-800 p-4 text-left transition-colors hover:border-blue-500 hover:bg-gray-750"
    >
      <div className="flex items-start justify-between">
        <div className="flex-1">
          <h3 className="font-semibold text-white">
            {session.instrument} — {durationMins}m
          </h3>
          <p className="text-xs text-gray-400">
            {date} at {time}
          </p>
          <p className="mt-2 text-sm text-gray-300">
            {session.phrase_count} phrase{session.phrase_count !== 1 ? "s" : ""}
          </p>
        </div>
      </div>
    </button>
  );
}
