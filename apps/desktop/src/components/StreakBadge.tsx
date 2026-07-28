import { useEffect } from "react";
import { useWarmupStore } from "../stores/warmupStore";

/**
 * #257 S4 — the chain made visible, plus the one-tap way to extend it.
 *
 * Flame + count, lit once today's warmup is done and greyed until then;
 * beside it the "Daily warmup" entry that throws the roulette. The badge
 * renders nothing until `get_streak` has answered — a made-up count is
 * worse than a beat of silence.
 */
export default function StreakBadge({ allowStart }: { allowStart: boolean }) {
  const streak = useWarmupStore((s) => s.streak);
  const phase = useWarmupStore((s) => s.phase);
  const fetchStreak = useWarmupStore((s) => s.fetchStreak);
  const startWarmup = useWarmupStore((s) => s.startWarmup);

  useEffect(() => {
    void fetchStreak();
  }, [fetchStreak]);

  return (
    <div className="flex items-center gap-2">
      {streak && (
        <span
          data-testid="streak-badge"
          title={
            streak.completed_today
              ? "Warmup done today — chain extended"
              : "Days in a row — do today's warmup to keep it"
          }
          className={`flex items-center gap-1 rounded-full border px-2.5 py-1 text-sm font-semibold ${
            streak.completed_today
              ? "border-amber-500/70 bg-amber-500/15 text-amber-300"
              : "border-gray-700 bg-gray-800/60 text-gray-500"
          }`}
        >
          <span aria-hidden="true">🔥</span>
          {streak.count}
        </span>
      )}
      {allowStart && phase === "idle" && (
        <button
          type="button"
          data-testid="daily-warmup-entry"
          onClick={() => void startWarmup().catch(console.error)}
          className="rounded bg-amber-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-amber-500"
        >
          Daily warmup
        </button>
      )}
    </div>
  );
}
