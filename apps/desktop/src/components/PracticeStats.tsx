import type { PracticeStatsDto } from "../types/brain";

interface PracticeStatsProps {
  stats: PracticeStatsDto;
}

export default function PracticeStats({ stats }: PracticeStatsProps) {
  const totalHours = (stats.total_time_secs / 3600).toFixed(1);
  const avgMinutes = Math.round(stats.avg_session_length_secs / 60);
  const trendEmoji = {
    up: "📈",
    down: "📉",
    stable: "➡️",
  }[stats.trend];

  return (
    <div className="grid grid-cols-2 gap-4 rounded-lg bg-gray-800 p-6 md:grid-cols-4">
      <div className="text-center">
        <div className="text-3xl font-bold text-blue-400">
          {stats.total_sessions}
        </div>
        <div className="text-xs text-gray-400">Total Sessions</div>
      </div>

      <div className="text-center">
        <div className="text-3xl font-bold text-green-400">
          {totalHours}h
        </div>
        <div className="text-xs text-gray-400">Total Time</div>
      </div>

      <div className="text-center">
        <div className="text-3xl font-bold text-purple-400">
          {stats.sessions_this_week}
        </div>
        <div className="text-xs text-gray-400">This Week</div>
      </div>

      <div className="text-center">
        <div className="flex items-center justify-center gap-2">
          <span className="text-3xl font-bold text-orange-400">
            {avgMinutes}m
          </span>
          <span className="text-xl">{trendEmoji}</span>
        </div>
        <div className="text-xs text-gray-400">Avg Length</div>
      </div>
    </div>
  );
}
